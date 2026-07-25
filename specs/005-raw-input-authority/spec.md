# Feature Specification: Authenticated Raw Input with Input Authority

**Feature Branch**: `005-raw-input-authority`

**Created**: 2026-07-25 | **Revised**: 2026-07-25 (see Revision note)

**Status**: Draft

**Input**: User description: "Genuine raw input with input authority. When a command running in a session asks for input — a password prompt, a REPL, an interactive installer, a confirmation prompt — the bytes a connected client sends must reach that waiting command, instead of being parsed as a brand-new shell command line. Today every input path treats whatever arrives as a fresh command to run, and a command that reads standard input falls through to the daemon process's own console, so no client can ever answer it. Input must also be attributable to the client that sent it: when a human and an AI agent are attached to the same session, exactly one holds input authority at a time, the others observe without disrupting, and authority can be handed over deliberately. Attaching, detaching, and disconnecting must leave authority in a sane state rather than stranding the session."

**Revision note**: The first draft assumed clients reaching a session were already authenticated, and scoped this feature to arbitrating between them. That assumption is false for the transport this feature depends on most — verified directly, and independently reported as finding A-01 in `docs/findings/2026-07-25-architecture-spec-codebase-audit.md`. Authenticated identity is therefore now User Story 1 rather than a precondition. See Assumptions for what changed and why the ordering matters.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Only an identified client can reach a session (Priority: P1)

A user expects that connecting to their terminal daemon requires being someone the daemon recognises. Today, on the connection path used by the interactive client, it does not: any process on the machine can connect, be told what sessions exist, attach to any one of them by naming its identifier, watch its output, and send it input. Nothing is checked at any point.

**Why this priority**: Every other story in this spec depends on knowing *who* sent something. "Exactly one client holds input authority" means nothing if a client's identity is merely self-asserted — it becomes a coordination convention, not a guarantee. Delivering interactive input first would make the situation actively worse, because password prompts and confirmation dialogs would become injectable by any local process. This must land first.

**Independent Test**: Connect without valid credentials and verify the connection is refused, that no session inventory was disclosed beforehand, and that the refusal happens promptly without leaving the daemon holding resources. Then connect with valid credentials and verify normal operation is unaffected.

**Acceptance Scenarios**:

1. **Given** a client that cannot prove its identity, **When** it connects, **Then** it is refused, and it learns nothing about which sessions exist.
2. **Given** a client that cannot prove its identity, **When** it connects, **Then** it cannot attach to, observe, resize, or send input to any session.
3. **Given** a client that connects and then stalls without completing identification, **When** it holds the connection open, **Then** the daemon releases it within a bounded time rather than retaining resources for it indefinitely.
4. **Given** many such stalled connections, **When** they accumulate, **Then** the daemon remains responsive to legitimate clients — an unidentified caller must not be able to exhaust it.
5. **Given** an authenticated client, **When** it names a session identifier it is not entitled to reach, **Then** the request is refused rather than honoured on the strength of the identifier alone.
6. **Given** an authenticated client, **When** it performs ordinary work, **Then** its experience is unchanged from today apart from the identification step.

---

### User Story 2 - An interactive command can actually be answered (Priority: P1)

A user runs something that stops and asks a question — a confirmation prompt, a password, a REPL waiting at its own prompt, an installer asking which options to enable. They type the answer, and the waiting command receives it and continues. Today this is impossible: whatever they send is treated as a brand-new command line, and the waiting command is reading from somewhere no client can reach, so the session is simply stuck.

**Why this priority**: This is the capability the feature exists to deliver. Without it an entire category of everyday commands cannot be used through MALT at all, and a user who tries gets a session that appears hung with no way to recover. It is second only because Story 1 is what makes it safe to ship.

**Independent Test**: Run a command that reads a line of input and echoes it back. Send an answer. Verify the command receives exactly what was sent, completes, and reports its result — and that the answer was not executed as a command in its own right.

**Acceptance Scenarios**:

1. **Given** a command waiting to read a line of input, **When** a client sends text, **Then** the waiting command receives that text and continues, and the text is not run as a separate command.
2. **Given** a command that reads several lines in sequence, **When** a client sends them one at a time, **Then** each read receives the corresponding line, in the order sent.
3. **Given** a client sends input containing leading or trailing whitespace, or bytes that are not valid text, **When** the waiting command reads it, **Then** it receives exactly what was sent — no trimming, no substitution of unrepresentable bytes.
4. **Given** a command waiting for input, **When** a client sends its answer, **Then** the answer does not appear in the session's command execution history and does not appear in the session's lifecycle event stream.
5. **Given** no command is currently waiting for input, **When** a client sends raw input anyway, **Then** it is retained for the next command that reads, rather than being discarded or executed.
6. **Given** a command that has finished, **When** a client sends a command line to run, **Then** it still runs as a command exactly as it does today — adding raw input must not break ordinary command submission.

---

### User Story 3 - Exactly one client can type at a time (Priority: P2)

A human and an AI agent are attached to the same session. Only one of them holds input authority; that one's input reaches the session. The other stays attached and sees everything happening, but its input does not interleave with the holder's. Without this, two clients answering the same prompt produce a corrupted, interleaved answer that neither intended — and for a password prompt, that is a security problem as well as a correctness one.

**Why this priority**: Depends on Stories 1 and 2 (there must be an identified sender and an input path to arbitrate). Story 2 is independently valuable and correct for the single-client case, which is today's normal situation — so this is P2. But it must land before the session is exposed to simultaneous human and agent use, which is the whole point of a daemon-authoritative design.

**Independent Test**: Attach two authenticated clients. Confirm one holds authority. Have both send input to a waiting command. Verify the command receives only the holder's input, intact and uninterleaved, and that the non-holder is told its input was not accepted rather than silently ignored.

**Acceptance Scenarios**:

1. **Given** two clients attached and one holding input authority, **When** both send input, **Then** the waiting command receives only the holder's input, with none of the other client's bytes mixed in.
2. **Given** a client that does not hold authority, **When** it sends input, **Then** it is told its input was rejected and why — never silently dropped, which would look identical to a lost connection.
3. **Given** two clients attached, **When** either asks who holds input authority, **Then** it receives a clear answer.
4. **Given** a client without authority, **When** it observes the session, **Then** it continues to receive all output and events normally — losing input rights must not degrade observation.

---

### User Story 4 - Authority changes hands without stranding the session (Priority: P2)

An agent is driving a session and hits a prompt it cannot answer — a password, an unexpected confirmation. A human takes over input, answers, and hands control back. Separately, whichever client holds authority may disconnect at any moment; the session must not be left with input rights owned by someone who is gone.

**Why this priority**: Independent of Story 3's arbitration and equally load-bearing. Arbitration without a transfer path produces the worst outcome available: a session that is provably stuck, where the one client that could answer is not the one that needs to. This is also the "temporary input authority changes hands" step of the project's guiding scenario.

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
- What happens if a client claims authority it already holds? A harmless no-op, not an error or a spurious change notification to everyone else.
- What happens to retained input when the session is persisted and later restored? Input typed ahead but never consumed should not silently resurface much later and be fed to an unrelated command.
- How does a client tell the difference between "my input was rejected because I lack authority" and "my input was accepted but the command ignored it"? These must be distinguishable.
- What happens to a client that authenticates successfully and then loses its right to the session while still connected? Its subsequent operations must be refused rather than continuing on the strength of the original handshake.
- What happens when identification fails repeatedly from the same source? Failures must not become a cheap way to consume daemon resources.

## Requirements *(mandatory)*

### Functional Requirements

**Identity and access**

- **FR-001**: A client MUST prove its identity before it can enumerate sessions, attach to a session, observe output, resize, or send input.
- **FR-002**: The set of existing sessions MUST NOT be disclosed to a client that has not proved its identity.
- **FR-003**: A connection that does not complete identification within a bounded time MUST be closed and its resources released.
- **FR-004**: Unidentified connections MUST NOT be able to exhaust the daemon's capacity to serve identified ones.
- **FR-005**: A client-supplied session identifier MUST be checked against what that client is entitled to reach; naming an identifier MUST NOT by itself grant access.
- **FR-006**: Every input submission MUST be attributable to an authenticated client identity, not to a self-asserted one.

**Raw input delivery**

- **FR-007**: When a command is waiting to read input, raw input sent by a client MUST be delivered to that command rather than being interpreted as a new command line.
- **FR-008**: A command reading input MUST receive it from the session's clients, never from the daemon process's own console.
- **FR-009**: Raw input MUST be delivered byte-for-byte as sent — no trimming of surrounding whitespace, and no substitution of bytes that are not valid text. A password or an exact-match payload must survive intact.
- **FR-010**: Raw input delivered to a waiting command MUST NOT be recorded in the session's command execution history and MUST NOT appear in the session's lifecycle event stream — those surfaces record commands, and an answer to a prompt is not one. This is a confidentiality requirement, not a tidiness one: prompts routinely carry passwords.
- **FR-011**: Ordinary command submission MUST continue to work unchanged; a client MUST be able to say whether it is sending a command to run or input for a waiting reader.
- **FR-012**: Raw input sent while nothing is reading MUST be retained for the next read, within a bounded limit; exceeding that limit MUST be refused with a clear error rather than growing without bound.

**Input authority**

- **FR-013**: At most one attached client MUST hold input authority for a session at any time.
- **FR-014**: Input from a client that does not hold authority MUST be rejected with a clear reason, never silently discarded.
- **FR-015**: A client MUST be able to discover which client currently holds input authority.
- **FR-016**: A client MUST be able to claim input authority, and the previous holder MUST be informed that it no longer holds it.
- **FR-017**: When the authority holder detaches or disconnects — cleanly or abruptly — authority MUST be released so that another attached client can claim it without waiting for a timeout.
- **FR-018**: A session with no clients attached MUST be claimable by the next client that attaches; a session MUST NOT become permanently unanswerable.
- **FR-019**: All attached clients MUST be informed when input authority changes hands.
- **FR-020**: Not holding input authority MUST NOT reduce a client's ability to observe the session's output or events.

### Key Entities

- **Client Identity**: The authenticated principal behind a connection. Established before any session-affecting operation, and the thing input submissions and authority are attributed to.
- **Session Input Channel**: The session-scoped destination for raw input. Holds input sent before a reader is ready, within a bounded limit, and delivers it to whichever command is currently reading.
- **Input Authority**: The right to send input to a session, held by at most one attached client at a time. Transferable by claim, released automatically when its holder goes away.
- **Input Submission**: One client's attempt to send raw input — the bytes, the authenticated identity that sent them, and whether they were accepted or rejected with a reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A process that cannot prove its identity can learn nothing about, and do nothing to, any session — zero information disclosed, zero operations honoured.
- **SC-002**: Connections that stall without identifying are released within a bounded time, and the daemon continues serving legitimate clients while many such connections are attempted.
- **SC-003**: A user can complete an interactive command that prompts for input — including a password prompt — entirely through a connected client, with no access to the daemon's own console.
- **SC-004**: Input delivered to a waiting command is byte-identical to what the client sent, including surrounding whitespace and bytes that are not valid text.
- **SC-005**: 100% of raw input delivered to a waiting command is absent from command execution history and from the lifecycle event stream.
- **SC-006**: With two clients attached and both sending input, the receiving command's input contains bytes from exactly one of them, with zero instances of interleaving.
- **SC-007**: A client whose input is rejected learns why in every case — zero silent drops.
- **SC-008**: After the authority holder disconnects abruptly, another attached client can take over and answer a waiting prompt within seconds, without restarting the session or the command.
- **SC-009**: Ordinary command submission continues to behave exactly as before this feature, verified by existing command, history, and event behavior remaining correct.

## Assumptions

- **The interactive transport is currently unauthenticated, and closing that is in scope.** The first draft of this spec assumed otherwise. Verified directly: the connection path used by the interactive client performs no identity check of any kind, and discloses the session inventory during its opening exchange. The HTTP surface's bearer-token authentication does not cover it. Independently reported as finding A-01. This is why Story 1 exists and why it is first — shipping interactive input onto an unauthenticated transport would make password prompts injectable by any local process.
- **Authority is claimed, not negotiated.** A client that claims input authority receives it immediately and the previous holder is notified. A grant-and-consent protocol was considered and rejected as the default: if the current holder must approve, an unresponsive or departed holder can strand the session, which is the exact failure FR-018 exists to prevent. Claiming is still deliberate, because it is an explicit action rather than a side effect of attaching.
- **The first client to attach an unheld session takes authority.** This matches how a terminal behaves for the overwhelmingly common single-client case, and means nothing extra is required to make ordinary use work.
- **The daemon never echoes raw input.** Whatever a command chooses to display in response to input appears through its normal output. A daemon that echoed input itself would print passwords to every observer.
- **Scope is input to commands running in a session.** Terminal control concerns that ride alongside input in a traditional terminal — window resize signalling, job-control key handling, terminal modes such as raw versus cooked — are deliberately out of scope and are their own work.
- **Retained input is not durable.** Type-ahead that no command consumed is session-lifetime state, not something to persist and replay after a restart, where it could be fed to an unrelated command much later.
- **Existing permission scopes are extended, not replaced.** The HTTP surface already distinguishes levels of access. Identity established for the interactive transport should slot into that same model rather than inventing a parallel one, so a client's rights do not depend on which door it came through.
