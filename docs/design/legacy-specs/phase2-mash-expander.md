# Phase 2: `mash` Sub-Project 3 — Expander

## Goal

Build the shell expansion module — parameter expansion, arithmetic evaluation, tilde expansion, word splitting, pathname expansion (glob), and quote removal. Command substitution is stubbed (filled in by the executor sub-project).

## Architecture

Single `expander.rs` module within the `mash` crate. Sync API (no async — command substitution is stubbed). Uses Unicode private-use sentinels (`\u{E001}`–`\u{E004}`) to track quoting state through the expansion pipeline, same as the proven vexil-shell reference (183/183 POSIX conformance).

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\expander.rs` (4,097 lines). Port the expansion logic. Rewrite sync (reference was async for command substitution). Command/process substitution stubbed — executor sub-project wires them up.

---

## Public API

```rust
/// Expand a word through the full pipeline: tilde → parameter → command sub
/// → arithmetic → word split → glob → quote removal.
/// Returns multiple fields (IFS splitting + glob may produce many).
pub fn expand_word(word: &str, env: &mut Env) -> Result<Vec<String>, ExpandError>;

/// Expand without word splitting or globbing. Returns single string.
/// Used for: assignment RHS, case word, redirect targets.
pub fn expand_word_nosplit(word: &str, env: &mut Env) -> Result<String, ExpandError>;

/// Like nosplit but escapes glob metacharacters from quoted regions.
/// Used for case patterns where ${var} shouldn't break bracket expressions.
pub fn expand_word_for_case_pattern(word: &str, env: &mut Env) -> Result<String, ExpandError>;

/// Heredoc body expansion. Quotes are literal (not delimiters).
/// Only expands $var, $(cmd), `cmd`.
pub fn expand_heredoc_body(body: &str, env: &mut Env) -> Result<String, ExpandError>;

/// Evaluate arithmetic expression. Used for $((expr)) and (( expr )).
pub fn eval_arithmetic(expr: &str, env: &mut Env) -> Result<i64, ExpandError>;
```

## Error Type

```rust
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
```

---

## Expansion Pipeline (POSIX order)

Applied inside `expand_word`:

1. **Tilde expansion** — `~` → `$HOME`, `~+` → `$PWD`, `~-` → `$OLDPWD`
2. **Parameter expansion** — `$var`, `${var:-default}`, `${var#pattern}`, etc.
3. **Command substitution** — `$(cmd)`, `` `cmd` `` — **STUB: returns empty string**
4. **Arithmetic expansion** — `$((expr))`
5. **Word splitting** — Split on `$IFS`
6. **Pathname expansion** — Glob matching (`*`, `?`, `[...]`, extglob)
7. **Quote removal** — Strip sentinel markers

---

## Sentinel System

| Sentinel | Meaning | Splitting | Globbing |
|----------|---------|-----------|----------|
| `\u{E001}` | Quoted (from `'`, `"`, `\`) | NO | NO |
| `\u{E002}` | Hard field boundary (from `$@`) | FORCE | N/A |
| `\u{E003}` | Zero-words (from `${x+y}` when unset) | SKIP | N/A |
| `\u{E004}` | Literal unquoted | NO | YES |

Sentinels are inserted during expansion and stripped during quote removal. They are Unicode private-use area codepoints that never appear in normal shell input.

---

## Core Engine

`expand_string_inner(word: &str, env: &mut Env) -> Result<String, ExpandError>`

Character-by-character walk:
- `'` → wrap content in `\u{E001}` sentinels, no expansion inside
- `"` → expand `$`, backtick, `\` inside, wrap in `\u{E001}` sentinels
- `$` → dispatch: `${...}` parameter, `$((...))` arithmetic, `$(...)` command sub (stub), `$var` simple
- `` ` `` → command substitution (stub)
- `~` → tilde expansion (at word start or after `:` in assignments)
- `\` → escape next char, wrap in `\u{E001}`
- Other → pass through

---

## Parameter Expansion

All forms from the reference — full POSIX + common extensions:

**Simple:** `$var`, `${var}`, `${#var}` (length), `${!var}` (indirect)

**Default/assign/error/alt:**
- `${var:-default}`, `${var-default}`
- `${var:=assign}`, `${var=assign}`
- `${var:?error}`, `${var?error}`
- `${var:+alt}`, `${var+alt}`

The `:` distinguishes "unset" from "unset or empty".

**Pattern operations:**
- `${var#pat}`, `${var##pat}` — strip prefix (shortest/longest)
- `${var%pat}`, `${var%%pat}` — strip suffix
- `${var/pat/rep}`, `${var//pat/rep}` — replace first/all
- `${var^pat}`, `${var^^pat}` — uppercase first/all
- `${var,pat}`, `${var,,pat}` — lowercase first/all

**Substring:** `${var:offset}`, `${var:offset:length}`

**Array:** `${arr[@]}`, `${arr[*]}`, `${arr[n]}`, `${!arr[@]}`

---

## Arithmetic Evaluation

Recursive descent parser for integer arithmetic.

**Tokenizer:** Decimal, hex (`0x`), octal (`0`), binary (`0b`) literals. Variable references expanded from env.

**Operators (by precedence, low to high):**
- Ternary: `? :`
- Logical OR: `||`
- Logical AND: `&&`
- Bitwise OR: `|`
- Bitwise XOR: `^`
- Bitwise AND: `&`
- Equality: `==`, `!=`
- Comparison: `<`, `>`, `<=`, `>=`
- Shift: `<<`, `>>`
- Additive: `+`, `-`
- Multiplicative: `*`, `/`, `%`
- Exponentiation: `**`
- Unary: `!`, `~`, unary `+`/`-`, `++var`, `--var`
- Postfix: `var++`, `var--`
- Assignment: `=`, `+=`, `-=`, `*=`, `/=`, `%=`

Division by zero → `ExpandError::Arithmetic`.

---

## Tilde Expansion

- `~` → `$HOME` (falls back to `$USERPROFILE` on Windows)
- `~+` → `$PWD`
- `~-` → `$OLDPWD`
- Only at word start or after `:` in assignment values
- Result wrapped in sentinels to prevent splitting/globbing

---

## Word Splitting

Split expanded string on `$IFS` characters.

**IFS rules (POSIX):**
- Unset → defaults to space/tab/newline
- Empty → no splitting (hard boundaries from `$@` still split)
- Non-empty: whitespace IFS chars collapse/trim, non-whitespace are explicit delimiters

**Sentinel behavior:**
- `\u{E001}` regions → not split
- `\u{E002}` boundaries → force split regardless of IFS
- `\u{E003}` → produces zero fields

---

## Pathname Expansion (Glob)

**Standard glob:** `*` (any), `?` (single char), `[abc]` (char class), `[a-z]` (range), `[!abc]` (negation)

**POSIX bracket classes:** `[[:alpha:]]`, `[[:digit:]]`, `[[:alnum:]]`, `[[:lower:]]`, `[[:upper:]]`, `[[:space:]]`, etc.

**Extglob:** `?(pat)` (0–1), `*(pat)` (0+), `+(pat)` (1+), `@(pat)` (exactly 1), `!(pat)` (negation)

**Behavior:**
- Quoted chars (in `\u{E001}` regions) escaped as `[*]` for literal matching
- No matches → return original pattern (POSIX default)
- Matches sorted lexicographically
- Skipped if `noglob` (`set -f`) is active

Uses `glob` crate for filesystem matching.

---

## Command Substitution Stub

```rust
fn capture_command(_cmd: &str, _env: &mut Env) -> Result<String, ExpandError> {
    Err(ExpandError::CommandSubstitution(
        "command substitution not yet available (executor not implemented)".into()
    ))
}
```

The executor sub-project (mash sub-project 4) will replace this with real execution. The expander's public API won't change.

**Note:** Tests that involve `$(cmd)` will either skip or expect the stub error. The POSIX conformance suite (which requires execution) runs after the executor is complete.

---

## Dependencies

```toml
# Added to mash Cargo.toml
glob = "0.3"
```

No other new dependencies. The expander uses `crate::env::Env` and `crate::ast::*`.

---

## Testing Strategy

### Parameter Expansion Tests
- `$var` lookup, unset returns empty
- `${var:-default}` with set/unset/empty variable
- `${var:=assign}` assigns and returns
- `${var:?msg}` errors when unset
- `${var:+alt}` returns alt when set, empty when unset
- `${#var}` returns length
- `${var#pat}`, `${var##pat}` prefix stripping
- `${var%pat}`, `${var%%pat}` suffix stripping
- `${var/old/new}` replace first, `${var//old/new}` replace all
- `${var^}` uppercase, `${var,,}` lowercase
- `${var:2}` substring, `${var:1:3}` substring with length
- `${!var}` indirect expansion
- Nested: `${var:-${OTHER:-default}}`

### Arithmetic Tests
- Basic: `1 + 2` → 3, `10 / 3` → 3, `2 ** 8` → 256
- Precedence: `2 + 3 * 4` → 14
- Variables: `x=5; $((x + 1))` → 6
- Assignment: `$((x = 10))` → 10, variable set in env
- Hex/octal: `$((0xFF))` → 255, `$((010))` → 8
- Ternary: `$((x > 0 ? 1 : -1))`
- Division by zero → error
- Pre/post increment: `$((++x))`, `$((x++))`

### Tilde Tests
- `~` → HOME
- `~+` → PWD
- `~-` → OLDPWD
- `~/foo` → HOME/foo
- Quoted `"~"` → literal tilde

### Word Splitting Tests
- Default IFS: `"a b  c"` → `["a", "b", "c"]`
- Custom IFS: `IFS=:; "a:b:c"` → `["a", "b", "c"]`
- Empty IFS: no splitting
- `$@` hard boundary forces split

### Glob Tests
- `*` matches files in temp directory
- `?` matches single char
- `[abc]` char class
- No match → return original pattern
- `noglob` → skip expansion
- Quoted glob chars → literal

### Full Pipeline Tests
- `expand_word("'hello' $USER", env)` → single field with hello + username
- `expand_word("$HOME/bin", env)` → expanded path
- `expand_word_nosplit(...)` → no splitting even with spaces
- `expand_heredoc_body("hello $var\n", env)` → expanded, quotes literal

### Command Substitution Stub Tests
- `expand_word("$(echo hello)", env)` → returns error (stub)
- `expand_word("hello", env)` → works fine without command sub
