# Feature Specification: Persistent Command Execution History

**Feature Branch**: `003-command-execution-history`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "Track the history of commands executed in a session pane — what was run, when, and whether it succeeded — make that history retrievable by users and by automation/API clients, and ensure it survives a daemon restart instead of disappearing when a session goes dormant and is later restored."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Review recent commands in a pane (Priority: P1)

A user working in a session pane wants to see what commands they (or a collaborator, or an automated agent) have recently run in that pane — without scrolling back through raw terminal output — including whether each command succeeded or failed and roughly when it ran.

**Why this priority**: This is the core value of the feature. Without it, there is no execution history at all — just live output that scrolls away. Every other story builds on this record existing.

**Independent Test**: Run several commands (a mix of succeeding and failing) in a session pane, then request that pane's command history. Verify the returned list shows each command's text, start time, completion time, and exit status, in the order they ran.

**Acceptance Scenarios**:

1. **Given** a session pane with no commands yet run, **When** a user requests its command history, **Then** an empty history is returned (not an error).
2. **Given** a pane where three commands have run (two succeeded, one failed), **When** a user requests its command history, **Then** all three appear in chronological order with correct command text and correct success/failure status for each.
3. **Given** a command that is still executing, **When** a user requests the pane's command history during that execution, **Then** the in-progress command appears in the history marked as still running, without an exit status or completion time.

---

### User Story 2 - History survives a daemon restart (Priority: P2)

A user whose session was persisted and later restored (for example, after the daemon process restarted) wants their command history from before the restart to still be there — otherwise the history feature only ever covers the current process lifetime, which undermines its usefulness for anything but the most recent few minutes of work.

**Why this priority**: Depends on Story 1 (there must be a history to persist). Without this, history silently resets on every restart, which is surprising and reduces trust in the feature — but the feature still has standalone same-session value even if this story isn't done yet.

**Independent Test**: Run several commands in a pane, persist/restore the session (e.g., via the existing dormant-session restore path) without any commands running, then request the pane's command history again and verify the pre-restart entries are still present and unchanged.

**Acceptance Scenarios**:

1. **Given** a pane with recorded command history, **When** the owning session becomes dormant and is later restored, **Then** requesting the pane's command history afterward returns the same entries (command text, timestamps, exit status) as before the restart.
2. **Given** a command that was still executing when the daemon stopped (unexpectedly or otherwise), **When** the session is later restored, **Then** that entry appears in history marked as not completed, rather than being silently dropped or shown as successful.

---

### User Story 3 - Retrieve history through automation and AI-agent tooling (Priority: P3)

An automation script or an AI agent connected through MALT's programmatic interfaces wants to retrieve a session pane's command history the same way a human user can — so that scripting, auditing, and agent-driven workflows can reason about what has already been run in a session.

**Why this priority**: Extends the reach of Stories 1–2 to non-interactive clients. Valuable but strictly additive — the underlying history already exists and is useful to interactive users even before this story is done.

**Independent Test**: Using only the automation/API surface (not the interactive client), request a pane's command history and verify the same data returned to interactive users is available, in a form a script can parse.

**Acceptance Scenarios**:

1. **Given** a pane with recorded command history, **When** an automation client requests that pane's history through the programmatic interface, **Then** it receives the same set of entries (command text, timestamps, exit status) that an interactive user would see.
2. **Given** an automation client without sufficient permission for a session, **When** it requests that session's command history, **Then** the request is refused rather than returning partial or full data.

---

### Edge Cases

- What happens when a pane's command count exceeds the retained history limit? The oldest entries are evicted first so the most recent commands are always available, and the eviction must behave consistently across a restart (no duplication, no gaps beyond the intended cap).
- How does the system handle a request for command history on a pane or session that does not exist? A clear "not found" response, not an empty history (which would look identical to a real pane with no commands yet).
- How does the system handle a command whose text is very large (e.g., a long pasted script)? History must not be silently truncated in a way that loses the ability to tell what was actually run.
- What happens if the daemon stops mid-command and is never restarted with that exact session again (session eventually expires/is deleted)? History for a deleted session should be discarded along with the rest of that session's persisted state, not orphaned indefinitely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST record, for every command executed in a session pane, the command text, the time it started, the time it completed (once known), and its exit/success status.
- **FR-002**: Users MUST be able to retrieve a session pane's command history in chronological order.
- **FR-003**: System MUST retain a bounded number of the most-recent command history entries per pane, evicting the oldest entries first once that bound is reached, so history storage does not grow without limit.
- **FR-004**: A command's history entry MUST be visible while the command is still executing, clearly distinguished from a completed entry (no exit status, no completion time yet).
- **FR-005**: Command history for a session pane MUST survive a daemon restart: after a persisted session is restored, its pre-restart history entries MUST be retrievable again with the same content as before the restart.
- **FR-006**: A command that was still executing when the daemon stopped MUST, after session restore, appear in history in a state that reflects it was not confirmed complete — never silently reinterpreted as succeeded or dropped.
- **FR-007**: Command history MUST be retrievable through both interactive user-facing clients and programmatic/automation clients (CLI, HTTP API, and AI-agent tooling), returning equivalent data through each.
- **FR-008**: Requests for command history MUST be subject to the same access-permission checks as other session/pane data — a client without sufficient access to a session MUST be refused, not given partial or full history.
- **FR-009**: Requesting history for a pane or session that does not exist MUST return a clear not-found response, distinguishable from a real pane that simply has no commands yet.

### Key Entities

- **Command Execution Record**: One executed command within a pane — its command text, start time, completion time (if finished), and exit/success status (or "still running" / "not confirmed complete" state). This is the unit of history.
- **Pane Command History**: The ordered, bounded collection of Command Execution Records belonging to one pane, oldest evicted first once the retention bound is reached.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can view the full recent command history of an active session pane (up to the retention bound) without needing to scroll back through raw terminal output.
- **SC-002**: 100% of commands executed through any supported client (interactive or automated) produce a corresponding history entry — no silent gaps between what was run and what history shows.
- **SC-003**: After a daemon restart and session restore, at least the most recent 1,000 command history entries per pane are available again, matching pre-restart content exactly.
- **SC-004**: Retrieving a pane's full command history (at the retention bound) completes without perceptible delay to an interactive user (under 1 second).
- **SC-005**: History requests for sessions/panes a client lacks access to are refused 100% of the time, with zero instances of partial or full data leaking through.

## Assumptions

- A default retention bound of 1,000 command entries per pane is an acceptable default cap unless the user specifies otherwise; it can be revisited later without changing the shape of this feature.
- "Automation/API clients" refers to MALT's existing programmatic interfaces (its CLI, HTTP API, and AI-agent tool integration) — this feature extends what those interfaces expose, it does not require building new client applications.
- Command history covers structured metadata about each execution (command text, timing, success/failure) — it does not need to include the full raw terminal output/scrollback of each command, which is a related but separate concern.
- Restart-survival assumes the session itself is already configured to be persisted and restorable; this feature extends that existing persistence to also cover command history, rather than introducing session persistence from scratch.
- Access control for history requests reuses whatever permission/scope model already governs access to other session and pane data, rather than introducing a new one specific to history.
