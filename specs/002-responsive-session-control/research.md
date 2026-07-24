# Phase 0 Research: Responsive Session Control During Execution

## Research result

All technical unknowns from the specification are resolved. The design uses
only existing workspace crates and Rust standard-library concurrency.

## Decision 1: Split control from execution, but keep one shell authority

**Decision**: Retain one responsive per-session control actor and add one
persistent execution worker. Move the session's original `mash::Env` into the
worker exactly once. The control actor owns runtime, presentation, clients,
renderer, editor, lifecycle, and the last finalized shell-state projection.

**Rationale**: `mash::execute_list` is synchronous and requires `&mut Env`.
Exclusive worker ownership preserves the current single-authority semantics
without allowing a long command to block attach, output, resize, ack, snapshot,
or shutdown handling.

**Alternatives rejected**:

- `Arc<Mutex<Env>>`: control operations that need shell state would block on
  the command for its full duration.
- Clone, execute, and merge `Env`: variables, functions, aliases, traps, jobs,
  descriptors, history, cwd, and isolation resources have no correct merge.
- One thread per command: creates unbounded threads and nondeterministic lock
  acquisition.
- Concurrent workers: violates the one-mutator shell invariant.
- Rewrite MASH as async: unnecessary scope expansion and changes shell
  semantics instead of isolating the blocking boundary.

## Decision 2: Use one bounded execution ingress for every producer

**Decision**: Gateway `exec`, the current Gateway `send_input` command path,
direct `WriteInput`, and accepted editor lines all submit to one
`ExecutionIngress`. Admission is serialized by a short mutex, ordered by an
internal submission sequence, and performed with nonblocking `try_send`.
`PoolConfig.session_channel_size` becomes the pending capacity, with its
existing default of 256.

Capacity means 256 accepted executions waiting behind at most one active
execution. A zero capacity is invalid configuration. A full queue rejects the
new request immediately; rejected requests receive no command ID and never
enter the control mailbox.

**Rationale**: An internal queue behind the current unbounded session mailbox
would still permit unbounded memory growth and would place command floods ahead
of controls before capacity is checked. A separate bounded ingress gives all
producers one observable FIFO acceptance order and preserves the control lane.

**Alternatives rejected**:

- Reuse only the unbounded `SessionCommand` channel: backpressure would be
  applied too late.
- Give each producer its own queue: there would be no single deterministic
  cross-producer order.
- Block senders until capacity becomes available: overload would make callers
  and possibly control paths wait rather than fail explicitly.

## Decision 3: Finalization is an ordering barrier

**Decision**: The worker executes one request, returns its result and stable
`PWD`/`SHELL` projection to the control actor, then waits for an
acknowledgement. Before acknowledging, the control actor:

1. applies stdout and stderr to the compatibility presentation,
2. publishes the existing output bus event,
3. refreshes the cached finalized view and stable shell metadata,
4. sends the initiating caller its existing command result or typed failure.

Only after those steps may the next accepted execution start. The worker owns
the existing per-session command ID counter and assigns an ID when execution
starts, not when admission succeeds.

**Rationale**: This makes "finalized before the next command begins" concrete.
Shell state never advances ahead of the result and presentation observers can
see.

**Alternative rejected**: Let the worker immediately take the next queue item.
That could advance `Env` before the control actor materializes the preceding
output and result.

## Decision 4: Bound high-output finalization without exposing partial output

**Decision**: Apply a completed command's buffered output on the control actor
in bounded turns, using the architecture's existing 128 KiB per-tick VT budget
as the maximum byte slice. Between slices, the actor services normal control
messages. Observation and attach use cached last-finalized command output until
all bytes and shell metadata are applied; the new command-output boundary is
published atomically at the finalization barrier. Live control-owned state such
as the accepted terminal size is composed with that cached output, so
processing a resize does not wait for command completion.

**Rationale**: Moving command execution off-thread is insufficient if feeding a
large buffered result can monopolize the control actor. Bounded slices preserve
responsiveness while the cached-view barrier preserves the specification's
explicit no-intermediate-streaming boundary.

**Alternatives rejected**:

- Feed the entire result in one actor turn: sustained high-volume output can
  recreate the same control starvation after command completion.
- Expose each slice immediately: that would implement intermediate output
  visibility, which this feature excludes.
- Parse VT outside `malt-compat`: violates the constitution.

## Decision 5: Busy observations use the last completed boundary

**Decision**: The control actor maintains:

- immutable cached command output for the latest finalized command boundary,
  composed with current control-owned terminal, layout, and client state;
- cached stable `PWD` and `SHELL` values from that boundary.

Attach, styled/plain output, and snapshot requests return from those caches
while execution or finalization is active. Persistence continues to store the
existing session/layout fields and cached `PWD`/`SHELL`; it does not persist the
active command, queue, command counter, result, or a full `EnvSnapshot`.

**Rationale**: The control actor must never lock or inspect the worker's
in-flight `Env`, and current persistence only needs `PWD` and `SHELL`. The
cached boundary is consistent, prompt, and truthful.

**Alternative rejected**: Use `Env::to_snapshot` during execution. The worker
would need to expose or lock mutable state, and the persistence schema does not
carry a full environment snapshot.

## Decision 6: Normal command failures continue; worker failures close execution

**Decision**: Parse errors, spawn failures represented by MASH, nonzero exits,
and stderr remain ordinary finalized command outcomes; the next accepted
command runs normally. The worker boundary catches unwindable panics and
reports channel failure explicitly. If worker integrity is lost:

- the control actor stays alive and observable;
- the active caller and every accepted pending caller receive exactly one
  `ExecutionUnavailable` terminal failure;
- new execution submissions fail immediately;
- the worker is not reconstructed from the incomplete cached projection.

A disconnected initiating receiver does not cancel execution or count as a
worker failure.

**Rationale**: Continuing after ordinary command failure matches shell
semantics. Continuing with a potentially corrupted `Env` after an execution
subsystem failure would invent state and violate authority.

**Alternative rejected**: Restart from cached `PWD`/`SHELL`. Those two fields
cannot reconstruct authoritative shell state.

## Decision 7: Lifecycle operations acknowledge promptly and never lose accepted work

**Decision**:

- **Last client detach while idle**: close admission, take the finalized
  snapshot, stop the worker, and transition Dormant as today.
- **Last client detach while active, finalizing, or pending**: unregister the
  client immediately, report `Busy` to lifecycle coordination, leave the
  session Active with zero VNP clients, and preserve all accepted work. This
  feature does not invent an automatic post-completion dormancy policy.
- **Explicit session shutdown**: close admission immediately, acknowledge the
  shutdown intent promptly, fail queued-not-started executions with
  `SessionClosing`, allow the uncancellable active execution to finish and
  finalize once, then reap the worker/control threads outside the coordinator's
  global mutex.
- **Daemon process shutdown**: persist the last finalized snapshot, close
  intake, and do not wait indefinitely for an active command. Process exit is
  the cleanup boundary because command cancellation is outside scope.

**Rationale**: Detach is not cancellation. Shutdown intent can be prompt even
though an already-running command cannot be synchronously stopped. Reaping
outside the global coordinator lock prevents one session from delaying every
other session.

**Alternatives rejected**:

- Current synchronous snapshot/shutdown/join on last detach: can wait behind a
  long command and invalidate accepted work.
- Drain every queued command during shutdown: arbitrarily extends shutdown and
  makes a closed session keep accepting previously queued mutation.
- Kill the active command: cancellation semantics are explicitly excluded.

## Decision 8: Add explicit overload and availability outcomes

**Decision**: Add typed daemon outcomes for:

- `ExecutionQueueFull { session_id, capacity }`;
- `ExecutionUnavailable(session_id)`;
- `SessionShuttingDown(session_id)`.

Map queue-full and unavailable to HTTP 503 with distinct stable codes
`execution_queue_full` and `execution_unavailable`. Map shutdown rejection to
HTTP 409 with `session_shutting_down`. Preserve the existing JSON error
envelope. Do not reuse HTTP 429, which remains reserved for Gateway
authentication/rate-limiter policy.

For an accepted VNP editor line, where no command-submission response schema
exists, materialize a stable diagnostic in the session's existing output
surface rather than inventing a wire response.

**Rationale**: Capacity and subsystem failure are temporary service
availability outcomes, not authentication rate-limit decisions. Stable codes
let clients distinguish them without changing success responses.

## Decision 9: Preserve external and persistence contracts

**Decision**: Do not change:

- successful Gateway execution response shapes;
- the existing 30-second Gateway synchronous result wait;
- VNP message schemas or bitpack encoding;
- session lifecycle vocabulary exposed to clients;
- `PersistedSession` or `PersistedPane`;
- public execution identity, history, or result lookup;
- requested isolation selection or enforcement boundaries.

Gateway authorization enforcement remains the preceding delivery prerequisite
from ADR-0003 and is not weakened or reimplemented here.

**Rationale**: This feature is the structural concurrency prerequisite only.
Every listed capability is either already owned by another subsystem or is an
explicitly later backlog item.

## Decision 10: Prove portability and preservation, not just thread movement

**Decision**: Verification must include:

- a compile-time `Send` assertion for `mash::Env`;
- deterministic long-running commands using MALT's portable in-process
  `sleep`;
- active-plus-pending capacity boundaries;
- cross-producer FIFO and state handoff;
- attach/output/control/snapshot/detach responsiveness;
- high-output bounded finalization;
- normal command and worker-failure behavior;
- idle and busy dormancy;
- prompt shutdown and asynchronous reaping;
- cross-session isolation;
- unchanged Gateway/VNP/persistence contracts;
- full workspace tests and native-Windows Smoosh conformance.

**Rationale**: Existing idle smoke tests cannot prove the feature. The tests
must exercise real blocking execution, real channel pressure, lifecycle
transitions, and the original shell semantics.
