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
  - `gateway_backend.rs::parse_isolation` (line 24-31) silently maps any
    unrecognized isolation string to `Bare` (`_ => IsolationTier::Bare`) —
    a typo'd `isolation` field in a request body (including one an agent
    constructs itself) produces a silently-`Bare` session with a 200
    response, no error.
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
- **`mash::Env::to_snapshot()`/`apply_snapshot()` — a fully-built, tested
  mechanism for exactly "session state survives daemon restart" — is never
  called by `malt-daemon`.** `build_persisted_session` only ever captures
  two scalar strings (`SHELL`, `PWD`) out of a live mash `Env`; everything
  else — exported and non-exported variables, functions, aliases, jobs,
  directory stack, traps — is silently dropped on every restart or
  detach/reattach, and restore creates a brand-new `Env::from_os()`. But
  `EnvSnapshot` (`crates/mash/src/env.rs:1150-1533`) already captures
  exactly this, is exercised by 34 passing tests, and was explicitly
  designed for this per the original Phase 2 spec ("on detach/reattach or
  daemon restart, the EnvSnapshot is restored and `FOO=bar` is back") —
  `malt-daemon` has simply never called either method. Same shape of gap
  as `CommandBlock` above: real subsystem, zero callers. Also needs a
  schema field (same constraint as the `CommandBlock` persistence gap).
  Silent, surprising data loss today, not a missing nicety. See
  `docs/findings/2026-07-24-audit-persistence-restore.md` §4
  (recommendation 1); also fix the small adjacent dead round-trip where
  `shell_path` is captured on persist and discarded on restore
  (`coordinator.rs:540-543`, recommendation 4, trivial, do alongside).
- **Compat-pane session restore is a confirmed stub** —
  `coordinator.rs:547-551` returns `DaemonError::RestoreFailed(id,
  "compat pane restore not yet implemented")`; `spawn_compat` was never
  written (zero grep hits). Shell-session restore is real and tested (26
  tests) and serves as the template. Minimal correct implementation
  sketched: re-launch via `spawn_with_pty` under the session's isolation
  tier, construct a fresh `CompatTranslator` (no VT state to restore —
  "process memory is not captured" is correct policy, not a shortcut),
  feed its output through the existing `PtyOutput` path, and wire the
  process handle into `ProcessSupervisor`'s existing bookkeeping. This is
  re-launch, not re-attach — no new design decision needed, only
  implementation. See
  `docs/findings/2026-07-24-audit-persistence-restore.md` §2
  (recommendation 3). Adjacent landmine, cheap to fix now: `restore_session`
  picks the first pane in `persisted.panes` by map order instead of
  consulting `persisted.focus` — harmless under today's single-pane model,
  a real bug waiting for Phase F multi-pane work (§2 caveat, recommendation 6).
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
- **Agent-facing `get_output` returns the wrong representation — the
  human rendering pipeline, not something built for a program to parse.**
  Confirmed still `StyledGrid` end-to-end
  (`crates/gateway/src/routes/sessions.rs:61-67` →
  `gateway_backend.rs:136-148` → `session_thread.rs::get_grid_output`).
  Zero plain-text/structured variant exists anywhere, including
  client-side in `malt-mcp` (bare passthrough). `exec`'s response is
  already plain-text-shaped after today's fix; `get_output` (used for
  monitoring ongoing/backgrounded sessions) isn't. Two-tier fix: cheap
  now — add `get_plain_text_output()` next to `get_grid_output()`, walking
  the same grid cells but emitting characters only, wired through a new
  command variant/format param and gateway route, leaving the existing
  `StyledGrid` route untouched for `malt-tui`/`maltty`. Real fix — deferred
  to the `CommandBlock`/execution-resource work above, since true
  stdout/stderr separation with command-boundary framing needs the
  per-command captured-output buffer ADR-0002 already flags as
  undecided. See
  `docs/findings/2026-07-24-audit-execution-correctness.md` §1
  (recommendation 2). Also recording a considered non-decision so it
  doesn't get re-litigated: Agent Client Protocol (ACP) was evaluated and
  doesn't apply — different actor model (ACP's client hosts an embedded
  agent's UI; MALT's is an external agent remotely driving a persistent
  session).
- **Three gateway pane-management routes are fake, not previously
  documented.** `DaemonBackend::list_panes` (`gateway_backend.rs:150-161`)
  always returns one hardcoded `PaneResponse`, ignoring the session's real
  panes; `split_pane` (`:163-176`) is an explicit stub returning a fake
  response without touching the daemon; `close_pane` (`:178-180`) is a
  no-op always returning `Ok(())`. Doesn't block the demo-lens scenario
  (single-pane), but any SDK work should not expose these as if they
  worked. See `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1.
- **`malt-bin` has no `get_output` client method at all**, and `malt-mcp`'s
  `create_session` silently defaults an omitted `name` to the literal
  string `"default"` (`unwrap_or("default")`) while the CLI/curl path
  correctly omits the field — a real behavioral divergence between two
  consumers meant to share one contract per ADR-0002 principle 4. Both
  small, worth fixing whenever next in the relevant files. See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §1, §2.
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
- **PTY/compat supervisor spawn path has no isolation wiring, and is
  currently unreachable** — `ProcessSupervisor::spawn`
  (`crates/malt-daemon/src/supervisor/mod.rs:29-70`) never reads
  `SpawnRequest.isolation` despite the field existing (populated only by
  test fixtures). Confirmed latent, not live: `Coordinator` holds a
  `supervisor` field but never calls `.spawn()` on it anywhere, because
  Compat-pane creation is only reachable via the restore path, which is
  the confirmed stub above. Becomes live risk the moment compat-pane
  restore or any PTY-pane feature is implemented — should land no later
  than that work, not be treated as already exposed today. See
  `docs/findings/2026-07-24-audit-isolation-safety.md` item 5.
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
