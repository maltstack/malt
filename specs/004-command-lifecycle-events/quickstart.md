# Quickstart: Validating Command Lifecycle Event Delivery

**Feature**: 004-command-lifecycle-events

Runnable scenarios proving the feature end to end. Frame shapes are in [contracts/event-stream.md](contracts/event-stream.md); field semantics in [data-model.md](data-model.md).

## Prerequisites

- `cargo build --workspace` green.
- A running daemon and its token: `cargo run -p malt-bin -- daemon --port 7700`, then `TOKEN=$(cat ~/.config/malt/api-token)`.
- `curl` with `-N` (unbuffered) — SSE looks like a hang without it.

## Scenario 1 — Live start and finish, no polling (User Story 1)

Terminal A, subscribe first — with the first-party client:

```bash
cargo run -p malt-bin -- watch 1
```

(`curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7700/sessions/1/events` shows the raw frames if you want to inspect the wire format directly.)

Terminal B, run something slow enough to see both halves:

```bash
cargo run -p malt-bin -- exec 1 "sleep 3; echo done"
```

**Expected**: a `command_started` frame appears in Terminal A **while the command is still running** (roughly 3 seconds before it finishes), then a `command_finished` frame with `exit_code: 0` and a `duration_us` near 3,000,000. Both carry the same `command_id`; the second has a higher `id`. Repeat with `false` and confirm `exit_code: 1` — never 0.

## Scenario 2 — Resume after disconnect (User Story 2)

With `malt watch` running, note the last sequence you saw, then kill it (Ctrl-C). Run two commands while disconnected:

```bash
cargo run -p malt-bin -- exec 1 "echo missed-one"
cargo run -p malt-bin -- exec 1 "echo missed-two"
```

Resume from the noted position:

```bash
curl -N -H "Authorization: Bearer $TOKEN" -H "Last-Event-ID: <noted-id>" \
  http://127.0.0.1:7700/sessions/1/events
```

**Expected**: the four missed frames (two starts, two finishes) replay in order before the stream goes live.

Also verify the *automatic* reconnect, which is the path `--resume-from` does not exercise: with `malt watch 1` running, restart the daemon underneath it. The client must reconnect on its own, resume from its highest seen sequence, and report a gap if the restart lost events — rather than exiting or silently restarting from now.

Then force the retention case — resume from a position far older than the retained window:

```bash
curl -N -H "Authorization: Bearer $TOKEN" -H "Last-Event-ID: 1" \
  http://127.0.0.1:7700/sessions/1/events
```

after more than 1024 events have accumulated. **Expected**: a `gap` frame with `reason: "retention_exceeded"` arrives **first**, naming the missed range, before any replayed events.

## Scenario 3 — A stalled subscriber cannot degrade the session (User Story 3)

Open a subscription that never reads, alongside one that does:

**Platform note**: this scenario needs a way to suspend a reader mid-stream.
`kill -STOP` is POSIX-only; the repo's primary target is native Windows, so
both forms are given.

Linux/WSL/macOS:

```bash
# Terminal A: connect, then stop reading
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7700/sessions/1/events &
CURL_PID=$!
sleep 1 && kill -STOP $CURL_PID

# Terminal B: a healthy subscriber
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7700/sessions/1/events
```

Windows (PowerShell) — suspend the process with Sysinternals `pssuspend`, or
if that is unavailable, get the same effect by attaching a subscriber and
simply not reading it (the automated test
`a_stalled_subscriber_never_exceeds_its_buffer_bound` covers this case
directly and is the reliable check on any platform):

```powershell
$p = Start-Process curl -PassThru -ArgumentList '-N','-H',"Authorization: Bearer $env:TOKEN",'http://127.0.0.1:7700/sessions/1/events'
pssuspend $p.Id     # Sysinternals; resume later with: pssuspend -r $p.Id
```

Then generate more events than the 256-entry subscriber buffer holds:

```bash
for i in $(seq 1 300); do cargo run -q -p malt-bin -- exec 1 "echo $i" >/dev/null; done
```

**Expected**: Terminal B receives every frame without delay. Command execution timing matches a no-subscriber baseline (compare with `time` against a run with no streams open). The stalled subscriber is closed rather than accumulating; on `kill -CONT $CURL_PID` it sees a terminal `gap` with `reason: "subscriber_lagged"` and end-of-stream. Daemon memory does not grow with the stalled client's backlog.

## Scenario 4 — Access control and error shapes

```bash
# No token
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:7700/sessions/1/events        # → 401
# Unknown session
curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:7700/sessions/999/events                                             # → 404
```

**Expected**: every failure is an HTTP status *before* the stream opens — never a `200` that then emits an error frame, which an SSE client would read as success.

## Automated verification

```bash
cargo test --workspace
```

Feature suites (each must drive the real path — subscribe, execute, receive, disconnect, resume — not construct event structs and assert on their fields):

- `malt-daemon/tests/events.rs`: sequence monotonicity and start/finish pairing; per-session isolation; replay after a position; gap on retention overrun; gap-then-close on sink lag; sink cleanup on unclean disconnect.
- `malt-daemon/tests/gateway_backend.rs`: end-to-end subscribe → exec → receive both events; unknown session is `NotFound` before any subscription is created; **execution timing with a fully stalled subscriber matches the no-subscriber baseline** (the SC-006 assertion).
- `malt-gateway/tests/routes.rs`: route registered; `Read` scope enforced (401/403); 404 for unknown session.

## Known caveat

The event log is in-memory and not persisted. A subscriber resuming across a daemon restart receives a `gap` rather than replayed events — deliberate (spec Assumptions); command history covers durability.
