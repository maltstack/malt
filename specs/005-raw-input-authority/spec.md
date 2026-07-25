# Feature Specification: Genuine Raw Input with Input Authority

**Feature Branch**: `005-raw-input-authority`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "Genuine raw input with input authority. When a command running in a session asks for input — a password prompt, a REPL, an interactive installer, a confirmation prompt — the bytes a connected client sends must reach that waiting command, instead of being parsed as a brand-new shell command line. Today every input path treats whatever arrives as a fresh command to run, and a command that reads standard input falls through to the daemon process's own console, so no client can ever answer it. Input must also be attributable to the client that sent it: when a human and an AI agent are attached to the same session, exactly one holds input authority at a time, the others observe without disrupting, and authority can be handed over deliberately. Attaching, detaching, and disconnecting must leave authority in a sane state rather than stranding the session."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - An interactive command can actually be answered (Priority: P1)

A user runs something that stops and asks a question — a confirmation prompt, a password, a REPL waiting at its own prompt, an installer asking which options to enable. They type the answer, and the waiting command receives it and continues. Today this is impossible: whatever they send is treated as a brand-new command line, and the waiting command is reading from somewhere no client can reach, so the session is simply stuck.

**Why this priority**: This is the feature. Without it an entire category of everyday commands cannot be used through MALT at all, and a user who tries gets a session that appears hung with no way to recover. Everything else in this spec governs *who* may answer; this is whether anyone can.

**Independent Test**: Run a command that reads a line of input and echoes it back. Send an answer. Verify the command receives exactly what was sent, completes, and reports its result — and that the answer was not executed as a command in its own right.

**Acceptance Scenarios**:

1. **Given** a command waiting to read a line of input, **When** a client sends text, **Then** the waiting command receives that text and continues, and the text is not run as a separate command.
2. **Given** a command that reads several lines in sequence, **When** a client sends them one at a time, **Then** each read receives the corresponding line, in the order sent.
3. **Given** a command waiting for input, **When** a client sends its answer, **Then** the answer does not appear in the session's command execution history as if it had been a command, and does not appear in the session's lifecycle event stream.
4. **Given** no command is currently waiting for input, **When** a client sends raw input anyway, **Then** it is retained for the next command that reads, rather than being discarded or executed.
5. **Given** a command that has finished, **When** a client sends a command line to run, **Then** it still runs as a command exactly as it does today — adding raw input must not break ordinary command submission.

---

### User Story 2 - Exactly one client can type at a time (Priority: P2)

A human and an AI agent are attached to the same session. Only one of them holds input authority; that one's input reaches the session. The other stays attached and sees everything happening, but its input does not interleave with the holder's. Without this, two clients answering the same prompt produce a corrupted, interleaved answer that neither intended — and for a password prompt, that is a security problem as well as a correctness one.

**Why this priority**: Depends on Story 1 (there must be an input path to arbitrate). Story 1 is independently valuable and correct for the single-client case, which is today's normal situation — so this is P2. But it must land before the stream is exposed to simultaneous human and agent use, which is the whole point of a daemon-authoritative design.

**Independent Test**: Attach two clients. Confirm one holds authority. Have both send input to a waiting command. Verify the command receives only the holder's input, intact and uninterleaved, and that the non-holder is told its input was not accepted rather than silently ignored.

**Acceptance Scenarios**:

1. **Given** two clients attached and one holding input authority, **When** both send input, **Then** the waiting command receives only the holder's input, with none of the other client's bytes mixed in.
2. **Given** a client that does not hold authority, **When** it sends input, **Then** it is told its input was rejected and why — never silently dropped, which would look identical to a lost connection.
3. **Given** two clients attached, **When** either asks who holds input authority, **Then** it receives a clear answer.
4. **Given** a client without authority, **When** it observes the session, **Then** it continues to receive all output and events normally — losing input rights must not degrade observation.

---

### User Story 3 - Authority changes hands without stranding the session (Priority: P2)

An agent is driving a session and hits a prompt it cannot answer — a password, an unexpected confirmation. A human takes over input, answers, and hands control back. Separately, whichever client holds authority may disconnect at any moment; the session must not be left with input rights owned by someone who is gone.

**Why this priority**: Independent of Story 2's arbitration and equally load-bearing. Arbitration without a transfer path produces the worst outcome available: a session that is provably stuck, where the one client that could answer is not the one that needs to. This is also the "temporary input authority changes hands" step of the project's guiding scenario.

**Independent Test**: With two clients attached and the first holding authority, have the second claim it, then verify input from the second is now accepted and input from the first is rejected. Separately, have the authority holder disconnect abruptly while a command waits for input, and verify another attached client can immediately take over and answer.

**Acceptance Scenarios**:

1. **Given** client A holds authority and client B is attached, **When** B claims authority, **Then** B's input is accepted from that point, A's is rejected, and A is told it no longer holds authority.
2. **Given** the authority holder disconnects cleanly, **When** another client is still attached, **Then** authority is released and an attached client can claim it without waiting for a timeout.
3. **Given** the authority holder disconnects abruptly while a command waits for input, **When** another client claims authority, **Then** it can answer the still-waiting prompt and the command proceeds.
4. **Given** a session with no clients attached, **When** a client attaches, **Then** it can hold authority — a session must never become permanently unanswerable because of who was attached previously.
5. **Given** an authority change, **When** it happens mid-session, **Then** every attached client is informed, so no client believes it can type when it cannot.

---

### Edge Cases

- What happens to input a client sends that no command ever reads? It must be bounded — retained input cannot accumulate without limit, and a client that floods input must be refused rather than allowed to grow the daemon's memory.
- What happens when a command asks for input while the session has no clients attached at all? It must not hang forever with no path to recovery; the command's wait needs to be observable so the situation is diagnosable rather than looking like a crash.
- What happens when a client sends input containing control characters or line endings that differ from the platform's? The waiting command must receive what was actually sent, without silent rewriting that would corrupt a password or a binary-ish payload.
- What happens if a client claims authority it already holds? A harmless no-op, not an error or a spurious change notification to everyone else.
- What happens to retained input when the session is persisted and later restored? Input typed ahead but never consumed should not silently resurface much later and be fed to an unrelated command.
- How does a client tell the difference between "my input was rejected because I lack authority" and "my input was accepted but the command ignored it"? These must be distinguishable.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When a command is waiting to read input, raw input sent by a client MUST be delivered to that command rather than being interpreted as a new command line.
- **FR-002**: A command reading input MUST receive it from the session's clients, never from the daemon process's own console.
- **FR-003**: Raw input MUST be delivered byte-for-byte as sent, without rewriting that would alter a password or other exact-match payload.
- **FR-004**: Raw input delivered to a waiting command MUST NOT be recorded in the session's command execution history and MUST NOT appear in the session's lifecycle event stream — those surfaces record commands, and an answer to a prompt is not one. This is a confidentiality requirement, not a tidiness one: prompts routinely carry passwords.
- **FR-005**: Ordinary command submission MUST continue to work unchanged; a client MUST be able to say whether it is sending a command to run or input for a waiting reader.
- **FR-006**: Raw input sent while nothing is reading MUST be retained for the next read, within a bounded limit; exceeding that limit MUST be refused with a clear error rather than growing without bound.
- **FR-007**: Every input submission MUST be attributable to the client that sent it.
- **FR-008**: At most one attached client MUST hold input authority for a session at any time.
- **FR-009**: Input from a client that does not hold authority MUST be rejected with a clear reason, never silently discarded.
- **FR-010**: A client MUST be able to discover which client currently holds input authority.
- **FR-011**: A client MUST be able to claim input authority, and the previous holder MUST be informed that it no longer holds it.
- **FR-012**: When the authority holder detaches or disconnects — cleanly or abruptly — authority MUST be released so that another attached client can claim it without waiting for a timeout.
- **FR-013**: A session with no clients attached MUST be claimable by the next client that attaches; a session MUST NOT become permanently unanswerable.
- **FR-014**: All attached clients MUST be informed when input authority changes hands.
- **FR-015**: Not holding input authority MUST NOT reduce a client's ability to observe the session's output or events.

### Key Entities

- **Session Input Channel**: The session-scoped destination for raw input. Holds input sent before a reader is ready, within a bounded limit, and delivers it to whichever command is currently reading.
- **Input Authority**: The right to send input to a session, held by at most one attached client at a time. Transferable by claim, released automatically when its holder goes away.
- **Input Submission**: One client's attempt to send raw input — the bytes, who sent them, and whether they were accepted or rejected with a reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can complete an interactive command that prompts for input — including a password prompt — entirely through a connected client, with no access to the daemon's own console.
- **SC-002**: 100% of raw input delivered to a waiting command is absent from command execution history and from the lifecycle event stream.
- **SC-003**: With two clients attached and both sending input, the receiving command's input contains bytes from exactly one of them, with zero instances of interleaving.
- **SC-004**: A client whose input is rejected learns why in every case — zero silent drops.
- **SC-005**: After the authority holder disconnects abruptly, another attached client can take over and answer a waiting prompt within seconds, without restarting the session or the command.
- **SC-006**: Ordinary command submission continues to behave exactly as before this feature, verified by the existing command, history, and event behavior remaining correct.

## Assumptions

- **Authority is claimed, not negotiated.** A client that claims input authority receives it immediately and the previous holder is notified. A grant-and-consent protocol was considered and rejected as the default: if the current holder must approve, an unresponsive or departed holder can strand the session, which is the exact failure FR-013 exists to prevent. Claiming is still deliberate, because it is an explicit action rather than a side effect of attaching.
- **The first client to attach an unheld session takes authority.** This matches how a terminal behaves for the overwhelmingly common single-client case, and means nothing extra is required to make ordinary use work.
- **The daemon never echoes raw input.** Whatever a command chooses to display in response to input appears through its normal output. A daemon that echoed input itself would print passwords to every observer.
- **Clients are already authenticated and authorized for the session.** Input authority arbitrates *between* clients that are already permitted to interact; it is not an additional access-control layer, and it does not replace the existing permission model.
- **Scope is input to commands running in a session.** Terminal control concerns that ride alongside input in a traditional terminal — window resize signalling, job-control key handling, terminal modes such as raw versus cooked — are deliberately out of scope here and are their own work.
- **Retained input is not durable.** Type-ahead that no command consumed is session-lifetime state, not something to persist and replay after a restart, where it could be fed to an unrelated command much later.
