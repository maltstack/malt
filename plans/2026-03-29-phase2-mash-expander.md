# `mash` Expander — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the shell expansion module — parameter expansion, arithmetic evaluation, tilde expansion, word splitting, pathname expansion (glob), and quote removal. Command substitution stubbed.

**Architecture:** Single `expander.rs` module (or `expander/` directory if it grows). Sync API. Sentinel-based quoting (`\u{E001}`–`\u{E004}`). Core engine walks characters, dispatches to expansion-specific functions. Pipeline: tilde → parameter → arithmetic → word split → glob → quote removal.

**Tech Stack:** Rust, glob crate, thiserror

**Spec:** `malt/specs/phase2-mash-expander.md`

**Reference:** `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\expander.rs` (4,097 lines) — port the logic sync, rewrite with quality.

---

## File Structure

```
orix/malt/crates/mash/
  Cargo.toml              # MODIFY — add glob = "0.3"
  src/
    lib.rs                # MODIFY — add pub mod expander
    expander.rs           # NEW — all expansion logic
  tests/
    expander.rs           # NEW — expansion tests
```

---

## Task 1: Expander Scaffold + Error Types + Tilde Expansion

**Files:**
- Modify: `orix/malt/crates/mash/Cargo.toml` (add `glob = "0.3"`)
- Modify: `orix/malt/crates/mash/src/lib.rs` (add `pub mod expander`)
- Create: `orix/malt/crates/mash/src/expander.rs`
- Create: `orix/malt/crates/mash/tests/expander.rs`

Start with the simplest expansion (tilde), the public API signatures, sentinel constants, and error types. This establishes the module structure everything else builds on.

- [ ] **Step 1: Add glob dependency and module**

Add `glob = "0.3"` to `[dependencies]` in `Cargo.toml`. Add `pub mod expander;` to `lib.rs`.

- [ ] **Step 2: Create expander.rs with scaffold**

Create `src/expander.rs` with:
- Sentinel constants
- `ExpandError` enum
- Public API function signatures (returning `todo!()` for now except tilde)
- Tilde expansion implementation
- `expand_string_inner` skeleton that handles tilde at word start

```rust
//! Shell expansion — parameter, arithmetic, tilde, word split, glob, quote removal.
//!
//! Command substitution is stubbed — the executor sub-project wires it up.

use crate::env::{Env, Variable, VarValue};
use std::collections::HashMap;

// ── Sentinels ──

/// Quoted text — no splitting, no globbing.
const S_QUOTED: char = '\u{E001}';
/// Hard field boundary from $@ — forces split.
const S_BOUNDARY: char = '\u{E002}';
/// Zero-words from ${x+y} when unset — produces no fields.
const S_ZERO: char = '\u{E003}';
/// Literal unquoted — no splitting, yes globbing.
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

/// Expand through full pipeline: tilde → param → cmd sub → arith → split → glob → quote removal.
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
pub fn eval_arithmetic(expr: &str, env: &mut Env) -> Result<i64, ExpandError> {
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
                // Check for $' (ANSI-C) — if result ends with $ and we just consumed that $
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
        Some(&c) if c.is_ascii_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '$' || c == '#' || c == '@' || c == '*' || c == '-' || c == '0' => {
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
    result: &mut String,
    env: &mut Env,
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

fn split_fields(s: &str, ifs: &str) -> Vec<(String, bool)> {
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
```

- [ ] **Step 3: Write tests for tilde and basic expansion**

Create `tests/expander.rs`:

```rust
use mash::env::Env;
use mash::expander::*;

#[test]
fn tilde_expands_to_home() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/user")).unwrap();
    let result = expand_word_nosplit("~/bin", &mut env).unwrap();
    assert_eq!(result, "/home/user/bin");
}

#[test]
fn tilde_plus_expands_to_pwd() {
    let mut env = Env::empty();
    env.set("PWD", mash::env::Variable::string("/tmp")).unwrap();
    let result = expand_word_nosplit("~+/foo", &mut env).unwrap();
    assert_eq!(result, "/tmp/foo");
}

#[test]
fn tilde_minus_expands_to_oldpwd() {
    let mut env = Env::empty();
    env.set("OLDPWD", mash::env::Variable::string("/var")).unwrap();
    let result = expand_word_nosplit("~-", &mut env).unwrap();
    assert_eq!(result, "/var");
}

#[test]
fn simple_var_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("$FOO", &mut env).unwrap();
    assert_eq!(result, "bar");
}

#[test]
fn simple_var_in_text() {
    let mut env = Env::empty();
    env.set("NAME", mash::env::Variable::string("world")).unwrap();
    let result = expand_word_nosplit("hello $NAME!", &mut env).unwrap();
    assert_eq!(result, "hello world!");
}

#[test]
fn unset_var_expands_to_empty() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$NONEXISTENT", &mut env).unwrap();
    assert_eq!(result, "");
}

#[test]
fn nounset_errors_on_unset() {
    let mut env = Env::empty();
    env.options_mut().nounset = true;
    let result = expand_word_nosplit("$NONEXISTENT", &mut env);
    assert!(result.is_err());
}

#[test]
fn special_var_question_mark() {
    let mut env = Env::empty();
    env.set_exit_code(42);
    let result = expand_word_nosplit("$?", &mut env).unwrap();
    assert_eq!(result, "42");
}

#[test]
fn special_var_dollar() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$$", &mut env).unwrap();
    assert!(!result.is_empty()); // PID
}

#[test]
fn single_quotes_prevent_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("'$FOO'", &mut env).unwrap();
    assert_eq!(result, "$FOO");
}

#[test]
fn double_quotes_allow_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("\"$FOO\"", &mut env).unwrap();
    assert_eq!(result, "bar");
}

#[test]
fn backslash_escape() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("hello\\$world", &mut env).unwrap();
    assert_eq!(result, "hello$world");
}

#[test]
fn command_sub_returns_error() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$(echo hello)", &mut env);
    assert!(matches!(result, Err(ExpandError::CommandSubstitution(_))));
}

#[test]
fn bare_text_unchanged() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("hello world", &mut env).unwrap();
    assert_eq!(result, "hello world");
}
```

- [ ] **Step 4: Run tests**

Run: `cd orix/malt && cargo test -p mash --test expander`

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/mash/Cargo.toml crates/mash/src/lib.rs crates/mash/src/expander.rs crates/mash/tests/expander.rs
git commit -m "feat(mash): expander scaffold — tilde, simple vars, quotes, sentinels, public API"
```

---

## Task 2: Parameter Expansion (`${var...}`)

**Files:**
- Modify: `orix/malt/crates/mash/src/expander.rs`
- Modify: `orix/malt/crates/mash/tests/expander.rs`

Replace the `expand_brace_param` todo with the full implementation. Port from reference lines 521-921.

- [ ] **Step 1: Implement expand_brace_param**

This function reads characters after `${` until `}`, parsing the operator and operand. Must handle:
- `${var}` — simple
- `${#var}` — length
- `${!var}` — indirect
- `${var:-default}`, `${var-default}` — with/without colon
- `${var:=assign}`, `${var=assign}`
- `${var:?error}`, `${var?error}`
- `${var:+alt}`, `${var+alt}`
- `${var#pat}`, `${var##pat}` — prefix strip
- `${var%pat}`, `${var%%pat}` — suffix strip
- `${var/pat/rep}`, `${var//pat/rep}` — replace
- `${var^pat}`, `${var^^pat}` — uppercase
- `${var,pat}`, `${var,,pat}` — lowercase
- `${var:offset}`, `${var:offset:length}` — substring
- `${arr[@]}`, `${arr[*]}`, `${arr[n]}`

Read the reference implementation at `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\expander.rs` lines 521-921 for the complete logic. Port it sync.

Key pattern: collect the variable name, then dispatch on the operator character (`-`, `=`, `?`, `+`, `#`, `%`, `/`, `^`, `,`, `:`).

- [ ] **Step 2: Write parameter expansion tests**

Add to `tests/expander.rs`:

```rust
#[test]
fn brace_simple() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_default_unset() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "fallback");
}

#[test]
fn brace_default_set() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "val");
}

#[test]
fn brace_default_empty_with_colon() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "fallback");
}

#[test]
fn brace_default_empty_without_colon() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("")).unwrap();
    assert_eq!(expand_word_nosplit("${X-fallback}", &mut env).unwrap(), "");
}

#[test]
fn brace_assign() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("${X:=hello}", &mut env).unwrap(), "hello");
    assert_eq!(env.get_str("X"), "hello"); // assigned
}

#[test]
fn brace_error_unset() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("${X:?variable X is required}", &mut env);
    assert!(matches!(result, Err(ExpandError::UnsetVarError { .. })));
}

#[test]
fn brace_alt_set() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    assert_eq!(expand_word_nosplit("${X:+alt}", &mut env).unwrap(), "alt");
}

#[test]
fn brace_alt_unset() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("${X:+alt}", &mut env).unwrap(), "");
}

#[test]
fn brace_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${#X}", &mut env).unwrap(), "5");
}

#[test]
fn brace_strip_prefix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file")).unwrap();
    assert_eq!(expand_word_nosplit("${X#*/}", &mut env).unwrap(), "path/to/file");
}

#[test]
fn brace_strip_prefix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file")).unwrap();
    assert_eq!(expand_word_nosplit("${X##*/}", &mut env).unwrap(), "file");
}

#[test]
fn brace_strip_suffix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz")).unwrap();
    assert_eq!(expand_word_nosplit("${X%.*}", &mut env).unwrap(), "file.tar");
}

#[test]
fn brace_strip_suffix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz")).unwrap();
    assert_eq!(expand_word_nosplit("${X%%.*}", &mut env).unwrap(), "file");
}

#[test]
fn brace_replace_first() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X/hello/bye}", &mut env).unwrap(), "bye world hello");
}

#[test]
fn brace_replace_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X//hello/bye}", &mut env).unwrap(), "bye world bye");
}

#[test]
fn brace_uppercase() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X^}", &mut env).unwrap(), "Hello");
}

#[test]
fn brace_uppercase_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X^^}", &mut env).unwrap(), "HELLO");
}

#[test]
fn brace_lowercase_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("HELLO")).unwrap();
    assert_eq!(expand_word_nosplit("${X,,}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_substring() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world")).unwrap();
    assert_eq!(expand_word_nosplit("${X:6}", &mut env).unwrap(), "world");
}

#[test]
fn brace_substring_with_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world")).unwrap();
    assert_eq!(expand_word_nosplit("${X:0:5}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_nested_default() {
    let mut env = Env::empty();
    env.set("FALLBACK", mash::env::Variable::string("default")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-$FALLBACK}", &mut env).unwrap(), "default");
}
```

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/expander.rs crates/mash/tests/expander.rs
git commit -m "feat(mash): parameter expansion — all ${var...} operators"
```

---

## Task 3: Arithmetic Evaluation

**Files:**
- Modify: `orix/malt/crates/mash/src/expander.rs`
- Modify: `orix/malt/crates/mash/tests/expander.rs`

Replace the `eval_arithmetic` todo with a recursive descent parser.

**Reference:** `vexil-shell/src/expander.rs` lines 1471-1776. Port the tokenizer and ArithParser.

- [ ] **Step 1: Implement arithmetic tokenizer and parser**

The tokenizer converts the expression string into tokens: integers (decimal, hex, octal, binary), variable names, and operators. The parser is recursive descent with operator precedence.

Port from reference. Key functions: `tokenize_arith(expr)`, `ArithParser::parse()` with precedence climbing.

- [ ] **Step 2: Write arithmetic tests**

```rust
#[test]
fn arith_basic_add() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1 + 2", &mut env).unwrap(), 3);
}

#[test]
fn arith_precedence() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("2 + 3 * 4", &mut env).unwrap(), 14);
}

#[test]
fn arith_parens() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("(2 + 3) * 4", &mut env).unwrap(), 20);
}

#[test]
fn arith_variable() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("5")).unwrap();
    assert_eq!(eval_arithmetic("x + 1", &mut env).unwrap(), 6);
}

#[test]
fn arith_assignment() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("x = 10", &mut env).unwrap(), 10);
    assert_eq!(env.get_str("x"), "10");
}

#[test]
fn arith_hex() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("0xFF", &mut env).unwrap(), 255);
}

#[test]
fn arith_octal() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("010", &mut env).unwrap(), 8);
}

#[test]
fn arith_power() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("2 ** 8", &mut env).unwrap(), 256);
}

#[test]
fn arith_ternary() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1 > 0 ? 42 : -1", &mut env).unwrap(), 42);
}

#[test]
fn arith_division_by_zero() {
    let mut env = Env::empty();
    assert!(eval_arithmetic("1 / 0", &mut env).is_err());
}

#[test]
fn arith_compound_assign() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("10")).unwrap();
    assert_eq!(eval_arithmetic("x += 5", &mut env).unwrap(), 15);
    assert_eq!(env.get_str("x"), "15");
}

#[test]
fn arith_pre_increment() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("5")).unwrap();
    assert_eq!(eval_arithmetic("++x", &mut env).unwrap(), 6);
    assert_eq!(env.get_str("x"), "6");
}

#[test]
fn arith_in_expansion() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("$((2 + 3))", &mut env).unwrap(), "5");
}

#[test]
fn arith_bitwise() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("0xFF & 0x0F", &mut env).unwrap(), 15);
}

#[test]
fn arith_logical() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1 && 0", &mut env).unwrap(), 0);
    assert_eq!(eval_arithmetic("1 || 0", &mut env).unwrap(), 1);
}
```

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/expander.rs crates/mash/tests/expander.rs
git commit -m "feat(mash): arithmetic evaluation — recursive descent parser, all C-like operators"
```

---

## Task 4: Word Splitting (IFS)

**Files:**
- Modify: `orix/malt/crates/mash/src/expander.rs`
- Modify: `orix/malt/crates/mash/tests/expander.rs`

Replace the `split_fields` placeholder with full IFS-based splitting that respects sentinels.

**Reference:** `vexil-shell/src/expander.rs` lines 2483-2662.

- [ ] **Step 1: Implement split_fields**

IFS splitting rules:
- Walk the string character by character
- Sentinel `\u{E001}` regions → don't split, mark field as quoted
- Sentinel `\u{E002}` → force field boundary
- Sentinel `\u{E003}` → skip (zero-words)
- IFS whitespace chars (those in IFS that are space/tab/newline) → collapse, trim leading/trailing
- IFS non-whitespace chars → each is an explicit delimiter (no collapsing)

Returns `Vec<(String, bool)>` where bool indicates if the field was fully quoted (skips glob).

- [ ] **Step 2: Write word splitting tests**

```rust
#[test]
fn word_split_default_ifs() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("a  b  c")).unwrap();
    let fields = expand_word("$X", &mut env).unwrap();
    assert_eq!(fields, vec!["a", "b", "c"]);
}

#[test]
fn word_split_custom_ifs() {
    let mut env = Env::empty();
    env.set("IFS", mash::env::Variable::string(":")).unwrap();
    env.set("X", mash::env::Variable::string("a:b:c")).unwrap();
    let fields = expand_word("$X", &mut env).unwrap();
    assert_eq!(fields, vec!["a", "b", "c"]);
}

#[test]
fn word_split_empty_ifs_no_split() {
    let mut env = Env::empty();
    env.set("IFS", mash::env::Variable::string("")).unwrap();
    env.set("X", mash::env::Variable::string("a b c")).unwrap();
    let fields = expand_word("$X", &mut env).unwrap();
    assert_eq!(fields, vec!["a b c"]);
}

#[test]
fn word_split_quoted_no_split() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("a b c")).unwrap();
    let fields = expand_word("\"$X\"", &mut env).unwrap();
    assert_eq!(fields, vec!["a b c"]);
}

#[test]
fn word_split_literal_no_split() {
    let mut env = Env::empty();
    let fields = expand_word("hello world", &mut env).unwrap();
    assert_eq!(fields, vec!["hello", "world"]);
}

#[test]
fn word_split_dollar_at_separate_fields() {
    let mut env = Env::empty();
    env.set_positional_params("sh", &["a".into(), "b b".into(), "c".into()]);
    let fields = expand_word("\"$@\"", &mut env).unwrap();
    assert_eq!(fields, vec!["a", "b b", "c"]);
}
```

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/expander.rs crates/mash/tests/expander.rs
git commit -m "feat(mash): word splitting — IFS-based field splitting with sentinel awareness"
```

---

## Task 5: Pathname Expansion (Glob)

**Files:**
- Modify: `orix/malt/crates/mash/src/expander.rs`
- Modify: `orix/malt/crates/mash/tests/expander.rs`

Replace the `expand_pathname` placeholder with real glob matching using the `glob` crate.

**Reference:** `vexil-shell/src/expander.rs` lines 2800-3068.

- [ ] **Step 1: Implement expand_pathname**

Key behavior:
- Check if field contains unquoted glob chars (`*`, `?`, `[`)
- Characters inside `\u{E001}` sentinels are NOT glob chars — escape them as `[*]`, `[?]`, `[[]` for literal matching
- Call `glob::glob()` with the pattern
- If matches found → return sorted matches
- If no matches → return original pattern (POSIX default)
- Strip sentinels from the pattern before globbing

- [ ] **Step 2: Write glob tests**

```rust
#[test]
fn glob_no_metachar_unchanged() {
    let mut env = Env::empty();
    let fields = expand_word("hello", &mut env).unwrap();
    assert_eq!(fields, vec!["hello"]);
}

#[test]
fn glob_noglob_skips() {
    let mut env = Env::empty();
    env.options_mut().noglob = true;
    let fields = expand_word("*.rs", &mut env).unwrap();
    assert_eq!(fields, vec!["*.rs"]); // Not expanded
}

#[test]
fn glob_no_match_returns_pattern() {
    let mut env = Env::empty();
    let fields = expand_word("*.nonexistent_extension_xyz", &mut env).unwrap();
    assert_eq!(fields, vec!["*.nonexistent_extension_xyz"]);
}

#[test]
fn glob_quoted_star_literal() {
    let mut env = Env::empty();
    let fields = expand_word("'*.rs'", &mut env).unwrap();
    assert_eq!(fields, vec!["*.rs"]); // Quoted — literal
}
```

Note: Real glob tests that match filesystem entries need a tempdir with known files. The implementer should create a tempdir, write test files, `cd` into it, and verify `*` matches.

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/expander.rs crates/mash/tests/expander.rs
git commit -m "feat(mash): pathname expansion — glob matching with sentinel-aware quoting"
```

---

## Task 6: Heredoc Expansion + Integration Tests

**Files:**
- Modify: `orix/malt/crates/mash/src/expander.rs` (if heredoc needs fixes)
- Modify: `orix/malt/crates/mash/tests/expander.rs`

- [ ] **Step 1: Write heredoc and integration tests**

```rust
#[test]
fn heredoc_expands_vars() {
    let mut env = Env::empty();
    env.set("NAME", mash::env::Variable::string("world")).unwrap();
    let result = expand_heredoc_body("hello $NAME\n", &mut env).unwrap();
    assert_eq!(result, "hello world\n");
}

#[test]
fn heredoc_quotes_literal() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    let result = expand_heredoc_body("it's \"here\" $X\n", &mut env).unwrap();
    assert_eq!(result, "it's \"here\" val\n");
}

#[test]
fn full_pipeline_mixed() {
    let mut env = Env::empty();
    env.set("USER", mash::env::Variable::string("alice")).unwrap();
    env.set("HOME", mash::env::Variable::string("/home/alice")).unwrap();
    let result = expand_word_nosplit("Welcome $USER to $HOME", &mut env).unwrap();
    assert_eq!(result, "Welcome alice to /home/alice");
}

#[test]
fn mixed_quoting_and_expansion() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("world")).unwrap();
    let result = expand_word_nosplit("'hello' \"$X\"", &mut env).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn nested_parameter_default() {
    let mut env = Env::empty();
    env.set("FALLBACK", mash::env::Variable::string("default")).unwrap();
    let result = expand_word_nosplit("${X:-${FALLBACK}}", &mut env).unwrap();
    assert_eq!(result, "default");
}

#[test]
fn arith_in_word() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("count=$((5 + 3))", &mut env).unwrap();
    assert_eq!(result, "count=8");
}
```

- [ ] **Step 2: Run full test suite**

Run: `cd orix/malt && cargo test -p mash`
Expected: All lexer + parser + env + expander tests pass.

Run: `cd orix/malt && cargo test --workspace`
Expected: All 266+ tests pass, no regressions.

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add crates/mash/tests/expander.rs
git commit -m "test(mash): expander integration tests — heredoc, mixed quoting, nested expansion"
```

---

## Verification

After all tasks:

1. `cargo test -p mash` — all tests pass (lexer 78 + parser 91 + env 31 + expander new)
2. `cargo test --workspace` — 266+ tests, 0 failures
3. `cargo clippy -p mash -- -D warnings` — clean
4. Parameter expansion: all `${var...}` operators tested with set/unset/empty states
5. Arithmetic: precedence, variables, assignment, hex/octal, ternary, error cases
6. Tilde: `~`, `~+`, `~-` expand correctly
7. Word splitting: default IFS, custom IFS, empty IFS, `$@` boundaries
8. Glob: noglob skips, quoted chars literal, no-match returns pattern
9. Command substitution stub returns error (not panic)
