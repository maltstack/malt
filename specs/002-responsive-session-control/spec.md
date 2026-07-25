# Feature Specification: Responsive Session Control During Execution

**Feature Branch**: `main`

**Created**: 2026-07-24

**Status**: Implemented — Delivered 2026-07-25. See `docs/findings/2026-07-25-responsive-session-control.md`.

**Input**: User description: "Keep an active MALT session responsive while commands execute. A long-running command must not prevent another human or agent from attaching, reading the latest available session state, or having ordinary session-control events processed. Preserve deterministic command ordering and one authoritative shell state, and do not make a healthy session appear unreachable to concurrent operations merely because it is busy. This is the structural prerequisite only; it does not add live input delivery to a running process, intermediate output streaming, input-authority policy, execution identifiers or history, or new client features."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Attach to and Observe a Busy Session (Priority: P1)

A human or agent can attach to a healthy session and inspect its latest
available state while another command is still running, so long-running work
does not make the session appear offline.

**Why this priority**: A session that cannot be observed during the work users
most need to monitor is not a dependable persistent session.

**Independent Test**: Start a command that remains active longer than the
normal attach and output-query wait periods, then attach a second client and
query the session repeatedly without waiting for the command to finish.

**Acceptance Scenarios**:

1. **Given** a healthy session with a long-running command in progress, **When** a second client attaches, **Then** the client receives a consistent initial view and remains connected without waiting for the command to finish.
2. **Given** a healthy session with a command in progress, **When** a client requests the current output, **Then** it receives the latest available snapshot promptly rather than a timeout or a false session-unreachable failure.
3. **Given** a command that runs beyond a caller's normal wait period, **When** another client attaches or observes the same session, **Then** the command's duration alone does not cause either operation to fail.

---

### User Story 2 - Keep Session Controls Responsive (Priority: P2)

An attached client can continue sending session-control events while work is
running, so the session remains manageable and ready for later live-input and
coexistence capabilities.

**Why this priority**: Attach alone is insufficient if routine control events
still accumulate behind the running command and make the client stale or
unresponsive.

**Independent Test**: While a long-running command is active, send client
registration changes, terminal-size changes, frame acknowledgements, and
input events, then verify that each is handled without waiting for command
completion.

**Acceptance Scenarios**:

1. **Given** an attached client and a command in progress, **When** the client changes the terminal size, **Then** the session handles the change promptly and later session views reflect the latest accepted size.
2. **Given** an attached client receiving rendered state, **When** it acknowledges a frame during command execution, **Then** the acknowledgement is handled promptly and is not delayed until the command finishes.
3. **Given** a client submits an input event during command execution, **When** the session evaluates that event, **Then** it does so promptly instead of starving the event behind the command; delivery of input to the running process is outside this feature.
4. **Given** a client disconnects while a command is running, **When** the session receives the disconnect, **Then** client cleanup is handled promptly and the command continues according to existing execution semantics.

---

### User Story 3 - Preserve Ordered, Authoritative Execution (Priority: P3)

An operator can submit more work to a busy session without commands
overlapping unpredictably or corrupting the session's shell state.

**Why this priority**: Responsiveness must not trade away the deterministic,
single-authority behavior that makes the daemon's session state trustworthy.

**Independent Test**: Submit commands with observable ordering and
state-dependent results while the first command is still active, then verify
that every accepted command runs once, in submission order, against the state
left by its predecessor.

**Acceptance Scenarios**:

1. **Given** one command is active in a session, **When** two additional commands are accepted, **Then** they begin in submission order and no two commands mutate that session's shell state at the same time.
2. **Given** a command changes session-scoped shell state, **When** the next queued command begins, **Then** it observes the completed state change.
3. **Given** an active command fails, **When** the failure is finalized, **Then** the session remains attachable and observable and the next accepted command can still run.
4. **Given** demand exceeds the session's safe pending-work capacity, **When** another command is submitted, **Then** the request is rejected clearly rather than silently dropped, duplicated, or run out of order.

### Edge Cases

- A command never exits: attach, observation, and non-execution controls remain
  responsive for as long as the session itself remains healthy.
- A command produces sustained high-volume output: control operations continue
  to receive service and the session does not become falsely unreachable.
- Several clients attach, detach, observe, resize, acknowledge, and submit input
  at nearly the same time: each accepted operation is applied once and the
  session remains internally consistent.
- Additional commands arrive from different client paths while one command is
  active: accepted commands share one deterministic ordering for the session.
- Command startup or execution fails before producing normal output: the
  failure is finalized once, waiting work remains ordered, and session controls
  continue to function.
- A command's initiating client disconnects before completion: the disconnect
  does not corrupt session state or block other clients; reconnectable result
  retrieval is reserved for later execution-identity and history work.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Running a command MUST NOT prevent the same session from handling non-execution operations.
- **FR-002**: FR-001 MUST apply regardless of whether the command was initiated by an agent-facing command request or by an attached human's accepted command line.
- **FR-003**: While a command is active, the session MUST continue handling client attach, client detach, output observation, terminal-size changes, frame acknowledgements, input events, state snapshots, and shutdown requests without waiting for that command to finish.
- **FR-004**: A client attaching during command execution MUST receive one internally consistent initial session view and MUST NOT be rejected solely because the session is busy.
- **FR-005**: An output request during command execution MUST return the latest available session snapshot or an accurate session-lifecycle error; it MUST NOT time out or report the session unreachable solely because a command is active.
- **FR-006**: Input and control events received during command execution MUST enter normal session handling promptly; this feature MUST NOT claim that input reaches the running process unless that separate capability is present.
- **FR-007**: A session MUST have at most one command actively mutating its shell state at a time.
- **FR-008**: Additional accepted commands for the same session MUST wait in submission order and MUST each begin exactly once after the preceding command is finalized.
- **FR-009**: The system MUST define and enforce a safe pending-command capacity; requests beyond that capacity MUST fail explicitly and MUST NOT be silently lost, duplicated, or reordered.
- **FR-010**: A completed command's shell-state changes and final result MUST be finalized before the next command for that session begins.
- **FR-011**: For the same command and starting session state, decoupled execution MUST preserve the observable output, error output, exit outcome, and shell-state changes produced when the session is otherwise idle.
- **FR-012**: A command failure or execution-subsystem failure MUST NOT make an otherwise healthy session permanently unattachable or unobservable, and MUST NOT prevent already accepted later work from reaching a defined outcome.
- **FR-013**: Work in one session MUST remain isolated from execution ordering and control responsiveness in every other session.
- **FR-014**: The feature MUST preserve existing authentication, isolation, session persistence, and client protocol boundaries.
- **FR-015**: The feature MUST NOT expand scope into live process input delivery, intermediate output streaming or render notification, input-authority enforcement, command cancellation, public execution identifiers, reconnectable result retrieval, execution history, or new client experiences.

### Key Entities

- **Session**: The authoritative unit of shell state, client attachments,
  current presentation state, and ordered command execution.
- **Execution Request**: A submitted command associated with one session,
  including its submission order, acceptance outcome, and eventual final
  result.
- **Active Execution**: The single command currently allowed to mutate a
  session's shell state.
- **Pending Execution**: An accepted command waiting behind the active
  execution while retaining its exact submission order.
- **Session Operation**: A non-command action such as attach, detach, observe,
  resize, acknowledge, input submission, snapshot, or shutdown that must
  remain serviceable during active execution.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100 consecutive trials with a healthy session running a command for at least 60 seconds, a second client attaches successfully within 1 second in every trial.
- **SC-002**: During a 60-second command, 100% of 100 mixed attach, detach, output-observation, resize, acknowledgement, and input-submission operations produce their defined acknowledgement or observable session effect within 1 second under normal local operating load.
- **SC-003**: No attach or output-observation attempt reports a healthy session as unreachable solely because a command is active.
- **SC-004**: Across 1,000 ordered multi-command trials, every accepted command begins exactly once in submission order, with zero overlapping shell-state mutation within a session.
- **SC-005**: In representative successful, failing, state-changing, and high-output command tests, 100% of final outcomes match the same commands run from the same starting state in an otherwise idle session.
- **SC-006**: A command that runs indefinitely in one session causes no failed responsiveness checks in another healthy session during a 10-minute observation period.
- **SC-007**: After each tested command-startup or execution failure, the affected session remains attachable and observable within 1 second and every already accepted later command reaches either execution or a clear terminal failure.

## Assumptions

- One foreground command at a time remains the authoritative execution model
  within each session; accepted additional commands wait in first-in,
  first-out order.
- Output observation during active execution returns the latest state already
  materialized by the session. Streaming intermediate output is a separate
  backlog item and is not required here.
- Input events must no longer be starved by command execution, but delivering
  those events to a live process is a separate feature.
- The initiating client continues waiting under the existing command-result
  contract. Public execution identities, later result lookup, and reconnectable
  result retrieval are separate work.
- Existing authentication and requested-isolation enforcement determine
  whether a client may operate on a session; this feature does not change
  authorization or isolation policy.
- Gateway authorization enforcement remains the preceding delivery
  prerequisite established by the correctness-first priority order; this
  feature neither replaces nor weakens it.
- Responsiveness targets apply to a healthy daemon under normal local load;
  network loss, host suspension, and exhausted machine resources are not
  treated as command-execution blocking.
- This feature is a prerequisite for genuine raw input and human-agent
  coexistence, not completion of those later capabilities.
