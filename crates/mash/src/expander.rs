//! Shell expansion — parameter, arithmetic, tilde, word split, glob, quote removal.
//!
//! Command substitution is stubbed — the executor sub-project wires it up.

use crate::env::Env;

// ── Sentinels ──

/// Quoted text — no splitting, no globbing.
const S_QUOTED: char = '\u{E001}';
/// Hard field boundary from $@ — forces split.
const S_BOUNDARY: char = '\u{E002}';
/// Zero-words from ${x+y} when unset — produces no fields.
#[allow(dead_code)]
const S_ZERO: char = '\u{E003}';
/// Literal unquoted — no splitting, yes globbing.
#[allow(dead_code)]
const S_LITERAL: char = '\u{E004}';

// ── Error type ──

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExpandError {
    #[error("undefined variable: {name}")]
    UndefinedVar { name: String },
    #[error("{message}")]
    UnsetVarError { message: String },
    #[error("arithmetic error: {reason}")]
    Arithmetic { reason: String },
    #[error("bad substitution: {expr}")]
    BadSubstitution { expr: String },
    #[error("command substitution not available: {0}")]
    CommandSubstitution(String),
    #[error("process substitution not available: {0}")]
    ProcessSubstitution(String),
}

// ── Public API ──

/// Expand through full pipeline: tilde -> param -> cmd sub -> arith -> split -> glob -> quote removal.
pub fn expand_word(word: &str, env: &mut Env) -> Result<Vec<String>, ExpandError> {
    let expanded = expand_string_inner(word, env, false)?;
    let ifs = env.get_str("IFS");
    let ifs = if env.is_set("IFS") { ifs.to_string() } else { " \t\n".to_string() };
    let fields = split_fields(&expanded, &ifs);
    let mut result = Vec::new();
    let noglob = env.options().noglob;
    for (field, fully_quoted) in fields {
        if fully_quoted || noglob {
            result.push(strip_sentinels(&field));
        } else {
            let globbed = expand_pathname(&field);
            result.extend(globbed.into_iter().map(|s| strip_sentinels(&s)));
        }
    }
    Ok(result)
}

/// Expand without word splitting or globbing.
pub fn expand_word_nosplit(word: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(word, env, false)?;
    Ok(strip_sentinels(&expanded))
}

/// Like nosplit but preserves glob escaping from quoted regions for case patterns.
pub fn expand_word_for_case_pattern(word: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(word, env, false)?;
    Ok(strip_sentinels_case_pattern(&expanded))
}

/// Heredoc body expansion — quotes are literal, only $var and $(cmd) expanded.
pub fn expand_heredoc_body(body: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(body, env, true)?;
    Ok(strip_sentinels(&expanded))
}

/// Evaluate arithmetic expression.
pub fn eval_arithmetic(_expr: &str, _env: &mut Env) -> Result<i64, ExpandError> {
    // Implemented in Task 3
    todo!("arithmetic evaluation")
}

// ── Core engine ──

fn expand_string_inner(word: &str, env: &mut Env, heredoc_mode: bool) -> Result<String, ExpandError> {
    let mut result = String::new();
    let mut chars = word.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            '\'' if !heredoc_mode => {
                chars.next();
                result.push(S_QUOTED);
                while let Some(&c) = chars.peek() {
                    if c == '\'' { chars.next(); break; }
                    chars.next();
                    result.push(c);
                }
                result.push(S_QUOTED);
            }
            '"' if !heredoc_mode => {
                chars.next();
                result.push(S_QUOTED);
                while let Some(&c) = chars.peek() {
                    match c {
                        '"' => { chars.next(); break; }
                        '\\' => {
                            chars.next();
                            if let Some(&next) = chars.peek() {
                                match next {
                                    '$' | '`' | '"' | '\\' | '\n' => {
                                        chars.next();
                                        result.push(next);
                                    }
                                    _ => {
                                        result.push('\\');
                                    }
                                }
                            }
                        }
                        '$' => {
                            chars.next();
                            expand_dollar(&mut chars, &mut result, env, true)?;
                        }
                        '`' => {
                            chars.next();
                            expand_backtick(&mut chars, &mut result, env)?;
                        }
                        _ => {
                            chars.next();
                            result.push(c);
                        }
                    }
                }
                result.push(S_QUOTED);
            }
            '$' => {
                chars.next();
                expand_dollar(&mut chars, &mut result, env, false)?;
            }
            '`' => {
                chars.next();
                expand_backtick(&mut chars, &mut result, env)?;
            }
            '~' if !heredoc_mode && result.is_empty() => {
                chars.next();
                let expanded = expand_tilde(&mut chars, env);
                result.push(S_QUOTED);
                result.push_str(&expanded);
                result.push(S_QUOTED);
            }
            '\\' if !heredoc_mode => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    if next == '\n' {
                        chars.next(); // line continuation
                    } else {
                        chars.next();
                        result.push(S_QUOTED);
                        result.push(next);
                        result.push(S_QUOTED);
                    }
                }
            }
            '\\' if heredoc_mode => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    match next {
                        '$' | '`' | '\\' => {
                            chars.next();
                            result.push(next);
                        }
                        _ => result.push('\\'),
                    }
                } else {
                    result.push('\\');
                }
            }
            _ => {
                chars.next();
                result.push(ch);
            }
        }
    }

    Ok(result)
}

// ── Tilde expansion ──

fn expand_tilde(chars: &mut std::iter::Peekable<std::str::Chars>, env: &Env) -> String {
    let mut suffix = String::new();
    while let Some(&c) = chars.peek() {
        if c == '/' || c == ':' { break; }
        chars.next();
        suffix.push(c);
    }

    match suffix.as_str() {
        "" => env.get_str("HOME").to_string(),
        "+" => env.get_str("PWD").to_string(),
        "-" => env.get_str("OLDPWD").to_string(),
        _ => format!("~{suffix}"), // ~user — return literally for now
    }
}

// ── Dollar expansion dispatch ──

fn expand_dollar(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
    in_double_quote: bool,
) -> Result<(), ExpandError> {
    match chars.peek() {
        Some(&'{') => {
            chars.next();
            expand_brace_param(chars, result, env, in_double_quote)?;
        }
        Some(&'(') => {
            chars.next();
            if chars.peek() == Some(&'(') {
                chars.next();
                // $((expr)) — arithmetic
                let expr = collect_until_double_paren(chars);
                let val = eval_arithmetic(&expr, env)?;
                result.push_str(&val.to_string());
            } else {
                // $(cmd) — command substitution (stub)
                let cmd = collect_until_close_paren(chars);
                return Err(ExpandError::CommandSubstitution(cmd));
            }
        }
        Some(&c) if c.is_ascii_alphanumeric() || c == '_' || c == '?' || c == '!'
            || c == '$' || c == '#' || c == '@' || c == '*' || c == '-' || c == '0' =>
        {
            // $var or special parameter
            expand_simple_var(chars, result, env, in_double_quote)?;
        }
        _ => {
            result.push('$'); // bare $ at end
        }
    }
    Ok(())
}

// ── Placeholder functions (implemented in subsequent tasks) ──

fn expand_brace_param(
    _chars: &mut std::iter::Peekable<std::str::Chars>,
    _result: &mut String,
    _env: &mut Env,
    _in_double_quote: bool,
) -> Result<(), ExpandError> {
    // Implemented in Task 2
    todo!("parameter expansion")
}

fn expand_simple_var(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
    in_double_quote: bool,
) -> Result<(), ExpandError> {
    // Collect variable name
    let mut name = String::new();
    if let Some(&c) = chars.peek() {
        if "?!$#@*-0".contains(c) {
            chars.next();
            name.push(c);
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    chars.next();
                    name.push(c);
                } else {
                    break;
                }
            }
        }
    }

    if name.is_empty() {
        result.push('$');
        return Ok(());
    }

    // Check nounset
    if env.options().nounset && !env.is_set(&name) && !"?!$#@*-0".contains(name.as_str()) {
        return Err(ExpandError::UndefinedVar { name });
    }

    let val = env.get_str(&name).to_string();

    if name == "@" && in_double_quote {
        // "$@" — each arg as separate field with hard boundaries
        let args: Vec<String> = (1..)
            .map(|i| env.get_str(&i.to_string()).to_string())
            .take_while(|s| !s.is_empty())
            .collect();
        for (i, arg) in args.iter().enumerate() {
            if i > 0 { result.push(S_BOUNDARY); }
            result.push_str(arg);
        }
    } else {
        result.push_str(&val);
    }

    Ok(())
}

fn expand_backtick(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    _result: &mut String,
    _env: &mut Env,
) -> Result<(), ExpandError> {
    let mut cmd = String::new();
    while let Some(&c) = chars.peek() {
        if c == '`' { chars.next(); break; }
        if c == '\\' {
            chars.next();
            if let Some(&next) = chars.peek() {
                chars.next();
                cmd.push(next);
            }
        } else {
            chars.next();
            cmd.push(c);
        }
    }
    Err(ExpandError::CommandSubstitution(cmd))
}

fn collect_until_double_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut expr = String::new();
    let mut depth = 1;
    while let Some(c) = chars.next() {
        if c == ')' {
            if chars.peek() == Some(&')') {
                depth -= 1;
                if depth == 0 { chars.next(); return expr; }
                expr.push(')');
                chars.next();
                expr.push(')');
            } else {
                expr.push(c);
            }
        } else if c == '(' && chars.peek() == Some(&'(') {
            depth += 1;
            expr.push('(');
            chars.next();
            expr.push('(');
        } else {
            expr.push(c);
        }
    }
    expr
}

fn collect_until_close_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut cmd = String::new();
    let mut depth = 1;
    while let Some(c) = chars.next() {
        if c == '(' { depth += 1; }
        if c == ')' {
            depth -= 1;
            if depth == 0 { return cmd; }
        }
        cmd.push(c);
    }
    cmd
}

// ── Word splitting ──

fn split_fields(s: &str, _ifs: &str) -> Vec<(String, bool)> {
    // Implemented in Task 4
    vec![(s.to_string(), false)]
}

// ── Pathname expansion ──

fn expand_pathname(field: &str) -> Vec<String> {
    // Implemented in Task 5
    vec![field.to_string()]
}

// ── Quote removal ──

fn strip_sentinels(s: &str) -> String {
    s.chars()
        .filter(|&c| !matches!(c, '\u{E001}' | '\u{E002}' | '\u{E003}' | '\u{E004}'))
        .collect()
}

fn strip_sentinels_case_pattern(s: &str) -> String {
    let mut result = String::new();
    let mut in_quoted = false;
    for c in s.chars() {
        match c {
            S_QUOTED => in_quoted = !in_quoted,
            S_BOUNDARY | S_ZERO | S_LITERAL => {}
            '*' | '?' | '[' | ']' if in_quoted => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}
