# Feature Specification: Streaming Command Output

**Feature Branch**: `006-streaming-command-output`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "Output from a running command must reach watchers while it is still running, not only when it finishes."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A long command's output arrives while it is still running (Priority: P1)

Someone starts a command that takes minutes and produces output steadily — a
test suite, a build, a package install. Today they see nothing at all until
the command finishes, and then everything at once. A session that has been
silent for two minutes is indistinguishable from a session that has hung, so
the only way to tell whether work is progressing is to wait for it to stop.

Output should arrive as it is produced, so a watcher can tell that the command
is alive and see how far it has got.

**Why this priority**: This is the whole feature. Everything else here is
either a consequence of it or a refinement of who receives it. Without it, no
amount of event or history reporting makes a running command observable — an
observer can see *that* something started and, eventually, that it ended, but
nothing in between.

**Independent Test**: Run a command that emits a line, waits, emits another
line, and exits. Confirm the first line is observable before the command has
exited, and that the observation carries no dependency on the command's
completion.

**Acceptance Scenarios**:

1. **Given** a command that prints a line and then continues running, **When**
   a watcher asks for the session's output, **Then** the line printed so far
   is returned without waiting for the command to end.
2. **Given** a command producing output over time, **When** a watcher observes
   continuously, **Then** it receives output progressively rather than in a
   single batch at the end.
3. **Given** a command that produces output and then fails, **When** it exits
   non-zero, **Then** the output produced before the failure has already been
   delivered and is not lost or replaced by the failure report.
4. **Given** a command that produces no output for a long time, **When** a
   watcher observes, **Then** silence is reported as an absence of output
   rather than the watcher blocking until the command ends.

---

### User Story 2 - An attached human sees a running command's output live (Priority: P2)

A person attached to a session watches a command run. What they see should
track what the command has produced, updating as it goes, the way a terminal
does. Today a human attached to a session where an agent started a long
command sees a frozen screen until it completes.

**Why this priority**: This is what makes the session usable by a person
during long work, and it is the difference between "the daemon knows the
output" and "the human can see it". It depends on US1 but is separable: US1
can be delivered and verified through a non-interactive observer first.

**Independent Test**: Attach a client to a session, run a command from
elsewhere that produces output over several seconds, and confirm the attached
client's view updates more than once before the command completes.

**Acceptance Scenarios**:

1. **Given** an attached client, **When** a command produces output over
   several seconds, **Then** the client receives more than one update before
   the command finishes.
2. **Given** two clients attached to one session, **When** a command produces
   output, **Then** both see the same content, and neither sees output the
   other does not.
3. **Given** a client that is slow to consume updates, **When** a command
   produces output faster than that client accepts it, **Then** the session
   and the other clients are unaffected.

---

### User Story 3 - An agent consumes output incrementally without polling blindly (Priority: P2)

A program driving a session — an AI agent, a CI script — wants to react to
output as it appears: to notice a prompt, to detect a failing test, to decide
whether to interrupt. Today its only options are to wait for completion or to
re-read the whole accumulated output repeatedly and diff it itself.

An agent should be able to consume output as a continuing stream, and to
resume where it left off rather than re-reading from the beginning.

**Why this priority**: Equal in importance to US2 and separable from it — one
serves a person watching, the other a program reacting. It is what lets an
agent resume from where it stopped instead of scraping and diffing, which is
the same problem command lifecycle events solved for start and finish.

**Independent Test**: Consume a session's output stream while a command runs,
disconnect partway, reconnect asking to resume from the last position
received, and confirm no output is duplicated and none is skipped.

**Acceptance Scenarios**:

1. **Given** a running command, **When** an agent consumes the output stream,
   **Then** it receives output as produced without re-reading prior output.
2. **Given** an agent that disconnects mid-command and reconnects stating
   where it stopped, **When** it resumes, **Then** it receives what it missed,
   with nothing duplicated and nothing skipped.
3. **Given** an agent that stops consuming, **When** output continues to be
   produced beyond what will be retained for it, **Then** it is told that it
   fell behind and what it missed, rather than being silently given an
   incomplete picture.

---

### User Story 4 - In-process utilities stream their output too (Priority: P3)

Some commands are served by the session's own built-in utilities rather than
by launching a separate program. Those should behave like any other command: a
utility that reads input and writes output continuously — echoing typed lines,
filtering a stream — should have its output observable as it works, not only
once it stops.

**Why this priority**: It is a real inconsistency and it is visible to anyone
who tries an interactive built-in, but it affects a bounded set of commands
and is the narrowest slice. It also depends on the delivery path US1
establishes, so it is genuinely last.

**Independent Test**: Run a built-in utility that copies its input to its
output, send it several lines over time without ending its input, and confirm
each line becomes observable before the next is sent.

**Acceptance Scenarios**:

1. **Given** a built-in utility copying input to output, **When** a line is
   sent, **Then** that line is observable before any further input is sent.
2. **Given** a built-in utility whose output is redirected to a file, **When**
   it runs, **Then** the redirection behaves exactly as it does today.

---

### Edge Cases

- **A command producing output faster than it can be delivered.** The session
  must stay responsive and must not grow without bound. Losing output is
  preferable to stalling the session, but loss must be reported, never silent
  — an observer given a quietly incomplete picture will draw confident wrong
  conclusions from it, which is worse than being told it missed something.
- **A watcher that stops consuming and never returns.** It must not be able to
  hold output, memory, or a session hostage. This is the same slow-consumer
  question command lifecycle events answered, and it must be answered the same
  way rather than a second way.
- **Output that is not valid text**, or that arrives split mid-character.
  Delivering half a character as a replacement symbol and the other half as a
  second one corrupts output that would be fine if reassembled.
- **A command producing a single enormous line** with no break for a long time.
- **A command producing output and then being interrupted or killed.** What it
  produced before dying must already have been delivered.
- **The daemon stopping mid-command.** Output delivered before the stop must
  remain consistent with what history and events report about the same
  command; the three must not disagree.
- **A watcher attaching partway through a running command.** It must get a
  coherent view rather than an arbitrary fragment starting wherever it
  happened to arrive.
- **Interleaving of a command's normal output and its error output**, and
  whether a watcher can still tell them apart once streamed.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Output produced by a running command MUST be observable before
  that command finishes.
- **FR-002**: Observing a session's output MUST NOT block until the running
  command completes.
- **FR-003**: Output MUST be delivered in the order the command produced it.
- **FR-004**: A command's normal output and its error output MUST remain
  distinguishable after delivery.
- **FR-005**: Output produced before a command fails, is interrupted, or is
  killed MUST still be delivered.
- **FR-006**: Clients attached to the same session MUST receive the same
  output content.
- **FR-007**: An attached client MUST receive more than one update during a
  command that produces output over time.
- **FR-008**: A consumer MUST be able to resume from a stated position and
  receive what it missed, with nothing duplicated and nothing skipped.
- **FR-009**: A consumer that falls too far behind MUST be told that it lagged
  and what it missed. Output MUST NOT be silently dropped from its view.
- **FR-010**: A consumer that stops consuming MUST NOT stall the session,
  block the command, or cause unbounded growth.
- **FR-011**: Delivery MUST NOT corrupt output that is not valid text, and
  MUST NOT corrupt characters split across delivery boundaries.
- **FR-012**: The daemon's memory use for a session's output MUST be bounded
  regardless of how much output a command produces.
- **FR-013**: What is delivered as output MUST be consistent with what the
  session reports about the same command through its execution history and its
  lifecycle events.
- **FR-014**: Existing redirection of a command's output MUST behave as it
  does today.
- **FR-015**: Commands served by the session's built-in utilities MUST have
  their output observable while they run, on the same terms as any other
  command.
- **FR-016**: A client attaching during a running command MUST receive a
  coherent view of the session rather than an arbitrary fragment.
- **FR-017**: Output delivery MUST be subject to the same access rules as
  existing output and event observation, so streaming does not become a way to
  read a session that a client could not otherwise read.

### Key Entities

- **Output chunk**: A piece of a command's output as produced, with the
  command it belongs to, whether it is normal or error output, and its
  position in that command's output. Position is what makes resumption
  possible.
- **Output stream position**: A consumer's place in a session's output, used
  to resume without duplication or loss.
- **Output subscriber**: A consumer receiving output as it is produced, with a
  bounded amount held for it and a defined outcome when it falls behind.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A command that prints a line and then keeps running has that
  line observable within 1 second of it being printed, with no dependency on
  when the command ends.
- **SC-002**: For a command producing output steadily for 30 seconds, an
  attached client receives at least 10 separate updates before completion.
- **SC-003**: A consumer that disconnects mid-command and resumes from its
  last stated position receives every intervening chunk exactly once —
  verified by content, not by count.
- **SC-004**: A command producing 100 MB of output completes successfully, and
  the daemon's memory use for that session stays bounded throughout.
- **SC-005**: A subscriber that stops reading entirely does not delay the
  command, does not delay other subscribers, and does not grow the daemon's
  memory without limit; it is told it lagged.
- **SC-006**: Byte-for-byte, streamed output matches what the same command
  produces when its output is captured at completion, including non-text bytes
  and multi-byte characters split across chunk boundaries.
- **SC-007**: With two clients attached and a command running, both clients'
  views converge on identical content.
- **SC-008**: A built-in utility copying input to output makes each line
  observable before the next is sent.

## Assumptions

- **"Observable" covers both a pull and a push consumer.** A watcher asking
  for output now and a watcher subscribed to output as it appears are both in
  scope; requirements are written so as not to presume one shape.
- **The existing slow-consumer policy is the model to follow, not to
  reinvent.** Command lifecycle events already settled how a session treats a
  subscriber that stops reading — bounded retention, told it lagged, dropped
  rather than accommodated. Output delivery should answer the same question
  the same way rather than introducing a second, differently-behaved policy.
- **Retention is bounded and output is not durable.** Streaming makes output
  observable as it is produced; it does not promise that arbitrary past output
  can be replayed later. Long-term retention of full output is out of scope.
- **The prerequisite is already met.** Command execution no longer occupies
  the session's control path, which is what previously made intermediate
  delivery impossible. This feature does not need to revisit that.
- **Existing observation surfaces stay.** Asking for a session's current
  output as a whole continues to work; this feature adds incremental
  observation rather than replacing what exists.
- **Scope excludes rendering improvements.** How a client draws what it
  receives, and the known terminal-grid rendering defects, are separate
  concerns tracked elsewhere.
- **Scope excludes interactive programs that drive the screen directly**
  (full-screen editors, pagers). Those need terminal emulation behaviour
  beyond incremental delivery of a command's output.
- **One command at a time per session.** A session runs one command at a time,
  so output attribution is unambiguous; this feature does not introduce
  concurrent commands in a session.
