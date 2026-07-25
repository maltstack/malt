# Phase 0 Research: Streaming Command Output

**Feature**: `specs/006-streaming-command-output/`
**Date**: 2026-07-25

Each decision below was checked against the code rather than inferred from
the architecture documents. File and line references are from this worktree.

---

## R1. Where output is buffered today, and why nothing streams

**Finding.** The buffering is inside `mash`, not in the daemon.
`ExecResult` (`crates/mash/src/executor.rs:42`) holds
`stdout: Vec<u8>` / `stderr: Vec<u8>`, and `execute_list` returns only when
the whole command is done. External commands are captured with
`out.read_to_end(&mut stdout_bytes)` (`executor.rs:1357`), which by
construction cannot return early. The worker calls `run_command` and only
then sends one `ExecutionCompleted`
(`crates/malt-daemon/src/executor/command_worker.rs:280`).

**Consequence.** No change confined to the daemon can make output
incremental. Every daemon-side path — the compat translator, the renderer,
`/exec` — is downstream of a value that does not exist until the command
ends. This is the single fact that shapes the whole feature.

**Decision.** The change starts in `mash`: a command's *top-level* output
must be written as it is produced rather than accumulated.

**Alternatives considered.**
- *Poll the child process from the daemon.* Rejected: it only helps external
  commands, and the daemon does not own the child — `mash` does. It would
  also require OS handle access outside `malt-platform` (Principle II).
- *Have the daemon diff `get_output` on a timer.* Rejected: this is exactly
  the blind polling US3 exists to remove, and it cannot see output that has
  not been returned yet, which is all of it.

---

## R2. How `mash` emits incrementally without breaking capture

**Constraint discovered.** Not all output may stream. `$(command)` must
capture its output as a value, and in `a | b` the left side's stdout belongs
to `b`. Streaming those to the session would corrupt substitution results and
duplicate pipeline data into the terminal. `ExecResult`'s own doc comment
already says stdout is "only populated when stdout is piped" — the
capture/inherit distinction exists, it just has no third "stream" case.

**Decision.** Add an optional output sink to `Env`, consulted **only for the
top-level command's stdout/stderr when they are not redirected and not part
of a pipeline**. `Env::clone()` is used for subshells and command
substitution, so the sink must not be inherited into a capturing context —
substitution explicitly clears it.

**Rationale.** It reuses the distinction `mash` already draws instead of
inventing a parallel notion of "the real stdout", and it keeps the change out
of every command's implementation: the executor decides once, at the point
where it currently chooses between capture and inherit.

**Alternatives considered.**
- *Register a pipe at fd 1 and let everything write to it.* Attractive
  because external children inherit it for free, and it is symmetric with
  what fd 0 does for input. Rejected as the primary mechanism because
  builtins and in-process tools return `BuiltinResult`/`ExecResult` buffers
  rather than writing to fd 1, so it would fix external commands only —
  the same partial fix that made "external processes can't read stdin" look
  true in feature 005 when the real gap was elsewhere. It remains the likely
  *implementation* for the external-command branch behind the sink.
- *Change `ToolFn` to take a writer.* Deferred to US4. It is the right end
  state for in-process tools but is not needed for US1–US3, and folding it in
  early would make the P1 slice depend on touching all 17 tools.

---

## R3. Whether streaming forces `/exec` to change

**Tension.** FR-012 requires bounded memory for a 100 MB command. The
existing `/exec` contract returns the command's whole output as a string
(`CommandOutput.output`, `session_thread.rs:127`). Both cannot hold: if
nothing accumulates, `/exec` has nothing to return; if everything
accumulates, memory is unbounded.

**Decision.** Keep `/exec` returning output, but **bounded and explicitly
truncated**. The daemon accumulates up to a cap for the reply; beyond it, the
reply states that it was truncated and how much was omitted, and directs the
caller to the stream.

**Rationale.** Silent truncation is the failure this project keeps finding —
an observer given a quietly incomplete picture draws confident wrong
conclusions. FR-009 already forbids silent loss for subscribers; the same
standard applies to the one-shot reply. Removing `/exec`'s output entirely
would break every existing caller for a benefit the stream already provides.

**Alternatives considered.**
- *Unbounded accumulation, cap only the stream.* Rejected: fails SC-004 and
  makes any single command a memory-exhaustion vector.
- *Drop output from `/exec` and require the stream.* Rejected: a breaking
  change to the surface the MCP tools and `malt exec` use, for no gain to a
  caller running a short command.

---

## R4. Getting chunks from the worker to the control actor

**Finding.** The two threads are already connected in both directions:
`ExecutionIngress` (control → worker) and `SessionCommand::ExecutionStarted`
/ `ExecutionCompleted` (worker → control, over `control_tx`). Feature 002
built this split, which is what makes this feature possible at all — the
control actor is now free while a command runs.

**Decision.** Chunks travel worker → control as a new `SessionCommand`
variant over the existing `control_tx`, with a bounded channel. When the
channel is full the worker **blocks**.

**Rationale.** Blocking the producer is the correct backpressure here and is
what a real pipe does when a reader falls behind. It is safe because the
consumer is the session's own control actor, which is guaranteed to drain and
is no longer occupied by command execution. This is deliberately *not* the
policy used for subscribers (R5): a subscriber is an untrusted third party
that may vanish, whereas the control actor is part of the session.

**Alternative considered.** *A separate chunk channel.* Rejected: two
channels between the same pair of threads cannot promise ordering between
chunks and `ExecutionCompleted`, so a command could report completion before
its last output. Feature 005 hit the same ordering question with per-client
messages and resolved it the same way — one ordered stream.

---

## R5. Delivering to subscribers: reuse the 004 policy, do not invent a second

**Finding.** `crates/malt-daemon/src/executor/events.rs` already answers this
exact question for lifecycle events: a bounded ring (`MAX_RETAINED_EVENTS =
1024`), a per-subscriber channel (`SUBSCRIBER_BUFFER = 256`) allocated one
slot larger so the **terminal gap notification can always be delivered**, and
a `Gap { reason }` telling a lagged subscriber what it missed before it is
dropped.

**Decision.** Output subscriptions use the same structure, sized for bytes
rather than events, and the same gap semantics. Where the shapes are
genuinely identical the code should be shared rather than copied.

**Rationale.** The spec makes this an explicit assumption. Two
differently-behaved answers to "what happens when a consumer stops reading"
is precisely the class of divergence that produced the three-copy slurp bug
and the two-parallel-attach-paths bug already fixed in this repo.

**Note for implementation.** The reserved-slot detail is not incidental. Its
absence caused subscribers to be dropped *silently* in feature 004 — the gap
notification's own send failed on the buffer that had just overflowed. Any
copy of this structure that omits it reintroduces that bug.

---

## R6. Byte fidelity over a text transport

**Constraint.** Output is arbitrary bytes; it may be invalid UTF-8, and a
multi-byte character can be split across chunk boundaries (FR-011, SC-006).
The events stream is Server-Sent Events, which is text and line-oriented.

**Decision.** Chunk payloads are **base64** on the HTTP stream. The VNP path
carries bytes natively and needs no encoding.

**Rationale.** Lossy decoding at the chunk boundary is not recoverable by the
client: `from_utf8_lossy` on half a character yields a replacement symbol,
and the other half yields a second one, so reassembly cannot restore the
original. Encoding at the transport keeps the daemon from having to buffer
until a character completes, which would otherwise be a second place output
gets held.

**Alternative considered.** *Buffer partial characters daemon-side and emit
only complete text.* Rejected: it makes the daemon's output path
text-oriented, which is wrong for binary output, and a command that emits a
lone invalid byte would stall its own stream waiting for a completion that
never comes.

---

## R7. Attached clients get US2 largely for free

**Finding.** `CompatTranslator::feed(&[u8])` (`crates/malt-compat/src/
translator.rs:46`) is already incremental, and `dispatch_render()` already
diffs and pushes `ClientMessage::Render` per client with lag/shed handling in
`malt-renderer`.

**Decision.** The control actor feeds each chunk to the translator as it
arrives and calls `dispatch_render()`, rather than feeding one slice at
finalization.

**Rationale.** The machinery exists and is tested; what is missing is being
called at the right time. Note that this is the *third* time in this feature
family that the gap turned out to be "the mechanism exists but nothing calls
it" — Gateway auth, `AuthorityTracker`, and now this. Worth checking that
assumption early rather than building a parallel path.

**Confirmed while writing the Phase 1 contracts.** `OutputChunk` already
exists in `schemas/shell.vexil:37` at `@domain(Shell) @type(0x04)`, with
`MSG_OUTPUT_CHUNK` already in `codec.rs:30`, and its doc comment says MASH
sets the command association "at emission time". The protocol anticipated
streaming output and nothing was ever wired to it. This is the fourth
instance of the pattern in this feature family, after Gateway auth,
`AuthorityTracker`, and `InputClaim`/`InputAuthorityChanged`. The rule that
follows: **before adding a message, constant, or type, check whether it is
already defined and merely unused.**

**Caution.** The known terminal-grid "staircase" rendering defect
(`docs/BACKLOG.md` P0) lives in this path. It is explicitly out of scope, but
streaming will make it *more* visible, and it must not be mistaken for a
regression introduced here.

---

## R8. Access control

**Decision.** The output stream requires the same scope as existing output
and event observation — `Read` — and no more.

**Rationale.** FR-017. Feature 005 established that observation is `Read`
while typing is `Interact`; streaming output is observation. Making it
anything else would create a way to read a session that a client could not
otherwise read, or gratuitously deny observation to clients that already have
it.

---

## Open questions carried into planning

None blocking. Two sizing choices are deliberately left to implementation
because they are tuning, not semantics, and both must be named constants with
their reasoning in a comment:

- The retained-output ring size, and the per-subscriber buffer size.
- The `/exec` truncation cap (R3).
