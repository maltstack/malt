# Agents Status (2026-04-10)

## Current Focus
- Modernish compatibility: alias-aware grammar macros (LOOP/DO/DONE) and `--test -eqq` smoke.
- Smoosh conformance is GREEN (186/186 WSL, 183/183 Windows) — no regressions.

## Verified Improvements (2026-04-08 → 2026-04-10)
- `for var; do ...; done` iterates positional parameters.
- Double-quoted backslash-newline continuation behavior fixed.
- `[[` / `]]` handling improved in parser/lexer and case-pattern parsing.
- Pipeline-fed while/read regression fixed.
- Redirected `read` now works in compound commands and shell functions.
- `command not found` respects stderr redirection.
- Interactive output flushing improved (`stdout`/`stderr` flushed in `main.rs`).
- Preparse alias isolation: alias expansion is scoped to script/source entry points, not global eval.
- PSREPLACE (`${|...}`) now expands correctly in the expander.
- `&&` / `||` chaining fixed: `execute_list_node` no longer short-circuits OrIf on AndIf failure.

## Smoosh Conformance Status
**GREEN — 186/186 WSL, 183/183 Windows.** No regressions.

The "9 shared failing Smoosh tests" listed in the 2026-04-08 AGENTS.md were never reproduced.
Verification method:
```bash
# WSL - clean build, clean run
CARGO_TARGET_DIR=/tmp/malt-clean cargo build -p mash
cp /tmp/malt-clean/debug/mash /tmp/malt-clean-debug-mash
MASH=/tmp/malt-clean-debug-mash cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 186, skipped unsupported: 0
```

The stale failure list was a phantom document state. If you see failures, rebuild first:
```bash
rm -rf target/debug/deps/libmash* target/debug/mash.d target/debug/.fingerprint/mash*
cargo build -p mash
```

## Modernish Status

### Direct Execution: WORKING
```bash
timeout 10s /mnt/c/Users/mamuk/projects/orix/malt/target/debug/mash /home/mamuk/work/.modernish_upstream/bin/modernish --test -eqq
# Exit 0 — completes successfully
```

### Capability Probes: PRE-EXISTING FAILURES (dash-specific, not mash)
The `modernish_capability_probes` test runs against `dash` as reference shell.
Some probes fail because dash lacks builtins that bash has:
- `BUG_CDPCANON.t`, `BUG_CMDOPTEXP.t`, `BUG_CMDPV.t` — dash lacks `push`, `str`, `cd -P`
- Other BUG_* probes are pre-existing dash incompatibilities, unrelated to mash changes

These failures existed before the 2026-04-08 checkpoint. They do not block mash progress.

### Upstream Smoke Test: BLOCKED ON HARNESS
The `modernish_upstream_optional_smoke` harness runs `bash -lc "yes n | install.sh -s mash"`.
The `install.sh` script relaunches itself as mash, which hangs (parse_for infinite loop — see below).
**Once the parse_for guard is applied, the smoke test should pass.**

## Critical Bug: parse_for Infinite Loop (NOT YET FIXED)

**Symptom:** `mash bin/modernish --version` hangs (infinite loop in parser).

**Root cause:** The `in words...` collection loop in `parse_for` (parser.rs lines 595-608) has no
span-advance guard. If the loop is entered with a token that matches `"do"` text, it breaks.
But nested `for` loops or complex body parsing can cause recursive calls that never advance.

**Fix needed:** Add a loop-count guard inside the `in words` collection loop:

```rust
loop {
    let before_span = self.peek().span;
    match &self.peek().node {
        Token::Word(span) => {
            let text = span.text(self.input);
            if text == "do" { break; }
            let s = *span;
            self.advance()?;
            ws.push(s);
            // Guard: verify we advanced
            if self.peek().span.start == before_span.start
                && self.peek().span.end == before_span.end
            {
                return Err(ParseError::Unexpected {
                    token: self.peek().node.clone(),
                    span: self.peek().span,
                });
            }
        }
        _ => break,
    }
}
```

**Status:** Identified but NOT YET applied. This is the primary blocker for Modernish smoke test.

## Environment Notes

### WSL + NTFS Caching Issue (Critical)
On WSL with NTFS-backed repo paths, cargo caching is unreliable due to NTFS mtime granularity.
File timestamps from `touch`, `sed -i`, and `patch` may all appear "stale" to cargo even when content changed.

**Always use `CARGO_TARGET_DIR=/tmp/malt-build`** for builds to bypass NTFS mtime caching:
```bash
CARGO_TARGET_DIR=/tmp/malt-build cargo build -p mash
# Binary lands at: /tmp/malt-build/debug/mash
```

**Force full recompile** when incremental build seems wrong:
```bash
rm -rf target/debug/deps/libmash* target/debug/mash.d target/debug/.fingerprint/mash*
cargo build -p mash
```

### Binary Paths
- **Repo build (default):** `target/debug/mash`
- **Tmp build:** `/tmp/malt-build/debug/mash`
- **Stale path (OLD):** `~/.targets/vexil-v2/debug/mash` — do not use
- **WSL symlink:** `~/.local/bin/mash` → `target/debug/mash` (may be stale)

### CARGO_* Environment Variables
The session's shell may inherit stale `CARGO_TARGET_DIR` from `.bashrc` or parent shell.
Current session may have: `CARGO_TARGET_DIR=/home/mamuk/work/.targets/vexil-v2` (wrong path)
This causes cargo to write builds to a non-existent directory, leaving `target/debug/mash` stale.

**Verify:**
```bash
env | grep CARGO_TARGET_DIR  # should show /tmp/malt-build or be empty
```

**In `.bashrc`:** Ensure `CARGO_TARGET_DIR` is NOT set, or set to a valid path.
The erroneous line `export CARGO_TARGET_DIR="$HOME/work/.targets/vexil-v2"` was removed.

## Last Known Green Checkpoints
- `dd7ad26` — preparse alias isolation + PSREPLACE fix (current tip, Smoosh 186/186 GREEN)
- `5748221` — WSL 186/186, Windows 183/183 (clean baseline)

## Conformance Runbook

### WSL / Linux
```bash
# Build (always use CARGO_TARGET_DIR=/tmp/... to bypass NTFS mtime caching)
CARGO_TARGET_DIR=/tmp/malt-build cargo build -p mash

# Smoosh (verify GREEN)
MASH=/tmp/malt-build/debug/mash cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 186, skipped unsupported: 0

# Modernish smoke (direct execution, works)
timeout 10s /tmp/malt-build/debug/mash /home/mamuk/work/.modernish_upstream/bin/modernish --test -eqq
# Expected: exit 0

# Modernish smoke test harness (blocked on parse_for fix)
MASH_CORRECTNESS_ENABLE=1 MASH_MODERNISH_UPSTREAM_DIR=/home/mamuk/work/.modernish_upstream cargo test -p mash --test correctness_runner modernish_upstream_optional_smoke -- --nocapture
```

### Windows (PowerShell)
```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner
# Expected: 183 runnable (3 policy skips)
```

## Next Steps
1. **Apply parse_for span-advance guard** in `parser.rs` — this unblocks the Modernish smoke test.
2. **Verify smoke test passes** after the guard is applied.
3. **Update AGENTS.md to reflect GREEN state** once smoke test passes.
4. **Then:** resume Modernish alias-grammar work (LOOP/DO/DONE alias expansion timing).
