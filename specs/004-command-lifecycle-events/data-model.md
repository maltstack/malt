# Data Model: Command Lifecycle Event Delivery

**Feature**: 004-command-lifecycle-events | **Date**: 2026-07-25

## Entities

### LifecycleEvent (NEW — `crates/malt-daemon/src/executor/events.rs`)

One occurrence in a command's life, plus the stream-control events needed to keep a subscriber honest about what it has and has not seen.

| Field | Type | Meaning |
|-------|------|---------|
| `sequence` | `u64` | Monotonic per-session position, starting at 1. The resume token (FR-006). Assigned by the `EventLog` at publish time, never reused within a session's lifetime. |
| `kind` | `LifecycleEventKind` | Which event this is (below). |

```rust
enum LifecycleEventKind {
    CommandStarted { command_id: u32, cmd: String, started_at: u64 },
    CommandFinished { command_id: u32, exit_code: i32, finished_at: u64, duration_us: u64 },
    Gap { missed_from: u64, missed_through: u64, reason: GapReason },
}

enum GapReason { RetentionExceeded, SubscriberLagged }
```

`CommandStarted`/`CommandFinished` mirror the schema messages in `schemas/shell.vexil` field-for-field, plus timestamps (epoch ms, matching the convention feature 003 established). `duration_us` is derived at finish time from the matching start, satisfying the existing schema shape without a second clock read.

**`Gap` is not a schema message and is deliberately not one.** It describes the *stream*, not the session — it says "this connection missed events," which is a per-subscriber fact, not something that happened in the shell. It never enters the `EventLog`; it is synthesized per subscriber at the moment the loss is detected.

### EventLog (NEW — per session, owned by the control actor)

The bounded catch-up window.

| Field | Type | Meaning |
|-------|------|---------|
| `entries` | `VecDeque<LifecycleEvent>` | Retained events, oldest first. Capped at `MAX_RETAINED_EVENTS` (1024). |
| `next_sequence` | `u64` | Next position to assign; starts at 1. |

- Publishing appends and evicts oldest-first at the cap — so `entries.front().sequence` is the oldest replayable position.
- `replay_after(seq)` returns everything with `sequence > seq`, plus a flag indicating whether `seq` was older than the oldest retained entry (which makes the caller synthesize a `Gap` first — FR-008).
- **Not persisted.** Lost on daemon restart by design; command history covers durability (spec Assumptions). A subscriber resuming across a restart gets a `Gap`, which is the honest answer.

### SubscriberSink (NEW — one per live subscription)

| Field | Type | Meaning |
|-------|------|---------|
| `id` | `u64` | Subscriber identity, for unsubscribe and diagnostics. |
| `tx` | `tokio::sync::mpsc::Sender<LifecycleEvent>` | Bounded at `SUBSCRIBER_BUFFER` (256). Written with `try_send` only — never `send().await`, never `blocking_send` (R3, R5). |
| `last_sent` | `u64` | Highest sequence successfully delivered; the basis of a `Gap`'s range if this sink lags. |

**State transitions**:

```
Active --try_send succeeds--> Active   (last_sent advances)
Active --try_send returns Full--> Lagged
Lagged --emit Gap{SubscriberLagged}, close channel--> Dropped
Active --receiver closed (client disconnected)--> Dropped
```

Invariant: a sink is never grown to accommodate a slow reader, and the control actor never awaits one. A `Dropped` sink is removed from the session's sink table on the next publish, satisfying FR-011 for both clean and unclean disconnects — an unclean disconnect closes the receiver, which `try_send` reports on the next event.

### EventSubscription (NEW — Gateway-facing handle, `crates/malt-gateway/src/backend.rs`)

What `GatewayBackend::subscribe_events` hands back to the route.

| Field | Type | Meaning |
|-------|------|---------|
| `rx` | `tokio::sync::mpsc::Receiver<LifecycleEventDto>` | The stream the SSE handler drains. |

`LifecycleEventDto` is the JSON-serializable projection (see [contracts/event-stream.md](contracts/event-stream.md)). Keeping a distinct DTO at the Gateway boundary — rather than serializing the daemon type directly — preserves the existing layering: `malt-gateway` does not depend on `malt-daemon` (the dependency runs the other way, through the trait).

## Relationships

```
SessionExecutor (control actor)
  ├── EventLog                 1 per session   (bounded, sequence authority)
  └── Vec<SubscriberSink>      N per session   (bounded each, independent positions)

ExecutionStarted   handler ──publish──> EventLog ──fan-out (try_send)──> SubscriberSink*
ExecutionCompleted handler ──publish──┘

GatewayBackend::subscribe_events ──> EventSubscription ──> SSE stream
```

## Validation rules

- **Ordering (FR-005)**: sequence is assigned inside the control actor's single-threaded handler, so publish order equals execution order. A `CommandFinished` always carries the same `command_id` as its `CommandStarted` and a strictly greater `sequence`.
- **Isolation (FR-004)**: each session owns its own `EventLog` and sink table; there is structurally no path for one session's event to reach another's subscriber.
- **Non-blocking (FR-009, SC-006)**: the only write operation on the publish path is `try_send`. A test must assert command execution timing is unaffected by a fully stalled subscriber, not merely that the code compiles with `try_send`.
- **Gap honesty (FR-008, FR-010, SC-007)**: every path that loses events for a subscriber — retention exceeded on resume, or sink lag — must produce a `Gap` naming the lost range. A test must force both, and assert the client observed the gap rather than a silently-shortened stream.
- **Interrupted commands (FR-014)**: a `CommandStarted` with no `CommandFinished` is a legitimate terminal state when the daemon stops. Subscribers must not be left waiting; the stream simply ends with the connection. After restart, command history (feature 003) is the authority on that command's not-confirmed-complete status — the event stream does not attempt to reconstruct it.
- **Access (FR-012, FR-013)**: unknown session → 404 *before* any stream is established; insufficient scope → 403; missing token → 401. None of these may open a connection first and fail later, since an SSE client treats an opened stream as success.
