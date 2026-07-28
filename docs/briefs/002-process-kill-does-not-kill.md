# Brief 002 — `ProcessSupervisor::kill` does not kill anything

**Severity**: High · **Verified**: 2026-07-26 · **Source**: audit A-06

## What is wrong

`crates/malt-daemon/src/supervisor/mod.rs:73`:

```rust
pub fn kill(&mut self, pane_id: &PaneId) -> Result<(), SupervisorError> {
    self.processes
        .remove(&pane_id.0)
        .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;
    Ok(())
}
```

It removes a `HashMap` entry and returns `Ok`. **No signal is sent. The
process keeps running.** The daemon then believes it is gone, because the
only record of it was the entry just deleted.

## Why it matters

This is the same shape as the isolation defect being fixed in spec 007 — an
operation that reports success while doing nothing — but with a worse
consequence: a process the user asked to terminate keeps running with no
remaining handle to it. Every subsequent `kill` for that pane returns
`ProcessNotFound`, so there is no way to try again through the product.

It also undermines fail-closed isolation's teardown requirement (007 FR-013,
SC-006: "no process or resource it held remains, verified by inspection").
If containment is torn down while the contained process survives, the
containment guarantee ends and the process does not.

## What done looks like

- `kill` signals the process, waits bounded for exit, and escalates if it
  does not go.
- The bookkeeping entry is removed only after the process is confirmed gone,
  or the error says it could not be killed and the entry is retained so a
  retry is possible.
- A test that **observes the process is gone** — by PID, from outside — not
  one that asserts `kill` returned `Ok`. The current defect passes that test
  today.

## Gotchas

- **Termination is platform-specific**, so it belongs in `malt-platform`
  (Invariant II). `malt-platform` already has process and isolation
  machinery; check what exists before writing new OS calls — twelve of its
  isolation modules turned out to have no callers, and the same may be true
  here.
- **Kill the tree, not the process.** A shell that spawned children leaves
  them orphaned otherwise. Where a session has a Job Object, closing it is
  the correct mechanism and is stronger than signalling the leader.
- Coordinate with 007: session teardown and process kill overlap. Doing this
  independently risks two mechanisms for one job — the pattern this codebase
  keeps producing.

---

## Tasks (added 2026-07-28, for handoff)

**This got worse on 2026-07-28 and the brief predates it.** `PtyProcess` now
owns the pty slave (`docs/briefs/007`), so removing a map entry closes the
pty. `kill()` today therefore: removes the entry → drops `PtyProcess` → closes
the master *and* slave → **and the child keeps running**, now with its stdio
pointing at a pty with no master, so it takes `SIGHUP`/`EIO` on its next
write. It was a silent no-op; it is now a no-op that also breaks the process's
terminal.

**What you have to work with**: `malt_platform::signals::send_signal(pid,
SignalKind)` is cross-platform and re-exported from
`crates/malt-platform/src/signals/mod.rs:20,25`. `SignalKind` comes from
`malt_protocol::common`. Its Windows arm emulates signals by terminating with
exit code 128+signum; its Unix arm sends the real signal. Both were given
real-process tests on 2026-07-28.

- [ ] T001 Make `ProcessSupervisor::kill` (`crates/malt-daemon/src/supervisor/mod.rs:73`) actually terminate the process before removing it from `self.processes`. **Order matters**: signal first, then reap, then remove — removing first drops `PtyProcess` and closes the pty out from under a still-running child.
- [ ] T002 Decide and record whether `kill` is graceful (`Term`, wait, escalate) or immediate. The brief does not settle this and the choice is visible to callers. If you escalate, bound the wait — a `kill` that can block indefinitely is a different bug.
- [ ] T003 Handle the already-exited case without treating it as failure. A process that exited between the caller's decision and the signal is the outcome the caller wanted; `ProcessNotFound` is right only when the pane was never tracked.
- [ ] T004 Add a test that spawns a **real long-running process**, kills it, and asserts the OS no longer has it — by observing the process, not by `kill` returning `Ok`. **The defect this prevents**: the current implementation returns `Ok` and passes any test that only checks the return value. That is precisely why this survived. Use the platform-appropriate long-runner: `sleep 30` on Unix, `ping -n 30 127.0.0.1` on Windows (`crates/malt-platform/tests/signals.rs` has a helper doing exactly this split).
- [ ] T005 Add a test that killing a pane with a pty does not leave the reader thread blocked, and that `check_exited` still behaves for processes that exit on their own. **The defect this prevents**: `check_exited`'s removal is now load-bearing for pty teardown (brief 007's recorded decision) — if `kill` and `check_exited` disagree about who removes an entry, threads leak.
- [ ] T006 Gates: `cargo test --workspace` (needs `MASH` set), `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`. Verify on Linux via `bash scripts/wsl-mirror.sh`. **Smoosh does not apply** — `mash` is untouched. **macOS is not a target** (ADR-0006).
