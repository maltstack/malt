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

## macOS confirms it, from the other end of the pipe (added 2026-07-28)

The brief originally said macOS was untested and must not be recorded as
verified without running it. CI has now run it, and it fails — differently,
which is itself the confirmation:

```
spawn_and_check_exit  (crates/malt-daemon/tests/supervisor.rs)
assertion failed: matches!(state, ProcessState::Exited(0))
```

That test spawns `/bin/echo hello` through `ProcessSupervisor::spawn`, the one
caller of `spawn_with_pty`. It passes on Linux and fails only on macOS.

The mechanism is the same inverted fd arrangement seen from the child's side
rather than the parent's:

| | Who touches the pty | What the OS does with no slave open |
|---|---|---|
| **Linux** | parent *reads* the master | `EIO`, zero bytes — output never arrives |
| **macOS** | child *writes* to the master | `SIGPIPE`, so the child dies |

`crates/malt-platform/src/process/unix.rs:161` maps `Signaled(sig)` to
`128 + sig`, so a `SIGPIPE` death surfaces as exit code **141** rather than 0.
BSD-derived kernels raise `SIGPIPE`/`EIO` for a write to a master with no
slave; Linux tolerates it, buffering into a slave side nobody reads.

**Still a prediction, not a measurement**: the assertion did not report the
observed code. It has been changed to print the state and name 141 explicitly
(`supervisor.rs`), so the next macOS CI run either confirms the mechanism or
falsifies it. Do not write "confirmed 141" here until a run says so.

**Consequence for the fix:** both symptoms come from the same line, so fixing
the fd arrangement should close this test and un-ignore the compat-pane one
together. If it closes only one, the diagnosis was incomplete.

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

## Decision: the parent retains the slave (settled 2026-07-28)

The brief asked for this to be chosen deliberately rather than fallen into.
Both options are defensible and the failure modes are opposite:

| | Parent closes slave after spawn | **Parent retains slave (chosen)** |
|---|---|---|
| Short-lived child | output can be lost — last slave closes at child exit, and Linux may return `EIO` instead of the buffered bytes | output always readable |
| EOF | arrives naturally when the child exits | only when the parent drops its slave |
| Failure mode | silent data loss | a reader thread blocked forever if nothing drops the pty |

**Chosen: the parent retains it**, because the losing case for the alternative
is exactly what this feature has to do —
`restore_compat_pane_relaunches_process_and_forwards_real_output` spawns
`/bin/echo`, which exits before anyone can read, and asserts the marker reaches
the grid. A design whose weakness is "loses the output of short-lived
processes" cannot pass that test for the right reason.

The cost is the blocked-reader risk, which is real: an unjoined thread waiting
on an fd nobody closes is precisely how
`docs/findings/2026-07-27-elevate-build-lock-and-teardown.md` began. It is
answered by making ownership explicit rather than by hoping:

- `PtyProcess` owns the slave; dropping it closes the slave.
- `ProcessSupervisor::check_exited` already removes a reaped process from its
  map, which drops `PtyProcess` — so the slave closes when the child is reaped.
- The reader treats `EIO` as end-of-stream, so it exits at that point instead
  of logging an error.

That chain is the whole safety argument: **if `check_exited` stops removing
reaped processes, reader threads leak.** Anyone changing that function should
know it is load-bearing for pty teardown.

## What done looks like

- `open_pty` retains the slave and `spawn_with_pty` gives it to the child as
  stdin/stdout/stderr; the parent keeps the master.
- The child calls `setsid()` and `TIOCSCTTY` before exec, so it has a real
  controlling terminal — otherwise job control and terminal-aware programs
  still misbehave even once bytes flow. **Note this needs work in
  `malt-platform::process`**: `SpawnConfig` exposes no `pre_exec` hook to
  callers today (it uses one internally for `setpgid`), so this half of the
  fix is not reachable from `pty/spawn.rs` as things stand.
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
- **macOS is affected too, via `SIGPIPE` rather than `EIO`** — see the section
  above. The original form of this gotcha said macOS was untested; CI has since
  run it and it fails. The exact exit code is still unmeasured.
- The restored compat process is also not placed in the session's isolation
  job object — already noted in `coordinator.rs:1217-1223` and
  `docs/BACKLOG.md`. Separate gap; do not bundle it into this fix.
