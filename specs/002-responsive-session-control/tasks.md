# Tasks: Responsive Session Control During Execution

**Input**: Design documents from `/specs/002-responsive-session-control/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md,
contracts/session-control.md, quickstart.md

**Tests**: Required. The feature specification defines independent concurrency
tests and measurable timing, ordering, failure, and cross-session outcomes.
Write each story's tests first and confirm they fail for the intended reason
before implementing that story.

**Organization**: Tasks are grouped by user story. Shared worker/admission
infrastructure is foundational because every story depends on moving
synchronous MASH execution off the control actor.

**Current baseline note**: Commits `ce2dbba` and `f44bfa5` landed after the
planning artifacts were written. Full `EnvSnapshot` persistence and compat-pane
restore are therefore existing behavior that this feature must preserve. This
feature does not modify `schemas/persist/session.vexil`; worker completion must
carry the current full shell snapshot needed by the control actor.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it uses different files and has no
  dependency on another incomplete task in the same group.
- **[Story]**: Maps the task to User Story 1, 2, or 3.
- Every task names the exact file or files it changes or validates.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Re-establish the current baseline and create deterministic
concurrency-test support before changing executor ownership.

- [X] T001 Run `cargo build --workspace` and `cargo test --workspace` against `Cargo.toml`, then record the commit, toolchain, test counts, and any pre-existing failure in `docs/findings/2026-07-25-responsive-session-control.md`
- [X] T002 Add shared deadline, portable MALT `sleep`, high-output command, and polling helpers for concurrency tests in `crates/malt-daemon/tests/support/mod.rs`

**Checkpoint**: The post-plan repository baseline is recorded, and later tests
can use deterministic local commands without platform-specific shell tools.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish bounded execution admission, single-owner MASH
execution, completion acknowledgement, and coordinator ownership. No user
story can begin until this phase passes.

**⚠️ CRITICAL**: Preserve the original authoritative `mash::Env`, current
isolation context, full `EnvSnapshot`, command-result shape, and per-session
command counter while changing thread ownership.

### Foundational Tests

- [X] T003 [P] Add failing unit tests for `Env: Send`, nonzero capacity, active-excluded capacity accounting, FIFO admission sequence, nonblocking full rejection, closing/unavailable rejection, and finalization acknowledgement in `crates/malt-daemon/src/executor/command_worker.rs`
- [X] T004 [P] Add failing integration tests proving zero execution capacity is rejected without panic and default capacity is 256 waiting requests in `crates/malt-daemon/tests/coordinator.rs`

### Foundational Implementation

- [X] T005 Add non-exhaustive `ExecutionQueueFull`, `ExecutionUnavailable`, and `SessionShuttingDown` variants with session context in `crates/malt-daemon/src/error.rs`, and matching internal Gateway error variants in `crates/malt-gateway/src/error.rs`
- [X] T006 Define and validate `PoolConfig.session_channel_size` as pending execution capacity, reject zero configuration through coordinator construction, and update affected constructors in `crates/malt-daemon/src/executor/pools.rs` and `crates/malt-daemon/src/executor/coordinator.rs`
- [X] T007 Implement `ExecutionIngress`, admission state, internal submission sequence, `ExecutionRequest`, `ExecutionCompletion`, worker dispatch/ack channels, and the panic-contained single-owner MASH worker in `crates/malt-daemon/src/executor/command_worker.rs`
- [X] T008 Register and explicitly re-export the new worker/admission types without wildcard exports in `crates/malt-daemon/src/executor/mod.rs` and `crates/malt-daemon/src/lib.rs`
- [X] T009 Refactor session spawning so the original isolation-configured `mash::Env` moves once into the worker and the session thread remains a control actor in `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T010 Implement the worker-to-control completion handshake, command-ID assignment at execution start, result delivery after finalization, and transfer of the current full `mash::env::EnvSnapshot` in `crates/malt-daemon/src/executor/command_worker.rs` and `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T011 Store control sender, execution ingress, control-thread handle, and worker/reaper ownership per active session; add a typed coordinator execution-submission API and migrate basic Gateway exec/send submission to it in `crates/malt-daemon/src/executor/coordinator.rs` and `crates/malt-daemon/src/gateway_backend.rs`
- [X] T012 Run the foundational unit and integration tests for `crates/malt-daemon/src/executor/command_worker.rs`, `crates/malt-daemon/tests/coordinator.rs`, and `crates/malt-daemon/tests/gateway_backend.rs`, fixing all regressions before story work

**Checkpoint**: One worker exclusively owns each session's MASH state, accepted
commands enter one bounded ingress, and the control actor no longer executes
MASH inline.

---

## Phase 3: User Story 1 - Attach to and Observe a Busy Session (Priority: P1) 🎯 MVP

**Goal**: A healthy session remains attachable and observable during a command
that runs longer than existing request timeouts, with observations sourced from
one consistent finalized boundary.

**Independent Test**: Start a portable MALT `sleep` command lasting longer than
the old two-second output and five-second attach waits. In 100 consecutive
trials, attach and styled/plain output observation must complete within one
second, return a consistent latest-finalized view, and never report
`SessionUnreachable` because the command is busy.

### Tests for User Story 1

- [X] T013 [P] [US1] Add failing long-command tests for prompt styled output, plain-text output, snapshot response, consistent latest-finalized output, and disconnected initiating receivers in `crates/malt-daemon/tests/session_thread.rs`
- [X] T014 [P] [US1] Add failing 100-trial VNP attach-during-execution test with a one-second per-attach bound and existing `InitialState` schema assertions in `crates/malt-daemon/tests/vnp_listener.rs`
- [X] T015 [P] [US1] Add failing Gateway tests proving busy-session output stays prompt and a long command in session A does not delay attach/output in session B in `crates/malt-daemon/tests/gateway_backend.rs`

### Implementation for User Story 1

- [X] T016 [US1] Implement cached finalized command output plus current control-state composition, and build snapshots from the last finalized full `EnvSnapshot` without reading or locking the worker in `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T017 [US1] Apply completed stdout/stderr in at-most-128-KiB control-actor turns, service control messages between turns, and atomically publish the new finalized output only after all bytes and shell state are committed in `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T018 [US1] Split attach and output request initiation from reply waiting so `DaemonBackend` and the VNP listener do not retain the global coordinator mutex during bounded waits in `crates/malt-daemon/src/executor/coordinator.rs`, `crates/malt-daemon/src/gateway_backend.rs`, and `crates/malt-daemon/src/vnp_listener.rs`
- [X] T019 [US1] Run and pass the User Story 1 tests in `crates/malt-daemon/tests/session_thread.rs`, `crates/malt-daemon/tests/vnp_listener.rs`, and `crates/malt-daemon/tests/gateway_backend.rs`

**Checkpoint**: User Story 1 is independently demonstrable: a long-running
command no longer makes its healthy session appear offline. This is the MVP.

---

## Phase 4: User Story 2 - Keep Session Controls Responsive (Priority: P2)

**Goal**: Attach/detach, resize, frame acknowledgement, editor input
evaluation, snapshot, and shutdown intent continue receiving service while
execution or bounded finalization is active.

**Independent Test**: During a 60-second command, issue 100 mixed control
operations. Every operation must receive its defined acknowledgement or
observable effect within one second; input is evaluated by the editor but is
not claimed to reach the active process.

### Tests for User Story 2

- [X] T020 [P] [US2] Add failing mixed-control tests for register, unregister, resize, frame acknowledgement, ordinary key editing, snapshot, and current terminal-size reflection during a long command in `crates/malt-daemon/tests/session_thread.rs`
- [X] T021 [P] [US2] Add failing VNP integration tests for resize, frame acknowledgement, key event, and disconnect cleanup during active execution in `crates/malt-daemon/tests/vnp_listener.rs`
- [X] T022 [P] [US2] Add failing coordinator tests proving idle last-detach persists and becomes Dormant while busy/finalizing/pending last-detach returns promptly, preserves accepted work, and remains Active in `crates/malt-daemon/tests/coordinator.rs`
- [X] T023 [US2] Add failing shutdown tests proving intake closes promptly, queued work receives `SessionShuttingDown`, an active command may finish once, and joining never holds the coordinator mutex in `crates/malt-daemon/tests/coordinator.rs`

### Implementation for User Story 2

- [X] T024 [US2] Route direct `WriteInput` and complete editor lines through the shared execution ingress, keep ordinary key evaluation on the control actor, and materialize stable queue/closing/unavailable diagnostics through existing session output in `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T025 [US2] Compose accepted resize, renderer acknowledgement, client registration, and editor state changes with cached finalized command output without exposing partial execution output in `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T026 [US2] Add a nonblocking quiescence response for last-client detach; transition idle sessions to Dormant but leave active, finalizing, or pending sessions Active with zero VNP clients in `crates/malt-daemon/src/executor/coordinator.rs` and `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T027 [US2] Implement prompt shutdown acceptance, execution-intake closure, queued-request failure, last-finalized persistence, and asynchronous thread reaping outside the coordinator mutex in `crates/malt-daemon/src/executor/coordinator.rs` and `crates/malt-bin/src/daemon.rs`
- [X] T028 [US2] Run and pass the User Story 2 tests in `crates/malt-daemon/tests/session_thread.rs`, `crates/malt-daemon/tests/vnp_listener.rs`, and `crates/malt-daemon/tests/coordinator.rs`

**Checkpoint**: User Story 2 is independently demonstrable: a busy session's
control plane remains manageable, and detach/shutdown no longer wait behind
MASH execution.

---

## Phase 5: User Story 3 - Preserve Ordered, Authoritative Execution (Priority: P3)

**Goal**: All command-producing paths share one bounded FIFO order, exactly one
command mutates shell state at a time, overload is explicit, normal command
failures continue, and worker failure gives every accepted request one defined
terminal outcome without killing session controls.

**Independent Test**: With capacity one, keep one command active, accept one
pending command, and reject the next immediately. Then run 1,000
state-dependent multi-command trials from concurrent Gateway and editor
producers; every accepted command must begin once in acceptance order, observe
its predecessor's finalized state, and never overlap shell mutation.

### Tests for User Story 3

- [X] T029 [P] [US3] Add failing capacity-one, 1,000-trial FIFO/exactly-once, state-handoff, normal-failure-continuation, stderr, and monotonic-start-ID tests in `crates/malt-daemon/tests/session_thread.rs`
- [X] T030 [P] [US3] Add failing concurrent Gateway exec/send and editor-producer admission tests, including timed-out or disconnected caller non-cancellation, in `crates/malt-daemon/tests/gateway_backend.rs`
- [X] T031 [P] [US3] Add failing HTTP contract tests for 503 `execution_queue_full`, 503 `execution_unavailable`, 409 `session_shutting_down`, unchanged success envelopes, and 429 remaining reserved for `rate_limited` in `crates/malt-gateway/tests/routes.rs`
- [X] T032 [P] [US3] Add failing injected worker-panic/channel-loss tests proving the control actor survives, active and pending replies fail exactly once, later admission rejects immediately, and no worker is reconstructed from cached state in `crates/malt-daemon/src/executor/command_worker.rs`

### Implementation for User Story 3

- [X] T033 [US3] Route Gateway exec, Gateway send, direct write, and accepted editor lines through the same serialized `ExecutionIngress`; remove command execution from the general control mailbox and preserve command IDs only for started work in `crates/malt-daemon/src/gateway_backend.rs`, `crates/malt-daemon/src/executor/coordinator.rs`, and `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T034 [US3] Map daemon queue-full, unavailable, and shutting-down outcomes to the exact HTTP statuses/codes in `crates/malt-daemon/src/gateway_backend.rs` and `crates/malt-gateway/src/error.rs` without changing successful Gateway response types
- [X] T035 [US3] On worker integrity failure, atomically close admission, fail the active and every accepted pending request exactly once, retain attach/output/snapshot controls, and reject future executions in `crates/malt-daemon/src/executor/command_worker.rs` and `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T036 [US3] Validate completion sequence against the active request, treat duplicate/stale/out-of-order completion as execution unavailability, acknowledge only fully finalized results, and continue after ordinary parse/nonzero/spawn outcomes in `crates/malt-daemon/src/executor/command_worker.rs` and `crates/malt-daemon/src/executor/session_thread.rs`
- [X] T037 [US3] Run and pass the User Story 3 tests in `crates/malt-daemon/tests/session_thread.rs`, `crates/malt-daemon/tests/gateway_backend.rs`, `crates/malt-gateway/tests/routes.rs`, and `crates/malt-daemon/src/executor/command_worker.rs`

**Checkpoint**: All user stories are complete: session responsiveness no longer
trades away authoritative ordering, bounded admission, or explicit failure.

---

## Phase 6: Polish & Cross-Cutting Verification

**Purpose**: Prove persistence/protocol compatibility, shell conformance,
cross-session isolation, and full-workspace readiness without expanding scope.

- [X] T038 Add regression coverage that a snapshot taken during execution uses the last finalized full `EnvSnapshot`, and that detach/restart still restores variables, aliases, functions, options, directory stack, cwd, and traps in `crates/malt-daemon/tests/gateway_backend.rs` and `crates/malt-daemon/tests/store.rs`
- [X] T039 Run VNP golden/protocol and persistence compatibility suites for `crates/malt-protocol`, `crates/malt-daemon/tests/vnp_listener.rs`, `crates/malt-daemon/tests/store.rs`, and `schemas/persist/session.vexil`, confirming this feature changes no schema or encoding
- [X] T040 Run native-Windows Smoosh conformance for `crates/mash/tests/smoosh_runner.rs` with `target/debug/mash.exe`, requiring 183 passed and 3 unsupported skipped
- [X] T041 Run every command and the 10-minute cross-session soak described in `specs/002-responsive-session-control/quickstart.md`, then run `cargo fmt --all --check`, `cargo build --workspace`, and `cargo test --workspace` against `Cargo.toml`
- [X] T042 Record measured responsiveness/order/soak results and exact test counts in `docs/findings/2026-07-25-responsive-session-control.md`, then mark only the verified 0b item complete in `docs/BACKLOG.md`

---

## Dependencies & Execution Order

### Phase Dependencies

```text
Phase 1: Setup
    ↓
Phase 2: Foundational worker + ingress (blocks every story)
    ↓
Phase 3: US1 busy attach/observation (MVP)
    ↓
Phase 4: US2 responsive controls/lifecycle
    ↓
Phase 5: US3 ordered execution/error contracts
    ↓
Phase 6: Cross-cutting verification and evidence
```

- **Setup** has no dependencies.
- **Foundational** depends on Setup and must complete before any story.
- **US1** depends on Foundational.
- **US2** depends on Foundational and uses US1's finalized-output boundary when
  composing live control state; implement in priority order.
- **US3** depends on Foundational and validates the producer paths completed by
  US2; implement after US2.
- **Polish** depends on all selected stories.

### User Story Dependency Graph

| Story | Depends on | Independently testable outcome |
|---|---|---|
| US1 (P1) | Foundational | Busy attach/output/snapshot return within one second from a consistent finalized boundary |
| US2 (P2) | Foundational + US1 output boundary | Mixed controls remain responsive; busy detach preserves work; shutdown intake closes promptly |
| US3 (P3) | Foundational + all producer routing from US2 | Capacity, FIFO, exactly-once, failure, and HTTP contracts hold across every producer |

### Within Each Story

1. Write the story tests and confirm they fail for the expected missing
   behavior.
2. Implement ownership/state changes before interface integration.
3. Preserve the finalization barrier before enabling the next execution.
4. Run the story's focused checkpoint suite.
5. Commit the coherent story checkpoint before starting the next phase.

### Parallel Opportunities

- T003 and T004 can run in parallel after test support exists.
- T013, T014, and T015 can run in parallel because they target separate
  integration-test files.
- T020, T021, and T022 can run in parallel; T023 follows T022 in the same file.
- T029, T030, T031, and T032 can run in parallel because they target distinct
  test/module files.
- Source tasks that touch `session_thread.rs`, `coordinator.rs`, or
  `gateway_backend.rs` are intentionally sequential to avoid conflicting
  ownership changes.

---

## Parallel Example: User Story 1

```text
Task T013: Add session-thread busy observation/snapshot tests.
Task T014: Add VNP attach-during-execution timing tests.
Task T015: Add Gateway busy and cross-session observation tests.
```

## Parallel Example: User Story 2

```text
Task T020: Add direct control-actor mixed-operation tests.
Task T021: Add VNP control-event integration tests.
Task T022: Add coordinator idle/busy dormancy tests.
```

## Parallel Example: User Story 3

```text
Task T029: Add session-level capacity, ordering, and state-handoff tests.
Task T030: Add concurrent Gateway/editor producer tests.
Task T031: Add HTTP status and stable error-code contract tests.
Task T032: Add worker-integrity failure tests.
```

---

## Implementation Strategy

### MVP First: User Story 1

1. Complete Setup.
2. Complete the full Foundational worker/ingress phase.
3. Complete US1 busy attach and observation.
4. Stop and run T019.
5. Demonstrate a command longer than five seconds while a second client
   attaches and observes the latest finalized state within one second.

US1 is the smallest useful delivery, but the complete Foundational phase is
non-negotiable: a partial thread split without authoritative Env ownership and
the finalization handshake would violate the feature's core invariants.

### Incremental Delivery

1. **Foundation**: one worker, one Env owner, one bounded ingress.
2. **US1**: healthy busy sessions remain attachable and observable.
3. **US2**: controls, detach, and shutdown remain responsive.
4. **US3**: all producers gain explicit bounded FIFO and failure contracts.
5. **Verification**: persistence, VNP, Smoosh, soak, and workspace gates.

### Checkpoint Discipline

- Commit after the Foundational phase, each completed user story, and the final
  evidence update.
- Never include concurrent unrelated work in these checkpoints.
- Do not modify VNP or persistence schemas for this feature.
- Do not expand into raw process input, intermediate streaming, cancellation,
  public execution identity/history, input authority, or new clients.

## Notes

- `[P]` means different files and no dependency on another incomplete task.
- Story labels map directly to the three prioritized scenarios in `spec.md`.
- Tests lead implementation because the spec explicitly requires measurable
  timing and ordering outcomes.
- The 30-second Gateway result wait remains unchanged and never cancels an
  accepted command.
- 0a Gateway authorization remains a prerequisite; these tasks neither replace
  nor weaken it.
