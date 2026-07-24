# Findings: Genuine Raw Input and Human/Agent Coexistence Audit

Date: 2026-07-24
Context: Strategic hardening push, following the decision to retire AGENTS.md's
phased Implementation Roadmap in favor of a correctness/hardening-first
strategy. This audit covers the subset of that strategy scoped as "genuine raw
input, human and agent coexistence" — i.e. whether a human can actually attach
to a session an agent is driving, see what's happening, and hand off input,
per the architecture's Multi-Client Input Authority design
(`docs/design/architecture.md` lines 109-129, 259-269, 2361-2370).

Method: read `crates/malt-daemon/src/executor/session_thread.rs`,
`crates/malt-daemon/src/executor/coordinator.rs`,
`crates/malt-daemon/src/gateway_backend.rs`,
`crates/malt-daemon/src/connection/authority.rs`,
`crates/malt-daemon/src/vnp_listener.rs`, `crates/malt-renderer/src/host.rs`,
`crates/malt-compat/src/translator.rs`, `crates/malt-tui/src/{connection,render,app,main}.rs`,
`crates/mash/src/executor.rs` (stdin/`read` builtin), `crates/malt-platform/src/process/*.rs`,
and `schemas/{session,common}.vexil`, then traced every real call site
(not just the type definitions) to determine what's actually wired versus
what's data-modeled but inert. Research only — nothing in this repo was
modified besides this file.

## Headline finding

**There is no code path anywhere in the daemon that delivers live input to an
in-flight process, and no code path that lets an attached human client see an
agent's execution happen in real time.** Both halves of "human and agent
coexistence" — an agent's work being visible to an attached human, and a
human's input reaching a running process — are currently absent, not merely
buggy. The gaps are structural (single-threaded per-session execution model,
render dispatch wired to only one of several input paths), not a handful of
one-line bugs. Sections 1-4 below give the evidence; the prioritized fix list
is at the end.

## 1. No real path for writing to a live process's stdin — CONFIRMED

All three daemon-facing "send input" entry points funnel into
`SessionExecutor::run_mash_command` (`crates/malt-daemon/src/executor/session_thread.rs:458`),
which does `parser::parse(input)` and `execute_list(&commands, ...)` — i.e.
**parses the bytes as a brand-new top-level command line and runs it to
completion**, not "write these bytes to whatever is already reading stdin."

- Gateway `send_input` (`crates/malt-daemon/src/gateway_backend.rs:117-134`):
  dispatches `SessionCommand::RunCommand { command: input, .. }` — identical
  to `exec_command`'s path (line 91-115). Already confirmed wrong in
  `docs/BACKLOG.md`'s P1 list before this audit.
- `SessionCommand::WriteInput` (`session_thread.rs:322-328`): does
  `String::from_utf8_lossy(&data)`, trims it, and calls
  `self.run_mash_command(input)` if non-empty — **exactly the same
  parse-as-new-command behavior as `send_input`**, just reached via a
  different enum variant. Nothing distinguishes "new command" from "stdin for
  the thing already running" at this layer; there is no "thing already
  running" state for it to be distinguished against, because `run_mash_command`
  is synchronous and only one is ever in flight per session (see §3).
- `SessionCommand::KeyInput` (`session_thread.rs:364-388`): feeds keys through
  `malt-term`'s `Editor` line editor; only on `EditResult::Accept(line)` does
  it call `run_mash_command(&line)` — again, a fully-formed line becomes a new
  top-level command. There is no lower-level "this scriptis blocked on `read`,
  route raw bytes there instead" branch.

**All three converge on the same synchronous, parse-and-run function.** This
was the specific fact requested for this audit, and it is definitively true.

**mash's own stdin model corroborates this — it has no concept of a "virtual
stdin fed by a remote client" either.** `builtin_read`
(`crates/mash/src/executor.rs:3579-3610`) resolves stdin as: explicit
`stdin_file` (from a pipeline or `N<file` redirect) → `env.open_fd_read(0)`
(the `fd_registry`, used for `exec N<file`-style FD table entries) →
**`std::io::stdin()` — the daemon process's own real OS-level stdin** (line
3605). Since MASH runs in-process inside the daemon
(`AGENTS.md`: "In-process, privileged"), a bare `read` with no redirect,
executed mid-script inside `run_mash_command`, blocks the session's executor
thread waiting on the *daemon binary's own* stdin — not on anything a VNP or
HTTP client sent. If the daemon runs headless/backgrounded, that read blocks
forever (or immediately EOFs, depending on how stdin was inherited at daemon
start); if the daemon happens to be running attached to an operator's own
terminal, keystrokes typed into *that* terminal — not the remote client's
session — would answer the prompt. Either way, it is disconnected from the
session's actual remote client.

**Spawned external processes are wired the same way.** For a plain external
command (e.g. an interactive `python3` REPL, `ssh`, a `sudo` password prompt)
with no explicit redirect and no pipe stdin, `mash::executor` sets
`config.stdin = malt_platform::process::Io::Inherit`
(`crates/mash/src/executor.rs:1264-1271`) — the child inherits the **daemon
process's own stdin handle** directly (`Io::Inherit => Stdio::inherit()` in
both `crates/malt-platform/src/process/unix.rs:16` and
`crates/malt-platform/src/process/windows.rs:57`). There is no PTY-per-session
abstraction interposed for MASH's own child processes (the PTY/Process
Supervisor path is a separate subsystem for app/compat panes, per
`architecture.md` line 137, and per `AGENTS.md`'s P2 backlog note is not even
isolation-wired yet). A remote client's keystrokes cannot reach such a child
at all today, through any path.

**Conclusion for item 1: confirmed, not suspected.** No REPL, no `read`
prompt, no password prompt, no interactive external program can currently
receive input from a VNP or HTTP client while it's running. This fully
explains the `docs/BACKLOG.md` P1 note that "simple standalone commands work
... while genuinely interactive cases ... wouldn't" — it's not a narrow gap in
`send_input`, it's the absence of the entire feature at every layer.

## 2. `InputAuthority` is a fully inert data model in production — CONFIRMED

`AuthorityTracker` (`crates/malt-daemon/src/connection/authority.rs`) is
real, correct, and has passing unit tests
(`crates/malt-daemon/tests/authority.rs`) for its own internal logic
(attach/detach/claim/FIFO fallback). But tracing every real call site shows it
is **never exercised by any production code path** — only by tests that
construct and send commands directly:

- `SessionCommand::AttachClient` / `SessionCommand::DetachClient` — the only
  two commands that touch `self.authority` at all
  (`session_thread.rs:291,295`) — are sent **only** from
  `crates/malt-daemon/tests/session_thread.rs:52,58` and
  `crates/malt-daemon/tests/coordinator.rs:129`. A repo-wide grep for
  `SessionCommand::AttachClient` and `SessionCommand::DetachClient` finds zero
  matches in any non-test file. `Coordinator` has no method that sends either
  variant (confirmed by grepping `coordinator.rs` for both).
- The real VNP attach path (`RegisterVnpClient`, `session_thread.rs:333-359`)
  never touches `self.authority` — it only calls `self.renderer.register_client`
  and stashes the render sender. The real VNP detach path
  (`UnregisterVnpClient`, line 360-363, invoked via `cleanup()` and
  `DetachSession` handling in `vnp_listener.rs:433-461` and `:514-530`)
  likewise never touches `self.authority`.
- The wire-level `AttachSession.authority` field the client actually sends
  is **parsed and discarded**: `vnp_listener.rs::wait_for_attach`
  (line 275-332) unpacks the full `AttachSession` message but returns only
  `attach.session_id.0` (line 315) — `attach.authority` is read off the wire
  and never looked at again. `malt-tui`'s `VnpConnection::connect`
  (`crates/malt-tui/src/connection.rs:358-362`) hardcodes
  `authority: InputAuthority::Exclusive` on every attach with no `--observe`
  flag or CLI support to send anything else (confirmed by grepping
  `malt-bin/src` for `observe`/`input-claim` — zero hits; the CLI commands
  architecture.md describes at line 117-118 don't exist).
- The `InputClaim`/`InputAuthorityChanged` wire messages
  (`schemas/session.vexil:41-53`) have real codec constants
  (`MSG_INPUT_CLAIM = 0x06`, `MSG_INPUT_AUTHORITY_CHANGED = 0x07` in
  `crates/malt-protocol/src/codec.rs:57-58`) but **zero handling** in
  `vnp_listener.rs::dispatch_frame`'s match (only `MSG_KEY_EVENT`,
  `MSG_RESIZE`, `MSG_FRAME_ACK`, `MSG_DETACH_SESSION` are handled; anything
  else — including a hypothetical `InputClaim` — falls into the `_ =>` "log
  and ignore" branch at line 463-470). Even a client that implemented the
  claim message today would be silently ignored server-side.
- Most decisively: `SessionCommand::KeyInput` — the only command that
  actually drives input into the line editor — **doesn't carry a `client_id`
  at all** (`session_thread.rs:96`, and `Coordinator::send_key_input`,
  `coordinator.rs:381-392`, takes only `session_id` and `key`). Structurally,
  there is no way to gate this on "is this the authoritative client," because
  the message doesn't identify who sent it. Every attached client's `KeyInput`
  — there being only ever one live sender in practice today, since only one
  VNP client can be attached usefully (see §3) — is processed unconditionally.

**Conclusion for item 2: confirmed, not suspected.** `InputAuthority` is
real, tested, well-designed data modeling with zero production wiring on
either end (claim is discarded on attach; nothing gates on the holder). It is
not "mostly working with an edge case gap" — it is not connected to anything
live at all.

## 3. Multi-client / concurrent-view correctness — worse than the known P0 follow-on gap, and the root cause is structural

The task asked me to check whether a second VNP client attaching mid-session
gets a real current-state snapshot or only today's known "latest chunk"
follow-on gap (`docs/BACKLOG.md`'s P0 entry, `CompatTranslator::frame_element()`
returning only `last_data`, the most recent `feed()` call). That gap is real —
confirmed by reading `crates/malt-compat/src/translator.rs:41-46`: `frame_element()`
always returns `FrameElement::VtPassthrough { data: self.last_data.clone() }`,
and `last_data` is overwritten (not appended, post-today's-fix) on every
`feed()`. `RegisterVnpClient`'s handler (`session_thread.rs:341-346`) uses
exactly this for the `InitialState` snapshot. So yes: a second client attaching
mid-session gets only the most recent output chunk as raw VT bytes, not the
full current screen. `CompatTranslator::grid()` (line 64-66) does hold the
full, correct cell-level state (`TerminalGrid`) — but nothing on the VNP path
reads it; only the separate HTTP `get_output`/`StyledGrid` route
(`coordinator.rs::get_session_output` → `session_thread.rs::get_grid_output`,
line 522 `let grid = compat.grid();`) does.

**But this turns out not to be the operative bug, because of two deeper
problems that make the "latest chunk" gap almost moot:**

### 3a. The live VNP client never renders shell output at all, regardless of attach timing

`crates/malt-tui/src/render.rs:130-132` — `TuiRenderer::apply_one`'s catch-all
arm explicitly does **not** handle `RenderCommand::WriteRaw` ("Flush,
SetCursor, WriteRaw, and other commands are handled at the application
level, not by the buffer renderer" — but nothing at the application level
handles it either; `crates/malt-tui/src/app.rs:36` just calls
`self.renderer.apply(commands, buf)`, the same function). A repo-wide grep for
`WriteRaw` in `malt-tui` finds only these two comments — no handler exists
anywhere in the crate.

Since `FrameWalker` turns `FrameElement::VtPassthrough` into exactly one
`RenderCommand::WriteRaw` (`crates/malt-renderer/src/walker.rs:195-199`), and
`CompatTranslator` — the only thing that ever produces shell-output
`FrameElement`s in this codebase — only ever emits `VtPassthrough`, **every
`RenderBatch`/`InitialState` a real VNP client receives for actual shell
output is silently dropped on arrival.** `malt-tui --vnp` shows only
`DrawText`/`DrawRect`/`DrawBorder`/`Clear` commands, which today only exist in
the hardcoded demo (`crates/malt-tui/src/main.rs:139-158`, `mock_connection()`).
Confirmed by tracing `main.rs::run_loop` (line 65-110) → `App::process_commands`
(`app.rs:33-36`) → `TuiRenderer::apply` with no other consumer of
`poll_commands()`'s output anywhere in the binary.

This is why the P0 "staircase" rendering bug from
`docs/findings/2026-07-24-live-daemon-session.md` was observed via the HTTP
`/sessions/{id}/output` route and not via a live TUI session: **`HttpConnection`**
(`crates/malt-tui/src/connection.rs:172-253`) is a completely separate,
parallel rendering path — it polls the `StyledGrid` JSON directly and
synthesizes its own `DrawText` commands client-side (line 189-224), bypassing
`WriteRaw`/`VtPassthrough`/the FrameElement pipeline entirely. It happens to
work (modulo the P0 bug, now fixed at the grid layer) precisely because it
ignores the "authoritative" RenderCommand pipeline architecture.md describes
as canonical. The VNP path — the one the architecture treats as real-time and
authoritative — currently shows nothing for shell content, on first attach or
any subsequent attach, independent of the `frame_element()` staleness gap.

### 3b. Gateway/agent-driven execution never triggers a render dispatch to attached clients at all

This is the more severe finding relevant to the demo's step 3/4. Grepping
`session_thread.rs` for `dispatch_render()` calls finds exactly three call
sites (lines 370, 377, 381) — all three inside the `KeyInput` handler's
`EditResult::Accept` / `Interrupt` / `Eof|Suspend` arms. **`RunCommand`
(gateway `/exec`, the ADR-0002 "canonical agent control plane" path),
`WriteInput`, and `PtyOutput` never call `dispatch_render()`.**
`run_mash_command` (line 458-510) feeds `compat.feed()` and publishes to the
bus, but that's it — no client's `render_pushers` entry is touched until some
*unrelated* subsequent event (a human's own next keystroke reaching
`KeyInput`'s Accept branch) happens to trigger a dispatch, at which point it
would push whatever the (already-stale, per §3a/the `frame_element()` gap)
latest frame happens to be.

**Practical consequence for the demo scenario:** if an agent runs `cargo test`
via `POST /exec` (step 1), and a human attaches via VNP while it's running
(step 3), the human's client will receive **no RenderBatch reflecting that
command's output, ever** — not during execution, not after it completes —
regardless of the `WriteRaw`-drop bug in §3a. The only thing that would ever
push a frame to that human is the human's *own* subsequent keystroke.
"Both see the same authoritative state" (the architecture's framing, line
1737) does not hold today: the gateway and VNP paths write to the same
session state but only one of them (VNP `KeyInput`, on line completion) ever
notifies attached renderer clients that anything changed.

### 3c. The single-threaded per-session executor blocks attach/input/output entirely during a long-running command

`SessionExecutor::run` (`session_thread.rs:271-415`) is one dedicated thread
per session, looping on a single blocking `rx.recv()` over one
`mpsc::Receiver<SessionCommand>` (spawned at `session_thread.rs:193,228`).
`RunCommand`'s handler (line 305-308) calls `self.run_mash_command(&command)`
**synchronously** — which calls mash's `execute_list`, explicitly documented
in `AGENTS.md` as "synchronous — perfect for session executor's thread
model." For a short command this is invisible; for `cargo test` (the demo's
own example) it means **the executor thread cannot process any other queued
`SessionCommand` — including `RegisterVnpClient` (attach),
`GetOutput`, `KeyInput`, `AckFrame`, or `Resize` — until the running command
finishes**, because they all share the one channel, drained one at a time.

This has a concrete, testable, user-visible failure mode:
`Coordinator::register_vnp_client` (`coordinator.rs:282-336`) waits on
`initial_rx.recv_timeout(Duration::from_secs(5))` (line 325). If a human
tries to attach (`malt attach`/`VnpConnection::connect`) while a
longer-than-5-second command is in flight on that session, `register_vnp_client`
times out, returns `Err(DaemonError::SessionUnreachable)`, and
`vnp_listener.rs::handle_client` (line 162-182) logs a warning and returns —
**closing the TCP connection outright.** The human's `malt attach` fails with
a connection error, not a "please wait, busy" state, even though the session
is healthy and merely busy. The same pattern applies to `GetOutput`
(`coordinator.rs:178-195`, 2-second timeout at line 190) — an agent or human
polling output during a long command gets a timeout error instead of "not
ready yet." `exec_command`'s own 30-second reply timeout
(`gateway_backend.rs:106-108`) means a `cargo test` run longer than 30s
returns `GatewayError::Internal("command timed out")` to the caller — while
the command **keeps running to completion inside the daemon** regardless
(the `reply.send(output)` at line 307 is a no-op `let _ =` if the receiver
already gave up), permanently occupying the session thread and blocking
every other operation on that session until it finishes, with no way for the
timed-out caller to learn when that happens or retrieve the result.

**Conclusion for item 3:** the follow-on gap noted in `docs/BACKLOG.md` (stale
`InitialState` snapshot) is real but is not the binding constraint. Even a
perfect full-state snapshot on attach would not help, because (a) the client
that would receive it doesn't render `WriteRaw` at all, (b) gateway-driven
execution never triggers a render push to begin with, and (c) attaching
during exactly the scenario the demo describes — a long-running command — is
liable to fail outright on a hardcoded timeout rather than degrade gracefully.
This is a structural gap in the single-threaded-per-session execution model,
not a rendering bug.

## 4. Detach/reattach during interactive input

Since `InputAuthority` is not gated on in production (§2), "what happens to
authority when the holder disconnects" is moot in terms of user-visible
behavior today — nothing currently checks who holds authority, so there is no
way to observe a session "stuck with no one able to claim input." But it's
worth recording precisely, because it reinforces how disconnected the
`AuthorityTracker` subsystem is from the real client lifecycle: the real VNP
disconnect paths (`cleanup()` at `vnp_listener.rs:513-530`, and the
`DetachSession` handler at line 433-461) both call
`coord.unregister_vnp_client()` → `SessionCommand::UnregisterVnpClient` —
which, per §2, never touches `self.authority`. Only the dead
`SessionCommand::DetachClient` variant calls `self.authority.detach()`
(`session_thread.rs:294-295`), and nothing in production ever sends it. So
even the `AuthorityTracker`'s own internal bookkeeping (which client IDs it
thinks are attached) silently accumulates stale entries across every real
VNP disconnect — harmless today only because nothing reads `.holder()`
in production, but it means the tracker's state does not reflect reality even
on its own terms, should something start wiring reads of `.holder()` later
without also fixing the detach wiring.

## Summary table

| # | Finding | Status |
|---|---|---|
| 1 | No live-stdin-write path anywhere (gateway, VNP, or mash's own `read`/external-spawn) | Confirmed |
| 2 | `InputAuthority`/`AuthorityTracker` fully unwired in production; claim discarded on attach, nothing gates `KeyInput`, `KeyInput` doesn't even carry `client_id` | Confirmed |
| 3a | VNP client (`malt-tui`) drops all `WriteRaw`/VT-passthrough content — shows no real shell output via the "authoritative" render pipeline, first or second attach | Confirmed |
| 3b | `RunCommand`/`WriteInput`/`PtyOutput` never call `dispatch_render()` — gateway/agent-driven execution is invisible to attached VNP clients regardless of timing | Confirmed |
| 3c | Single-threaded per-session executor blocks attach/output/input entirely during a long command; attach can hard-fail after a 5s timeout | Confirmed |
| 3 (orig.) | `frame_element()` staleness gap noted in `docs/BACKLOG.md` — real, but dominated by 3a/3b/3c | Confirmed, secondary |
| 4 | Authority state not cleaned up on real VNP detach (only the dead `DetachClient` path clears it) | Confirmed, currently inert |

## Prioritized recommendations

1. **Decouple command execution from the session's command-dispatch loop.**
   Severity: Critical. Effort: Large (architectural). Unblocks demo steps
   3, 4, 6. Root cause of §3c and a precondition for §1 and §3b ever working:
   as long as `RunCommand` blocks the same thread that must service
   `RegisterVnpClient`/`KeyInput`/`GetOutput`, no fix to rendering or input
   plumbing can make "attach while running" or "send input mid-execution"
   actually work. Likely needs `execute_list` (or at least the top-level
   command dispatch) to run on a worker thread/task per invocation while the
   session executor's main loop keeps servicing control-plane commands
   (attach, resize, ack, and — once built — stdin writes) concurrently. This
   is the highest-leverage single change: several of the other findings stop
   being reachable once it's in place, but none of them can be fixed
   correctly without it first.

2. **Wire `dispatch_render()` into `RunCommand`/`WriteInput`/`PtyOutput`,
   ideally incrementally (per output chunk), not just at command
   completion.** Severity: Critical. Effort: Medium (blocked on #1 for
   correctness during long commands — pushing one frame at completion is a
   partial fix available without #1, but streaming intermediate output needs
   #1 first). Unblocks demo step 3, 4. Directly fixes §3b, the most
   surprising and severe individual finding — today an attached human simply
   never finds out an agent did anything.

3. **Make `malt-tui`/the TUI renderer actually handle `RenderCommand::WriteRaw`.**
   Severity: Critical. Effort: Medium. Unblocks demo step 3, 4. Without this,
   fixing #1 and #2 still produces a blank screen for the human. Needs a
   real VT-consuming terminal widget (e.g. drive `malt-compat`'s own grid or
   an equivalent client-side emulator) fed by `WriteRaw` payloads, not the
   current no-op.

4. **Build a genuine stdin-write path: a `WriteInput` that writes bytes to
   whatever the session's currently-running foreground command is reading
   from**, distinguished from "new top-level command." Severity: Critical.
   Effort: Large — needs design, not a plumbing fix (echoing
   `docs/BACKLOG.md`'s existing note). Unblocks demo step 5 (and step 4 for
   the "provide temporary input authority" half). Requires #1 as a
   precondition (a blocked foreground command needs somewhere to route bytes
   to while the main loop stays responsive) and a real mash-side hook (a
   session-scoped virtual stdin the `read` builtin and spawned children's
   `Io::Inherit` can be pointed at instead of the daemon's own OS stdin).

5. **Either wire real enforcement for `InputAuthority` end-to-end (read
   `attach.authority` on attach instead of discarding it, add `client_id` to
   `KeyInput`, gate the `KeyInput` handler on `self.authority.holder()`,
   route `AttachClient`/`DetachClient` from the real VNP attach/detach paths
   instead of `RegisterVnpClient`/`UnregisterVnpClient`, and implement
   `InputClaim`/`InputAuthorityChanged` dispatch) — or explicitly downgrade
   the architecture doc's claims about it until that work happens.**
   Severity: High (currently a documentation/reality mismatch with no
   observable harm only because nothing depends on it yet — but "human takes
   temporary input authority" in the demo scenario is entirely unimplemented,
   not partially working). Effort: Medium once #1 exists (gating input is
   easy; the hard part, genuine stdin routing, is #4). Unblocks demo step 4,
   5.

6. **Give `GetOutput`/`register_vnp_client`/`exec_command` a "session busy,
   still running" response distinct from a hard timeout/connection close.**
   Severity: Medium. Effort: Small once #1 exists (the timeouts are only
   this damaging because of §3c; without #1 they're closer to "correctly
   reporting one bad symptom of a real problem"). Unblocks demo step 3
   (graceful attach during a busy session) as a stopgap even before #1/#2/#3
   land.

7. **Clean up the dead `AuthorityTracker`/`AttachClient`/`DetachClient` code
   path or fold it into whichever real wiring #5 produces**, so there's one
   source of truth instead of a fully-tested subsystem nothing calls.
   Severity: Low. Effort: Small. Doesn't unblock a demo step by itself, but
   removes a trap for the next person who greps for "authority," finds
   passing tests, and reasonably (wrongly) concludes it works.
