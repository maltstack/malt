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
