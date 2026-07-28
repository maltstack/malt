# Brief 002 — `ProcessSupervisor::kill` does not kill anything

**Severity**: High · **Verified**: 2026-07-26 · **Status**: Resolved 2026-07-28 · **Source**: audit A-06

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

## Tasks (completed 2026-07-28)

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

- [x] T001 `kill` now signals, reaps, then removes; failure leaves the `ManagedProcess` and its PTY handles intact for a retry.
- [x] T002 Policy: graceful `TERM` for 500 ms, then force termination and a 2-second reap deadline. Unix escalates to `SIGKILL` for the PTY process group; Windows uses bounded `taskkill /T /F` for the process tree.
- [x] T003 `kill` reaps an already-exited tracked child as success. `ProcessNotFound` remains only for an untracked pane.
- [x] T004 `crates/malt-daemon/tests/supervisor.rs` kills a real `sleep`/`ping` process and confirms its PID is no longer live. The Windows force-termination path is separately exercised in `crates/malt-platform/tests/signals.rs`.
- [x] T005 The PTY reader test blocks in a real read, kills the pane, and observes EOF/EIO; it also verifies `check_exited` reaps a self-exiting process. `ManagedProcess` now retains the Unix slave through reaping, preserving the ownership required by brief 007.
- [x] T006 Passed on Windows: `cargo build --workspace`, `cargo test --workspace` with `MASH` set, `cargo fmt --all -- --check`, and `cargo clippy --workspace --all-targets -- -D warnings`. The targeted supervisor suite also passed on Linux with artifacts stored under the Linux home directory. **Smoosh does not apply** — `mash` is untouched. **macOS is not a target** (ADR-0006).
