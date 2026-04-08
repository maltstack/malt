# Agents Status (2026-04-08)

## Current Focus
- Stabilize `mash` shell correctness with evidence-first checks.
- Continue Modernish compatibility after daily-driver shell behavior is stable.

## Verified Recent Improvements
- `for var; do ...; done` iterates positional parameters.
- Double-quoted backslash-newline continuation behavior fixed.
- `[[` / `]]` handling improved in parser/lexer and case-pattern parsing.
- Pipeline-fed while/read regression fixed:
  - `printf 'one\n' | while read l; do echo "$l"; done` now matches `dash`.
- Redirected `read` now works in compound commands and shell functions:
  - `while IFS='' read -r line; do ...; done < file`
  - `f() { IFS='' read -r line; ...; }; f < file`
- `command not found` now respects stderr redirection:
  - `PATH=/dev/null missing-command 2>/dev/null` stays silent while returning `127`.
- Interactive output flushing improved:
  - `stdout`/`stderr` are now explicitly flushed after each command result in `main.rs`.

## Current Known Gaps
- The current WIP worktree is below the last green Smoosh checkpoints:
  - Windows fresh run: `174/183` passed, `9` shell failures, `3` policy skips.
  - WSL fresh run: `177/186` passed, `9` shell failures, `0` skips.
  - Shared failing Smoosh tests in both environments:
    - `builtin.eval.trap`
    - `builtin.trap.supershell`
    - `parse.emptyvar`
    - `semantics.-h.nonposix`
    - `semantics.backtick.ppid`
    - `semantics.escaping.quote`
    - `semantics.subshell.redirect`
    - `semantics.var.dashu`
    - `sh.env.ppid`
- Modernish upstream now gets past the earlier preload/harden relaunch failures, but still breaks later on alias-like grammar macros:
  - Repro lane: `target/debug/mash /home/mamuk/work/.modernish_upstream/bin/modernish --test -eqq`
  - Current visible failure shape includes:
    - `trap: ZERR: invalid signal specification`
    - `LOOP: command not found`
    - `DO: command not found`
    - `undefined variable: testspec`
  - Current hypothesis: alias expansion is happening too late for grammar-introducing Modernish macros such as `LOOP` / `DO` / `DONE`.
- `readonly` diagnostic text differs from `dash` wording (exit code matches).

## Environment Notes
- WSL PATH now includes a short `mash` command via:
  - `~/.local/bin/mash -> /mnt/c/Users/mamuk/projects/orix/malt/target/debug/mash`
- For current Modernish debugging, prefer the fresh build directly:
  - `/mnt/c/Users/mamuk/projects/orix/malt/target/debug/mash`
- `~/.local/bin/mash-local` may be stale and should not be treated as the authoritative WSL probe binary unless it has been refreshed.
- Last known green conformance checkpoints:
  - `ad9de8f` - default Windows target lock issue resolved; continue Modernish from WSL.
  - `5748221` - WSL `186/186`, Windows `183/183`.

## Conformance Runbook

### Windows (PowerShell, native runner)
- Build mash:
  - `cargo build -p mash`
- Run native Smoosh runner:
  - `cargo test -p mash --test smoosh_runner`
- Optional explicit mash path:
  - `$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path`
  - `cargo test -p mash --test smoosh_runner`
- Current expected target on Windows:
  - 183 runnable Smoosh tests (3 Windows-unsupportable cases skipped by policy).
- Current WIP status on 2026-04-08:
  - `174/183` passed, `9` shell failures, `3` skips.

### Linux/WSL (bash)
- Build mash:
  - `cargo build -p mash`
- Run native Smoosh runner:
  - `cargo test -p mash --test smoosh_runner`
- Optional explicit mash path:
  - `MASH=/mnt/c/Users/mamuk/projects/orix/malt/target/debug/mash cargo test -p mash --test smoosh_runner`
- Linux target:
  - 186/186 Smoosh.
- Current WIP status on 2026-04-08:
  - `177/186` passed, `9` shell failures.

### Modernish lanes (Linux/WSL only)
- Capability probes from vendored corpus:
  - `MASH_CORRECTNESS_ENABLE=1 MASH_MODERNISH_CAP_ENABLE=1 cargo test -p mash --test correctness_runner modernish_capability_probes -- --nocapture`
- Upstream Modernish smoke (requires checkout with `install.sh`):
  - `MASH_CORRECTNESS_ENABLE=1 MASH_MODERNISH_UPSTREAM_DIR=/path/to/modernish cargo test -p mash --test correctness_runner modernish_upstream_optional_smoke -- --nocapture`
- Full correctness lane (Linux-focused differential + Modernish-cap options):
  - `MASH_CORRECTNESS_ENABLE=1 cargo test -p mash --test correctness_runner -- --nocapture`

### Notes
- `correctness_runner` Modernish/differential lanes are Linux-focused; use WSL/CI Linux.
- If interactive shell output feels delayed, ensure you are running the latest built `target/debug/mash`.

## Next Steps
1. Recover the 9-test Smoosh regression cluster first, starting with trap/PPID/subshell behavior (`builtin.eval.trap`, `builtin.trap.supershell`, `semantics.backtick.ppid`, `semantics.subshell.redirect`, `sh.env.ppid`).
2. Once the daily-driver baselines are green again, return to Modernish alias-aware script/source parsing for `LOOP` / `DO` / `DONE`.
3. Keep WSL `186/186` and Windows `183/183` as the green checkpoint gates; treat `ad9de8f` / `5748221` as the last known clean rollback points.
