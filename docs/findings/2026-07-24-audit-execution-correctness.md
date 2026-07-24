# Findings: Execution-Correctness Audit — Output Shape, Command IDs, Lifecycle Events, History Wiring

Date: 2026-07-24
Context: Strategic hardening push. The team retired AGENTS.md's phased
Implementation Roadmap in favor of a correctness/hardening-first strategy,
scoped by a 9-step "agent + human coexist on one session" demo used only to
judge priority, not to build against. This audit covers one slice of that
priority list: correct plain stdout/stderr, real exit codes + execution IDs,
command lifecycle events, and persistent execution history. It builds
directly on today's earlier fixes (real exit-code plumbing via
`CommandOutput`, `send_input` confirmed wrong) and on `ADR-0002`'s
already-identified gaps (`command_id: 0`, `CommandBlock` unwired, `get_output`
returns the wrong shape). Nothing here was fixed — read-only investigation.

Method: read the four documents specified (AGENTS.md, BACKLOG.md, ADR-0002,
both existing findings docs) for calibration, then traced each of the four
assigned questions through actual current source with `grep`/`Read`, file:line
by file:line, distinguishing confirmed-in-code from suspected.

## 1. `get_output` — confirmed still `StyledGrid`; zero plain/structured variant exists anywhere

Traced the full call path:

- Route: `crates/malt-gateway/src/routes/sessions.rs:61-67` (`output` handler)
  calls `backend.get_output(id)` and returns whatever JSON value comes back,
  wrapped in `ApiResponse`. No transformation happens at this layer.
- Backend trait: `crates/malt-gateway/src/backend.rs:25` —
  `fn get_output(&self, session_id: u32) -> Result<serde_json::Value, GatewayError>`.
  The trait's own signature is untyped `serde_json::Value`, so there's no
  compile-time pressure toward a specific shape either.
- Impl: `crates/malt-daemon/src/gateway_backend.rs:136-148`. Calls
  `coord.get_session_output(...)`, parses the returned string as JSON (an
  array of styled rows), and wraps it: `{"type": "StyledGrid", "rows": rows}`.
  This literal `"type": "StyledGrid"` string is the only shape tag that
  exists anywhere in the response.
- Coordinator: `crates/malt-daemon/src/executor/coordinator.rs:178-195`
  (`get_session_output`) sends `SessionCommand::GetOutput` to the session
  thread and returns the raw string reply.
- Session thread: `crates/malt-daemon/src/executor/session_thread.rs:329-332`
  handles `GetOutput` by calling `self.get_grid_output()`
  (`session_thread.rs:522` onward), which walks `compat.grid().rows_data()`
  and emits one JSON span per contiguous run of matching `(fg, bg, bold)`
  styling — character cells with RGB/bold flags, exactly the representation
  built for `malt-tui`/`maltty`. This is a distinct code path from
  `RunCommand`/`CommandOutput` (today's exit-code fix does not touch it at
  all) — confirms `get_output` and `exec` are genuinely separate mechanisms,
  as BACKLOG.md already notes.

Grepped the whole workspace for any second `get_output` implementation, any
`PlainOutput`/`StructuredOutput` route, or any place a client flattens
`StyledGrid` back to text: only one `get_output` trait method, one impl, one
route exist (`crates/malt-gateway/tests/routes.rs:86` is a mock backend for
tests, not a second real implementation). `crates/malt-mcp/src/main.rs:180-189`
(`get_output` MCP tool) does a bare passthrough of the HTTP response text —
no client-side flattening exists anywhere in the repo. This matches the
2026-07-24 live-daemon finding that reading output as an agent required a
throwaway script.

**Smallest correct fix, assessed against the actual code shape:** `get_output`
cannot simply reuse the `CommandOutput`/exit-code plumbing from `exec`,
because it serves a fundamentally different query — "what does this
pane look like right now" (arbitrary point-in-time poll of an
ongoing/backgrounded session), not "what did the last command produce."
`CommandOutput` is scoped to one `RunCommand` invocation's result and doesn't
exist as persisted state to poll later. Two tiers of fix, not one:

- **Cheap, available today:** add a second flattening function next to
  `get_grid_output()` (`session_thread.rs:522`) that walks the same
  `compat.grid().rows_data()` cells but emits `cell.ch` characters only, no
  span/style JSON — a `get_plain_text_output()` sibling, wired through a new
  `SessionCommand::GetOutput` variant (or a bool flag) and a new gateway route
  (e.g. `/sessions/{id}/output?format=text` or a separate `/text-output`
  path), keeping the existing `StyledGrid` route untouched for
  `malt-tui`/`maltty`. This reuses data that is already correct (post the
  2026-07-24 staircase fix) and needs no new capture/storage — pure
  presentation-layer work.
- **Real, deferred fix (overlaps ADR-0002 Phase 4, "genuinely new
  engineering"):** the plain-text-from-grid approach still can't give an
  agent stdout/stderr separation or command-boundary framing — it's whatever
  is currently on screen, prompt echoes and all. That requires the
  per-command captured-output buffer ADR-0002 flags as undecided (extend
  `CommandBlock` with output bytes, or a companion structure keyed by
  `command_id`) — see finding 4 below. The grid-flattening fix is a
  legitimate interim step, not a substitute for that engineering.

## 2. Execution IDs — confirmed: no per-command id exists anywhere in the exec path today

`gateway_backend.rs:111` (`exec_command`) hardcodes `command_id: 0` in the
`ExecResult` it returns. Traced every candidate source of an existing
identifier:

- **mash's own job-id allocator is scoped to background jobs only, and is
  not a monotonic counter.** `crates/mash/src/env.rs:1008-1017`
  (`next_job_id`) returns the *smallest currently-unused* integer among live
  jobs (`let mut candidate = 1u32; loop { if jobs.iter().all(|job| job.job_id
  != candidate) { return candidate; } ... }`), not an ever-increasing
  sequence. IDs get reused once a job completes and is reaped
  (`mark_job_done`/`complete_job`, `env.rs:1025-1031` and `:1444+`). It is
  only ever called from the `&`-backgrounding path
  (`crates/mash/src/executor.rs:473`: `let bg_id = env.next_job_id();`) —
  ordinary foreground commands (the entire `exec`/`RunCommand` path) never
  call it at all. Confirmed by grep: `next_job_id`/`register_job` only
  appear at the background-job call site in `executor.rs`, nowhere in
  `session_thread.rs`'s `run_mash_command`. This id is unsuitable as a
  history/execution identifier even if reused, both because it's
  reused-not-monotonic and because it's simply never assigned to the
  synchronous commands `exec` actually runs.
- **`OutputChunk`'s `command_tag` schema field (`schemas/shell.vexil:37-40`,
  `optional<string>`, meant to correlate output with the command that
  produced it) has zero producers.** `session_thread.rs`'s two bus-publish
  call sites (`:314-320` for PTY output, `:489-497` for mash stdout) both
  construct a raw `BusMessage { domain: 1, msg_type: 4, ... payload }`
  directly — not the generated `malt_protocol::shell::OutputChunk` struct —
  so there is no `command_tag` field being populated at all; the bus payload
  is just raw bytes with no correlation metadata. Grep confirms
  `command_tag` appears only in `crates/malt-protocol/tests/roundtrip.rs:91`
  (an encode/decode round-trip test of the schema type itself, exercising
  nothing in the daemon).
- **`malt-session::pane::CommandBlock` has the field
  (`crates/malt-session/src/pane.rs:13`: `pub command_id: u32`) but nothing
  populates it** — see finding 4.
- **No counter of any kind exists in the daemon for this purpose.** Grepped
  `malt-daemon/src` for `AtomicU32`/`AtomicU64`/`next_command_id`/
  `command_counter`: the only atomic counter in the crate is
  `vnp_listener.rs:36` (`client_counter`, for VNP client connection IDs —
  unrelated).

**Where a real monotonic id would need to be assigned:** the `SessionExecutor`
struct owned by `session_thread.rs` (the same struct that owns
`mash_env`/`compat`/`renderer`) would need its own `next_command_id: u32`
field, incremented once per `RunCommand`/`WriteInput`-triggered execution —
i.e. inside `run_mash_command` (`session_thread.rs:458`) right where it's
called from the three sites that invoke it (`:306` `RunCommand` handler,
`:326` `WriteInput` handler, `:368` `KeyInput`/editor-accept handler). This
needs to be a field on the executor (survives across commands within a
session's lifetime), not derived from mash's job-id allocator, which solves a
different problem (identifying live background jobs, with reuse) and is
scoped to `&` only. It would also need to survive session
persistence/restore (Phase B2) to stay monotonic across a dormant/active
transition — not investigated here, flagged as a dependency on the
persistence-focused agent's area.

## 3. Command lifecycle events — confirmed: `CommandStarted`/`CommandFinished` are schema-only, zero producers anywhere

Grepped the entire workspace for `CommandStarted` and `CommandFinished`.
Every hit:

- `crates/malt-protocol/tests/codec.rs:63-65` — asserts the numeric
  `MSG_COMMAND_STARTED`/`MSG_COMMAND_FINISHED`/`MSG_PROMPT_READY` constants
  equal their schema `@type()` values. Tests the constant, not any producer.
- `crates/malt-protocol/tests/roundtrip.rs:7-18` — constructs a
  `CommandStarted` struct directly and round-trips it through
  pack/unpack. Tests the generated codec, not any producer.
- `crates/malt-protocol/src/priority.rs:14,44` — a doc comment classifying
  `CommandStarted`/`CommandFinished`/`PromptReady` as `Reliable` priority.
  Documentation, not a producer.

Zero hits anywhere in `malt-daemon`, `mash`, `malt-session`, or any client
crate. `run_mash_command` (`session_thread.rs:458-510`), the one place that
actually executes a command end-to-end, publishes exactly one thing onto the
bus: a raw `OutputChunk`-shaped `BusMessage` for stdout only (`:489-497`,
confirmed below has its own bug). It never constructs or publishes a
`CommandStarted` before calling `execute_list`, nor a `CommandFinished`
after. Same for the `PtyOutput` handler (`:309-321`) and the `WriteInput`
handler (`:322-328`) — both call `run_mash_command`/feed compat but never
touch lifecycle events either.

**This means an agent driving a session today has no event-based way to
know a command started or finished — polling `get_output`/`exec` is the only
option**, which is exactly the gap the 9-step demo's "structured progress
instead of raw screen output" and "resume from the last execution event"
steps depend on not existing.

**A further, structural finding not called for by the schema-grep alone but
directly relevant to sizing this gap:** the `Bus` itself has zero consumers
in non-test code today, for *any* message type, not just these two. Grepped
`malt-daemon/src` for `.subscribe(`/`.drain(` outside `bus/ring_buffer.rs`'s
own internal `Vec::drain` (`ring_buffer.rs:48`, unrelated — that's the ring
buffer's own storage drain) and `store/debounce.rs:123` (an unrelated
`HashMap::drain` over sessions, nothing to do with the message bus): nothing
calls `Bus::subscribe`/`Bus::drain`/`Bus::drain_critical` outside the bus's
own unit tests. `session_thread.rs` only ever calls `self.bus.publish(...)`
(`:285`, `:314-320`, `:489-497`); no code path anywhere reads it back. So
even if `CommandStarted`/`CommandFinished` were published today, there is
currently no live wiring that would deliver them anywhere (no SSE endpoint,
no VNP forwarding of bus messages, no gateway polling of the bus) — the
render pipeline (VNP `RenderBatch`) is a completely separate delivery
mechanism (`RendererHost`/`dispatch_render`, `session_thread.rs:418-454`)
that doesn't go through `Bus` at all. Adding producers for these two message
types is necessary but not sufficient on its own; a consumer path needs to
exist too. Worth flagging precisely because ADR-0002's "Bus is still the
right tool for live notification" framing assumes a working publish→consume
loop that doesn't exist yet for any message type.

## 4. Persistent execution history — confirmed unwired; concrete touch points to wire `CommandBlock`

Reconfirmed the ADR-0002/BACKLOG claim: grepped `CommandBlock {` (construction)
and `push_command_block` (its one mutator) across the whole workspace — the
only hits are `crates/malt-session/tests/pane.rs:4-5,18,29,41-42` (test
helper `make_block` and its call sites). Zero non-test callers anywhere in
`malt-daemon` or `malt-session`.

`CommandBlock` (`crates/malt-session/src/pane.rs:11-18`) already has exactly
the fields needed: `command_id: u32`, `cmd: String`, `started_at: u64`,
`finished_at: Option<u64>`, `exit_code: Option<i32>`. `PaneRuntime` (same
file, `:36-90`) already owns a bounded ring buffer (`push_command_block`
evicts oldest at 1000 entries, `:69-74`) and accessors
(`current_block()`, `command_blocks()`, `block_count()`). This is a real,
tested, working data structure — the gap is purely that nothing constructs
or pushes into it from the daemon side.

Concrete touch points to wire it, given `exit_code` is now available via
`CommandOutput` (today's fix):

1. **Where the pane lives relative to the session thread.** `SessionExecutor`
   in `session_thread.rs` owns `self.session: SessionRuntime` — need to
   confirm/trace that `SessionRuntime` (`crates/malt-session/src/session.rs`)
   exposes a mutable path to the focused pane's `PaneRuntime` (it already
   tracks `focused_pane()`, used at `session_thread.rs:347,444`, so the
   accessor for read exists; a mutable equivalent — or a
   `push_command_block_to_focused(...)` helper — would need to exist or be
   added). This is the one piece not fully confirmed in this pass; the
   read-only accessor's existence is confirmed, a mutable one was not traced
   line-by-line in `SessionRuntime` itself.
2. **`run_mash_command` (`session_thread.rs:458-510`) is the single
   call site for both start and finish of a command's lifecycle** — it
   already has everything a `CommandBlock` needs at the point it computes
   `result` (`:482`, `execute_list(...)`): the command text (`input:
   &str`, the function's own parameter), a start timestamp (would need to be
   captured immediately before `:482`, there is no existing timestamp
   capture in this function today — grepped for
   `SystemTime`/`Instant`/`UNIX_EPOCH` in this file and found it only at
   `dispatch_render` for the shedding timer, `:419-422`, not in
   `run_mash_command`), and `result.exit_code` (already flowing to
   `CommandOutput.exit_code` at `:508`). A monotonic `command_id` per
   finding 2 would also need to be assigned here.
3. **Concrete edit shape (not applied — read-only audit):** capture
   `started_at` right before `execute_list` is called (:482), construct the
   `CommandBlock` right after `execute_list` returns (after :482, before the
   final `CommandOutput { ... }` at :506), with `finished_at` captured at
   that same point, and call the pane-mutation method from step 1 to push
   it. This would need to happen on **all three call sites that reach
   `run_mash_command`** if history is meant to cover input typed
   interactively too, not just `exec`-driven commands — `:306` (`RunCommand`
   reply handler), `:326` (`WriteInput`, raw-input path — also the path
   `send_input` incorrectly reuses per the already-confirmed bug, so fixing
   `send_input`'s forwarding semantics and history-wiring here are not
   independent; sequencing them together may be more efficient than treating
   them as separate tickets), and `:368` (`KeyInput`/editor-accept, i.e.
   commands typed directly into an attached TUI).
4. **Not investigated further here (out of scope, flagged for the
   persistence/restore agent):** whether `PaneRuntime`'s in-memory
   `command_blocks` ring buffer needs to be included in
   `build_persisted_session`/`PersistedSession`
   (`session_thread.rs` imports `PersistedPane`/`PersistedSession` from
   `malt_protocol::persist::session` at `:9`, and `Snapshot` handling exists
   at `:395-408`) for history to actually survive a daemon restart, per the
   demo's "daemon restarts and restores session history" step. This audit
   only confirms the in-memory data-flow shape per the task's explicit
   scoping ("data-flow only, not the storage mechanism").

## New finding, not previously flagged: `exec`/`RunCommand` silently drops stderr entirely — not just a shape problem, a data-loss bug

While tracing `run_mash_command` for finding 1/4, found that **stderr text
never reaches the caller of `exec` at all, through any path** — not
mis-shaped, actually absent:

`session_thread.rs:458-510` — after `execute_list` returns `result: ExecResult`
(which has separate `stdout: Vec<u8>` and `stderr: Vec<u8>` fields per
`crates/mash/src/executor.rs:27-34`):

- stdout is fed to the compat translator for grid rendering (`:485-487`) and
  published to the bus (`:489-497`).
- stderr is fed to the compat translator too (`:500-504`, "Feed stderr
  through compat translator too") — **but is never published to the bus, and
  never included in the returned `CommandOutput`.** The function's final
  return value (`:506-509`) is
  `CommandOutput { output: String::from_utf8_lossy(&result.stdout).to_string(), exit_code: result.exit_code }`
  — `result.stderr` does not appear anywhere in it.

This `CommandOutput` flows unchanged through `gateway_backend.rs:110-114`
into `ExecResult { output, exit_code, .. }`
(`crates/malt-gateway/src/types.rs:49-54`, a single `output: String` field,
no `stderr` field exists in the type at all), through the HTTP route
(`sessions.rs:43-50`) to any caller, and through
`crates/malt-bin/src/client.rs:66-72`'s `ExecResultData` (same single
`output` field) to the CLI. **A command that fails and writes only to
stderr — e.g. a `cargo test` compile error, which is exactly the demo
scenario's opening step — returns `output: ""` and just an exit code to an
agent driving it via `exec`, with the actual error text completely
unrecoverable through that path.** It's only visible via the VT
compat-grid path (`get_output`'s `StyledGrid`, or an attached TUI), which
per finding 1 an agent isn't consuming anyway. This is the single most
concrete, highest-severity gap found in this audit relative to the "correct
plain stdout/stderr" priority item, and is a straightforward, small,
self-contained fix (thread `result.stderr` alongside `result.stdout` through
`CommandOutput`, `ExecResult`, `ExecResultData`, same shape as the existing
`exit_code` fix from earlier today) — not something that needs the larger
`CommandBlock`/execution-resource engineering to land first.

## Prioritized recommendations

1. **[Highest severity, lowest effort] Fix stderr being silently dropped
   from `exec`/`RunCommand`.** Add a `stderr: String` field alongside
   `output` to `CommandOutput` (`session_thread.rs:57-60`), `ExecResult`
   (`malt-gateway/src/types.rs:49-54`), and `ExecResultData`
   (`malt-bin/src/client.rs:66-72`); populate it in `run_mash_command`
   (`session_thread.rs:506-509`) from `result.stderr`. Same mechanical shape
   as today's exit-code fix — no design work needed, no new engineering.
   **Unblocks demo step 1** directly (an agent running `cargo test` needs
   compiler/test failure text, which is largely stderr, to do anything
   useful with a failed run) and is a correctness bug independent of the
   demo. Estimated effort: small (a few hours, mirrors a fix already done
   today for exit codes).

2. **[High severity, low-to-medium effort] Add a plain-text `get_output`
   variant alongside the existing `StyledGrid` one.** Add
   `get_plain_text_output()` next to `get_grid_output()`
   (`session_thread.rs:522`), a new `SessionCommand::GetOutput` variant or
   format parameter, and a new gateway route/query param, per finding 1's
   "cheap tier." Keeps the grid route untouched for `malt-tui`/`maltty`.
   **Unblocks demo step 2** ("MALT reports structured progress instead of
   raw screen output") for the *ongoing/backgrounded* monitoring case in
   particular — `exec`'s synchronous response is already plain-text-shaped
   after today's fix; this closes the same gap for polling a live session.
   Effort: small-medium (pure presentation change, no new capture/storage).

3. **[Medium severity, medium effort] Wire real monotonic `command_id`
   generation.** Add a `next_command_id: u32` field to `SessionExecutor`,
   increment and assign it inside `run_mash_command`
   (`session_thread.rs:458`), thread it through `CommandOutput` →
   `ExecResult.command_id` (replacing the `gateway_backend.rs:111` literal
   `0`). Do **not** reuse mash's `next_job_id()` — confirmed scoped to
   background jobs only and reused, not monotonic (finding 2). Consider
   whether this counter needs to survive session persistence/restore before
   finalizing its type/placement (flagged, not resolved, as a dependency on
   the persistence agent's area). **Unblocks demo step 8** ("agent resumes
   from the last execution event instead of scraping the terminal" needs a
   stable id to resume *from*). Effort: medium — straightforward on its own,
   but only actually useful once paired with recommendation 5 below
   (nothing durable to resume from otherwise).

4. **[Medium severity, medium effort] Publish real `CommandStarted`/
   `CommandFinished` messages.** Construct and `self.bus.publish(...)` an
   actual `malt_protocol::shell::CommandStarted` (before `execute_list`) and
   `CommandFinished` (after, with `exit_code`/`duration_us` and the
   `command_id` from recommendation 3) in `run_mash_command`
   (`session_thread.rs:458-510`), using `Priority::Reliable` per the
   schema's own doc comment. This alone is **not sufficient** — see next
   item. **Partially unblocks demo step 2** (lifecycle awareness) but only
   once paired with recommendation 6.

5. **[Medium severity, medium-high effort] Wire real `CommandBlock`
   construction into `run_mash_command`.** Concrete touch points identified
   in finding 4: capture `started_at` before `execute_list`
   (`session_thread.rs:482`), construct `CommandBlock` with the
   `command_id` from recommendation 3 immediately after, push it via a
   mutable-pane-access path from `SessionExecutor` into
   `PaneRuntime::push_command_block` (the one piece needing a small amount
   of new plumbing — a read accessor for the focused pane exists,
   `session_thread.rs:347,444`, a mutation path was not confirmed to exist
   already). Apply at all three call sites that reach `run_mash_command`
   (`:306`, `:326`, `:368`) if interactively-typed commands should also be
   in history, not just `exec`-driven ones. **Unblocks demo step 8**
   (durable, indexable history to resume from) together with
   recommendation 3. Sequencing note: this and fixing `send_input`'s
   forwarding bug (already confirmed wrong, tracked separately, not
   re-litigated here) both touch the `WriteInput`/`RunCommand` call sites in
   this same file — worth planning together rather than as two unrelated
   patches that each touch the same lines.

6. **[Lower priority — infrastructure gap, not this area's direct scope, but
   blocks the payoff of 4 above] Give the Bus an actual consumer.** Confirmed
   in finding 3: nothing in non-test code calls `Bus::subscribe`/`drain`/
   `drain_critical` today, for any message type. Publishing
   `CommandStarted`/`CommandFinished` (recommendation 4) delivers them
   nowhere without this. Likely needs an SSE endpoint or VNP-forwarded
   lifecycle-event channel in `malt-gateway`/`malt-daemon` — flagged as a
   real, currently-invisible dependency of the "command lifecycle events"
   priority item, not previously called out in BACKLOG.md or ADR-0002 in
   this specific "the bus has zero consumers period" framing. Effort:
   medium-high, and likely belongs to whichever agent/plan owns the
   gateway's streaming surface rather than this execution-correctness slice
   specifically — noted here so it isn't lost.
