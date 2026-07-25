# Feature Specification: Command Lifecycle Event Delivery

**Feature Branch**: `004-command-lifecycle-events`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "Deliver command lifecycle events to clients. Every command executed in a session should emit structured start and finish events (command id, command text, timestamps, exit status) that connected clients — AI agents and humans — can subscribe to and receive as they happen, instead of polling for output or scraping the terminal. Today the daemon already tracks these transitions internally but has no way to deliver them to anyone: the internal message bus has zero consumers. Includes catching a subscriber up on events it missed while not connected, and not letting a slow subscriber degrade the session."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - An agent learns a command started and finished, without polling (Priority: P1)

An AI agent runs a long command in a session and needs to know when it starts and when it completes, along with whether it succeeded. Today its only options are to poll for output on a timer or scrape the terminal grid looking for a prompt — both of which guess at state the system already knows precisely. The agent instead subscribes to the session and receives a start event when execution begins and a finish event carrying the exit status when it ends.

**Why this priority**: This is the core value and the reason the feature exists. Everything else in this spec makes it reliable; without it there is nothing to make reliable. It is also what turns "the daemon knows" into "the client knows," which is the whole point of a daemon-authoritative design.

**Independent Test**: Subscribe to a session, run a command that takes a few seconds, and verify a start event arrives promptly after submission and a finish event arrives promptly after completion, each carrying the command's identifier, the command text, a timestamp, and (on finish) the exit status. Verify no polling was required to observe either.

**Acceptance Scenarios**:

1. **Given** a client subscribed to a session, **When** a command begins executing, **Then** the client receives a start event identifying the command, its text, and when it started — while the command is still running.
2. **Given** a client subscribed to a session, **When** a command finishes, **Then** the client receives a finish event with the same command identifier as the start event, plus the completion time and exit status.
3. **Given** a command that fails, **When** it finishes, **Then** the finish event reports its real failure status, never a success.
4. **Given** several commands run in sequence, **When** the client observes the events, **Then** start and finish events arrive in the order the commands actually executed, and each finish is matched to its start by a shared command identifier.
5. **Given** a client subscribed to one session, **When** a command runs in a *different* session, **Then** the client does not receive events for it.

---

### User Story 2 - A subscriber that reconnects finds out what it missed (Priority: P2)

A client disconnects — network blip, agent restart, human closing a laptop — and reconnects a short time later. Commands ran while it was away. Rather than silently resuming from "now" and leaving the client with a false picture of what happened, the system delivers the events it missed, or tells it plainly that some events are no longer available.

**Why this priority**: Depends on Story 1 (there must be events to miss). Without it, an agent that briefly disconnects can conclude a command never ran, or never finished — worse than not subscribing at all, because it looks authoritative. Story 1 is still independently valuable for a continuously-connected client, which is why this is P2 rather than P1.

**Independent Test**: Subscribe, note a position, disconnect, run several commands while disconnected, then resubscribe from the noted position and verify the missed events are delivered in order. Separately, disconnect long enough to exceed the retained window, resubscribe, and verify the client is explicitly told there is a gap rather than being handed a silently-incomplete stream.

**Acceptance Scenarios**:

1. **Given** a client that has previously received events up to a known position, **When** it resubscribes from that position after commands ran while it was away, **Then** it receives the events it missed, in order, before receiving new live events.
2. **Given** a client resubscribing from a position so old that the events are no longer retained, **When** it resubscribes, **Then** it is explicitly told that events were dropped, rather than receiving a stream that silently skips them.
3. **Given** a client subscribing for the first time with no prior position, **When** it subscribes, **Then** it begins receiving events from that moment forward and is not required to process the session's entire past.

---

### User Story 3 - One slow subscriber cannot degrade the session (Priority: P2)

A subscriber stops reading — it hangs, its network stalls, or it simply cannot keep up with a command producing events faster than it consumes them. The session it is watching must continue executing commands at full speed, other subscribers must keep receiving events normally, and the daemon must not accumulate unbounded memory on the stalled client's behalf.

**Why this priority**: Independent of Story 2 and equally load-bearing. This project has a specific history of "designed but never wired" safety mechanisms; a delivery path without a defined slow-consumer policy is exactly the kind of thing that works in testing and fails under a real agent. Story 1 remains demonstrable without it, which is why it is P2, but shipping event delivery without it would be shipping a known liability.

**Independent Test**: Subscribe two clients, have one stop consuming entirely, then run commands. Verify the session's command execution timing is unaffected, the healthy subscriber continues receiving every event, and the daemon's memory attributable to the stalled subscriber stays bounded. Verify the stalled subscriber is eventually dropped or told it fell behind, rather than being kept alive indefinitely.

**Acceptance Scenarios**:

1. **Given** a subscriber that has stopped reading, **When** commands execute in that session, **Then** command execution completes in the same time it would with no subscribers at all.
2. **Given** one stalled subscriber and one healthy subscriber on the same session, **When** commands execute, **Then** the healthy subscriber receives every event without delay or loss.
3. **Given** a subscriber that falls further behind than the system will buffer, **When** the limit is reached, **Then** the system stops retaining events for it and the subscriber is told it fell behind — it is never left believing it received a complete stream.
4. **Given** a subscriber that disconnects without a clean goodbye, **When** the system detects this, **Then** its resources are released rather than retained indefinitely.

---

### Edge Cases

- What happens to the start event's counterpart when the daemon stops between a command starting and finishing? The subscriber must not be left waiting forever for a finish event that will never come; on reconnection the command must be observably in a "started, never confirmed complete" state rather than appearing still-running forever.
- What happens when a command produces no output but still succeeds or fails? Lifecycle events are independent of output — the start and finish events must be emitted regardless.
- What happens when a client subscribes to a session that does not exist? A clear "not found" answer, distinguishable from a real session that simply has not run anything yet.
- What happens when a client subscribes to a session that is dormant? It must not silently receive nothing forever; the client needs to be able to tell the difference between "connected, nothing happening" and "connected to something that cannot produce events."
- What happens when a client lacks permission for the session it is subscribing to? Refused outright — a subscription must not become a way to observe a session that the same client could not read directly.
- What happens when many clients subscribe to the same session at once? Each receives the same events independently; one subscriber's behavior must not alter what another receives.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST emit a start event when a command begins executing, carrying the command's identifier, the command text, and the time it started.
- **FR-002**: System MUST emit a finish event when a command completes, carrying the same identifier as its start event, the completion time, and the command's real exit status.
- **FR-003**: Clients MUST be able to subscribe to a session and receive its lifecycle events as they occur, without polling.
- **FR-004**: Events for a session MUST be delivered only to subscribers of that session — a subscription MUST NOT leak events from other sessions.
- **FR-005**: Events MUST be delivered in the order the underlying commands executed, and each finish event MUST be matchable to its start event by a shared command identifier.
- **FR-006**: Each event MUST carry a position identifier that a client can record and later use to resume.
- **FR-007**: Clients MUST be able to resubscribe from a previously recorded position and receive the events they missed, in order, before live events resume.
- **FR-008**: When a client resumes from a position whose events are no longer retained, the system MUST explicitly signal that events were dropped rather than delivering a silently-incomplete stream.
- **FR-009**: A subscriber that stops consuming MUST NOT delay command execution, MUST NOT affect delivery to other subscribers, and MUST NOT cause unbounded resource growth in the daemon.
- **FR-010**: A subscriber that exceeds the system's buffering limit MUST be dropped or explicitly told it fell behind — never silently starved while appearing healthy.
- **FR-011**: System MUST release the resources of a disconnected subscriber, including one that disconnected without a clean goodbye.
- **FR-012**: Subscription requests MUST be subject to the same access-permission checks as other session data; a client without sufficient access MUST be refused.
- **FR-013**: Subscribing to a session that does not exist MUST return a clear not-found response, distinguishable from a valid subscription that has not yet produced events.
- **FR-014**: A command interrupted by daemon shutdown MUST NOT leave subscribers waiting indefinitely for a finish event; after restart, its state MUST be observable as not-confirmed-complete.

### Key Entities

- **Lifecycle Event**: One occurrence in a command's life — either its start or its finish. Carries the command identifier that links the pair, the command text, a timestamp, the exit status (finish only), and a position identifier for resumption.
- **Subscription**: One client's live attachment to one session's event stream. Has an independent delivery position and its own buffering state, so subscribers neither block nor observe each other.
- **Retained Event Window**: The bounded set of recent events kept so a reconnecting subscriber can catch up. Bounded by design; exhausting it is a reportable condition, not a silent loss.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An agent can determine that a command started and finished, and whether it succeeded, without ever requesting output or inspecting terminal contents.
- **SC-002**: A subscriber observes a command's start while that command is still running — verified with a command lasting several seconds.
- **SC-003**: 100% of commands executed through any client produce exactly one start event and, once complete, exactly one matching finish event — no gaps, no duplicates.
- **SC-004**: Events reach a connected subscriber promptly enough to feel immediate (under 1 second from the underlying transition).
- **SC-005**: A subscriber reconnecting within the retained window recovers every event it missed, in the correct order.
- **SC-006**: With a fully stalled subscriber attached, command execution time is indistinguishable from having no subscribers, and daemon memory attributable to that subscriber stays bounded.
- **SC-007**: Zero instances of a subscriber receiving an incomplete stream while believing it complete — every gap is signalled.

## Assumptions

- **Scope is command lifecycle only.** Session and pane lifecycle events (created, destroyed, dormant, restored) are a natural future extension of the same delivery path but are deliberately out of scope here; this feature is judged on command start/finish. The delivery mechanism should not be designed in a way that forecloses adding them.
- **Subscriptions are per-session.** Every existing client-facing operation is scoped to a session, and a session is the unit of access control, so a per-session subscription is the consistent choice. A daemon-wide "all sessions" stream would be genuinely useful to an orchestrating agent and is a plausible follow-up, but it raises its own access-control questions and is not assumed here.
- **The retained event window is bounded and modest.** Catch-up is intended for brief disconnections (a reconnecting agent, a dropped connection), not as a durable event log or an audit trail. A client needing the full history of what ran has the existing command history for that; this feature's retention exists to make reconnection correct, not to replace persistence.
- **Events describe execution, not output.** A lifecycle event says a command started or finished and how it ended; it does not carry the command's output. Streaming output is a related but separate concern with different volume and buffering characteristics.
- **Access control reuses the existing permission model** rather than introducing a subscription-specific one, at the same sensitivity level as reading a session's output or history (command text can contain secrets typed at the prompt).
- **The underlying start/finish transitions are already tracked.** The daemon observes them today; this feature is about delivering them to clients, not about instrumenting execution. That makes the work a delivery-path problem, and the risk concentrates in subscription lifecycle, catch-up, and slow-consumer behavior rather than in detecting the events.
