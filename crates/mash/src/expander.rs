//! Shell expansion — parameter, arithmetic, tilde, word split, glob, quote removal.
//!
//! Command substitution is stubbed — the executor sub-project wires it up.

use crate::env::{Env, VarValue, Variable};

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
    let expanded = expand_string_inner(word, env, false, false)?;
    let ifs = env.get_str("IFS");
    let ifs = if env.is_set("IFS") {
        ifs.to_string()
    } else {
        " \t\n".to_string()
    };
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
    let expanded = expand_string_inner(word, env, false, false)?;
    Ok(strip_sentinels(&expanded))
}

/// Expand an assignment value without word splitting or globbing.
pub fn expand_assignment_word_nosplit(word: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(word, env, false, true)?;
    Ok(strip_sentinels(&expanded))
}

/// Like nosplit but preserves glob escaping from quoted regions for case patterns.
pub fn expand_word_for_case_pattern(word: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(word, env, false, false)?;
    Ok(strip_sentinels_case_pattern(&expanded))
}

/// Heredoc body expansion — quotes are literal, only $var and $(cmd) expanded.
pub fn expand_heredoc_body(body: &str, env: &mut Env) -> Result<String, ExpandError> {
    let expanded = expand_string_inner(body, env, true, false)?;
    Ok(strip_sentinels(&expanded))
}

/// Evaluate arithmetic expression.
pub fn eval_arithmetic(expr: &str, env: &mut Env) -> Result<i64, ExpandError> {
    let trace = std::env::var_os("MASH_ARITH_TRACE").is_some();
    let tokens = match tokenize_arith(expr) {
        Ok(tokens) => tokens,
        Err(err) => {
            if trace {
                eprintln!("ARITH_TRACE tokenize expr={expr:?} err={err}");
            }
            return Err(err);
        }
    };
    let mut parser = ArithParser::new(&tokens, env);
    let result = match parser.parse_expr(0) {
        Ok(result) => result,
        Err(err) => {
            if trace {
                eprintln!("ARITH_TRACE parse expr={expr:?} err={err}");
            }
            return Err(err);
        }
    };
    Ok(result)
}

// ── Core engine ──

fn expand_string_inner(
    word: &str,
    env: &mut Env,
    heredoc_mode: bool,
    assignment_mode: bool,
) -> Result<String, ExpandError> {
    let mut result = String::new();
    let mut chars = word.chars().peekable();
    let mut tilde_can_expand = true;

    while let Some(&ch) = chars.peek() {
        match ch {
            '\'' if !heredoc_mode => {
                chars.next();
                result.push(S_QUOTED);
                while let Some(&c) = chars.peek() {
                    if c == '\'' {
                        chars.next();
                        break;
                    }
                    chars.next();
                    result.push(c);
                }
                result.push(S_QUOTED);
                tilde_can_expand = false;
            }
            '"' if !heredoc_mode => {
                chars.next();
                result.push(S_QUOTED);
                while let Some(&c) = chars.peek() {
                    match c {
                        '"' => {
                            chars.next();
                            break;
                        }
                        '\\' => {
                            chars.next();
                            if let Some(&next) = chars.peek() {
                                match next {
                                    '\n' => {
                                        chars.next();
                                    }
                                    '$' | '`' | '"' | '\\' => {
                                        chars.next();
                                        result.push(next);
                                    }
                                    '\r' => {
                                        chars.next();
                                        if chars.peek() == Some(&'\n') {
                                            chars.next();
                                        }
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
                            tilde_can_expand = false;
                        }
                        '`' => {
                            chars.next();
                            expand_backtick(&mut chars, &mut result, env)?;
                            tilde_can_expand = false;
                        }
                        _ => {
                            chars.next();
                            result.push(c);
                        }
                    }
                }
                result.push(S_QUOTED);
                tilde_can_expand = false;
            }
            '$' => {
                chars.next();
                expand_dollar(&mut chars, &mut result, env, false)?;
                tilde_can_expand = false;
            }
            '`' => {
                chars.next();
                expand_backtick(&mut chars, &mut result, env)?;
                tilde_can_expand = false;
            }
            '~' if !heredoc_mode && tilde_can_expand => {
                chars.next();
                if matches!(chars.peek().copied(), Some('\'' | '"' | '\\' | '$' | '`')) {
                    result.push('~');
                } else {
                    let expanded = expand_tilde(&mut chars, env, assignment_mode);
                    result.push(S_QUOTED);
                    result.push_str(&expanded);
                    result.push(S_QUOTED);
                }
                tilde_can_expand = false;
            }
            '\\' if !heredoc_mode => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    if next == '\n' {
                        chars.next(); // line continuation
                    } else if next == '\r' {
                        chars.next();
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    } else {
                        chars.next();
                        result.push(S_QUOTED);
                        result.push(next);
                        result.push(S_QUOTED);
                        tilde_can_expand = false;
                    }
                }
            }
            '\\' if heredoc_mode => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    match next {
                        '\n' => {
                            chars.next();
                        }
                        '\r' => {
                            chars.next();
                            if chars.peek() == Some(&'\n') {
                                chars.next();
                            }
                        }
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
                tilde_can_expand = assignment_mode && ch == ':';
            }
        }
    }

    Ok(result)
}

// ── Tilde expansion ──

fn expand_tilde(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    env: &Env,
    assignment_mode: bool,
) -> String {
    let mut suffix = String::new();
    while let Some(&c) = chars.peek() {
        if c == '/' || (assignment_mode && c == ':') {
            break;
        }
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
                // Expand any nested constructs (variables, command substitutions)
                // within the arithmetic expression before evaluating
                let expanded_expr = expand_string_inner(&expr, env, false, false)?;
                let stripped = strip_sentinels(&expanded_expr);
                let val = eval_arithmetic(&stripped, env)?;
                result.push_str(&val.to_string());
            } else {
                // $(cmd) — command substitution
                let cmd = collect_until_close_paren(chars);
                match crate::executor::capture_command(&cmd, env) {
                    Ok(output) => result.push_str(&output),
                    Err(e) => return Err(e),
                }
            }
        }
        Some(&c)
            if c.is_ascii_alphanumeric()
                || c == '_'
                || c == '?'
                || c == '!'
                || c == '$'
                || c == '#'
                || c == '@'
                || c == '*'
                || c == '-'
                || c == '0' =>
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
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
    in_double_quote: bool,
) -> Result<(), ExpandError> {
    // Collect everything between ${ and } with balanced brace tracking.
    let expr = collect_brace_expr(chars);

    if expr.is_empty() {
        return Err(ExpandError::BadSubstitution {
            expr: String::new(),
        });
    }

    // ${!VAR} — indirect expansion
    if let Some(name) = expr.strip_prefix('!') {
        if name.ends_with("[@]") || name.ends_with("[*]") {
            // ${!arr[@]} — array keys
            let arr_name = &name[..name.len() - 3];
            if let Some(var) = env.get(arr_name) {
                match &var.value {
                    VarValue::Array(arr) => {
                        let keys: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| v.as_ref().map(|_| i.to_string()))
                            .collect();
                        result.push_str(&keys.join(" "));
                    }
                    VarValue::AssocArray(map) => {
                        let mut keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
                        keys.sort();
                        result.push_str(&keys.join(" "));
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        let indirect_name = env.get_str(name).to_string();
        if !indirect_name.is_empty() {
            result.push_str(&env.get_str(&indirect_name).to_string());
        }
        return Ok(());
    }

    // ${#VAR} — string length or array length
    if let Some(name) = expr.strip_prefix('#') {
        if name.is_empty() {
            result.push_str(&env.get_str("#").to_string());
            return Ok(());
        }
        if name.ends_with("[@]") || name.ends_with("[*]") {
            let arr_name = &name[..name.len() - 3];
            if let Some(var) = env.get(arr_name) {
                match &var.value {
                    VarValue::Array(arr) => {
                        result.push_str(&arr.iter().filter(|v| v.is_some()).count().to_string());
                    }
                    VarValue::AssocArray(map) => {
                        result.push_str(&map.len().to_string());
                    }
                    _ => {
                        result.push('0');
                    }
                }
            } else {
                result.push('0');
            }
            return Ok(());
        }
        if env.options().nounset && !env.is_set(name) && !"?!$#@*-0".contains(name) {
            return Err(ExpandError::UndefinedVar {
                name: name.to_string(),
            });
        }
        let val = env.get_str(name);
        result.push_str(&val.chars().count().to_string());
        return Ok(());
    }

    // Parse variable name from the expression
    let (name, rest) = parse_param_name(&expr);

    if rest.is_empty() {
        // Simple ${VAR}
        if name == "-" {
            result.push_str(&env.options().flags_string());
        } else if name == "$" {
            result.push_str(&env.get_str("$").to_string());
        } else {
            if env.options().nounset && !env.is_set(&name) && !"?!$#@*-0".contains(name.as_str()) {
                return Err(ExpandError::UndefinedVar { name });
            }
            result.push_str(&env.get_str(&name).to_string());
        }
        return Ok(());
    }

    // Array access: ${arr[@]} or ${arr[*]} or ${arr[n]}
    if rest.starts_with('[') {
        if let Some(idx_end) = rest.find(']') {
            let idx = &rest[1..idx_end];
            let after_bracket = &rest[idx_end + 1..];

            if idx == "@" || idx == "*" {
                if let Some(var) = env.get(&name) {
                    let joined = match &var.value {
                        VarValue::Array(arr) => arr
                            .iter()
                            .filter_map(|v| v.as_deref())
                            .collect::<Vec<_>>()
                            .join(" "),
                        VarValue::AssocArray(map) => {
                            let mut vals: Vec<&str> = map.values().map(|v| v.as_str()).collect();
                            vals.sort();
                            vals.join(" ")
                        }
                        _ => String::new(),
                    };
                    if !joined.is_empty() {
                        if after_bracket.is_empty() {
                            result.push_str(&joined);
                        } else {
                            result.push_str(&apply_string_ops(&joined, after_bracket)?);
                        }
                    }
                }
                return Ok(());
            }

            // ${arr[n]}
            if let Some(var) = env.get(&name) {
                let val = match &var.value {
                    VarValue::Array(arr) => {
                        if let Ok(n) = idx.parse::<usize>() {
                            arr.get(n).and_then(|v| v.as_deref()).unwrap_or("")
                        } else {
                            ""
                        }
                    }
                    VarValue::AssocArray(map) => map.get(idx).map(|s| s.as_str()).unwrap_or(""),
                    _ => "",
                };
                if after_bracket.is_empty() {
                    result.push_str(val);
                } else {
                    result.push_str(&apply_string_ops(val, after_bracket)?);
                }
            }
            return Ok(());
        }
    }

    let val = if name == "-" {
        env.options().flags_string()
    } else {
        env.get_str(&name).to_string()
    };
    let val_is_set = name == "-" || env.is_set(&name);
    let val_is_nonempty = !val.is_empty();

    // ${VAR:-default} / ${VAR-default}
    if let Some(default) = try_strip_op(&rest, ":-") {
        if val_is_nonempty {
            result.push_str(&val);
        } else {
            result.push_str(&expand_string_inner(&default, env, false, in_double_quote)?);
        }
        return Ok(());
    }
    if let Some(default) = try_strip_op(&rest, "-") {
        if val_is_set {
            result.push_str(&val);
        } else {
            result.push_str(&expand_string_inner(&default, env, false, in_double_quote)?);
        }
        return Ok(());
    }

    // ${VAR:=assign} / ${VAR=assign}
    if let Some(default) = try_strip_op(&rest, ":=") {
        if val_is_nonempty {
            result.push_str(&val);
        } else {
            let expanded = expand_string_inner(&default, env, false, in_double_quote)?;
            let stored = strip_sentinels(&expanded);
            let _ = env.set(&name, Variable::string(&stored));
            result.push_str(&expanded);
        }
        return Ok(());
    }
    if let Some(default) = try_strip_op(&rest, "=") {
        if val_is_set {
            result.push_str(&val);
        } else {
            let expanded = expand_string_inner(&default, env, false, in_double_quote)?;
            let stored = strip_sentinels(&expanded);
            let _ = env.set(&name, Variable::string(&stored));
            result.push_str(&expanded);
        }
        return Ok(());
    }

    // ${VAR:+alt} / ${VAR+alt}
    if let Some(alt) = try_strip_op(&rest, ":+") {
        if val_is_nonempty {
            result.push_str(&expand_string_inner(&alt, env, false, in_double_quote)?);
        } else if !in_double_quote {
            result.push(S_ZERO);
        }
        return Ok(());
    }
    if let Some(alt) = try_strip_op(&rest, "+") {
        if val_is_set {
            result.push_str(&expand_string_inner(&alt, env, false, in_double_quote)?);
        } else if !in_double_quote {
            result.push(S_ZERO);
        }
        return Ok(());
    }

    // ${VAR:?error} / ${VAR?error}
    if let Some(msg) = try_strip_op(&rest, ":?") {
        if val_is_nonempty {
            result.push_str(&val);
        } else {
            let expanded_msg =
                strip_sentinels(&expand_string_inner(&msg, env, false, in_double_quote)?);
            let message = if expanded_msg.is_empty() {
                format!("{}: parameter null or not set", name)
            } else {
                format!("{}: {}", name, expanded_msg)
            };
            return Err(ExpandError::UnsetVarError { message });
        }
        return Ok(());
    }
    if let Some(msg) = try_strip_op(&rest, "?") {
        if val_is_set {
            result.push_str(&val);
        } else {
            let expanded_msg =
                strip_sentinels(&expand_string_inner(&msg, env, false, in_double_quote)?);
            let message = if expanded_msg.is_empty() {
                format!("{}: parameter not set", name)
            } else {
                format!("{}: {}", name, expanded_msg)
            };
            return Err(ExpandError::UnsetVarError { message });
        }
        return Ok(());
    }

    // ${VAR:offset} / ${VAR:offset:length} — substring expansion
    if let Some(substr_expr) = rest.strip_prefix(':') {
        if !substr_expr.is_empty()
            && !substr_expr.starts_with('-')
            && !substr_expr.starts_with('=')
            && !substr_expr.starts_with('+')
            && !substr_expr.starts_with('?')
        {
            let (offset_str, length_str) = match substr_expr.find(':') {
                Some(i) => (&substr_expr[..i], Some(&substr_expr[i + 1..])),
                None => (substr_expr, None),
            };

            let offset: i64 = offset_str
                .trim()
                .parse()
                .map_err(|_| ExpandError::Arithmetic {
                    reason: format!("invalid offset: {}", offset_str),
                })?;
            let chars_vec: Vec<char> = val.chars().collect();
            let char_len = chars_vec.len() as i64;

            let start = if offset < 0 {
                (char_len + offset).max(0) as usize
            } else {
                (offset as usize).min(chars_vec.len())
            };

            let end = if let Some(l) = length_str {
                let length: i64 = l.trim().parse().map_err(|_| ExpandError::Arithmetic {
                    reason: format!("invalid length: {}", l),
                })?;
                if length < 0 {
                    let end_pos = (char_len + length) as usize;
                    if end_pos <= start {
                        start
                    } else {
                        end_pos
                    }
                } else {
                    (start + length as usize).min(chars_vec.len())
                }
            } else {
                chars_vec.len()
            };

            let substr: String = chars_vec[start..end].iter().collect();
            result.push_str(&substr);
            return Ok(());
        }
    }

    // ${VAR%%pattern} — largest suffix strip
    if let Some(pat) = try_strip_op(&rest, "%%") {
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&strip_largest_suffix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR%pattern} — smallest suffix strip
    if let Some(pat) = try_strip_op(&rest, "%") {
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&strip_smallest_suffix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR##pattern} — largest prefix strip
    if let Some(pat) = try_strip_op(&rest, "##") {
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&strip_largest_prefix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR#pattern} — smallest prefix strip
    if let Some(pat) = try_strip_op(&rest, "#") {
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&strip_smallest_prefix(&val, &expanded_pat));
        return Ok(());
    }

    // ${VAR//pattern/replacement} — global substitution
    if let Some(pat_rep) = try_strip_op(&rest, "//") {
        let (pat, rep) = split_subst(&pat_rep);
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&shell_replace_all(&val, &expanded_pat, &rep));
        return Ok(());
    }
    // ${VAR/pattern/replacement} — first substitution
    if let Some(pat_rep) = try_strip_op(&rest, "/") {
        let (pat, rep) = split_subst(&pat_rep);
        let expanded_pat = expand_string_inner(&pat, env, false, false)?;
        result.push_str(&shell_replace_first(&val, &expanded_pat, &rep));
        return Ok(());
    }

    // ${VAR^^[pattern]} — uppercase all
    if let Some(pat) = rest.strip_prefix("^^") {
        if pat.is_empty() {
            result.push_str(&val.to_uppercase());
        } else {
            let expanded_pat = expand_string_inner(pat, env, false, false)?;
            let s: String = val
                .chars()
                .map(|c| {
                    if shell_pattern_match(&c.to_string(), &expanded_pat) {
                        c.to_uppercase().next().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect();
            result.push_str(&s);
        }
        return Ok(());
    }
    // ${VAR^[pattern]} — uppercase first
    if let Some(pat) = rest.strip_prefix('^') {
        let mut val_chars = val.chars();
        match val_chars.next() {
            Some(c) => {
                let matches = if pat.is_empty() {
                    true
                } else {
                    let expanded_pat = expand_string_inner(pat, env, false, false)?;
                    shell_pattern_match(&c.to_string(), &expanded_pat)
                };
                let upper = if matches {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
                result.push(upper);
                for ch in val_chars {
                    result.push(ch);
                }
            }
            None => {}
        }
        return Ok(());
    }
    // ${VAR,,[pattern]} — lowercase all
    if let Some(pat) = rest.strip_prefix(",,") {
        if pat.is_empty() {
            result.push_str(&val.to_lowercase());
        } else {
            let expanded_pat = expand_string_inner(pat, env, false, false)?;
            let s: String = val
                .chars()
                .map(|c| {
                    if shell_pattern_match(&c.to_string(), &expanded_pat) {
                        c.to_lowercase().next().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect();
            result.push_str(&s);
        }
        return Ok(());
    }
    // ${VAR,[pattern]} — lowercase first
    if let Some(pat) = rest.strip_prefix(',') {
        let mut val_chars = val.chars();
        match val_chars.next() {
            Some(c) => {
                let matches = if pat.is_empty() {
                    true
                } else {
                    let expanded_pat = expand_string_inner(pat, env, false, false)?;
                    shell_pattern_match(&c.to_string(), &expanded_pat)
                };
                let lower = if matches {
                    c.to_lowercase().next().unwrap_or(c)
                } else {
                    c
                };
                result.push(lower);
                for ch in val_chars {
                    result.push(ch);
                }
            }
            None => {}
        }
        return Ok(());
    }

    Err(ExpandError::BadSubstitution { expr })
}

// ── Brace expression collector ──

/// Collect the content between `${` and its matching `}`, handling nested `${...}`.
fn collect_brace_expr(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut expr = String::new();
    let mut depth = 1;
    while let Some(&c) = chars.peek() {
        if c == '}' {
            depth -= 1;
            if depth == 0 {
                chars.next();
                return expr;
            }
            chars.next();
            expr.push(c);
        } else if c == '$' {
            chars.next();
            expr.push(c);
            if let Some(&'{') = chars.peek() {
                chars.next();
                expr.push('{');
                depth += 1;
            }
        } else if c == '\\' {
            chars.next();
            expr.push(c);
            if let Some(&next) = chars.peek() {
                chars.next();
                expr.push(next);
            }
        } else {
            chars.next();
            expr.push(c);
        }
    }
    expr
}

// ── Parameter name parser ──

/// Parse the variable name from the start of a parameter expression.
/// Returns (name, rest_of_expression).
fn parse_param_name(expr: &str) -> (String, String) {
    let bytes = expr.as_bytes();
    if let Some(&first) = bytes.first() {
        // Special parameters: single-char
        if matches!(first, b'$' | b'?' | b'!' | b'-' | b'@' | b'*') {
            return (expr[..1].to_string(), expr[1..].to_string());
        }
        // Single digit positional (only if not followed by another digit)
        if first.is_ascii_digit()
            && (bytes.len() == 1 || !bytes.get(1).is_some_and(|b| b.is_ascii_digit()))
        {
            return (expr[..1].to_string(), expr[1..].to_string());
        }
        // # is special: ${#} = number of params
        if first == b'#'
            && (bytes.len() == 1 || (!bytes[1].is_ascii_alphanumeric() && bytes[1] != b'_'))
        {
            return (expr[..1].to_string(), expr[1..].to_string());
        }
    }
    // Regular variable name: [a-zA-Z_0-9]*
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' {
            i += 1;
        } else {
            break;
        }
    }
    (expr[..i].to_string(), expr[i..].to_string())
}

/// Try to strip a parameter expansion operator from the rest string.
fn try_strip_op(rest: &str, op: &str) -> Option<String> {
    rest.strip_prefix(op).map(|s| s.to_string())
}

/// Split a substitution pattern/replacement: "pat/rep" -> ("pat", "rep").
fn split_subst(s: &str) -> (String, String) {
    match s.find('/') {
        Some(i) => (s[..i].to_string(), s[i + 1..].to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// Apply string operations (%, #, etc.) on a value — used for array element post-ops.
fn apply_string_ops(val: &str, ops: &str) -> Result<String, ExpandError> {
    if let Some(pat) = ops.strip_prefix("%%") {
        return Ok(strip_largest_suffix(val, pat));
    }
    if let Some(pat) = ops.strip_prefix('%') {
        return Ok(strip_smallest_suffix(val, pat));
    }
    if let Some(pat) = ops.strip_prefix("##") {
        return Ok(strip_largest_prefix(val, pat));
    }
    if let Some(pat) = ops.strip_prefix('#') {
        return Ok(strip_smallest_prefix(val, pat));
    }
    Ok(val.to_string())
}

// ── Pattern matching ──

/// Shell glob-style pattern matching against a string (not filesystem).
pub fn shell_pattern_match(s: &str, pattern: &str) -> bool {
    pattern_match_impl(s.as_bytes(), pattern.as_bytes())
}

/// Check if the byte slice at `pos` starts with a 3-byte UTF-8 sentinel.
#[inline]
fn sentinel_at(b: &[u8], pos: usize) -> Option<char> {
    if pos + 2 < b.len() && b[pos] == 0xEE && b[pos + 1] == 0x80 {
        match b[pos + 2] {
            0x81 => Some('\u{E001}'),
            0x82 => Some('\u{E002}'),
            0x83 => Some('\u{E003}'),
            0x84 => Some('\u{E004}'),
            _ => None,
        }
    } else {
        None
    }
}

const SENTINEL_LEN: usize = 3;

fn pattern_match_impl(s: &[u8], p: &[u8]) -> bool {
    let mut si = 0;
    let mut pi = 0;
    let mut star_pi = usize::MAX;
    let mut star_si = 0;
    let mut literal_mode = false;

    while si < s.len() {
        if pi < p.len() {
            if let Some(sent) = sentinel_at(p, pi) {
                if sent == '\u{E001}' {
                    literal_mode = !literal_mode;
                }
                pi += SENTINEL_LEN;
                continue;
            }

            match p[pi] {
                b'*' if !literal_mode => {
                    star_pi = pi;
                    star_si = si;
                    pi += 1;
                    continue;
                }
                b'?' if !literal_mode => {
                    si += 1;
                    pi += 1;
                    continue;
                }
                b'[' if !literal_mode => {
                    if let Some((matched, new_pi)) = match_bracket_class(s[si], &p[pi..]) {
                        if matched {
                            si += 1;
                            pi += new_pi;
                            continue;
                        }
                    }
                }
                b'\\' if pi + 1 < p.len() && !literal_mode => {
                    if s[si] == p[pi + 1] {
                        si += 1;
                        pi += 2;
                        continue;
                    }
                }
                c => {
                    if s[si] == c {
                        si += 1;
                        pi += 1;
                        continue;
                    }
                }
            }
        }

        if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_si += 1;
            si = star_si;
            literal_mode = false;
            continue;
        }

        return false;
    }

    while pi < p.len() {
        if sentinel_at(p, pi).is_some() {
            pi += SENTINEL_LEN;
        } else if p[pi] == b'*' && !literal_mode {
            pi += 1;
        } else {
            break;
        }
    }

    pi == p.len()
}

/// Match a bracket expression `[...]` or `[!...]` against a character.
fn match_bracket_class(ch: u8, pattern: &[u8]) -> Option<(bool, usize)> {
    if pattern.is_empty() || pattern[0] != b'[' {
        return None;
    }
    let mut i = 1;
    let negate = i < pattern.len() && (pattern[i] == b'!' || pattern[i] == b'^');
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut consumed_any = false;

    if i < pattern.len() && pattern[i] == b']' {
        matched |= ch == b']';
        i += 1;
        consumed_any = true;
    }
    if i < pattern.len() && pattern[i] == b'-' {
        matched |= ch == b'-';
        i += 1;
        consumed_any = true;
    }

    while i < pattern.len() {
        if pattern[i] == b']' && consumed_any {
            return Some((matched ^ negate, i + 1));
        }
        if pattern[i] == b'\\' && i + 1 < pattern.len() {
            matched |= ch == pattern[i + 1];
            i += 2;
            consumed_any = true;
            continue;
        }
        if let Some((class_matched, consumed)) = match_posix_bracket_item(ch, &pattern[i..]) {
            matched |= class_matched;
            i += consumed;
            consumed_any = true;
            continue;
        }
        if i + 2 < pattern.len() && pattern[i + 1] == b'-' && pattern[i + 2] != b']' {
            let lo = pattern[i];
            let hi = pattern[i + 2];
            if ch >= lo && ch <= hi {
                matched = true;
            }
            i += 3;
            consumed_any = true;
        } else {
            if pattern[i] == ch {
                matched = true;
            }
            i += 1;
            consumed_any = true;
        }
    }
    None // unclosed bracket
}

fn match_posix_bracket_item(ch: u8, pattern: &[u8]) -> Option<(bool, usize)> {
    if pattern.len() < 4 || pattern[0] != b'[' {
        return None;
    }

    match pattern[1] {
        b':' => {
            let end = pattern[2..].windows(2).position(|window| window == b":]")?;
            let class_name = std::str::from_utf8(&pattern[2..2 + end]).ok()?;
            Some((matches_posix_class(ch, class_name), 2 + end + 2))
        }
        b'.' | b'=' => {
            if pattern.len() >= 5 && pattern[3] == pattern[1] && pattern[4] == b']' {
                Some((ch == pattern[2], 5))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn matches_posix_class(ch: u8, class_name: &str) -> bool {
    match class_name {
        "alnum" => ch.is_ascii_alphanumeric(),
        "alpha" => ch.is_ascii_alphabetic(),
        "blank" => matches!(ch, b' ' | b'\t'),
        "cntrl" => ch.is_ascii_control(),
        "digit" => ch.is_ascii_digit(),
        "graph" => ch.is_ascii_graphic(),
        "lower" => ch.is_ascii_lowercase(),
        "print" => !ch.is_ascii_control(),
        "punct" => ch.is_ascii_punctuation(),
        "space" => matches!(ch, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'),
        "upper" => ch.is_ascii_uppercase(),
        "xdigit" => ch.is_ascii_hexdigit(),
        _ => false,
    }
}

// ── Strip / replace helpers ──

fn strip_smallest_suffix(val: &str, pattern: &str) -> String {
    for i in (0..=val.len()).rev() {
        if val.is_char_boundary(i) && shell_pattern_match(&val[i..], pattern) {
            return val[..i].to_string();
        }
    }
    val.to_string()
}

fn strip_largest_suffix(val: &str, pattern: &str) -> String {
    for i in 0..=val.len() {
        if val.is_char_boundary(i) && shell_pattern_match(&val[i..], pattern) {
            return val[..i].to_string();
        }
    }
    val.to_string()
}

fn strip_smallest_prefix(val: &str, pattern: &str) -> String {
    for i in 0..=val.len() {
        if val.is_char_boundary(i) && shell_pattern_match(&val[..i], pattern) {
            return val[i..].to_string();
        }
    }
    val.to_string()
}

fn strip_largest_prefix(val: &str, pattern: &str) -> String {
    for i in (0..=val.len()).rev() {
        if val.is_char_boundary(i) && shell_pattern_match(&val[..i], pattern) {
            return val[i..].to_string();
        }
    }
    val.to_string()
}

fn shell_replace_first(val: &str, pattern: &str, replacement: &str) -> String {
    // Try each starting position, find shortest match
    for start in 0..val.len() {
        if !val.is_char_boundary(start) {
            continue;
        }
        for end in start + 1..=val.len() {
            if !val.is_char_boundary(end) {
                continue;
            }
            if shell_pattern_match(&val[start..end], pattern) {
                return format!("{}{}{}", &val[..start], replacement, &val[end..]);
            }
        }
        // Also try zero-length match if pattern matches empty
        if shell_pattern_match("", pattern) {
            return format!("{}{}{}", &val[..start], replacement, &val[start..]);
        }
    }
    val.to_string()
}

fn shell_replace_all(val: &str, pattern: &str, replacement: &str) -> String {
    let bytes = val.as_bytes();
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let mut matched = false;
        // Try to find a match starting at i (prefer longest)
        for end in (i + 1..=bytes.len()).rev() {
            if val.is_char_boundary(end) && shell_pattern_match(&val[i..end], pattern) {
                result.push_str(replacement);
                i = end;
                matched = true;
                break;
            }
        }
        if !matched {
            let ch_len = utf8_char_len(bytes[i]);
            result.push_str(&val[i..i + ch_len]);
            i += ch_len;
        }
    }
    result
}

fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

fn expand_simple_var(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
    in_double_quote: bool,
) -> Result<(), ExpandError> {
    fn positional_args(env: &Env) -> Vec<String> {
        let count = env.get_str("#").parse::<usize>().unwrap_or(0);
        (1..=count)
            .map(|i| env.get_str(&i.to_string()).to_string())
            .collect()
    }

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
        // "$@" — each arg as separate field with hard boundaries.
        let args = positional_args(env);
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                result.push(S_BOUNDARY);
            }
            result.push_str(arg);
        }
    } else if name == "*" {
        let args = positional_args(env);
        let ifs = if env.is_set("IFS") {
            env.get_str("IFS")
        } else {
            " \t\n"
        };
        let sep = if ifs.is_empty() {
            String::new()
        } else {
            ifs.chars().next().unwrap().to_string()
        };

        if in_double_quote {
            result.push_str(&args.join(&sep));
        } else if ifs.is_empty() {
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    result.push(S_BOUNDARY);
                }
                result.push_str(arg);
            }
        } else {
            result.push_str(&args.join(&sep));
        }
    } else {
        result.push_str(&val);
    }

    Ok(())
}

fn expand_backtick(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
) -> Result<(), ExpandError> {
    let mut cmd = String::new();
    while let Some(&c) = chars.peek() {
        if c == '`' {
            chars.next();
            break;
        }
        if c == '\\' {
            chars.next();
            if let Some(&next) = chars.peek() {
                match next {
                    '$' | '`' | '\\' => {
                        chars.next();
                        cmd.push(next);
                    }
                    '\n' => {
                        chars.next();
                    }
                    '\r' => {
                        chars.next();
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    }
                    _ => cmd.push('\\'),
                }
            } else {
                cmd.push('\\');
            }
        } else {
            chars.next();
            cmd.push(c);
        }
    }
    match crate::executor::capture_command(&cmd, env) {
        Ok(output) => result.push_str(&output),
        Err(e) => return Err(e),
    }
    Ok(())
}

fn collect_until_double_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut expr = String::new();
    let mut depth = 1; // Arithmetic nesting depth
    let mut cmd_depth = 0; // Command substitution nesting depth

    while let Some(c) = chars.next() {
        // If we're inside a command substitution, just collect until we exit it
        if cmd_depth > 0 {
            if c == '(' {
                cmd_depth += 1;
            } else if c == ')' {
                cmd_depth -= 1;
            }
            expr.push(c);
            continue;
        }

        // Not inside command substitution - check for special tokens
        if c == '$' && chars.peek() == Some(&'(') {
            // Start of command substitution $(...)
            cmd_depth = 1;
            expr.push(c);
            chars.next(); // consume '('
            expr.push('(');
        } else if c == ')' {
            if chars.peek() == Some(&')') {
                depth -= 1;
                if depth == 0 {
                    chars.next();
                    return expr;
                }
                expr.push(')');
                chars.next();
                expr.push(')');
            } else {
                expr.push(c);
            }
        } else if c == '(' && chars.peek() == Some(&'(') {
            // Nested arithmetic
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
    let mut at_word_start = true;
    while let Some(c) = chars.next() {
        if c == '#' && at_word_start {
            cmd.push(c);
            while let Some(next) = chars.next() {
                cmd.push(next);
                if next == '\n' {
                    at_word_start = true;
                    break;
                }
                if next == '\r' {
                    if chars.peek() == Some(&'\n') {
                        cmd.push(chars.next().expect("peeked newline should exist"));
                    }
                    at_word_start = true;
                    break;
                }
            }
            continue;
        }
        if c == '\\' {
            cmd.push(c);
            if let Some(next) = chars.next() {
                cmd.push(next);
                at_word_start = matches!(next, '\n' | '\r');
            }
            continue;
        }
        if c == '\'' {
            cmd.push(c);
            while let Some(next) = chars.next() {
                cmd.push(next);
                if next == '\'' {
                    break;
                }
                if next == '\\' {
                    if let Some(escaped) = chars.next() {
                        cmd.push(escaped);
                    }
                }
            }
            at_word_start = false;
            continue;
        }
        if c == '"' {
            cmd.push(c);
            while let Some(next) = chars.next() {
                cmd.push(next);
                if next == '"' {
                    break;
                }
                if next == '\\' {
                    if let Some(escaped) = chars.next() {
                        cmd.push(escaped);
                    }
                }
            }
            at_word_start = false;
            continue;
        }
        if c == '(' {
            depth += 1;
        }
        if c == ')' {
            depth -= 1;
            if depth == 0 {
                return cmd;
            }
        }
        cmd.push(c);
        at_word_start = matches!(c, ' ' | '\t' | '\n' | '\r' | ';' | '&' | '|' | '<' | '>' | '(' | ')');
    }
    cmd
}

// ── Word splitting ──

/// Check if a string contains unquoted glob metacharacters.
/// Respects `\u{E001}..\u{E001}` sentinel pairs: chars inside are treated as literal.
/// `\u{E004}` (literal-unquoted) regions are glob-eligible.
fn has_unquoted_glob_chars(s: &str) -> bool {
    let mut in_truly_quoted = false;
    for c in s.chars() {
        match c {
            '\u{E001}' => in_truly_quoted = !in_truly_quoted,
            '\u{E004}' => {} // literal-unquoted — glob allowed
            '*' | '?' | '[' if !in_truly_quoted => return true,
            _ => {}
        }
    }
    false
}

/// Split a string into fields according to POSIX IFS rules.
///
/// Returns `Vec<(field_text, fully_quoted)>` where `fully_quoted=true` means
/// the field came entirely from quoted regions and glob should be skipped.
///
/// Sentinel characters:
/// - `\u{E001}` (S_QUOTED): toggle quoted region — no splitting, no globbing
/// - `\u{E002}` (S_BOUNDARY): hard field break (from `$@`)
/// - `\u{E003}` (S_ZERO): skip entirely (zero fields)
/// - `\u{E004}` (S_LITERAL): toggle literal region — no splitting, yes globbing
fn split_fields(s: &str, ifs: &str) -> Vec<(String, bool)> {
    // ── Empty IFS: no splitting, but S_BOUNDARY still forces split ──
    if ifs.is_empty() {
        if s.contains('\u{E002}') {
            let mut fields = Vec::new();
            for part in s.split('\u{E002}') {
                let inner: String = part.chars().filter(|&c| c != '\u{E003}').collect();
                if inner.is_empty() {
                    continue;
                }
                let has_unquoted = has_unquoted_glob_chars(part);
                let preserved = part.replace('\u{E003}', "");
                fields.push((preserved, !has_unquoted));
            }
            return fields;
        }
        // Entire word is zero-words sentinels → no fields.
        if !s.is_empty() && s.chars().all(|c| c == '\u{E003}') {
            return vec![];
        }
        let has_unquoted = has_unquoted_glob_chars(s);
        let preserved = s.replace('\u{E003}', "");
        return vec![(preserved, !has_unquoted)];
    }

    // ── Classify IFS characters ──
    let ifs_ws: Vec<char> = ifs.chars().filter(|c| " \t\n".contains(*c)).collect();
    let ifs_non_ws: Vec<char> = ifs.chars().filter(|c| !" \t\n".contains(*c)).collect();

    let mut fields: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    let mut in_quoted = false;
    let mut in_literal = false;
    let mut has_content = false;
    let mut field_has_unquoted = false;

    // Skip leading IFS whitespace (only outside sentinel regions)
    while let Some(&c) = chars.peek() {
        if c == '\u{E001}' || c == '\u{E004}' {
            break;
        }
        if ifs_ws.contains(&c) {
            chars.next();
        } else {
            break;
        }
    }

    while let Some(c) = chars.next() {
        if c == '\u{E003}' {
            // Zero-words sentinel — contributes nothing.
            continue;
        }

        if c == '\u{E002}' {
            // Hard field boundary from "$@" — always split here.
            if has_content || !current.is_empty() {
                let fully_quoted = !field_has_unquoted;
                fields.push((std::mem::take(&mut current), fully_quoted));
            }
            has_content = false;
            field_has_unquoted = false;
            in_quoted = false;
            in_literal = false;
            continue;
        }

        if c == '\u{E001}' {
            in_quoted = !in_quoted;
            current.push('\u{E001}');
            has_content = true;
            continue;
        }

        if c == '\u{E004}' {
            in_literal = !in_literal;
            current.push('\u{E004}');
            has_content = true;
            continue;
        }

        if in_quoted || in_literal {
            // Inside sentinel region — no splitting.
            current.push(c);
            has_content = true;
            if in_literal {
                field_has_unquoted = true;
            }
        } else if ifs_non_ws.contains(&c) {
            // Non-whitespace IFS delimiter: always produces a field boundary.
            let fully_quoted = !field_has_unquoted;
            fields.push((std::mem::take(&mut current), fully_quoted));
            has_content = false;
            field_has_unquoted = false;
            // Skip adjacent IFS whitespace after non-ws delimiter.
            while let Some(&next) = chars.peek() {
                if next == '\u{E001}' || next == '\u{E004}' {
                    break;
                }
                if ifs_ws.contains(&next) {
                    chars.next();
                } else {
                    break;
                }
            }
        } else if ifs_ws.contains(&c) {
            // IFS whitespace: collapse consecutive.
            while let Some(&next) = chars.peek() {
                if next == '\u{E001}' || next == '\u{E004}' {
                    break;
                }
                if ifs_ws.contains(&next) {
                    chars.next();
                } else {
                    break;
                }
            }
            // POSIX: IFS whitespace adjacent to a non-ws IFS char forms a compound
            // delimiter. Absorb the non-ws char and its trailing whitespace.
            let absorbed_nonws = if let Some(&next) = chars.peek() {
                if next != '\u{E001}' && next != '\u{E004}' && ifs_non_ws.contains(&next) {
                    chars.next();
                    while let Some(&next2) = chars.peek() {
                        if next2 == '\u{E001}' || next2 == '\u{E004}' {
                            break;
                        }
                        if ifs_ws.contains(&next2) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            // Pure IFS whitespace only ends a field when content exists.
            // Compound ws+nonws always produces a boundary.
            if has_content || absorbed_nonws {
                let fully_quoted = !field_has_unquoted;
                fields.push((std::mem::take(&mut current), fully_quoted));
                has_content = false;
                field_has_unquoted = false;
            }
        } else {
            current.push(c);
            has_content = true;
            field_has_unquoted = true;
        }
    }

    if has_content {
        let fully_quoted = !field_has_unquoted;
        fields.push((current, fully_quoted));
    }

    fields
}

// ── Pathname expansion ──

/// Check if a string contains unquoted glob metacharacters for pathname expansion.
/// Characters inside `\u{E001}..\u{E001}` sentinel pairs are treated as literal.
fn contains_glob_chars(s: &str) -> bool {
    let mut in_quoted = false;
    for c in s.chars() {
        match c {
            '\u{E001}' => in_quoted = !in_quoted,
            '*' | '?' | '[' if !in_quoted => return true,
            _ => {}
        }
    }
    false
}

fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Convert a sentinel-decorated field to a glob pattern string.
///
/// Chars inside `\u{E001}..\u{E001}` pairs are quoted (literal); glob metacharacters
/// inside them are wrapped in single-char bracket expressions (`[*]`, `[?]`, `[[]`, `[]]`)
/// so the `glob` crate treats them as literals. Sentinel chars are stripped.
fn sentinel_to_glob_pattern(field: &str) -> String {
    let mut result = String::with_capacity(field.len() * 2);
    let mut in_quoted = false;
    for c in field.chars() {
        match c {
            '\u{E001}' => in_quoted = !in_quoted,
            '\u{E002}' | '\u{E003}' | '\u{E004}' => {}
            '*' if in_quoted => result.push_str("[*]"),
            '?' if in_quoted => result.push_str("[?]"),
            '[' if in_quoted => result.push_str("[[]"),
            ']' if in_quoted => result.push_str("[]]"),
            _ => result.push(c),
        }
    }
    result
}

/// Expand a field using pathname (glob) expansion.
///
/// If the field contains unquoted glob metacharacters (`*`, `?`, `[`), match
/// against the filesystem. Characters inside `\u{E001}` sentinel regions are
/// treated as literals. If no matches, return the original pattern (POSIX default).
fn expand_pathname(field: &str) -> Vec<String> {
    if !contains_glob_chars(field) {
        return vec![strip_sentinels(field)];
    }

    let original_stripped = strip_sentinels(field);
    if !original_stripped.starts_with('/') && !has_windows_drive_prefix(&original_stripped) {
        let matches = expand_relative_pathname(field);
        if !matches.is_empty() {
            return matches;
        }
    }

    let glob_pat = sentinel_to_glob_pattern(field);

    let opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: false,
        require_literal_leading_dot: true,
    };

    match glob::glob_with(&glob_pat, opts) {
        Ok(paths) => {
            let mut matches: Vec<String> = paths
                .filter_map(|r| r.ok())
                .map(|p| {
                    #[cfg(windows)]
                    {
                        p.to_string_lossy().replace('\\', "/")
                    }
                    #[cfg(not(windows))]
                    {
                        p.to_string_lossy().into_owned()
                    }
                })
                .collect();

            if matches.is_empty() {
                vec![original_stripped]
            } else {
                matches.sort();
                matches
            }
        }
        Err(_) => vec![original_stripped],
    }
}

#[derive(Clone)]
struct PathnameCandidate {
    fs_path: std::path::PathBuf,
    display_path: String,
}

fn expand_relative_pathname(field: &str) -> Vec<String> {
    let Ok(start_dir) = std::env::current_dir() else {
        return Vec::new();
    };

    let mut candidates = vec![PathnameCandidate {
        fs_path: start_dir,
        display_path: String::new(),
    }];

    for (index, segment) in field.split('/').enumerate() {
        if index > 0 {
            for candidate in &mut candidates {
                candidate.display_path.push('/');
            }
        }

        if segment.is_empty() {
            continue;
        }

        if !contains_glob_chars(segment) {
            let literal = strip_sentinels(segment);
            for candidate in &mut candidates {
                candidate.fs_path.push(&literal);
                candidate.display_path.push_str(&literal);
            }
            continue;
        }

        let segment_pattern = strip_sentinels_case_pattern(segment);
        let mut next = Vec::new();
        for candidate in &candidates {
            next.extend(match_path_segment(candidate, &segment_pattern));
        }
        if next.is_empty() {
            return Vec::new();
        }
        candidates = next;
    }

    let mut matches: Vec<String> = candidates
        .into_iter()
        .map(|candidate| candidate.display_path)
        .collect();
    matches.sort();
    matches.dedup();
    matches
}

fn match_path_segment(candidate: &PathnameCandidate, pattern: &str) -> Vec<PathnameCandidate> {
    let mut matches = Vec::new();
    let match_dotfiles = pattern.starts_with('.');

    if match_dotfiles && shell_pattern_match(".", pattern) {
        let mut next = candidate.clone();
        next.display_path.push('.');
        matches.push(next);
    }

    if match_dotfiles && shell_pattern_match("..", pattern) {
        let mut next = candidate.clone();
        next.display_path.push_str("..");
        if let Some(parent) = candidate.fs_path.parent() {
            next.fs_path = parent.to_path_buf();
        }
        matches.push(next);
    }

    let read_dir = match std::fs::read_dir(&candidate.fs_path) {
        Ok(read_dir) => read_dir,
        Err(_) => return matches,
    };

    for entry in read_dir.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !match_dotfiles {
            continue;
        }
        if !shell_pattern_match(&name, pattern) {
            continue;
        }

        let mut next = candidate.clone();
        next.fs_path = entry.path();
        next.display_path.push_str(&name);
        matches.push(next);
    }

    matches
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
            '*' | '?' | '[' | ']' | '\\' if in_quoted => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

// ── Arithmetic evaluation ──

#[derive(Debug, Clone, PartialEq)]
enum ArithToken {
    Num(i64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    LShift,
    RShift,
    Amp,
    Pipe,
    Caret,
    AmpAmp,
    PipePipe,
    EqEq,
    BangEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Bang,
    Tilde,
    Question,
    Colon,
    Comma,
    LParen,
    RParen,
    PlusPlus,
    MinusMinus,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    Eof,
}

fn tokenize_arith(expr: &str) -> Result<Vec<ArithToken>, ExpandError> {
    let mut tokens = Vec::new();
    let mut chars = expr.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            '0'..='9' => {
                let mut num_str = String::new();
                if ch == '0' {
                    chars.next();
                    match chars.peek() {
                        Some('x') | Some('X') => {
                            chars.next();
                            while let Some(&c) = chars.peek() {
                                if c.is_ascii_hexdigit() {
                                    num_str.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            if num_str.is_empty() {
                                return Err(ExpandError::Arithmetic {
                                    reason: "invalid hex literal: 0x".into(),
                                });
                            }
                            let n = i64::from_str_radix(&num_str, 16).map_err(|_| {
                                ExpandError::Arithmetic {
                                    reason: format!("invalid hex: 0x{}", num_str),
                                }
                            })?;
                            tokens.push(ArithToken::Num(n));
                            continue;
                        }
                        Some('b') | Some('B') => {
                            chars.next();
                            while let Some(&c) = chars.peek() {
                                if c == '0' || c == '1' {
                                    num_str.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            if num_str.is_empty() {
                                return Err(ExpandError::Arithmetic {
                                    reason: "invalid binary literal: 0b".into(),
                                });
                            }
                            let n = i64::from_str_radix(&num_str, 2).map_err(|_| {
                                ExpandError::Arithmetic {
                                    reason: format!("invalid binary: 0b{}", num_str),
                                }
                            })?;
                            tokens.push(ArithToken::Num(n));
                            continue;
                        }
                        Some(c) if c.is_ascii_digit() => {
                            // Octal
                            num_str.push('0');
                            while let Some(&c) = chars.peek() {
                                if c.is_ascii_digit() {
                                    num_str.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            let n = i64::from_str_radix(&num_str, 8).map_err(|_| {
                                ExpandError::Arithmetic {
                                    reason: format!("invalid octal: {}", num_str),
                                }
                            })?;
                            tokens.push(ArithToken::Num(n));
                            continue;
                        }
                        _ => {
                            tokens.push(ArithToken::Num(0));
                            continue;
                        }
                    }
                }
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        num_str.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n: i64 = num_str.parse().map_err(|_| ExpandError::Arithmetic {
                    reason: format!("invalid number: {}", num_str),
                })?;
                tokens.push(ArithToken::Num(n));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(ArithToken::Ident(name));
            }
            '*' => {
                chars.next();
                if chars.peek() == Some(&'*') {
                    chars.next();
                    tokens.push(ArithToken::StarStar);
                } else if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::StarEq);
                } else {
                    tokens.push(ArithToken::Star);
                }
            }
            '+' => {
                chars.next();
                if chars.peek() == Some(&'+') {
                    chars.next();
                    tokens.push(ArithToken::PlusPlus);
                } else if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::PlusEq);
                } else {
                    tokens.push(ArithToken::Plus);
                }
            }
            '-' => {
                chars.next();
                if chars.peek() == Some(&'-') {
                    chars.next();
                    tokens.push(ArithToken::MinusMinus);
                } else if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::MinusEq);
                } else {
                    tokens.push(ArithToken::Minus);
                }
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::SlashEq);
                } else {
                    tokens.push(ArithToken::Slash);
                }
            }
            '%' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::PercentEq);
                } else {
                    tokens.push(ArithToken::Percent);
                }
            }
            '<' => {
                chars.next();
                if chars.peek() == Some(&'<') {
                    chars.next();
                    tokens.push(ArithToken::LShift);
                } else if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::LtEq);
                } else {
                    tokens.push(ArithToken::Lt);
                }
            }
            '>' => {
                chars.next();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(ArithToken::RShift);
                } else if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::GtEq);
                } else {
                    tokens.push(ArithToken::Gt);
                }
            }
            '&' => {
                chars.next();
                if chars.peek() == Some(&'&') {
                    chars.next();
                    tokens.push(ArithToken::AmpAmp);
                } else {
                    tokens.push(ArithToken::Amp);
                }
            }
            '|' => {
                chars.next();
                if chars.peek() == Some(&'|') {
                    chars.next();
                    tokens.push(ArithToken::PipePipe);
                } else {
                    tokens.push(ArithToken::Pipe);
                }
            }
            '^' => {
                chars.next();
                tokens.push(ArithToken::Caret);
            }
            '=' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::EqEq);
                } else {
                    tokens.push(ArithToken::Eq);
                }
            }
            '!' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(ArithToken::BangEq);
                } else {
                    tokens.push(ArithToken::Bang);
                }
            }
            '~' => {
                chars.next();
                tokens.push(ArithToken::Tilde);
            }
            '?' => {
                chars.next();
                tokens.push(ArithToken::Question);
            }
            ':' => {
                chars.next();
                tokens.push(ArithToken::Colon);
            }
            ',' => {
                chars.next();
                tokens.push(ArithToken::Comma);
            }
            '(' => {
                chars.next();
                tokens.push(ArithToken::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(ArithToken::RParen);
            }
            other => {
                return Err(ExpandError::Arithmetic {
                    reason: format!("unexpected character: '{}'", other),
                });
            }
        }
    }
    tokens.push(ArithToken::Eof);
    Ok(tokens)
}

struct ArithParser<'a> {
    tokens: &'a [ArithToken],
    pos: usize,
    env: &'a mut Env,
}

impl<'a> ArithParser<'a> {
    fn new(tokens: &'a [ArithToken], env: &'a mut Env) -> Self {
        Self {
            tokens,
            pos: 0,
            env,
        }
    }

    fn peek(&self) -> &ArithToken {
        self.tokens.get(self.pos).unwrap_or(&ArithToken::Eof)
    }

    fn advance(&mut self) -> &ArithToken {
        let tok = self.tokens.get(self.pos).unwrap_or(&ArithToken::Eof);
        self.pos += 1;
        tok
    }

    /// Read the current integer value of a shell variable (0 if unset or non-numeric).
    fn var_value(&self, name: &str) -> i64 {
        let s = self.env.get_str(name);
        if s.is_empty() {
            0
        } else {
            s.trim().parse::<i64>().unwrap_or(0)
        }
    }

    /// Write an integer back to a shell variable.
    fn set_var(&mut self, name: &str, val: i64) {
        let _ = self.env.set(name, Variable::string(val.to_string()));
    }

    /// Pratt parser: parse expression with minimum binding power `min_bp`.
    fn parse_expr(&mut self, min_bp: u8) -> Result<i64, ExpandError> {
        // Prefix / atom
        let mut lhs = match self.peek().clone() {
            ArithToken::Num(n) => {
                self.advance();
                n
            }
            ArithToken::Ident(name) => {
                self.advance();
                // Check for postfix ++ / -- or assignment operators
                match self.peek() {
                    ArithToken::PlusPlus => {
                        self.advance();
                        let old = self.var_value(&name);
                        self.set_var(&name, old.wrapping_add(1));
                        old // post-increment returns old value
                    }
                    ArithToken::MinusMinus => {
                        self.advance();
                        let old = self.var_value(&name);
                        self.set_var(&name, old.wrapping_sub(1));
                        old // post-decrement returns old value
                    }
                    ArithToken::Eq => {
                        self.advance();
                        let val = self.parse_expr(2)?; // right-assoc, low bp
                        self.set_var(&name, val);
                        val
                    }
                    ArithToken::PlusEq => {
                        self.advance();
                        let rhs = self.parse_expr(2)?;
                        let val = self.var_value(&name).wrapping_add(rhs);
                        self.set_var(&name, val);
                        val
                    }
                    ArithToken::MinusEq => {
                        self.advance();
                        let rhs = self.parse_expr(2)?;
                        let val = self.var_value(&name).wrapping_sub(rhs);
                        self.set_var(&name, val);
                        val
                    }
                    ArithToken::StarEq => {
                        self.advance();
                        let rhs = self.parse_expr(2)?;
                        let val = self.var_value(&name).wrapping_mul(rhs);
                        self.set_var(&name, val);
                        val
                    }
                    ArithToken::SlashEq => {
                        self.advance();
                        let rhs = self.parse_expr(2)?;
                        if rhs == 0 {
                            return Err(ExpandError::Arithmetic {
                                reason: "division by zero".into(),
                            });
                        }
                        let val = self.var_value(&name).wrapping_div(rhs);
                        self.set_var(&name, val);
                        val
                    }
                    ArithToken::PercentEq => {
                        self.advance();
                        let rhs = self.parse_expr(2)?;
                        if rhs == 0 {
                            return Err(ExpandError::Arithmetic {
                                reason: "division by zero".into(),
                            });
                        }
                        let val = self.var_value(&name).wrapping_rem(rhs);
                        self.set_var(&name, val);
                        val
                    }
                    _ => {
                        // Plain variable lookup
                        if self.env.options().nounset && !self.env.is_set(&name) {
                            return Err(ExpandError::UndefinedVar { name });
                        }
                        self.var_value(&name)
                    }
                }
            }
            ArithToken::PlusPlus => {
                // Pre-increment: ++var
                self.advance();
                if let ArithToken::Ident(name) = self.peek().clone() {
                    self.advance();
                    let val = self.var_value(&name).wrapping_add(1);
                    self.set_var(&name, val);
                    val
                } else {
                    return Err(ExpandError::Arithmetic {
                        reason: "expected variable after '++'".into(),
                    });
                }
            }
            ArithToken::MinusMinus => {
                // Pre-decrement: --var
                self.advance();
                if let ArithToken::Ident(name) = self.peek().clone() {
                    self.advance();
                    let val = self.var_value(&name).wrapping_sub(1);
                    self.set_var(&name, val);
                    val
                } else {
                    return Err(ExpandError::Arithmetic {
                        reason: "expected variable after '--'".into(),
                    });
                }
            }
            ArithToken::LParen => {
                self.advance();
                let val = self.parse_expr(0)?;
                if *self.peek() != ArithToken::RParen {
                    return Err(ExpandError::Arithmetic {
                        reason: "expected ')'".into(),
                    });
                }
                self.advance();
                val
            }
            ArithToken::Minus => {
                self.advance();
                let val = self.parse_expr(14)?; // unary prefix bp
                val.wrapping_neg()
            }
            ArithToken::Plus => {
                self.advance();
                self.parse_expr(14)?
            }
            ArithToken::Bang => {
                self.advance();
                let val = self.parse_expr(14)?;
                if val == 0 {
                    1
                } else {
                    0
                }
            }
            ArithToken::Tilde => {
                self.advance();
                let val = self.parse_expr(14)?;
                !val
            }
            ArithToken::Eof => {
                // Empty expression evaluates to 0
                return Ok(0);
            }
            other => {
                return Err(ExpandError::Arithmetic {
                    reason: format!("unexpected token: {:?}", other),
                });
            }
        };

        // Infix
        loop {
            let (op, bp, right_assoc) = match self.peek() {
                ArithToken::Comma => (ArithToken::Comma, 1, false),
                ArithToken::Question => {
                    // Ternary ? : — handle specially
                    if 2 < min_bp {
                        break;
                    }
                    self.advance(); // consume ?
                    let then_val = self.parse_expr(0)?;
                    if *self.peek() != ArithToken::Colon {
                        return Err(ExpandError::Arithmetic {
                            reason: "expected ':' in ternary".into(),
                        });
                    }
                    self.advance(); // consume :
                    let else_val = self.parse_expr(2)?;
                    lhs = if lhs != 0 { then_val } else { else_val };
                    continue;
                }
                ArithToken::PipePipe => (ArithToken::PipePipe, 3, false),
                ArithToken::AmpAmp => (ArithToken::AmpAmp, 4, false),
                ArithToken::Pipe => (ArithToken::Pipe, 5, false),
                ArithToken::Caret => (ArithToken::Caret, 6, false),
                ArithToken::Amp => (ArithToken::Amp, 7, false),
                ArithToken::EqEq => (ArithToken::EqEq, 8, false),
                ArithToken::BangEq => (ArithToken::BangEq, 8, false),
                ArithToken::Lt => (ArithToken::Lt, 9, false),
                ArithToken::Gt => (ArithToken::Gt, 9, false),
                ArithToken::LtEq => (ArithToken::LtEq, 9, false),
                ArithToken::GtEq => (ArithToken::GtEq, 9, false),
                ArithToken::LShift => (ArithToken::LShift, 10, false),
                ArithToken::RShift => (ArithToken::RShift, 10, false),
                ArithToken::Plus => (ArithToken::Plus, 11, false),
                ArithToken::Minus => (ArithToken::Minus, 11, false),
                ArithToken::Star => (ArithToken::Star, 12, false),
                ArithToken::Slash => (ArithToken::Slash, 12, false),
                ArithToken::Percent => (ArithToken::Percent, 12, false),
                ArithToken::StarStar => (ArithToken::StarStar, 13, true),
                _ => break,
            };

            if bp < min_bp {
                break;
            }

            self.advance();
            let next_bp = if right_assoc { bp } else { bp + 1 };
            let rhs = self.parse_expr(next_bp)?;

            lhs = match op {
                ArithToken::Plus => lhs.wrapping_add(rhs),
                ArithToken::Minus => lhs.wrapping_sub(rhs),
                ArithToken::Star => lhs.wrapping_mul(rhs),
                ArithToken::Slash => {
                    if rhs == 0 {
                        return Err(ExpandError::Arithmetic {
                            reason: "division by zero".into(),
                        });
                    }
                    if lhs == i64::MIN && rhs == -1 {
                        return Err(ExpandError::Arithmetic {
                            reason: "integer overflow: i64::MIN / -1".into(),
                        });
                    }
                    lhs / rhs
                }
                ArithToken::Percent => {
                    if rhs == 0 {
                        return Err(ExpandError::Arithmetic {
                            reason: "division by zero".into(),
                        });
                    }
                    if lhs == i64::MIN && rhs == -1 {
                        return Err(ExpandError::Arithmetic {
                            reason: "integer overflow: i64::MIN % -1".into(),
                        });
                    }
                    lhs % rhs
                }
                ArithToken::StarStar => {
                    if rhs < 0 {
                        return Err(ExpandError::Arithmetic {
                            reason: "negative exponent".into(),
                        });
                    }
                    pow_i64(lhs, rhs as u64)
                }
                ArithToken::LShift => lhs.wrapping_shl(rhs as u32),
                ArithToken::RShift => lhs.wrapping_shr(rhs as u32),
                ArithToken::Amp => lhs & rhs,
                ArithToken::Pipe => lhs | rhs,
                ArithToken::Caret => lhs ^ rhs,
                ArithToken::AmpAmp => {
                    if lhs != 0 && rhs != 0 {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::PipePipe => {
                    if lhs != 0 || rhs != 0 {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::EqEq => {
                    if lhs == rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::BangEq => {
                    if lhs != rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::Lt => {
                    if lhs < rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::Gt => {
                    if lhs > rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::LtEq => {
                    if lhs <= rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::GtEq => {
                    if lhs >= rhs {
                        1
                    } else {
                        0
                    }
                }
                ArithToken::Comma => rhs,
                _ => {
                    return Err(ExpandError::Arithmetic {
                        reason: format!("unhandled operator: {:?}", op),
                    });
                }
            };
        }

        Ok(lhs)
    }
}

/// Integer exponentiation via squaring.
fn pow_i64(mut base: i64, mut exp: u64) -> i64 {
    let mut result: i64 = 1;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exp >>= 1;
    }
    result
}
