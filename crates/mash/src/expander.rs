//! Shell expansion — parameter, arithmetic, tilde, word split, glob, quote removal.
//!
//! Command substitution is stubbed — the executor sub-project wires it up.

use crate::env::{Env, Variable, VarValue};

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
    chars: &mut std::iter::Peekable<std::str::Chars>,
    result: &mut String,
    env: &mut Env,
    in_double_quote: bool,
) -> Result<(), ExpandError> {
    // Collect everything between ${ and } with balanced brace tracking.
    let expr = collect_brace_expr(chars);

    if expr.is_empty() {
        return Err(ExpandError::BadSubstitution { expr: String::new() });
    }

    // ${!VAR} — indirect expansion
    if let Some(name) = expr.strip_prefix('!') {
        if name.ends_with("[@]") || name.ends_with("[*]") {
            // ${!arr[@]} — array keys
            let arr_name = &name[..name.len() - 3];
            if let Some(var) = env.get(arr_name) {
                match &var.value {
                    VarValue::Array(arr) => {
                        let keys: Vec<String> = arr.iter().enumerate()
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
                    _ => { result.push('0'); }
                }
            } else {
                result.push('0');
            }
            return Ok(());
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
                        VarValue::Array(arr) => arr.iter()
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
            result.push_str(&expand_string_inner(&default, env, false)?);
        }
        return Ok(());
    }
    if let Some(default) = try_strip_op(&rest, "-") {
        if val_is_set {
            result.push_str(&val);
        } else {
            result.push_str(&expand_string_inner(&default, env, false)?);
        }
        return Ok(());
    }

    // ${VAR:=assign} / ${VAR=assign}
    if let Some(default) = try_strip_op(&rest, ":=") {
        if val_is_nonempty {
            result.push_str(&val);
        } else {
            let expanded = expand_string_inner(&default, env, false)?;
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
            let expanded = expand_string_inner(&default, env, false)?;
            let stored = strip_sentinels(&expanded);
            let _ = env.set(&name, Variable::string(&stored));
            result.push_str(&expanded);
        }
        return Ok(());
    }

    // ${VAR:+alt} / ${VAR+alt}
    if let Some(alt) = try_strip_op(&rest, ":+") {
        if val_is_nonempty {
            result.push_str(&expand_string_inner(&alt, env, false)?);
        } else if !in_double_quote {
            result.push(S_ZERO);
        }
        return Ok(());
    }
    if let Some(alt) = try_strip_op(&rest, "+") {
        if val_is_set {
            result.push_str(&expand_string_inner(&alt, env, false)?);
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
            let expanded_msg = strip_sentinels(&expand_string_inner(&msg, env, false)?);
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
            let expanded_msg = strip_sentinels(&expand_string_inner(&msg, env, false)?);
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

            let offset: i64 = offset_str.trim().parse().map_err(|_| ExpandError::Arithmetic {
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
                    if end_pos <= start { start } else { end_pos }
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
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&strip_largest_suffix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR%pattern} — smallest suffix strip
    if let Some(pat) = try_strip_op(&rest, "%") {
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&strip_smallest_suffix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR##pattern} — largest prefix strip
    if let Some(pat) = try_strip_op(&rest, "##") {
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&strip_largest_prefix(&val, &expanded_pat));
        return Ok(());
    }
    // ${VAR#pattern} — smallest prefix strip
    if let Some(pat) = try_strip_op(&rest, "#") {
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&strip_smallest_prefix(&val, &expanded_pat));
        return Ok(());
    }

    // ${VAR//pattern/replacement} — global substitution
    if let Some(pat_rep) = try_strip_op(&rest, "//") {
        let (pat, rep) = split_subst(&pat_rep);
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&shell_replace_all(&val, &expanded_pat, &rep));
        return Ok(());
    }
    // ${VAR/pattern/replacement} — first substitution
    if let Some(pat_rep) = try_strip_op(&rest, "/") {
        let (pat, rep) = split_subst(&pat_rep);
        let expanded_pat = expand_string_inner(&pat, env, false)?;
        result.push_str(&shell_replace_first(&val, &expanded_pat, &rep));
        return Ok(());
    }

    // ${VAR^^[pattern]} — uppercase all
    if let Some(pat) = rest.strip_prefix("^^") {
        if pat.is_empty() {
            result.push_str(&val.to_uppercase());
        } else {
            let expanded_pat = expand_string_inner(pat, env, false)?;
            let s: String = val.chars().map(|c| {
                if shell_pattern_match(&c.to_string(), &expanded_pat) {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                }
            }).collect();
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
                    let expanded_pat = expand_string_inner(pat, env, false)?;
                    shell_pattern_match(&c.to_string(), &expanded_pat)
                };
                let upper = if matches { c.to_uppercase().next().unwrap_or(c) } else { c };
                result.push(upper);
                for ch in val_chars { result.push(ch); }
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
            let expanded_pat = expand_string_inner(pat, env, false)?;
            let s: String = val.chars().map(|c| {
                if shell_pattern_match(&c.to_string(), &expanded_pat) {
                    c.to_lowercase().next().unwrap_or(c)
                } else {
                    c
                }
            }).collect();
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
                    let expanded_pat = expand_string_inner(pat, env, false)?;
                    shell_pattern_match(&c.to_string(), &expanded_pat)
                };
                let lower = if matches { c.to_lowercase().next().unwrap_or(c) } else { c };
                result.push(lower);
                for ch in val_chars { result.push(ch); }
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
fn shell_pattern_match(s: &str, pattern: &str) -> bool {
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
    let start = i;
    while i < pattern.len() {
        if pattern[i] == b']' && i > start {
            return Some((matched ^ negate, i + 1));
        }
        if i + 2 < pattern.len() && pattern[i + 1] == b'-' && pattern[i + 2] != b']' {
            let lo = pattern[i];
            let hi = pattern[i + 2];
            if ch >= lo && ch <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if pattern[i] == ch {
                matched = true;
            }
            i += 1;
        }
    }
    None // unclosed bracket
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
        if !val.is_char_boundary(start) { continue; }
        for end in start + 1..=val.len() {
            if !val.is_char_boundary(end) { continue; }
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
