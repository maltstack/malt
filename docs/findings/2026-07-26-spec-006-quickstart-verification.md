# Spec 006 quickstart verification — streaming command output

**Date:** 2026-07-26
**Feature:** `specs/006-streaming-command-output/`
**Daemon:** built from this worktree, HTTP on 7700 (default), no VNP port used
**Method:** a live `malt daemon`, driven by `malt` CLI calls and raw HTTP
(`curl`, and one hand-rolled `/dev/tcp` socket for the stalled-subscriber
case) — no scenario here starts from an injected value; every one starts a
real command.

This records what was actually observed running the product, matching the
practice established by 003/004/005's own quickstart-verification findings
(three of which turned up real defects that reading the code or unit tests
alone had missed). Two real, reproducible things turned up here too — one
pre-existing and out of this feature's scope, one squarely inside it — plus
one false alarm that a second, cleaner run ruled out.

## Scenario 1 (US1) — output arrives before the command ends: CONFIRMED

`malt output <ID>` (the terminal-grid text snapshot) turned out to be an
unreliable way to check this — see "Honest limits" below. Using
`malt watch <ID> --output` instead, with the checkpoints inside a single
`sleep`-timed script (to avoid inter-tool-call latency corrupting the
timing):

```
=== watch log at t=2s since exec launch (mid-sleep) ===
first
=== watch log at t=7s (should be done) ===
first
second
```

for `echo first; sleep 5; echo second`. `first` was observable while the
command was still inside its 5-second sleep; `second` only appeared after
completion. Confirmed.

## Scenario 2 (US1) — output survives a failing command: CONFIRMED

`echo produced-before-failure; sleep 2; false` — the exec call's own exit
code was 1, and `malt watch --output` had already delivered
`produced-before-failure` before the failure. Confirmed.

## Scenario 3 (US3) — an agent consumes and resumes: CONFIRMED, byte-for-byte

A 10-line, 0.4s-apart command was captured in full over
`GET /sessions/{id}/output/stream`, then a second connection with
`?resume_from=5` was opened. Decoding both SSE captures' base64 payloads and
comparing:

```
full stream bytes    == direct exec output           MATCH
resumed tail (id 6-10) == corresponding tail of full  MATCH
```

Reconnecting with `resume_from=5` replayed exactly ids 6 through 10 from the
retained backlog — no duplication, no loss, verified by content per SC-003's
own instruction, not by count.

**Honest limit:** this verifies resume-from-backlog after the fact, not a
live mid-command disconnect-and-reconnect. Simulating a genuine mid-stream
client kill from this shell proved unreliable (see below) — every attempt to
externally terminate `curl` mid-response (via `timeout`, `--max-time`, or a
closed downstream pipe) either hung the whole test past its own bound or lost
already-received bytes to buffering before they were confirmed, both of which
are properties of driving a native Windows `curl.exe` from Git Bash, not of
the daemon. The equivalent server-side code path (`subscribe_output` with a
non-`None` `resume_from`) is exercised identically whether the reconnect
follows a live disconnect or a later one, and is additionally covered
deterministically by
`resuming_after_disconnect_reproduces_the_full_output_byte_for_byte` in
`crates/malt-daemon/tests/output_stream.rs`, which does drive an actual
mid-command disconnect (in-process, not over a real socket from a shell).

## Scenario 4 (US3) — a stalled subscriber is told it lagged: CONFIRMED

A raw TCP socket (`/dev/tcp`, bash builtin) sent the SSE request and then was
never read from at all — a genuine zero-consumption stall, not a rate-limited
trickle. A healthy `curl` subscriber and a 2000-line flood command ran
concurrently.

```
raw stalled socket: total bytes received: 387938
raw stalled socket: SSE ids seen: 693
raw stalled socket: gap frame:
  id: 693
  event: gap
  data: {"from":693,"to":693,"reason":"subscriber_lagged"}
  (nothing after)
healthy subscriber event count: 2000
```

The stalled subscriber received 693 buffered events, then a `gap` frame with
`subscriber_lagged`, then nothing further (disconnected). The healthy
subscriber got all 2000. Confirmed: told it lagged, not silently dropped;
other subscribers unaffected.

**False alarm, corrected:** the first two runs of this scenario, against a
daemon that had already handled ~70 sessions' worth of earlier testing in
this same pass, showed the driving command taking 4-5.4s versus a ~1.8-2s
baseline with no subscriber — which looked like the stalled subscriber
delaying the command, a direct SC-005 violation. A clean daemon, restarted
for this reason and re-run with the identical stall methodology, showed
1.592s — *faster* than that daemon's own 2.589s baseline. The earlier
timing was noise from the heavily-used daemon process, not from the
stalled-subscriber mechanism, which is `try_send`-only by design
(`session_thread.rs`'s `publish_output`, confirmed by reading the code this
result prompted). Recorded as a corrected observation, not a filed defect —
but see the two real findings below, which this same investigation surfaced.

## Scenario 5 (US2) / Scenario 6 (US2) — attached human sees it live: NOT DIRECTLY EXERCISED

`malt attach` opens an interactive `ratatui`/`crossterm` TUI that needs a
real terminal; this pass had no interactive TTY available to drive it, so
neither scenario was run against the actual TUI. What stands in for it:

- `an_attached_client_receives_more_than_one_render_during_a_running_command`
  and `two_attached_clients_converge_on_identical_content` in
  `crates/malt-daemon/tests/output_stream.rs` exercise the same VNP-level
  mechanism (real `RegisterVnpClient`, real `RenderBatch` delivery) that the
  TUI consumes, and both pass.
- The known terminal-grid "staircase" defect (`docs/BACKLOG.md` P0) was
  independently reconfirmed live in Scenario 1's investigation (see below) —
  it predates this feature and is out of scope for it (Principle IX), but it
  means a human attaching today would see the same distorted rendering this
  defect already describes, now updating incrementally instead of only at
  the end.

**What this does not establish:** whether a human actually perceives
multiple visible updates during a running command, as opposed to the daemon
correctly emitting them.

## Scenario 7 (US4) — a built-in utility streams: CONFIRMED

```
malt exec <ID> 'cat' &
malt send <ID> 'first line\n'
malt output <ID>        -> "first line" (nothing else)
malt send <ID> 'second line\n'
malt eof <ID>
-> final output: "first line\nsecond line\n"
```

`first line` was observable via the terminal-grid snapshot before `second
line` was ever sent. Confirmed, and on a simple one-line case the terminal
grid rendered correctly (the staircase defect above did not obscure this
particular result).

**A methodology note, not a product defect:** the first attempt at this
scenario hung for 20+ seconds on `malt send`, with no daemon-side log output
at all. Root cause: the daemon process from earlier in this session had
accumulated a stray, never-cleanly-closed `malt watch --output` process from
Scenario 1 (confirmed via `Get-CimInstance Win32_Process`), and `malt stop`
did not actually terminate the daemon process (still present after `sleep 1`
and a `daemon stopped` message — worth a small, separate look, noted in
`docs/BACKLOG.md`, but not investigated further here). Force-killing both and
starting a genuinely fresh daemon made `send` return in well under a second.
`write_session_input`'s own `recv_timeout(2s)` (`coordinator.rs`) should have
bounded this regardless of what the session was doing; it is not yet
understood why the client-observed hang exceeded that bound against the
stale daemon, and is called out here rather than silently ignored.

## Scenario 8 — byte fidelity and bounded memory: PARTIALLY CONFIRMED, ONE FEATURE-SCOPED FINDING

### Byte fidelity

The quickstart's literal command
(`printf "caf\303\251\n"; printf "\377\376 binary\n"`) does not exercise this
at all: mash's `printf` octal-escape handling (a separate, pre-existing,
out-of-scope quirk documented in this feature's own task notes) turns
`\377` into the two-byte UTF-8 encoding of U+00FF, never a raw `0xFF` byte.
Worked around by writing a real fixture with genuinely invalid UTF-8
(`caf\xc3\xa9\n\xff\xfe binary\n`) via the outer shell (not mash) and `cat`-ing
it through the session:

```
SSE-streamed bytes (base64-decoded) == original fixture, byte-for-byte: MATCH
```

The new streaming path (`GET /sessions/{id}/output/stream`) is byte-perfect,
including the invalid `0xFF 0xFE` sequence. **However**, the same command's
`malt exec` / `POST /sessions/{id}/exec` JSON response is not:

```
exec output (hex):   6361 66c3 a90a efbf bdef bfbd 2062 696e 6172 790a
original fixture:     6361 66c3 a90a fffe 2062 696e 6172 790a
```

`0xFF 0xFE` became `0xEF 0xBF 0xBD 0xEF 0xBF 0xBD` — the UTF-8 replacement
character, twice. Root cause, found by reading the code this result
prompted: `command_worker.rs`'s `run_command` builds `CommandOutput` via
`String::from_utf8_lossy(&result.stdout)` — `CommandOutput.output` has been a
`String`, not bytes, since before this feature (likely since 003). This
predates spec 006 and is not a regression it introduced, but it is a directly
relevant gap: FR-011 ("delivery MUST NOT corrupt output that is not valid
text") now holds for the new streaming surface and does not hold for the
older one-shot `/exec` summary, which is the more commonly used entry point.
Filed in `docs/BACKLOG.md`.

### Volume (100 MB)

The quickstart's literal command (`yes hello | head -c 100000000`) does not
run: malt-tools' in-process `head` has no `-c` (byte-count) mode, only
`-n` — a separate, pre-existing gap, not this feature's. Substituted
`yes hello | head -n 16666667` (≈100,000,002 bytes). Result:

```
real: completed after 211.6s (session history, after the fact)
peak combined `malt.exe` working set: ~332 MB (vs. ~26 MB idle, ~2 MB baseline growth for a no-flood command)
```

The command **did complete successfully** (exit 0, confirmed via
`malt history`) — SC-004's "completes" holds. Two things do not cleanly
hold:

- **Memory was not obviously bounded during the run.** ~332 MB for the
  session, against a stated 4 MiB `OutputLog` retention bound
  (`crates/malt-daemon/src/executor/output_log.rs`), is roughly 80x that
  bound. It came back down (~219 MB shortly after completion) rather than
  staying pinned, so this is not evidence of a permanent leak — but "stays
  bounded *throughout*" (SC-004's own wording) is not established by this
  result. The likely account, not yet confirmed by profiling: 16.6 million
  separate `OutputChunk` publishes each also feed `self.compat` (the VT
  grid) and call `dispatch_render()`, and neither of those is subject to the
  4 MiB output bound.
- **The session's own control operations were unresponsive for a large part
  of the run.** `malt history 82` returned `internal error: session history
  timed out` while the flood was in progress, and again ~25 seconds later.
  **Other sessions and the daemon's own `/health`/`/sessions` calls were
  unaffected the entire time** (`malt exec 83 'echo still-alive'` returned
  immediately while session 82 was stuck) — this is isolated per-session
  backpressure, not a daemon-wide outage, which is the better of the two
  possible shapes this could have taken. `history`'s and `send_input`'s
  round-trip both funnel through the same per-session `cmd_tx`/control-actor
  queue that `OutputChunk` handling also occupies; 16.6 million
  `OutputChunk` handler invocations ahead of a `history` request in that
  queue is a plausible, not yet confirmed, explanation.

Filed in `docs/BACKLOG.md` rather than fixed here: this is squarely the
"needs a bigger rethink, write it down instead of pivoting into it"
situation AGENTS.md calls for, not a Phase 7 polish-sized change.

## Honest limits of this verification overall

- `malt output <ID>` (the terminal-grid plain-text snapshot) is not a
  reliable way to observe streaming in this environment: repeated runs
  against a long-lived session showed the pre-existing "staircase"
  cursor-position defect (`docs/BACKLOG.md` P0) rendering the same handful of
  screen rows regardless of how many times the same two-line command had
  actually run, and old content from earlier scenario attempts remained
  visible. `malt watch --output` (SSE-backed, not grid-backed) was used
  instead wherever `malt output` gave an ambiguous result, and is the
  authority for the scenario-1/2 conclusions above.
- Driving a genuine mid-stream client disconnect from Git Bash against a
  native Windows `curl.exe` was unreliable across several methodologies
  (external `timeout`/`kill`, `curl --max-time`, closing a downstream pipe)
  — either the whole test hung past its own bound, or already-received bytes
  were lost to buffering before being checked, in ways traced to the
  test tooling (signal delivery across the POSIX/Win32 boundary, and
  process-exit-triggered stdio flushing) rather than to the daemon. Where
  this mattered (Scenario 3), the equivalent behavior was verified a
  different way and cross-checked against the existing automated coverage
  that does drive a real disconnect.
- Scenarios 5 and 6 were not driven by an actual interactive TUI (see above).
- The volume scenario used `head -n` in place of the quickstart's `head -c`
  (unsupported) and was measured with polling (2-second `Get-Process`
  samples plus point checks), not continuous RSS sampling — the reported
  peak (~332 MB) is a sampled maximum, not a guaranteed true maximum.
