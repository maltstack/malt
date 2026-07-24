# Agents Status (2026-04-10)

## Session ritual (added 2026-07-24, see ADR-0001)

This project has been abandoned-via-rewrite three times (vexil-v2 → malt →
malt-stack) after multi-day uncommitted sprints. Two habits meant to prevent
a fourth:

1. **Rebuild + retest before resuming work after any gap.** Don't trust a
   stale status doc — `cargo build --workspace && cargo test --workspace`
   (see docs/adr/ for `vexilc` PATH setup) and update the numbers below if
   they've drifted.
2. **Commit at real checkpoints, not multi-day piles.** If a change is big
   enough to feel risky to commit, that's a signal to commit sooner, not
   later. If something starts looking like it needs a bigger architectural
   rethink mid-task, write that down as a proposal (an ADR draft is fine)
   instead of silently pivoting into it.

## Test Results (evidence)

### Windows (PowerShell, commit tip)
```
cargo test -p mash --test executor   → 228 passed, 0 failed
cargo test -p mash --test env         →  34 passed, 0 failed
cargo test -p mash --test expander     →  86 passed, 0 failed
cargo test -p mash --test smoosh_runner → 183 passed, 0 failed (3 policy skips)
cargo test -p mash --bin fds           →   2 passed, 0 failed
```

### Smoosh conformance: 183/183 Windows (3 policy skips for `devfd`, `exitstatus`, `statuswait`).

## Changes Made (2026-04-10)

### 1. `#` removed from `is_word_break` (`lexer.rs`)
- **Before**: `#` was in the word-break character set, causing `echo hello#world` to tokenize as `echo`, `hello` (with `#world` silently dropped as a comment).
- **After**: `#` only starts a comment at token-start position (handled at lexer.rs:949). Inside a word, `#` is a literal character. POSIX-compliant behavior verified with Smoosh 183/183.

### 2. `${#@}` and `${#*}` return parameter count (`expander.rs`)
- **Before**: `${#@}` and `${#*}` fell through to `env.get_str(name).chars().count()`, returning the character length of the joined string (e.g., 5 for "a b c") instead of the count of positional parameters.
- **After**: Added explicit handling for `name == "@"` and `name == "*"` in the `${#VAR}` expansion code path (expander.rs:498-530), returning `env.get_str("#")` (the count of positional parameters).

### 3. Multi-pass alias expansion (`parser.rs`)
- **Before**: `preparse_expanded` ran a single pass over alias expansion, so `alias LOOP='while'; alias DO='do'; alias DONE='done'` wouldn't fully expand (LOOP wouldn't see that DO is an alias).
- **After**: Refactored into `preparse_expanded` (public, iterates up to 100 passes until output is stable) and `preparse_expanded_pass` (single-pass logic). Also expanded `ends_with_sep` to include `;|&` characters (not just whitespace) so aliases ending with command-separator tokens reset `in_command_position`.

### 4. `collect_grammar_aliases_from_script` preserves all aliases (`parser.rs`)
- **Before**: Only kept LOOP/DO/DONE aliases from the script, filtering out all others. User-defined aliases in scripts were unavailable for preparse expansion.
- **After**: Changed to call `collect_aliases_from_script` directly, preserving all aliases.

### 5. Heredoc expansion error message prefix (`executor.rs:5188`)
- **Before**: `format!("{e}\n")` — no prefix, producing stderr like `x: z`.
- **After**: `format!("mash: heredoc expansion: {e}\n")` — matches the pattern used by here-string (`mash: here-string expansion: {e}\n`) and regular redirect (`mash: redirect: {e}\n`). The test `heredoc_expansion_error_aborts_noninteractive_script` and the `redirect_error_aborts_noninteractive_shell` check both expect `"heredoc expansion:"` in stderr.

### 6. Windows env var case normalization (`env.rs:410-429`)
- **Problem**: On Windows, `std::env::vars()` returns keys with their original casing (e.g., `Path` not `PATH`). POSIX shell variable names are case-sensitive and the codebase uses uppercase names like `"PATH"`, `"HOME"`, etc. for lookups. The HashMap lookup `env.get("PATH")` wouldn't find `Path`, causing `find_in_path` to fail and pipelines like `echo hello | findstr hello` to produce empty output.
- **Fix**: In `Env::from_os()`, selected well-known environment variable names are uppercased via `key.to_ascii_uppercase()` before insertion. Only variables that POSIX shells reference by uppercase name are normalized (PATH, HOME, TEMP, TMP, COMSPEC, SYSTEMROOT, WINDIR, USERPROFILE, HOMEDRIVE, HOMEPATH, PSMODULEPATH, PATHEXT). Other variable names are preserved in their original case to maintain POSIX case-sensitivity for user-defined variables.
- **Evidence**: `echo hello | findstr hello` now outputs `hello`. `echo hello world | findstr world` outputs `hello world`. Pipeline tests `pipeline_echo_findstr` and `pipeline_filters` now pass.

## Known Issues (evidence-based)

### `builtin_read_uses_redirected_function_stdin` (1 flaky failure)
- Intermittently fails when run as part of the full test suite but passes individually.
- Likely a file system race condition on Windows with the temporary `infile` used in the test.

### Process substitution (`<(...)`, `>()`) unimplemented
- Lexer tokenizes these as `Word`, executor has no support.
- No executor code exists for process substitution.

### `{`/`}` brace tokenization context sensitivity
- Current `is_word_break` treats `{` and `}` contextually, which may need further analysis for edge cases.

## Build Notes

### Windows PowerShell
```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 183, skipped unsupported: 0

cargo test -p mash --test executor
# Expected: 228 passed, 0 failed
```

### NTFS Caching (Critical)
On WSL with NTFS-backed repo paths, cargo caching is unreliable due to NTFS mtime granularity.
Use `CARGO_TARGET_DIR=/tmp/malt-build` for builds on WSL, or `cargo clean -p mash` followed by rebuild on Windows.
Stale binary symptoms: test failures that don't match expected behavior from source changes.

### Binary Paths
- **Repo build (default):** `target/debug/mash`
- **WSL build:** `/tmp/malt-build/debug/mash`

## Last Known Green Checkpoints
- `dd7ad26` — preparse alias isolation + PSREPLACE fix (Smoosh 186/186 WSL, 183/183 Windows)
- Current tip — all executor tests pass (228/228), Smoosh 183/183 Windows