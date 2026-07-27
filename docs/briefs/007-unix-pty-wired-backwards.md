# Brief 007 — The Unix PTY hands the child the master and drops the slave

**Severity**: High · **Verified**: 2026-07-27 · **Source**: CI's Linux/macOS
advisory job, root-caused in WSL with a live repro

## What is wrong

`crates/malt-platform/src/pty/unix.rs:80-81` drops the slave fd:

```rust
// Drop slave fd — it will be passed to child process via SpawnConfig.
drop(slave);
```

It is not passed to anything. `crates/malt-platform/src/pty/spawn.rs:70-82`
gives the child **dups of the master** instead, and says so:

```rust
// On Unix, the reader/writer are duped from the master fd.
// We pass clones as the child's stdin/stdout/stderr so the child
// writes to the master, and we read from it.
let stdin_file = writer.try_clone()?;
let stdout_file = reader.try_clone()?;
let stderr_file = reader.try_clone()?;
```

`reader` and `writer` are both dups of the master (`unix.rs:72-79`). So both
ends hold masters and **nothing holds the slave** from the moment `open_pty`
returns.

That inverts the arrangement a pty requires: the parent keeps the master, the
child gets the slave. There is also no `setsid` and no `TIOCSCTTY` in the
child, so even with the right fd the child would have no controlling terminal.

**Observed consequence**, from an instrumented run in WSL (Ubuntu 26.04):

```
[ptydbg] read error: kind=Uncategorized raw=Some(5) err=Input/output error (os error 5)
```

`EIO`, on the *first* read, with **zero bytes ever read** — no `read N bytes`
line appears at all. Linux returns `EIO` on a master when no slave is open,
which here is always.

## Why it matters

**Unix compat panes have never worked.** Not intermittently, not under load —
the path cannot deliver a byte. `spawn_with_pty` has exactly one caller,
`ProcessSupervisor::spawn` (`crates/malt-daemon/src/supervisor/mod.rs:36`),
which is what restores a Compat pane, so this is the whole of the feature on
Linux and macOS.

It went unnoticed because the shape looks right and the comment reads as
deliberate. Windows takes a separate ConPTY path (`spawn_windows`) that works,
and the test's own comment records that "there is no live creation path for
Compat panes today" — so one restore test is the only thing that has ever
exercised it, and it only started running on Linux once the cross-platform
build was repaired on 2026-07-27.

This is the repo's recurring defect class in a variant worth naming: not a
mechanism that is *unwired*, but one that is **wired backwards**. Harder to
catch than the usual kind, because every reachability check passes.

## What done looks like

- `open_pty` retains the slave and `spawn_with_pty` gives it to the child as
  stdin/stdout/stderr; the parent keeps the master.
- The child calls `setsid()` and `TIOCSCTTY` before exec, so it has a real
  controlling terminal — otherwise job control and terminal-aware programs
  still misbehave even once bytes flow.
- A decision, recorded, on **when the parent closes its slave copy**. Holding
  it open keeps buffered output readable after the child exits (which is the
  bug being fixed); never closing it means the reader thread blocks forever
  instead of seeing EOF. Both halves must be chosen deliberately — an
  unjoined thread blocked on a fd that never closes is how
  `docs/findings/2026-07-27-elevate-build-lock-and-teardown.md` started.
- `spawn_pty_reader` (`crates/malt-daemon/src/executor/coordinator.rs:1493`)
  treats `EIO` as end-of-stream rather than logging it as a read error, since
  on a pty master that is what it means.
- `restore_compat_pane_relaunches_process_and_forwards_real_output` passes on
  Linux **by observing the marker text reach the grid**, not by the spawn
  call returning `Ok`.

## Gotchas

- **Do not gate the failing test to Windows.** It would read as "compat panes
  are a Windows feature" when the truth is "the Unix implementation is
  inverted", and it deletes the only evidence this gap exists. It is
  `#[ignore]`d on Unix with a pointer to this brief instead — visible, not
  hidden.
- **Do not "fix" it by treating `EIO` as EOF alone.** That silences the error
  and still delivers nothing; the output is already lost because no slave ever
  existed. The fd arrangement is the fix; the `EIO` handling is cleanup after
  it.
- **macOS is untested here.** It shares the Unix path so it is almost
  certainly affected, but the repro above is Linux only. Do not record macOS
  as verified without running it.
- The restored compat process is also not placed in the session's isolation
  job object — already noted in `coordinator.rs:1217-1223` and
  `docs/BACKLOG.md`. Separate gap; do not bundle it into this fix.
