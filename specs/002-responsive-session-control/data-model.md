# Data Model: Responsive Session Control During Execution

This feature adds daemon-internal concurrency state. It does not add a VNP,
Gateway success, or persistence schema.

## 1. Session Control Actor

The single responsive authority for non-execution session behavior.

### Owned state

- `SessionRuntime`
- session bus and authority tracker
- terminal dimensions and layout
- compatibility translator
- renderer, editor, clients, and render senders
- cached `FinalizedOutput`
- cached `StableShellProjection`
- `ExecutionIngress` receiver/control
- active execution metadata, if any
- finalization state, if any
- execution availability state

### Invariants

- Never owns or locks `mash::Env`.
- Processes control operations while an execution is active.
- Exposes only fully finalized command output and shell projection, composed
  with current control-owned terminal, layout, renderer, and client state.
- Dispatches at most one request to the execution worker.
- Does not dispatch the next request until finalization is acknowledged.

## 2. Execution Ingress

The single admission point shared by every command-producing path.

### Fields

- session identity
- bounded pending sender
- admission mutex
- next internal submission sequence
- admission state: `Accepting`, `Closing`, or `Unavailable`
- configured pending capacity

### Validation

- Capacity MUST be greater than zero.
- Capacity counts waiting requests only; the active request is separate.
- A request is accepted only if its `try_send` succeeds while state is
  `Accepting`.

### Invariants

- The mutex acquisition order defines simultaneous producers' acceptance
  order.
- Accepted submission sequences are strictly increasing within one session.
- A rejected request is never executed and receives no command ID.
- Admission never blocks waiting for queue capacity.

## 3. Execution Request

A command accepted for one session.

### Fields

- internal submission sequence
- command text
- source kind: Gateway exec, Gateway send command, direct write command, or
  accepted editor line
- one-shot result sender where the source has a reply contract

### Lifecycle

```text
Submitted
  ├─ queue has room ─> Pending
  ├─ queue full ─────> Rejected(ExecutionQueueFull)
  ├─ closing ────────> Rejected(SessionShuttingDown)
  └─ unavailable ────> Rejected(ExecutionUnavailable)

Pending
  ├─ selected in FIFO order ─> Active
  ├─ shutdown ────────────────> Failed(SessionShuttingDown)
  └─ worker failure ──────────> Failed(ExecutionUnavailable)

Active
  ├─ worker returns ──────────> Finalizing
  └─ worker integrity lost ───> Failed(ExecutionUnavailable)

Finalizing
  ├─ all state/result applied ─> Completed
  └─ execution subsystem lost ─> Failed(ExecutionUnavailable)
```

## 4. Execution Worker

One persistent worker per active session.

### Owned state

- authoritative `mash::Env`
- command parser/executor invocation
- next per-session command ID
- one-job dispatch receiver
- completion/finalization acknowledgement channel

### Invariants

- `mash::Env` moves into the worker once and has one mutable owner.
- Exactly one command executes at a time.
- Command ID is assigned when a request becomes active.
- The worker waits for the control actor's finalization acknowledgement before
  accepting the next dispatch.
- An integrity failure permanently transitions execution to `Unavailable`; the
  cached shell projection is not used to recreate `Env`.

## 5. Active Execution

The request currently mutating authoritative shell state.

### Fields

- internal submission sequence
- internal command ID
- initiating result sender
- start status

### Invariants

- At most one exists per session.
- It is excluded from pending-capacity accounting.
- Initiator disconnect does not cancel it.
- It cannot overlap with finalization of a preceding command or execution of a
  succeeding command.

## 6. Execution Completion

The worker-to-control handoff after MASH stops mutating `Env`.

### Fields

- internal submission sequence and command ID
- stdout bytes
- stderr bytes
- exit outcome in the existing internal representation
- updated `StableShellProjection`
- normal result or execution-subsystem failure
- finalization acknowledgement sender

### Validation

- Completion sequence MUST match the actor's active execution.
- A completion is finalized exactly once.
- A stale, duplicate, or out-of-order completion is an integrity failure, not
  silently ignored.

## 7. Finalization State

Private control-actor state for bounded output application.

### Fields

- execution completion
- next stdout/stderr byte offsets
- maximum 128 KiB byte budget per actor turn
- staged updated shell projection
- final result sender

### Invariants

- Normal control messages are serviced between finalization turns.
- The cached finalized view is not replaced until all bytes and metadata are
  applied.
- The worker does not start the next command until finalization completes.
- Finalization does not expose intermediate output or add streaming.

## 8. Finalized Output

An immutable command-output boundary used to build internally consistent
observations together with current control-owned state.

### Fields

- latest committed command-output presentation/frame source
- styled output representation
- plain-text output representation
- finalized generation marker

### Invariants

- Attach and output requests always read this output boundary, including during
  active execution and finalization.
- Attach composes it with the actor's current terminal dimensions, layout,
  renderer, and client state; accepted control changes are not frozen behind a
  command.
- The output boundary is replaced atomically after command finalization.
- It never claims to contain intermediate command output.

## 9. Stable Shell Projection

The minimal shell metadata already required by persistence.

### Fields

- working directory (`PWD`)
- shell path (`SHELL`)

### Invariants

- Initialized before the worker starts.
- Replaced only at a completed finalization boundary.
- Snapshot requests never read the worker's in-flight `Env`.
- It is not a full shell snapshot and cannot restart a failed worker.

## 10. Session Execution Availability

### States

```text
Accepting
  ├─ explicit shutdown ─> Closing
  └─ worker failure ────> Unavailable

Closing
  └─ active execution finalized/reaped ─> Closed

Unavailable
  └─ session teardown ────────────────> Closed
```

### Rules

- `Accepting`: new work may be admitted within capacity.
- `Closing`: no new work; pending work receives `SessionShuttingDown`; active
  work may finish because cancellation is excluded.
- `Unavailable`: no new work; active and pending work receive
  `ExecutionUnavailable`; controls remain available.
- There is no automatic transition from `Unavailable` back to `Accepting`.

## 11. Dormancy and Shutdown Transitions

```text
Active session + last VNP detach
  ├─ execution idle and pending empty ─> snapshot -> worker stop -> Dormant
  └─ active/finalizing/pending ────────> remain Active, zero VNP clients

Active session + explicit shutdown
  -> close admission
  -> fail pending
  -> acknowledge shutdown intent
  -> active command finishes and finalizes, if present
  -> asynchronous reap
  -> Closed
```

Queued executions, active execution, command counters, and results are never
persisted by this feature.
