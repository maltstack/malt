# Finding: command lifecycle event delivery against a live daemon (2026-07-25)

Validation of `specs/004-command-lifecycle-events/quickstart.md` against a
real daemon, after implementing feature 004. Recorded because the most
important thing this session produced was not the feature — it was three
bugs that only a real client could have found, two of which had passing unit
tests sitting directly on top of them.

## The case for shipping a consumer with the endpoint

Feature 004 was originally specified with no first-party client: `curl` in
the quickstart, and tests driving the backend directly. `malt watch` was
added mid-planning (research R10) specifically so the reconnect-with-resume
path would be written by somebody. That decision paid for itself
immediately.

**Bug 1 — the client received nothing at all.** `BufRead::read_line` keeps
the line terminator. The frame parser stripped `\r` but never `\n`, so
`"id: 1\n"` was parsed as the number `"1\n"` (which fails, discarding the
resume position) and the frame-terminating blank line arrived as `"\n"`,
which never matched the `is_empty()` check. No frame was ever emitted.

Eight unit tests covered that parser and all eight passed, because the test
helper fed pre-trimmed lines — input the real reader never produces. The
tests proved the parser correct against a fiction. This is precisely the
pattern AGENTS.md documents for `job_objects.rs`: a suite that passes by
construction. The helper now appends the terminator, and two regression
tests cover the bare-newline and id-with-terminator cases by name.

Worth noting how it was found: the server was verified correct with `curl`
first (frames were perfect on the wire), which localized the fault to the
client immediately instead of leaving both halves suspect.

**Bug 2 — a resuming subscriber was told it missed events it had seen.** A
sink starts with `last_sent == 0`, and the gap range was computed from it.
A client resuming from sequence 10 was therefore told the gap began at 1.
Reporting loss that never happened is as wrong as hiding loss that did —
both leave the client with a false picture. Fixed by seeding the sink's
position on resume.

**Bug 3 — a lagged subscriber could not be told it lagged.** The terminal
gap notification shared the same 256-slot buffer that had just overflowed,
so its own `try_send` failed and the subscriber was dropped *silently*,
believing its stream was complete. That is the exact SC-007 violation the
feature exists to prevent. Fixed by allocating one extra slot and reserving
it for the gap. Caught by an integration test that asserted the gap was
*received*, not merely that memory stayed bounded — an assertion of the
latter alone would have passed.

## What was verified working

Real daemon, real mash, real HTTP:

```
    1  started       1  sleep 2; echo slow
    2  finished      1  exit 0  2.0s
    3  started       2  false
    4  finished      2  exit 1  1ms
```

The start frame arrives while the command is still running (visible as the
~2s gap before its finish), exit codes are real, and `duration_us` matches
the actual runtime.

**Resume** — with nothing watching, two commands ran; `malt watch 13
--resume-from 4` then replayed exactly the four missed events (5–8) and
nothing already seen. A fresh `malt watch` with no resume position correctly
replayed nothing.

**Refusals** — `malt watch 9999` exits 1 with `session not found: 9999`
rather than retrying forever, which matters: a reconnect loop that treats a
permanent refusal as a transient blip spins indefinitely.

**Shutdown** — `malt stop` returned in 0s with a confirmed active subscriber
attached. Worth checking explicitly because `axum::serve` with graceful
shutdown waits for in-flight requests, and an SSE stream is an in-flight
request that never completes on its own; a hang here would have made `malt
stop` unusable whenever anyone was watching. It does not hang.

## Known limitation, not fixed

**Automatic reconnect after an abrupt daemon death is not verified, and is
likely slow.** When the daemon was restarted underneath a running `malt
watch`, the client did not promptly reconnect — it sat blocked in `read_line`
on a half-open socket. The reconnect loop itself is correct (a *graceful*
stream close is detected immediately and reconnects), but detecting a
vanished peer depends on the OS TCP timeout, which on Windows can be
minutes.

The clean fix is a read timeout on the streaming response, so a stream that
goes silent longer than the server's SSE keep-alive interval is treated as
dead. `reqwest` 0.13.2's blocking API does not expose `read_timeout`, and
its total-request `timeout` would tear down healthy streams too. Options are
a bounded stream lifetime with automatic resume (simple, adds a periodic
reconnect), or moving the client to the async API. Deliberately not decided
here — it is a real design choice, not a one-line fix, and belongs in its
own change. Added to `docs/BACKLOG.md`.

## Incidental

A rebuild trap worth remembering: `cargo test -p malt-bin` compiles the
binary as a test harness but does **not** relink `target/debug/malt.exe`.
Two rounds of "the fix didn't work" were actually the old binary. Worse, a
running daemon holds a lock on `malt.exe`, so the rebuild fails with
`Access is denied` unless the daemon is stopped first. Stop the daemon,
`cargo build -p malt-bin`, then restart.
