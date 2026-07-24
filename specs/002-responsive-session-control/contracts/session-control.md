# Contract: Responsive Session Control During Execution

## Scope

This contract defines observable behavior while a session has a long-running
command. It preserves existing success payloads and wire schemas. Only explicit
temporary execution-admission errors are added.

## Session responsiveness

For a healthy local daemon under normal load, an active command MUST NOT by
itself delay any of these operations beyond one second:

| Operation | Busy-session result |
|---|---|
| VNP client registration/attach | Existing consistent `InitialState`, composed from latest finalized command output and current control-owned state |
| VNP client unregister/detach | Client removed promptly; command continues |
| Styled output observation | Latest finalized styled snapshot |
| Plain-text output observation | Latest finalized plain-text snapshot |
| Resize | Accepted and applied by the control actor |
| Frame acknowledgement | Accepted and applied by the renderer state |
| Ordinary key/input event | Evaluated by the editor/control path; no claim of delivery to the active process |
| Persistence snapshot | Existing `PersistedSession` built from finalized presentation/lifecycle state and cached `PWD`/`SHELL` |
| Shutdown request | Intake closes and shutdown intent is acknowledged promptly; active-command cancellation is not implied |

A busy, healthy session MUST NOT be reported as `SessionUnreachable`.
Lifecycle errors remain accurate if the session is genuinely missing, dormant,
closing, or otherwise unhealthy.

## Execution admission

All command-producing paths share one session-local order:

1. Gateway command execution.
2. The existing Gateway `send_input` top-level-command behavior.
3. Direct session `WriteInput`.
4. A complete command line accepted by the editor.

Simultaneous submissions are ordered by the execution ingress's serialized
acceptance order. The configured capacity is 256 waiting requests by default,
plus one active request. Admission is nonblocking.

| Condition | Admission result |
|---|---|
| Accepting and pending queue has capacity | Accepted exactly once |
| Pending queue full | `ExecutionQueueFull { session_id, capacity }` |
| Session shutdown intake closed | `SessionShuttingDown(session_id)` |
| Execution worker integrity lost | `ExecutionUnavailable(session_id)` |

Rejected requests are never executed, do not consume an internal command ID,
and cannot appear later after capacity becomes available.

## Ordered execution and finalization

For every accepted request:

1. One worker begins it in accepted FIFO order.
2. No other command mutates that session's `mash::Env` concurrently.
3. The command observes all shell-state changes finalized by its predecessor.
4. Normal output, error output, and shell state use the existing MASH behavior.
5. The control actor fully materializes the command result and updates its
   stable shell projection.
6. The initiating result contract completes.
7. Only then may the next command begin.

Parse errors, nonzero exits, and ordinary execution failures are command
results. They do not close the queue or make controls unavailable.

## Gateway HTTP compatibility

Successful request and response bodies remain unchanged. The existing
synchronous command-result wait, including its 30-second timeout behavior,
remains unchanged; a timed-out caller does not cancel the accepted command.

New error mappings use the existing JSON envelope:

```json
{
  "ok": false,
  "error": {
    "code": "execution_queue_full",
    "message": "session 7 execution queue is full (capacity 256)"
  }
}
```

| Daemon outcome | HTTP status | Stable error code |
|---|---:|---|
| `ExecutionQueueFull` | 503 Service Unavailable | `execution_queue_full` |
| `ExecutionUnavailable` | 503 Service Unavailable | `execution_unavailable` |
| `SessionShuttingDown` | 409 Conflict | `session_shutting_down` |

HTTP 429 and `rate_limited` remain reserved for Gateway authorization/rate
policy. Gateway authorization enforcement remains the delivery prerequisite
from ADR-0003 and is not changed by this feature.

## VNP compatibility

No VNP message or schema changes are permitted.

- Attach still returns the existing `InitialState`.
- Resize, key event, and frame acknowledgement retain their existing message
  types and bitpack encoding.
- A resize accepted during execution is reflected in subsequent session views
  without exposing partial command output.
- Render messages retain their existing types.
- No execution ID, queue-state message, intermediate-output message, or new
  error reply is added.

If an accepted editor line cannot enter execution because the queue is full,
closing, or unavailable, the daemon writes one stable diagnostic through the
existing session output surface. It MUST NOT silently discard the line or
invent an ad-hoc VNP response.

## Persistence compatibility

`PersistedSession`, `PersistedPane`, and `schemas/persist/session.vexil` remain
unchanged.

Snapshots during active execution:

- contain the latest finalized session, layout, and presentation boundary;
- use cached finalized `PWD` and `SHELL`;
- do not read, clone, or lock the in-flight `mash::Env`;
- do not persist active or pending executions, internal command IDs, execution
  results, or a full environment snapshot.

## Worker failure

An unexpected worker panic or execution-channel integrity failure has these
observable effects:

- attach, output, resize, acknowledgement, snapshot, detach, and shutdown stay
  serviceable;
- the active request and every already accepted pending request receive exactly
  one `ExecutionUnavailable` terminal result;
- later execution admission immediately returns `ExecutionUnavailable`;
- the worker is not automatically recreated from partial cached state.

A dropped initiating reply receiver does not count as worker failure and does
not cancel the command.

## Detach, dormancy, and shutdown

### Last VNP client detaches

- If execution is idle and the pending queue is empty, the existing snapshot
  and Dormant transition proceed.
- If a command is active, output is finalizing, or accepted work is pending,
  detach completes promptly and the session remains Active with zero VNP
  clients.
- Detach never cancels or loses accepted work.
- Automatic dormancy after the busy work later completes is not added by this
  feature.

### Explicit session shutdown

- New execution admission closes immediately.
- Queued but not started requests receive `SessionShuttingDown`.
- Shutdown intent is acknowledged promptly.
- An active command may finish and finalize naturally.
- Thread joining/reaping occurs outside the coordinator's global mutex.
- A nonterminating active command cannot block controls or work in another
  session; cancellation remains outside scope.

### Daemon shutdown

The daemon persists the last finalized snapshot and does not wait indefinitely
for an uncancellable active command. Process exit remains the final cleanup
boundary.

## Cross-session isolation

Execution order, queue capacity, worker health, output finalization, and
lifecycle waiting are session-local. A long or nonterminating command in one
session MUST NOT delay attach, observation, control, or command admission in
another healthy session.
