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

**Delivered** (kept short; detail lives in the sections below and in
`docs/findings/`): 0a Gateway auth enforcement · 0b responsive session
control · 1 correct stdout/stderr · 2 exit codes and execution IDs ·
3 command lifecycle events · 4 persistent execution history · 8 the TUI
rendering path.

**Open, in priority order:**

- **5 + 6. Genuine raw input and human/agent coexistence** — being specified
  together as `specs/005-raw-input-authority/`, now including transport
  authentication after audit finding A-01 showed the interactive transport
  performs no identity check at all.
- **7. Fail-closed requested isolation** — see P1. Audit A-02 rates this
  Critical: a tier can be reported as applied while the session runs
  uncontained. Spec 001 is reopened for the same reason.
- **9. Session restoration** — largely delivered; the remaining gap is
  restored Compat panes running outside the session's isolation group
  (audit A-06/A-11).
- **10. One excellent agent client or Gateway SDK** — unblocked now that the
  Gateway contract has stabilized; nucleus exists in `malt-bin/src/events.rs`.

**Audit-raised, not previously on this list** (see
`docs/findings/2026-07-25-architecture-spec-codebase-audit.md` for the full
set and its recommended closure order): VNP transport authentication and
pre-handshake resource exhaustion (A-01/A-08, folded into spec 005);
predictable/leaked Gateway credentials (A-03); the clean-machine CLI
startup regression (A-04); lifetime-counter rate limiting (A-05); process
termination that only removes bookkeeping (A-06).

## P0 — blocks daily-driver usability and safe agent use

- **Gateway auth was not enforced anywhere — FIXED 2026-07-25.** Added
  `crates/malt-gateway/src/middleware.rs`: a real axum middleware
  (`with_auth`) wiring the existing `TokenStore`/`AuthContext`/
  `RateLimiter` into every route. Extracts the `Authorization: Bearer`
  header, validates it, checks per-route required scope against a table
  matching architecture.md's scope model (Monitor/Read/Interact/Admin —
  unrecognized routes fail closed to `Admin`), applies rate limiting, and
  returns real 401/403/429 via the `GatewayError` variants that already
  existed but were never constructed. **Must be applied last, after every
  route including `malt-bin`'s ad hoc `/shutdown`** — axum's `.layer()`
  only wraps routes registered before it's called, so `build_router()`
  itself stays auth-free and `daemon.rs` calls `with_auth(router, ...)`
  as the final step once `/shutdown` is added, or that route would have
  shipped unauthenticated.
  **Every first-party client that needs to keep working was updated to
  send the token**: `malt-bin`'s `MaltClient` and `malt-mcp` both now read
  the same well-known token file
  (`malt_gateway::auth::dirs_token_path()`, `~/.config/malt/api-token`)
  the daemon already wrote and attach it as a bearer token on every
  request. `malt-mcp` duplicates the path-resolution logic rather than
  depending on `malt-gateway`, deliberately preserving its
  zero-internal-dependency design (ADR-0002).
  **Real, known consequence, not silently introduced**: `malt-tui`'s
  orphaned HTTP mode and `malt-web` have no token mechanism and will now
  get 401 on every request. Both were already flagged for freeze/no-further-
  investment in the client-SDK audit (§5) before this change — `malt-web`
  in particular has no way to add one without a real design (browser JS
  can't read `~/.config/malt/api-token`), which is out of scope here as
  new feature work. `malt-tui`'s VNP mode (the actual `malt attach` path,
  and the one the audit recommends as the sole reference client) is
  unaffected — it talks to the VNP socket, not the HTTP Gateway.
  New tests (`crates/malt-gateway/tests/routes.rs`): all 7 existing route
  tests updated to send a valid token (proving the change didn't silently
  loosen anything), plus 5 new tests —
  `request_with_no_token_is_rejected`, `request_with_invalid_token_is_rejected`,
  `insufficient_scope_is_forbidden` (a Monitor-scoped token can't create a
  session), `monitor_scope_can_still_read_health` (the same low-privilege
  token still works for a Monitor-level route — proves the scope check
  isn't accidentally requiring Admin everywhere), and
  `rate_limit_exceeded_is_rejected`. See
  `docs/findings/2026-07-24-audit-client-sdk-surface.md` §4.
- **0b. Responsive session control during execution — FIXED 2026-07-25.**
  Each active session now has a responsive control actor and a single-owner
  MASH worker connected by one bounded FIFO ingress. The worker waits for the
  control actor to finalize output and the full shell snapshot before starting
  the next request. Attach, output, resize, frame acknowledgement, editor
  evaluation, snapshots, detach, and shutdown intent therefore stay off the
  execution path. Admission is explicit (256 pending by default), with stable
  queue-full, unavailable, and closing errors. Evidence: 100 busy VNP attaches
  each completed within one second, the native Windows Smoosh suite remains
  183/183, and a 600.20-second cross-session observation soak passed; see
  `docs/findings/2026-07-25-responsive-session-control.md`.
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
- **The VNP client (`malt-tui`) never rendered shell output at all — FIXED
  2026-07-25.** `TuiRenderer::apply_one` had no handler for
  `RenderCommand::WriteRaw`, so every `RenderBatch`/`InitialState` a real
  VNP client received for actual shell content was silently dropped —
  this was the "authoritative" client showing a blank screen regardless
  of the P0 rendering fix, which was only ever observed via the HTTP
  `/output` route's separate client-side JSON synthesis. Went through a
  short design pass before implementing (per the project owner's request)
  rather than full Spec Kit ceremony, since the shape was clear
  (reuse `malt-compat`, don't reinvent VT parsing) but still had a real
  design question (state lifetime, invariant compliance) worth confirming
  first. Fix: `TuiRenderer` now owns a `CompatTranslator`
  (`malt-compat` added as a `malt-tui` dependency) — the sanctioned way to
  get VT rendering in a second place without violating Hard Invariant #1
  ("VT codes in `malt-compat` only"). On `WriteRaw`, resizes the
  translator to match the command's `(width, height)` if needed, feeds
  the bytes, then paints each cell's `(ch, fg, bg, bold, ...)` onto the
  ratatui buffer via the existing clip-aware `set_cell`. Three new tests
  in `crates/malt-tui/tests/render.rs`: plain text actually renders
  (previously silently dropped), SGR bold survives the full
  `CompatTranslator` → `TerminalGrid` → ratatui `Style` pipeline, and a
  second `WriteRaw` in a later frame doesn't wipe out content from an
  earlier one (the grid is persistent state, matching the P0 fix's
  replace-not-truncate semantics). See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §3a.
- **Gateway/agent-driven execution never notified attached VNP clients
  that anything happened — PARTIAL FIX 2026-07-25.** `dispatch_render()`
  was called only from `KeyInput`'s handler; `RunCommand` (the Gateway
  `/exec` path — ADR-0002's "canonical agent control plane"), `WriteInput`,
  and `PtyOutput` never called it, so if an agent ran `cargo test` via
  `POST /exec` while a human was attached via VNP, the human's client
  received no `RenderBatch` reflecting that command, ever. Now all three
  call `dispatch_render()` once their handling completes — `RunCommand`
  after the command finishes, `PtyOutput` after each chunk is fed to the
  grid, `WriteInput` after its (currently `RunCommand`-equivalent, see the
  raw-input item above) execution completes. New test
  `run_command_triggers_render_to_attached_client`
  (`malt-daemon/tests/session_thread.rs`) proves an attached VNP client
  now receives a real `RenderBatch` after a gateway-driven `RunCommand`.
  **Still partial, as scoped**: this pushes one frame at completion, not
  incremental output during a long-running command. See
  `docs/findings/2026-07-24-audit-input-concurrency.md` §3b (recommendation 2).

  **Unblocked 2026-07-25.** This entry used to say true streaming needed 0b
  first — the control actor could not push intermediate frames while blocked
  inside `run_mash_command`. Feature 002 did exactly that decoupling: MASH now
  runs on a dedicated worker and the control actor is free while a command
  runs. So the stated prerequisite is met and this is now buildable, not
  blocked.

  It is a feature in its own right and belongs in its own spec, not folded
  into 005 (input and authority). Two layers, in dependency order:

  1. **Session output to clients.** The worker produces output continuously;
     the control actor should feed the compat translator and dispatch render
     batches as it arrives rather than once at finalization. This is what
     ADR-0003's guiding demo means by "MALT reports structured progress", and
     what makes a two-minute `cargo test` watchable instead of silent. It also
     needs a decision on how incremental output reaches the Gateway — today
     `/exec` returns one final payload, so an agent has the same blind spot.
  2. **Tool output within mash.** `ToolFn` returns a buffered `BuiltinResult`,
     so an interactive `cat` shows its echo only when the command ends. Needs
     a writer-based signature and a different result type; note the output
     redirect machinery (`apply_output_redirects`) currently operates on that
     buffer, so it changes with it. Smaller than (1) and largely falls out of
     it.


### From the 2026-07-25 architecture/spec/codebase audit

Full evidence with file:line in
`docs/findings/2026-07-25-architecture-spec-codebase-audit.md`. Its own
recommended closure order is: transport auth, then fail-closed isolation,
then credentials/rate limiting, then process termination and raw input.

- **A-01 (Critical). The VNP transport is an unauthenticated, authority-free
  control plane.** Verified independently: there is no token, credential,
  scope, or peer check anywhere in `crates/malt-daemon/src/vnp_listener.rs`.
  The listener accepts every connection, starts a thread, and sends the
  session inventory during the handshake — before any identity is
  established, because none ever is. It then accepts caller-supplied session
  IDs and routes keys by session alone. `SessionCommand::KeyInput` carries no
  client ID, so there is structurally nothing to enforce authority against,
  and `AttachClient`/`DetachClient` (the only commands touching
  `AuthorityTracker`) have zero production call sites. Any local process can
  enumerate, observe, resize, and inject input into any session. HTTP bearer
  auth does not cover this path — it is a different transport.
  **CLOSED 2026-07-25** by `specs/005-raw-input-authority/`. Every element
  above is now false: the transport authenticates before disclosing anything
  (`crates/malt-daemon/src/connection/handshake.rs` takes the inventory as a
  `FnOnce` so the ordering is structural, not a convention);
  `SessionCommand::KeyInput` carries an `InputOrigin`; and the vestigial
  `AttachClient`/`DetachClient` pair is deleted, so there is one attach path
  and it drives `AuthorityTracker`.
  Evidence: `vnp_listener::an_unauthenticated_client_is_refused_and_learns_no_session_names`,
  `attaching_over_the_wire_applies_the_requested_authority`,
  `attaching_as_an_observer_over_the_wire_does_not_take_authority`,
  `dropping_the_connection_releases_authority`, plus the live byte-level check
  in `docs/findings/2026-07-25-spec-005-quickstart-verification.md`.
  **Still open, deliberately:** local identification uses a shared bearer
  token over TCP, not the OS peer credentials
  (`SO_PEERCRED`/`GetNamedPipeClientProcessId`) that
  `docs/design/architecture.md` specifies for local transports. The token
  closes the disclosure and injection hole; it does not make "same user"
  structural, and any local process that can read the token file is
  indistinguishable from the owner. Migrating the local transport to a Unix
  socket / named pipe with peer credentials remains the intended end state.
- **A-08 (High). VNP allows pre-handshake resource exhaustion.** The accept
  loop spawns an unbounded OS thread per connection and only sets a read
  timeout *after* blocking handshake work, so connect-and-stall clients
  retain threads and sockets indefinitely. Same listener, same fix window —
  folded into spec 005 as FR-003/FR-004 ("an unidentified caller must not
  harm the daemon").
  **CLOSED 2026-07-25.** A 10-second deadline is applied before the first
  blocking read, and `MAX_PENDING_HANDSHAKES` (64) bounds concurrent
  unidentified connections. The deadline is set on the stream itself rather
  than on the write-side clone: `try_clone` duplicates the descriptor and a
  receive timeout is per-descriptor on Windows, so the obvious placement
  silently does nothing. Evidence:
  `vnp_listener::a_connection_that_never_identifies_is_closed` and
  `vnp_listener::stalled_connections_do_not_block_a_legitimate_client`.
- **A-02 (Critical). Requested isolation can succeed without isolating.**
  `apply_session_isolation` returns `()` and logs Job Object failure while
  continuing uncontained; `Capped` and `Contained` resolve to the same
  placeholder limits; `Contained` never launches inside HCS; non-Windows has
  no enforcement path at all; and session creation reports the *requested*
  tier rather than a verified one. This is why `specs/001-cli-isolation-flag/`
  was reopened as Partially implemented on 2026-07-25 — its fail-closed
  requirement is unmet, and marking it Complete asserted a security guarantee
  the code does not provide. See the isolation-policy entry in P1 for the
  concrete plumbing proposal.
- **A-03 (High). Gateway credentials are predictable, leaked, and possibly
  non-durable.** `generate_random_token` derives its value from epoch
  nanoseconds and fixed arithmetic rather than a CSPRNG, so tokens are
  guessable by anyone who can approximate daemon start time. Directory
  creation and token-file write errors are ignored, so a daemon can come up
  believing it persisted a token it did not. And `malt-bin/src/daemon.rs`
  prints the admin token to stdout on every start. Fix: OS CSPRNG bytes, an
  atomic owner-only write that fails startup on error, and stop printing the
  secret.
- **A-04 (High). Gateway auth broke the clean-machine CLI flow — a
  regression from the 0a work.** `MaltClient` reads the token once at
  construction, but `main.rs` constructs the client *before* starting the
  daemon. On a machine with no existing token file the daemon then creates
  the token while the already-built client holds `None`, so its health
  probes go out unauthenticated and the default `malt` flow fails. An
  existing token file masks this entirely, which is why it survived the
  session it was introduced in. Compounding it, `MaltClient::shutdown`
  ignores the HTTP status and body, so `malt stop` can report success after a
  401. Fix: construct or reload the client after daemon startup, and treat
  non-success statuses as errors.
- **A-05 (High). Rate limiting is a permanent lifetime counter.**
  `RateLimiter` keeps only a token→count map and increments forever; no
  production path ever refills or evicts. A valid token therefore reaches a
  permanent 429 after 100 requests for the life of the daemon. Also missing
  the architecture's per-session and global dimensions, rate metadata, and
  request-body size limits. Fix: a monotonic-clock window or bucket with
  eviction, plus the missing dimensions, plus bounding bodies before handlers
  allocate. (Wired in during 0a without noticing it never refills.)
- **A-06 (High). `ProcessSupervisor::kill` only removes bookkeeping.** It
  drops the map entry; dropping the `Child` does not terminate anything —
  Unix only nonblocking-reaps and Windows just closes the handle. Restored
  Compat panes create supervisor-owned processes, and session destruction
  never kills them, so they outlive their session. Fix: terminate and wait on
  the process (or its isolation group) before removal, invoke it on
  destroy/shutdown, and test with a real PID that actually dies.


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
  across three successive `exec` calls on one session. **Restore-survival
  added 2026-07-25** as part of the persistent-execution-history work
  below: `spawn_with_cwd` now sets `next_command_id` to
  `max(command_id)` over the restored history (0 if none), so
  post-restore executions continue the sequence instead of restarting at
  1 and colliding with persisted ids. Derived from the history itself
  rather than persisted separately — a second source of truth could only
  disagree. Asserted by
  `command_history_survives_dormant_restore_and_ids_stay_monotonic`.
- **Command lifecycle event delivery (ADR-0003 priority 3) — DELIVERED
  2026-07-25.** Full Spec Kit treatment:
  `specs/004-command-lifecycle-events/`. `GET /sessions/{id}/events`
  streams `command_started`/`command_finished` as Server-Sent Events with
  `Last-Event-ID` resume, `AuthScope::Read`, and a defined slow-consumer
  policy. `malt watch <ID>` is the first-party consumer.
  Published from the two control-actor handlers that already drive command
  history, so events and history derive from one source and cannot
  disagree; the finish event is emitted at the *commit* point, so it never
  arrives before the output that produced it is visible.
  Evidence: 12 unit + 12 integration tests in `malt-daemon`, 5 route tests
  in `malt-gateway`, 10 frame-parser tests in `malt-bin`, plus live-daemon
  validation (`docs/findings/2026-07-25-command-lifecycle-events.md`).
  **Three real bugs were caught by driving the real path rather than by
  review** — a frame parser that never stripped the line terminator (the
  client received nothing while eight unit tests passed on pre-trimmed
  input), an off-by-one that made a resuming subscriber's gap claim events
  it had already seen, and a terminal gap that shared the buffer which had
  just overflowed so a lagged subscriber could not be told it lagged.
- **⚠️ The `Bus` STILL has zero consumers — this feature deliberately did
  not use it, contradicting how this backlog previously framed the work.**
  Recorded under Constitution IX rather than quietly re-scoped. The reason
  is concrete: `Bus`'s `Reliable` priority is documented and implemented to
  grow its ring "beyond capacity rather than evict", and both
  `CommandStarted` and `CommandFinished` are specified `Reliable`. Routing
  them through the Bus would have handed an untrusted, possibly-stalled
  subscriber unbounded daemon memory growth — the exact failure the
  feature existed to prevent. Delivery instead uses a bounded
  per-subscriber channel plus a bounded per-session event log, following
  the `render_pushers` precedent.
  **The open question this exposes**: a message bus whose highest
  reliability tier grows without bound has no safe consumer. It needs
  either (a) a bounded `Reliable` policy with explicit gap signalling —
  essentially what `executor/events.rs` now implements, which could be
  generalized back into it — or (b) an honest re-scoping to trusted
  in-daemon consumers only, with the unbounded growth documented as
  intentional and the "zero consumers" status accepted. Deciding that is
  its own change with its own tests; it was explicitly *not* fixed
  opportunistically inside feature 004. See
  `specs/004-command-lifecycle-events/research.md` R2.
- **`malt watch` does not promptly reconnect after an abrupt daemon
  death.** Found 2026-07-25 during feature 004's live validation
  (`docs/findings/2026-07-25-command-lifecycle-events.md`). The reconnect
  loop is correct — a *graceful* stream close is detected immediately and
  resumes via `Last-Event-ID` — but a vanished peer leaves the client
  blocked in `read_line` on a half-open socket until the OS TCP timeout,
  which on Windows can be minutes. The clean fix is a read timeout on the
  streaming response (a stream silent for longer than the server's SSE
  keep-alive interval is dead), but `reqwest` 0.13.2's blocking API has no
  `read_timeout`, and its total-request `timeout` would tear down healthy
  streams too. Options: a bounded stream lifetime with automatic resume
  (simple, adds a periodic reconnect), or move the client to the async
  API. A real design choice, not a one-liner — deliberately left for its
  own change.
- **Deferred from feature 004, deliberately, not overlooked:**
  - **VNP lifecycle-event forwarding to `malt-tui`.** The two schema
    messages exist and the daemon-side event source is transport-neutral
    precisely so this is additive. Gateway SSE was served first because
    ADR-0002 makes the Gateway canonical and the P1 story was an agent;
    `malt-tui` already renders command output live, so it gains less.
    See research R1.
  - **An MCP-native subscription.** A long-lived stream does not fit MCP's
    request/response tool shape, and a polling tool over a streaming
    endpoint would be a worse interface than the endpoint agents can
    already consume. Belongs with a real MCP notifications design. See
    research R8.
  - **`malt-gateway-sdk`.** Still correctly sequenced *after* contract
    stabilization, not into it — the decisive test was whether designing
    for an SDK would change the events contract, and it would not
    (standard `Last-Event-ID` resume, explicit gap events, errors as HTTP
    status before the stream opens). `crates/malt-bin/src/events.rs` is
    written as its nucleus: CLI-free and self-contained, so extraction is
    a move rather than a rewrite. See research R10.
- **Persistent execution history (ADR-0002 Phase 3/4) — both stacked gaps
  FIXED 2026-07-25**, sequenced as one coordinated change per the
  original recommendation. Full Spec Kit treatment:
  `specs/003-command-execution-history/`.
  **Gap A (in-memory, `CommandBlock` had zero non-test constructors):**
  `SessionExecutor` now owns a `PaneRuntime`, and `run_mash_command`
  pushes an *open* block (`finished_at`/`exit_code` absent) **before**
  parsing or executing, finalizing it on all three return paths (parse
  error → 1, empty input → 0, real execution → real exit code). Recording
  before rather than after is load-bearing, not stylistic: it is what
  makes a daemon that dies mid-command leave an honest "started, never
  confirmed complete" record instead of no record at all, with no
  special-case code. Two new `PaneRuntime` methods back this:
  `finalize_current_block` (no-op on an already-completed block, which is
  what keeps restored history immutable) and `with_blocks` (restore
  seeding, truncating oldest-first past `max_blocks`).
  **Gap B (persistence schema):** new `PersistedCommandBlock` message and
  a `command_blocks : array<PersistedCommandBlock>` field on
  `PersistedPane` — on the message, not the `Shell` variant, since
  history is a pane property that `Compat` panes should inherit. Additive:
  pre-existing `.vxb` files decode to an empty list, `schema_version`
  stays 1. `build_persisted_session` writes it; `spawn_with_cwd` seeds
  the `PaneRuntime` and resumes `next_command_id` from it.
  **Retrieval surface** (follows the `get_output_text` chain end-to-end):
  `SessionCommand::GetCommandHistory` →
  `Coordinator::get_session_command_history` →
  `GatewayBackend::get_command_history` → `GET /sessions/{id}/history`
  (`AuthScope::Read`, same sensitivity class as `/output` — command text
  can contain secrets typed at the prompt), plus `malt history <ID>` and
  a 7th MCP tool `get_command_history`. A **dormant** session answers
  from its persisted snapshot rather than being woken — listing what a
  session already ran should not restore it as a side effect. An unknown
  session is 404, never an empty 200, so "no such session" stays
  distinguishable from "ran nothing yet".
  Evidence: `command_history_records_each_execution_with_real_status`,
  `command_history_records_a_parse_error_as_a_failed_execution`,
  `command_history_for_unknown_session_is_not_found`,
  `command_history_survives_dormant_restore_and_ids_stay_monotonic`
  (`malt-daemon/tests/gateway_backend.rs`);
  `get_command_history_reflects_executions_on_the_session_thread`,
  `restored_history_seeds_the_pane_and_resumes_command_ids`
  (`malt-daemon/tests/session_thread.rs`);
  `command_history_survives_the_bitpack_round_trip`,
  `a_session_with_no_command_history_round_trips_as_empty`
  (`malt-daemon/tests/store.rs`); 4 route/auth tests in
  `malt-gateway/tests/routes.rs`; 8 `PaneRuntime` tests in
  `malt-session/tests/pane.rs`; 2 MCP tool tests.
  **Integrated with feature 002 (responsive session control) on rebase,
  2026-07-25.** 002 moved MASH execution onto a worker thread that solely
  owns `Env`, so recording moved to that same boundary: the worker
  announces `SessionCommand::ExecutionStarted` immediately before running
  (the control actor pushes the open block) and `ExecutionCompleted`
  finalizes it, committed alongside the env snapshot so history and shell
  state become visible together. Two things this caught that neither
  branch had alone: (a) `command_id` assignment now lives in the worker,
  which hardcoded its counter to `0` — restore-resume is seeded via a new
  `start_command_id` parameter on `spawn_command_worker`, or post-restore
  ids would silently have collided with persisted ones again; (b) the
  Gateway history read originally blocked up to 2s *holding the
  coordinator mutex*, which under 002's architecture would stall every
  other session's control traffic — restructured to 002's `begin_*`
  pattern (return the receiver, drop the lock, then wait). The payoff:
  a still-running command is now genuinely visible in history rather than
  only structurally recorded — `a_running_command_is_visible_in_history_before_it_finishes`
  asserts a live in-flight entry returns in well under a second while a
  1-second command runs, and is finalized in place afterwards rather than
  duplicated.
  **Still open, deliberately not folded in:** history records execution
  *metadata* only, not per-command output/scrollback (a separate concern
  — see the terminal-grid items); and `CommandStarted`/`CommandFinished`
  bus events remain producerless, since that needs the bus-consumer work
  above, not this. Original analysis:
  `docs/findings/2026-07-24-audit-execution-correctness.md` §4
  (recommendation 5) and
  `docs/findings/2026-07-24-audit-persistence-restore.md` §1
  (recommendation 2).
- **Gateway error mapping for unknown/dormant sessions — RESOLVED
  2026-07-25.** Two separate issues, only one of which was real by the
  time it was checked:
  - *Unknown session reported as an internal error* — already fixed by
    feature 002's `map_execution_error`, which maps
    `DaemonError::SessionNotFound` to `GatewayError::SessionNotFound`.
    The original finding predated that rebase. Verified live: `exec`,
    `output`, `output/text`, and `history` all return 404 for a
    nonexistent session.
  - *Dormant session reported as an internal error* — this one was real
    and is now fixed. `DaemonError::SessionDormant` fell through
    `map_execution_error`'s catch-all to `GatewayError::Internal`, so
    `exec`/`output` on a restored-but-not-yet-attached session returned
    **HTTP 500** with the message "session ... is dormant — attach to
    restore it". A caller-actionable lifecycle state was being reported
    as the daemon having broken. Added `GatewayError::SessionDormant`
    → **409 Conflict** (`session_dormant`), the same class as the
    existing `SessionShuttingDown`. One enum variant, one status
    mapping, one match arm — every affected route already funnels
    through `map_execution_error`, and all nine dormant checks in
    `coordinator.rs` already produced the right `DaemonError`.
    Test: `a_dormant_session_reports_a_conflict_not_an_internal_error`
    (`malt-daemon/tests/gateway_backend.rs`) drives a real
    persist→dormant cycle and asserts 409-class for exec and output,
    404 still distinct for a missing session, and that `history`
    deliberately still succeeds on a dormant session (it answers from
    the snapshot rather than requiring a restore).
- **Two more `DaemonError` variants still land on 500, deliberately left
  alone 2026-07-25.** Noticed while fixing the dormant-session mapping,
  not fixed with it because neither is reachable through
  `map_execution_error` on a normal Gateway path, so changing them would
  be speculative rather than evidence-driven:
  - `NameConflict` (no unique session name after 100 attempts) — reaches
    the Gateway via `create_session`, which maps errors with its own
    inline `GatewayError::Internal(...)` rather than
    `map_execution_error`. Arguably 409. Needs a test that can actually
    provoke it (100 colliding names) before it's worth changing.
  - `AppRestoreNotSupported` — only constructed in `restore_session`,
    which today is reached through VNP attach, not any HTTP route. If a
    restore-triggering HTTP route is ever added, this should be a 501 or
    409, not a 500.
  Fix either only alongside a test that reaches it for real; both are
  currently unreachable-by-inspection, and an untestable status change is
  how the "looks done but isn't" pattern starts.
- **`vexilc` 0.5.1 attaches a field-level `@doc` to the *preceding*
  field.** Found 2026-07-25 while adding `PersistedCommandBlock`: a
  `@doc` written immediately above a field lands on the field before it
  in the generated Rust. Not new — the already-committed `EnvSnapshot`
  has the same misattribution in its generated docs (`"name -> source
  text, re-parsed on restore"` sits on `aliases` instead of `functions`;
  `"signal -> action string"` sits on `cwd` instead of `traps`).
  Cosmetic only: field order, wire format, and encode/decode are all
  correct — it misleads readers of the generated code, nothing else.
  Worked around in the new message by putting all field documentation in
  the message-level `@doc` block. Needs reporting upstream to
  `vexil-lang`; the existing `EnvSnapshot` annotations should get the
  same treatment once it's fixed or confirmed wontfix.
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
  2026-07-24 (audit finding **A-09**), `split_pane`/`close_pane`
  intentionally left as honest
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


### From the 2026-07-25 audit (medium severity)

- **A-10. Gateway responses are not authoritative.** `create_session` echoes
  the *requested* name, while the coordinator auto-suffixes duplicates — so a
  response can say `foo` when the session is really `foo-2`, and a later list
  disagrees with the create that made it. Separately, destroying a
  nonexistent session returns success, because the backend returns `Ok` and
  the coordinator silently no-ops. The route tests use a mock backend, which
  is exactly why neither surfaced. Fix: return stored coordinator data,
  return typed `NotFound`, and add coverage against the real backend rather
  than the mock.
- **A-13. Hard invariants have production violations.** The constitution bans
  `unwrap`/`expect` outside tests and OS calls outside `malt-platform`. Both
  are violated: `DebouncedStore` panics if the flush thread fails to spawn;
  nine `SharedFdRegistry` methods panic on mutex poison; `Io::File::clone`
  panics on descriptor exhaustion; `malt-daemon`'s supervisor uses
  `std::os::windows::io` directly; and `malt-tools` makes direct OS FD and
  symlink calls. Fix: propagate structured errors, recover from poison
  deliberately where that is the right call, and move OS-specific work behind
  `malt-platform`. Worth doing as its own pass — these are the invariants
  AGENTS.md claims are clean.
- **A-14. `FrameWriter` lacks the reader's size bound.** `FrameReader`
  rejects payloads over `PROTOCOL_MAX_FRAME_SIZE`, but `FrameWriter` casts
  `payload.len()` to `u32` without the same check, so it can emit a frame the
  peer will reject, or truncate an oversized advertised length. Fix: validate
  before narrowing, return `FrameTooLarge`, and test both boundaries.
- **A-15. Vi `dd` state leaks across sessions.** The pending-`d` operator
  lives in a process-global `AtomicBool` in `malt-term/src/keymap.rs` rather
  than on `Editor`, so a `d` typed in one session can make a `d` in another
  session delete that second editor's line. Fix: move the pending-operator
  state onto `Editor` and add an interleaved two-editor test. Same class as
  the shared-process-state bugs AGENTS.md already documents.
- **A-11 (update). Compat restore also picks the wrong pane.** Already
  tracked here for running outside the session's isolation group; the audit
  adds that restore selects the first `BTreeMap` entry rather than the
  persisted `focus`, and that a stale comment still claims Compat restore is
  unimplemented while the code launches it.
- **A-16 (update). Active stubs in user-facing paths.** Already partly
  tracked; the audit's specifics are `maltty` (startup `expect`s, ignores
  render lines), `ThemeResolver` (no-op returning default colours),
  `malt-term` keymap (history navigation, completion, ex mode, and search all
  ignored), and `mash` (`select`, `coproc`, `umask`, limited `ulimit`). The
  correction stands: implement, hide behind an unavailable capability, or
  document as unsupported — but do not return apparent success.

### Smaller items found while closing the audit's quality gates (2026-07-25)

- **`rm` treats an unrecognized `-flag` as a path.** Real `rm` rejects it as
  an invalid option. Found because clippy flagged an `if`/`else` whose
  branches were identical; the branches were collapsed but the behaviour was
  deliberately left alone, since changing it needs its own tests. See
  `crates/malt-tools/src/custom/rm.rs`.
- **`vexilc` codegen emits lint-dirty Rust.** Beyond the `@doc` off-by-one
  already recorded above: redundant borrows (`write_string(&inner_val)`),
  same-type casts (`self.timestamp as u64` where it is already `u64`), and
  unused glob imports (`use vexil_runtime::*` in generated config modules).
  Harmless, but they force the generated trees to be exempted from the
  workspace lint gate. Worth reporting upstream together with the `@doc`
  issue.
- **The lint gate is now green but not enforced.** `cargo fmt --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` both pass as of
  2026-07-25, but nothing runs them automatically. The audit's correction
  asks for deterministic CI gates; without one this will drift back, as it
  already did once.


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
  success while doing nothing** (audit finding **A-12**; `stub_success()`,
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

### 2026-07-25 audit findings closed the same day

- **A-17 — formatting and lint gates were red.** `cargo fmt --check` failed
  across 39 tracked files and `cargo clippy -- -D warnings` failed. Both now
  pass. Fixing the clippy gate surfaced two real defects (a duplicated
  comparison in `sed`, and `lines().flatten()` looping forever on a
  persistently-failing reader) plus a backoff that reset on connect rather
  than on success. Generated vexilc trees and a small number of
  deliberate-by-design lints are scoped with written justification rather
  than obeyed. **Not yet CI-enforced** — see the smaller-items entry in P1.
- **A-18 — the priority header contradicted the completed-work record.**
  It listed delivered items as pending, which could have routed someone back
  into finished work. Replaced with delivered-vs-open.
- **A-19 — delivered specs still said Draft.** 002/003/004 marked
  Implemented with pointers to their findings docs. Spec 001 was worse than
  reported: it claimed *Complete* while its fail-closed guarantee is unmet,
  and is now reopened as Partially implemented (see A-02).
- **A-20 — governing documents were stale.** ADR-0004 written for the
  Bus/bounded-channel divergence (a backlog note was not sufficient for a
  design contradiction); `architecture.md` amended in place and its
  `PersistedPane` model refreshed to include `env_snapshot` and
  `command_blocks`; the `EnvSnapshot` schema comment no longer claims it is
  unwired; feature 004's quickstart is platform-labelled with a PowerShell
  path instead of Bash-only `kill -STOP`. Still open from A-20: ADR-0001 and
  ADR-0002 retain superseded present-tense claims, and there is still no
  root README defining supported commands, token handling, the VNP trust
  boundary, or source-of-truth order.



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

### `read` treats an explicitly-empty `IFS=` as "use the default" (mash, POSIX)

Found 2026-07-25 while live-verifying raw input delivery (spec 005). It is a
pre-existing shell bug, not part of that feature — recorded here so it is not
mistaken for one next time it shows up in an input test.

`crates/mash/src/executor.rs:3648` reads:

```rust
let ifs = env.get_str("IFS");
let ifs = if ifs.is_empty() { " \t\n" } else { ifs };
```

This conflates two distinct states POSIX keeps separate: **IFS unset** (use
the default `<space><tab><newline>`) and **IFS set to the empty string** (no
field splitting and no trimming at all). Because the empty case falls into
the default branch, `IFS= read -r Z` still trims, so sending `"  padded  "`
yields `padded`.

Evidence: live against a real daemon, `IFS= read -r Z; echo "[$Z]"` with
`"  padded  \n"` printed `[padded]`, expected `[  padded  ]`.

Note this is **not** a delivery-path defect — the bytes arrive intact, which
the `SessionInputChannel` unit tests prove independently at the channel level.
The corruption happens afterwards, inside `read`.

Fix requires distinguishing unset from empty (`env.get` returning
`Option<&str>` rather than `get_str`'s flattened `&str`), then skipping both
the `trim_matches` at :3651 and the split at :3653 when IFS is set-but-empty.
Both call sites at :3691 and :3700 in `split_on_ifs` need the same treatment.
Guard with a Smoosh run — field splitting is heavily covered there.

