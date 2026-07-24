# MALT Backlog

Living document. Update it as work lands or priorities change — this is the
"what's next and why" record, distinct from `docs/design/architecture.md` (target
design) and `docs/adr/` (decisions already made and their reasoning).

Each item should be concrete enough to act on without re-deriving context.
Link to a findings doc or ADR for the "how do we know this" evidence instead
of restating it here.

**2026-07-24: reorganized around ADR-0003's correctness-first priority
order**, following a five-agent audit swarm scoped against that priority
list (`docs/findings/2026-07-24-audit-{execution-correctness,input-concurrency,
persistence-restore,isolation-safety,client-sdk-surface}.md`). AGENTS.md's
old phased "Implementation Roadmap" is retired — see
`docs/adr/ADR-0003-correctness-first-strategic-pivot.md` for why, and
AGENTS.md's "Priorities" section for the current ordering this file is
organized against.

## Priority order (see ADR-0003 for full reasoning)

Two audit-discovered structural prerequisites, then the project owner's
original ten, in order. Each links to its detail below.

- **0a. Gateway auth not enforced at all** — see P0.
- **0b. Single-threaded session executor blocks attach/output/input** — see P0.
- 1. Correct plain stdout and stderr — see P0 (stderr), P1 (`get_output` shape).
- 2. Real exit codes and execution IDs — exit codes done (2026-07-24); execution IDs — see P1.
- 3. Command lifecycle events — see P1.
- 4. Persistent execution history — see P1 (two-part: in-memory + schema).
- 5. Genuine raw input — see P0 (0b is the precondition), P1.
- 6. Human and agent coexistence — see P0 (render-dispatch gap), P1 (input authority).
- 7. Fail-closed requested isolation — see P0 (live gaps), P1 (policy design).
- 8. A correct TUI rendering path — see P0 (`WriteRaw` unhandled).
- 9. Session restoration — see P1 (compat-pane stub, `EnvSnapshot` unwired).
- 10. One excellent agent client or Gateway SDK — see P1, P2.

## P0 — blocks daily-driver usability and safe agent use

- **Gateway auth is not enforced anywhere — every route is Admin-equivalent
  open to any process that can reach the port.** `TokenStore`/`AuthContext`/
  `RateLimiter` (`crates/malt-gateway/src/auth.rs`, `rate_limit.rs`) are real
  and well-tested in isolation (8 + 4 tests), but `build_router()`
  (`crates/malt-gateway/src/server.rs:10-27`) attaches no auth extractor and
  no rate-limit layer at all. `malt-bin/src/daemon.rs:29-31` generates and
  prints an "API token" that is never passed into `build_router` or used
  anywhere. Every `GatewayError::Unauthorized`/`Forbidden`/`RateLimited`
  variant has a correct HTTP mapping but is never constructed outside
  tests. Priority 0a — this is not "is `Interact` scope sufficient," it's
  "nothing is checked at all." See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §4.
- **The session executor is a single thread blocking on one command queue
  — a long command freezes attach, output, and input for its whole
  duration.** `SessionExecutor::run` (`crates/malt-daemon/src/executor/session_thread.rs:271-415`)
  loops on one blocking `mpsc::Receiver`; `RunCommand`'s handler calls
  `run_mash_command` synchronously. While a `cargo test`-length command
  runs, `RegisterVnpClient`/`GetOutput`/`KeyInput`/`AckFrame`/`Resize` all
  queue behind it. Concretely: `register_vnp_client`'s 5s timeout
  (`coordinator.rs:325`) means attaching mid-command **hard-fails the TCP
  connection** rather than degrading gracefully; `exec_command`'s 30s
  timeout returns an error to the caller while the command keeps running
  to completion inside the daemon regardless, with no way to retrieve the
  result later. Priority 0b — root cause behind the raw-input gap, the
  render-notification gap below, and the attach-timeout behavior; nothing
  downstream can be fixed correctly without this first. See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §3c (recommendation 1).
- **`exec`/`RunCommand` silently dropped stderr — FIXED 2026-07-24.** Added
  `stderr: String` to `CommandOutput`, `ExecResult`
  (`malt-gateway/src/types.rs`), and `ExecResultData` (`malt-bin/src/client.rs`,
  `#[serde(default)]` for backward compatibility), populated from
  `result.stderr` in `run_mash_command`. `malt exec` now prints stderr to
  the CLI's own stderr instead of swallowing it. New tests:
  `exec_reports_stderr_separately_from_stdout` (real `Coordinator`,
  `malt-daemon/tests/gateway_backend.rs`) and
  `parse_exec_response_with_stderr` (`malt-bin/src/client.rs`). Full
  workspace green.
- **Requested isolation is fail-open and tier-blind on Windows, and
  entirely unenforced on Linux/macOS, for every tier.** Confirmed and
  broader than the originating concern:
  - Windows Job Objects: `apply_session_isolation`
    (`crates/malt-daemon/src/executor/session_thread.rs:30-52`) silently
    continues on `create_job_object` failure (`warn!` only, function
    returns `()` not `Result`) — for `Restricted`, `Capped`, *and*
    `Contained` alike, since the only branch is Bare-vs-not-Bare. Even on
    the success path, all three tiers get an identical, uncapped Job
    Object (`create_job_object(&job_name, 0, 0)` — zero memory/CPU limits,
    no HCS) — `Capped`'s promised resource limits and `Contained`'s
    promised HCS container are not provided by anything today.
  - **Linux and macOS have zero isolation enforcement wired to any spawn
    path, for any tier, success or failure.** The namespace/cgroup/
    overlayfs/seccomp/sandbox/rlimit modules are called only from the
    capability prober (`tier_available()`), never from anything that
    spawns a process — stronger than AGENTS.md's old "unverified" framing.
  - `tier_available()` itself is never consulted by session creation on
    any platform, contradicting architecture.md's stated intent that an
    unsupported-tier request should fail up front with a clear error, not
    fall through silently.
  - `gateway_backend.rs::parse_isolation` silently mapped any unrecognized
    isolation string to `Bare` — **FIXED 2026-07-24**: now returns
    `Result<IsolationTier, GatewayError>`, rejecting unrecognized values
    with `GatewayError::BadRequest` (→ HTTP 400) instead of silently
    defaulting. New test: `create_session_rejects_unrecognized_isolation_string`.
  - `malt-bin`'s `validate_created_session` (`main.rs:131-145`) — the one
    thing that looks like a client-side safety net — cannot catch any of
    the above, because it compares against a value computed *before*
    isolation setup runs and never updated by the outcome.
  All four are live today, reachable via `malt new --isolation
  restricted|capped|contained` on every platform. See
  `docs/findings/2026-07-24-audit-isolation-safety.md` (full catalog + a
  concrete `IsolationPolicy` proposal — implementation detail in P1 below).
- **The VNP client (`malt-tui`) — the path architecture.md treats as
  authoritative — never renders shell output at all, on first attach or
  any subsequent one.** `TuiRenderer::apply_one`
  (`crates/malt-tui/src/render.rs:130-132`) has no handler for
  `RenderCommand::WriteRaw`, and nothing at the application level handles
  it either (`app.rs:36` calls the same function). Since `CompatTranslator`
  only ever emits `VtPassthrough` → `WriteRaw`, every `RenderBatch`/
  `InitialState` a real VNP client receives for actual shell content is
  silently dropped on arrival. The reason this wasn't caught earlier: the
  P0 rendering bug fixed today was observed via the HTTP `/output` route
  (`HttpConnection`), which bypasses the RenderCommand pipeline entirely
  with its own client-side JSON synthesis — the "authoritative" VNP path
  shows a blank screen for shell content regardless. See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §3a.
- **Gateway/agent-driven execution never notifies attached VNP clients
  that anything happened — "both see the same authoritative state" does
  not hold today.** `dispatch_render()` is called from exactly three call
  sites in `session_thread.rs` (lines 370, 377, 381), all inside
  `KeyInput`'s handler. `RunCommand` (the Gateway `/exec` path — ADR-0002's
  "canonical agent control plane"), `WriteInput`, and `PtyOutput` never
  call it. If an agent runs `cargo test` via `POST /exec` while a human is
  attached via VNP, the human's client receives **no RenderBatch
  reflecting that command, ever** — not during execution, not after
  completion — independent of the `WriteRaw` bug above. Only the human's
  *own* next keystroke happens to trigger a (stale) frame push. A partial
  fix (push one frame at command completion) is available without 0b;
  streaming intermediate output correctly needs 0b first. See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §3b (recommendation 2).

## P1 — real gaps, worth fixing soon

- **No genuine raw-input path exists anywhere in the daemon — confirmed,
  not a narrow `send_input` bug.** All three input entry points —
  Gateway `send_input` (`gateway_backend.rs:117-134`), `WriteInput`
  (`session_thread.rs:322-328`), and `KeyInput`'s line-accept branch
  (`session_thread.rs:364-388`) — converge on the same
  `run_mash_command`: parse the bytes as a brand-new top-level command
  line and run it to completion. There is no "write these bytes to
  whatever is already reading stdin" path at any layer. Worse: mash's
  `read` builtin falls back to `std::io::stdin()` — **the daemon
  process's own real OS stdin** — when nothing is redirected
  (`crates/mash/src/executor.rs:3579-3610`), and external command spawns
  default to `Io::Inherit`, so an interactive child (REPL, password
  prompt) inherits the *daemon's* stdin, not any client's. Building a real
  path (a session-scoped virtual stdin the `read` builtin and spawned
  children can be pointed at) is genuine design work, and needs 0b as a
  precondition. See `docs/findings/2026-07-24-audit-input-concurrency.md`
  §1 (recommendation 4).
- **`InputAuthority` is a fully inert data model in production — real,
  tested, zero live wiring on either end.** `AttachClient`/`DetachClient`
  (the only commands that touch `AuthorityTracker`) are sent exclusively
  from test files — zero production call sites. The real VNP attach path
  (`RegisterVnpClient`) never touches authority; `vnp_listener.rs`'s
  `wait_for_attach` parses the wire `AttachSession.authority` field and
  discards it (`malt-tui` always sends `Exclusive` with no way to send
  anything else — no `--observe` flag exists). `InputClaim`/
  `InputAuthorityChanged` have real codec constants but zero handling in
  `vnp_listener.rs::dispatch_frame`'s match. Most decisive:
  `SessionCommand::KeyInput` doesn't carry a `client_id` at all, so there's
  structurally no way to gate on "is this the authoritative client."
  Needs either real end-to-end wiring or an explicit downgrade of
  architecture.md's claims about it until that work happens. See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §2 (recommendation 5),
  and §4 for the related dead-cleanup-on-detach note.
- **Real monotonic `command_id` generation — FIXED 2026-07-24.** Added a
  `next_command_id: u32` field to `SessionExecutor`, incremented once at
  the top of `run_mash_command` (covering all three return paths — parse
  error, empty input, and real execution — and all three call sites that
  reach it: exec, `WriteInput`, `KeyInput`-accept), threaded through
  `CommandOutput` → `ExecResult.command_id`, replacing
  `gateway_backend.rs`'s hardcoded `0`. Confirmed unsuitable and not
  reused: mash's own `next_job_id()` is scoped to `&`-backgrounded jobs
  only and reuses freed IDs (not monotonic). New test
  `exec_reports_a_real_monotonic_command_id`
  (`malt-daemon/tests/gateway_backend.rs`) asserts strictly increasing IDs
  across three successive `exec` calls on one session. Does **not** yet
  survive session persistence/restore — the counter resets to 0 on
  restore, same as any other in-memory session state; revisit once
  persistent execution history (below) is wired, since that's the point
  restart-survival for this counter would actually matter.
- **`CommandStarted`/`CommandFinished` are schema-defined with correct
  codec constants but have zero producers anywhere — and even if
  published, nothing would deliver them.** Grepped the whole workspace:
  every hit is a test or a doc comment. `run_mash_command` publishes only
  an `OutputChunk`-shaped `BusMessage` for stdout. Separately, **the `Bus`
  itself has zero consumers in non-test code, for any message type** —
  nothing calls `subscribe`/`drain`/`drain_critical` outside the bus's own
  unit tests, so publishing these two events alone delivers them nowhere;
  a real consumer path (SSE endpoint, or VNP-forwarded lifecycle channel)
  is a separate, necessary piece of work this depends on. Not previously
  called out in this specific "the bus has zero consumers period" framing
  by ADR-0002. See
  `docs/findings/2026-07-24-audit-execution-correctness.md` §3
  (recommendations 4 and 6).
- **Persistent execution history has two independent, stacked gaps — both
  need closing, not just the one ADR-0002 already flagged.** Gap A
  (in-memory): `malt-session::pane::CommandBlock`/`push_command_block`
  are fully built, ring-buffered, and unit-tested but never constructed
  from `malt-daemon` — zero non-test callers. Concrete touch points:
  capture `started_at` before `execute_list` and construct/push the block
  right after, in `run_mash_command`
  (`session_thread.rs:458-510`), reusing the `exit_code` already available
  via `CommandOutput`. Gap B (persistence schema, not previously flagged):
  even once Gap A lands, history still wouldn't survive a daemon restart —
  `schemas/persist/session.vexil`'s `PersistedPane` has **no field for
  command history at all**, and `build_persisted_session`
  (`session_thread.rs:599-658`) has no access path to `PaneRuntime`'s
  `command_blocks()`. Needs a `command_blocks: array<CommandBlock>`-shaped
  schema field plus read/write wiring in `build_persisted_session`/
  restore. Recommend sequencing both gaps as one coordinated piece of
  work, not two independent tickets — this is ADR-0002 Phase 3/4. See
  `docs/findings/2026-07-24-audit-execution-correctness.md` §4
  (recommendation 5) and
  `docs/findings/2026-07-24-audit-persistence-restore.md` §1
  (recommendation 2).
- **`mash::Env::to_snapshot()`/`apply_snapshot()` wired into persistence —
  FIXED 2026-07-25, and a real latent bug in `apply_snapshot` itself found
  and fixed in the process.** Extended `schemas/persist/session.vexil`
  with `EnvSnapshot`/`PersistedVariable`/`PersistedVarValue`/
  `PersistedShellOptions` message types and an `env_snapshot:
  optional<EnvSnapshot>` field on `PersistedPaneType::Shell` (alongside
  `shell_path`). `build_persisted_session` now calls `env.to_snapshot()`
  and converts it via a new `to_persisted_env_snapshot()`;
  `SessionExecutor::spawn_with_cwd` (restore-only) now takes the persisted
  snapshot, converts it back via `from_persisted_env_snapshot()`, and
  calls `env.apply_snapshot()` after `Env::from_os()`. Also fixed the
  adjacent dead `shell_path` round-trip (captured on persist, discarded on
  restore) in the same change — `spawn_with_cwd` now restores it into the
  `SHELL` env var.
  **Real bug found by actually exercising this path for the first time:**
  `mash::Env::apply_snapshot` (`crates/mash/src/env.rs`) re-parses each
  function's stored source text, but stored the *entire reparsed
  top-level command* (a `Command::FunctionDef` node) as the function's
  `body`, instead of extracting the function's actual inner body from it
  — a restored function's body was itself a function-definition wrapper,
  not the function's real logic. The existing unit test
  (`snapshot_roundtrip_functions`) didn't catch this because it used
  unrealistic source text (just `"echo hello"`, not a full `"name() {
  body }"` definition matching what `executor.rs` actually stores) and
  only checked that a `FunctionDef` entry existed, never that calling the
  restored function actually worked. Fixed to extract the inner body
  (matching `executor.rs`'s own `Command::FunctionDef` handling) and
  rewrote the test to define/snapshot/restore/**call** a real function
  through `execute_list`, asserting on its actual output. New end-to-end
  test in the daemon too:
  `shell_env_state_survives_dormant_restore_via_env_snapshot`
  (`malt-daemon/tests/gateway_backend.rs`) — creates a session, sets an
  exported variable + alias + function, forces a dormant→restore cycle by
  detaching and reattaching against a fresh `Coordinator` reading the same
  on-disk store (simulating a daemon restart), and confirms all three
  survive by actually invoking them. See
  `docs/findings/2026-07-24-audit-persistence-restore.md` §4
  (recommendation 1).
- **Compat-pane session restore — FIXED 2026-07-25, with one real,
  explicitly-flagged gap introduced (isolation).** `restore_session`'s
  Compat branch now: spawns a `SessionExecutor` exactly like Shell restore
  (still needed even for a Compat-typed pane — it owns the renderer/
  editor/CompatTranslator regardless of pane kind), separately re-launches
  the real external process via the previously-dead
  `ProcessSupervisor::spawn`, and forwards its output into the session via
  a new `spawn_pty_reader` thread sending `SessionCommand::PtyOutput`.
  Rolls back the session thread on process-spawn failure rather than
  leaving an Active session with no process behind it. Re-launch, not
  re-attach, per the documented restore policy — no VT state to restore,
  a blank grid is correct. New end-to-end test
  (`restore_compat_pane_relaunches_process_and_forwards_real_output`,
  `malt-daemon/tests/coordinator.rs`) hand-crafts a persisted Compat
  session (there's still no live creation path for one), restores it, and
  polls until the real spawned process's real output actually reaches the
  session's grid.
  **Real gap, introduced knowingly, not silently: the restored process is
  not assigned to the session's isolation Job Object.**
  `ProcessSupervisor` has no access to it — the Job Object lives inside
  the `SessionExecutor` thread's `mash::Env`, created inside a spawned
  closure that never returns it to the `Coordinator`. Giving
  `ProcessSupervisor` real access (open the same named Job Object, or
  restructure `spawn_with_cwd` to return it) is a real design decision, not
  a parameter change — deliberately not attempted here, matching the same
  judgment call as the HCS wiring above. Until this is closed, a
  Restricted/Capped/Contained session's *restored* compat pane process
  runs uncontained, unlike mash's own external-command spawn path (which
  is Job-Object-wired). This sharpens the existing "PTY/compat supervisor
  spawn path has no isolation wiring" P2 item below — it's no longer
  latent for this one path, it's live the moment any persisted Compat
  session exists.
  See `docs/findings/2026-07-24-audit-persistence-restore.md` §2
  (recommendation 3). Adjacent landmine, still open, cheap to fix
  whenever next in this code: `restore_session` picks the first pane in
  `persisted.panes` by map order instead of consulting `persisted.focus`
  — harmless under today's single-pane model, a real bug waiting for
  Phase F multi-pane work (§2 caveat, recommendation 6).
- **Windows Job Object tier differentiation — FIXED 2026-07-24, HCS
  wiring deliberately NOT attempted (see below).** `apply_session_isolation`
  now calls `create_job_object` with real, tier-specific
  `(memory_limit_mb, cpu_rate)` — Restricted stays uncapped (group-kill
  only), Capped/Contained get real limits (2048 MB / 200% CPU, placeholder
  defaults pending a real config surface) instead of the previous
  identical-to-Restricted `(0, 0)` for all three non-Bare tiers. Extracted
  as a pure `job_object_limits_for_tier()` function with direct unit tests
  (`bare_and_restricted_get_uncapped_job_objects`,
  `capped_and_contained_get_real_nonzero_limits`,
  `restricted_and_capped_are_actually_different`).
  **Deliberately did not attempt real HCS container wiring for Contained**
  even though `hcs::create_compute_system` would compile and could be
  called: the function exists regardless of the `hcs` feature flag (it
  cleanly errors via `ensure_hcs_runtime()` if the feature isn't compiled
  in, which it isn't for `malt-daemon` today), but creating a compute
  system that no process actually launches inside is worse than not
  creating one — real HCS containment needs
  `malt_platform::process::spawn`/mash's external-command spawn path to
  become HCS-aware (launch via `hcs::create_process`, not a normal OS
  spawn), which is genuinely new engineering, not a parameter change. That
  remains open — Contained gets the same Job-Object-only containment as
  Capped today, nothing more. Restricted tokens (`tokens.rs`) are unwired
  for the same underlying reason (using a token means
  `CreateProcessAsUser` in the actual spawn path) and remain a separate
  open item.
- **Isolation policy: implement `required`/`preferred`/`disabled`
  per the project owner's request, as a new field alongside
  `IsolationTier`, not folded into it.** Concrete proposal from the audit:
  add `enum IsolationPolicy { Required, Preferred, Disabled }` to
  `schemas/common.vexil`, add `policy` to `CreateSession` and
  `PersistedSession` (default omitted → `Preferred`, preserving today's
  behavior for existing callers). Wiring needed: `apply_session_isolation`
  must return `Result<(), IsolationError>` instead of `()`; `SessionExecutor::spawn`/
  `spawn_with_cwd` need a rendezvous channel so the parent thread can
  observe the spawned closure's isolation-setup outcome before returning
  (today it returns unconditionally the instant the OS thread is created —
  a structural change, not a signature tweak); a new `DaemonError` variant
  (e.g. `IsolationRequired`); a new `GatewayError` variant mapped to 4xx,
  not 500 (the request was well-formed, the guarantee just can't be met);
  and `parse_isolation` needs to reject unrecognized strings instead of
  silently defaulting to `Bare` (fix this half regardless of `required`/
  `preferred` — see P0). Explicitly sequence real Linux/macOS enforcement
  (P0 above) before or alongside adding `required` checks there — a
  `required` policy that "succeeds" by checking nothing would be worse
  than today's honest fail-open. Full plumbing trace with exact file:line
  touch points in
  `docs/findings/2026-07-24-audit-isolation-safety.md` (item 7 and the
  Proposal section).
- **Agent-facing `get_output` returned the wrong representation — cheap
  tier FIXED 2026-07-24.** Added `get_plain_text_output()`
  (`session_thread.rs`, next to `get_grid_output()`) walking the same
  `TerminalGrid` cells but emitting characters only, no span/style JSON; a
  new `SessionCommand::GetOutputText` variant; `Coordinator::get_session_output_text`;
  a new `GatewayBackend::get_output_text` trait method; and a new route,
  `GET /sessions/{id}/output/text`, left alongside the existing
  `StyledGrid` route (`GET /sessions/{id}/output`) rather than replacing
  it — `malt-tui`/`maltty` still need the styled one. **`malt-mcp`'s
  `get_output` tool now calls the new text route instead of the grid
  one** — this is the actual fix for the agent-facing bug, not just an
  unused addition. New tests:
  `get_output_text_returns_plain_text_matching_executed_output` (real
  `Coordinator`, asserts real command output round-trips with no JSON
  span markup) and `output_text_returns_plain_text_shape` (route-level).
  Real fix — still deferred to the `CommandBlock`/execution-resource work
  above, since true stdout/stderr separation with command-boundary
  framing (not just "whatever's on screen right now") needs the
  per-command captured-output buffer ADR-0002 already flags as undecided.
  See `docs/findings/2026-07-24-audit-execution-correctness.md` §1
  (recommendation 2). Also recording a considered non-decision so it
  doesn't get re-litigated: Agent Client Protocol (ACP) was evaluated and
  doesn't apply — different actor model (ACP's client hosts an embedded
  agent's UI; MALT's is an external agent remotely driving a persistent
  session).
- **Three gateway pane-management routes were fake — `list_panes` FIXED
  2026-07-24, `split_pane`/`close_pane` intentionally left as honest
  stubs.** `DaemonBackend::list_panes` always returned a hardcoded
  `PaneResponse { id: 1, .. }` regardless of which session's real pane id
  actually was — added `first_pane: PaneId` to `Coordinator`'s
  `SessionHandle` (populated at creation and at daemon-startup restore
  from `persisted.panes`' first key) and a `session_first_pane()`
  accessor, so `list_panes` now returns the session's real pane id.
  `kind`/`title` remain hardcoded (`"Shell"`/`None`) since every reachable
  pane genuinely is untitled Shell-kind today — real per-pane
  kind/title tracking is multi-pane feature work, out of scope per "no new
  features." New test: `list_panes_returns_the_sessions_real_pane_id_not_always_one`
  (creates and destroys a session first so the next one's pane id isn't 1,
  proving it's not hardcoded). `split_pane`/`close_pane` deliberately left
  as stubs, not built out — implementing real pane split/close is
  multi-pane feature work, not a bugfix. See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1.
- **`malt-bin` had no `get_output` client method, and `malt-mcp`'s
  `create_session` name-default divergence — both FIXED 2026-07-24.**
  Added `MaltClient::get_output_text` (calls the new
  `/output/text` route) plus a `malt output <session_id>` CLI subcommand
  so the client method has a real caller. `malt-mcp`'s
  `create_session` no longer sends the literal string `"default"` for an
  omitted `name` — extracted the body-building logic into a pure
  `create_session_body()` function with direct unit tests
  (`create_session_body_omits_name_when_absent`,
  `create_session_body_includes_name_when_present`), since the previous
  divergence was exactly the kind of bug a real test would have caught.
  See `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1, §2.
- **Recommendation: build a new `malt-gateway-sdk` Rust crate, once the
  above land — not hardening `malt-mcp` into that role.** MCP's tool-call
  round-trip has no first-class subscription primitive for the lifecycle-
  event stream item 3 above needs; bending MCP to carry it (SSE-over-MCP,
  polling in a loop) repeats the token-cost/failure-rate pattern ADR-0002
  already cites against MCP-first design. `malt-mcp` is already the thin,
  disposable, zero-internal-dependency adapter ADR-0002 phase 7 wants —
  investing in it now would be wasted before the Gateway contract
  stabilizes. Sequence after, not instead of, the fixes above: a typed SDK
  wrapping already-correct shapes, used by `malt-bin`, `malt-tui`'s HTTP
  mode, and a rebuilt thin `malt-mcp` alike, removing the class of bug
  found in the name-default divergence above by construction. See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §3, and the
  10-item prioritized work list at the end of that document for the full
  sequencing.
- **Backgrounded commands (`&`) don't appear to survive through the
  daemon's `/exec` route — confirmed to need new design, not a regression
  fix.** The process-supervisor's original design never covered detached
  background jobs surviving past their pane's lifecycle; it's scoped to
  interactive foreground processes tied to a PTY. `ping -n 30 127.0.0.1 &`
  returns empty output and no process survives afterward. See
  `docs/findings/2026-07-24-live-daemon-session.md#2-backgrounded-commands--dont-appear-to-survive-through-exec--medium-priority-not-root-caused`.

## P2 — deliberately deferred, not forgotten (explicitly paused per ADR-0003)

- **`maltty` cannot render a single character today, confirmed** —
  `GpuRenderer::render`'s `_lines` parameter is never read; the frame is a
  fixed-color clear-and-present, nothing else. Every other dimension
  (window, resize, input mapping, HTTP polling data path) works; only the
  one thing a terminal exists to do is unimplemented, and the fix needs
  the full cosmic-text atlas/shader pipeline its own TODO describes, not a
  small patch. Confirms the "pause GPU client" decision is the right call
  with no partial credit being left on the table. Frozen, no further work
  planned. See `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1, §5.
- **`malt-web` is honestly an MVP and the code backs that up** — no tests,
  silent error-swallowing on failed `exec` calls, a client-side local-echo
  input buffer never reconciled against the daemon's real echo (will
  visibly diverge under any raw-mode program), no resize support. Frozen
  per the "pause competing clients" decision; not a foundation to build on
  without first fixing echo-reconciliation as its own real design task.
  See `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1, §5.
- **`malt-tui`'s HTTP connection mode is orphaned** — real code, but
  nothing launches `malt-tui` without `--vnp` today, and it contains a
  dead `text`-field fallback branch that has no live caller (`get_output`
  never emits that shape). Candidate for deletion or explicit
  "manual-testing-only" documentation once the plain-output `get_output`
  work lands anyway (it would need rewriting regardless). See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1, §5.
- **PTY/compat supervisor spawn path has no isolation wiring — UPGRADED
  TO LIVE 2026-07-25, was latent.** `ProcessSupervisor::spawn`
  (`crates/malt-daemon/src/supervisor/mod.rs`) never reads
  `SpawnRequest.isolation`. This was previously latent because
  `Coordinator` never called `.spawn()` on it at all — compat-pane restore
  (above) now does, for real, so this is no longer "becomes live risk
  eventually," it's reachable today via any persisted Compat session.
  Needs `ProcessSupervisor` to get real access to the session's isolation
  Job Object (it currently can't — see the compat-pane restore entry
  above for why) before this can be closed; likely worth doing together
  with the isolation-policy work elsewhere in this list, since both touch
  the same "how does a spawned-outside-mash process get containment"
  question. See `docs/findings/2026-07-24-audit-isolation-safety.md`
  item 5.
- **`malt-elevate` has 9 of 10 privileged operations that actively report
  success while doing nothing** (`stub_success()`,
  `crates/malt-elevate/src/dispatch.rs:44-65`) — only `CreateSymlink` is
  real. This is categorically worse than fail-open (it's not an attempt
  that failed, it's a lie), but confirmed **unreachable from any live code
  path today** — no crate depends on `malt-elevate`, nothing spawns it as
  a subprocess. Latent risk, becomes live the moment anything wires
  elevation-requiring isolation operations (real Linux namespace/cgroup/
  seccomp support, unprivileged restricted-token issuance) through it —
  should be fixed or explicitly gated before that wiring lands, not
  treated as urgent today. See
  `docs/findings/2026-07-24-audit-isolation-safety.md` item 4.
- **Restricted tokens and HCS bindings are genuinely fail-closed and
  well-tested at the function level, but completely unreferenced by any
  live spawn path** — lower urgency than the P0 Windows Job Object items
  because the functions themselves are already correct; the fix is wiring
  them into `apply_session_isolation` once it's made tier-aware (P1
  above), not correctness work on the functions. HCS's real native path
  specifically remains unverified on real hardware (needs Windows
  Containers installed; only fake-mode tested). See
  `docs/findings/2026-07-24-audit-isolation-safety.md` items 2, 3.
- **AppContainer and CRIU were not ported from carboy-isolation, on
  purpose** — both were stubs/placeholders in carboy itself (AppContainer's
  restricted-token function is a documented zero-isolation fallback; CRIU
  is a 5-line `Err(Unsupported)` placeholder). Porting them would have
  added a false sense of security. If real implementations are ever
  wanted, build as original work, not adopted scaffolding. See ADR-0001.
- **Corruption quarantine has no file cap or eviction**, drifting from
  architecture.md's documented 50-file limit — a recurring serialization
  bug could accumulate quarantine files unboundedly. Not a correctness bug
  in the corruption handling itself (that works, confirmed by test); cheap
  fix, low urgency. See
  `docs/findings/2026-07-24-audit-persistence-restore.md` §5
  (recommendation 5).
- **`vexil.std` types (Duration/IpAddr/SemVer/Url) are still missing in
  vexil-lang.** External dependency — vexil-lang's own docs defer this to
  their package-manager milestone. Tracked, not actionable from this repo.

## Not a gap — resolved by cross-referencing, noted so it doesn't get re-flagged

- **Session persistence API "missing" was a false alarm from checking only
  one crate.** `phase2-malt-session.md` specified
  `SessionRuntime::to_persisted()`/`from_persisted()` and a `PersistedSession`
  type living in `malt-session` — grepping that crate alone finds nothing.
  But the functionality is real: `PersistedSession`/`PersistedPane` are
  schema-generated types from `malt_protocol::persist::session`, and the
  conversion logic (`build_persisted_session`, `SessionCommand::Snapshot`)
  lives in `malt-daemon`'s `session_thread.rs` instead. Relocated during
  implementation, not abandoned. See
  `docs/findings/2026-07-24-plan-implementation-audit.md#4-session-persistence-api-exists-but-not-wherehow-originally-specced`.
- **Scrollback's absence is intentional design, not a bug** —
  architecture.md states scrollback is ephemeral by design ("gone on
  restart, same model as tmux"). Confirmed genuinely 100% unimplemented
  (schema-only: no mmap usage, no `ScrollbackRequest`/`Response` handler in
  `vnp_listener.rs`, no `scrollback` module anywhere), which is exactly
  correct against that stated intent — but it does mean scrollback was
  never meant to be the recovery mechanism for "resume from the last
  execution event" (priority 4/9 above); `CommandBlock`-based persistent
  history is architecturally the only path there, and it has its own
  two-part gap (see P1). See
  `docs/findings/2026-07-24-audit-persistence-restore.md` §3.

## Done (for context — remove once stale)

- 2026-07-24: **Strategic audit swarm + ADR-0003.** Five parallel agents
  audited execution correctness, input/concurrency, persistence/restore,
  isolation safety, and the client/SDK surface against a correctness-first
  priority order (superseding AGENTS.md's old phased roadmap). Findings
  folded into this backlog above; full evidence in
  `docs/findings/2026-07-24-audit-*.md`; decision recorded in
  `docs/adr/ADR-0003-correctness-first-strategic-pivot.md`.
- 2026-07-24: Fixed the P0 terminal-rendering "staircase" bug
  (`CompatTranslator::feed()` was accumulating instead of replacing raw VT
  bytes); fixed `malt-protocol/src/codec.rs`'s `MSG_*`/`DOMAIN_*` constants
  against real schema `@type()` values (Shell/Mux/Task/System were
  substantially fabricated); removed dead `mash::builtins` scaffolding and
  documented why `malt-compat`'s second VT parser API is parked, not
  orphaned; wired `malt-renderer`'s `max_output_bytes` truncation and
  `ClientState::should_shed()` client-eviction (both designed, neither
  previously connected to anything); propagated real exit codes through
  `exec` via a new `CommandOutput` struct (`command_id` still hardcoded,
  see P1); added test coverage for `malt-session::GroupManager` and
  `malt-platform::env`, finding and fixing a real duplicate-session-id bug
  in `add_session`. Commits `750fd37..13d298c`.
- 2026-07-24: Exposed existing session isolation through
  `malt new --isolation <bare|restricted|capped|contained>`; preserves the
  omitted-field/Bare default and rejects a mismatched reported tier. Verified
  against an isolated daemon across all four tiers and by `cargo test --workspace`
  (commit `4c892fd`).
- 2026-07-24: Reverted the uncommitted malt-stack (carboy/keg) dependency
  from `malt-platform`/`malt-elevate`; established vendor-not-depend policy
  (ADR-0001).
- 2026-07-24: Doc audit and fixes — `CLAUDE.md`, `AGENTS.md`,
  `docs/design/architecture.md`, `VEXIL_GAPS.md`, `docs/REFACTORING_PLAN.md` all
  brought back in line with verified reality.
- 2026-07-24: Ported HCS + capability-report model from carboy-isolation as
  owned source.
- 2026-07-24: Wired session `IsolationTier` through to real Job Object
  process containment in `mash`'s external-command spawn path (all 5 spawn
  call sites), with a real end-to-end test. (Subsequently found to be
  fail-open and tier-blind — see P0 above.)
- 2026-07-24: Test rigor pass — found and fixed 2 real bugs in
  `job_objects.rs` (undersized `IO_COUNTERS` struct silently failing every
  job creation; hardcoded active-process-limit of 1 silently rejecting the
  second process in any job). Implemented real `query_active_processes`
  (was a hardcoded stub). Audited `tokens.rs` and `pty/windows.rs` — both
  genuinely solid. Added missing success-path tests to `signals/windows.rs`.
  Fixed a `CWD_LOCK` poison-cascade bug and unguarded env-var races causing
  intermittent test failures in `mash`'s and `malt-platform`'s test suites.
- 2026-07-24: Consolidated `CLAUDE.md`/`AGENTS.md` into a single `AGENTS.md`
  (Claude Code only reads `CLAUDE.md`, which is now a one-line `@AGENTS.md`
  import — official supported pattern). Adopted GitHub Spec Kit for future
  feature specs, moved `specs/` to `docs/design/`. Deprecated
  `docs/superpowers/`.
- 2026-07-24: Full plan-vs-code audit — 6 parallel agents cross-referenced
  all 44 historical planning documents against actual current code. Result:
  the large majority checked out as accurate; findings folded into this
  backlog (P0 rendering-bug lead, compat-pane restore stub, dead-code
  items, codec.rs bug, test-coverage gaps). See
  `docs/findings/2026-07-24-plan-implementation-audit.md`.
- 2026-07-24: Landscape check against current (mid-2026) agentic-tooling
  reality, not just the March 2026 design-time assumptions. Verdict: the
  core architectural bet (daemon-owned session, structured protocol
  instead of raw VT codes) is directionally validated by where the market
  independently went (Warp, Ghostty, Microsoft Intelligent Terminal are
  all "agent-aware terminal" products now). ACP was evaluated and doesn't
  apply (wrong actor model — see the `get_output` P1 entry above).
- 2026-07-24: ADR-0002 — Gateway is the canonical agent control plane,
  MCP is a replaceable adapter, phased migration plan. Investigated where
  execution events should actually live: not `Bus`/`RingBuffer` (drops
  messages under pressure, destructive `drain()`, no sequence numbers —
  wrong for a durable log), but `malt-session::pane::CommandBlock`
  extended (right shape, currently unwired — see P1). Found two concrete
  bugs/gaps in the process: hardcoded `command_id`/`exit_code` in the
  Gateway, and `CommandBlock` never actually constructed anywhere outside
  tests.
