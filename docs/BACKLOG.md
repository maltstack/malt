# MALT Backlog

Living document. Update it as work lands or priorities change — this is the
"what's next and why" record, distinct from `docs/design/architecture.md` (target
design) and `docs/adr/` (decisions already made and their reasoning).

Each item should be concrete enough to act on without re-deriving context.
Link to a findings doc or ADR for the "how do we know this" evidence instead
of restating it here.

For larger, multi-week feature phases (plugin system completion, full
cross-platform isolation enforcement, etc.), see AGENTS.md's "Implementation
Roadmap" (Phase B2 through Phase I) — this backlog tracks concrete,
near-term, evidence-based items, not the whole product roadmap.

## P0 — blocks daily-driver usability

- **Terminal grid rendering "staircase" bug — RESOLVED 2026-07-24.**
  `crates/malt-compat/src/translator.rs:36` did
  `self.last_data.extend_from_slice(data)` on every `feed()` call instead of
  replacing it. Fixed to `self.last_data = data.to_vec()`, matching the
  original design doc and implementation plan. Added a regression test
  (`feed_replaces_previous_data_instead_of_accumulating`) exercising
  multiple `feed()` calls, which the prior test suite never did. Full
  `malt-compat` suite green.
  Follow-on gap noted, not fixed here (would be scope creep on a one-line
  bug fix): `RegisterVnpClient` (`crates/malt-daemon/src/executor/session_thread.rs:334`)
  now hands a newly-attaching client only the most recent chunk via
  `frame_element()`, not the full current-screen state. Per
  `docs/design/architecture.md`'s attach-sync section, a new client should
  receive "the current FrameElement tree (current visible state)" — for a
  `VtPassthrough` pane that means enough VT bytes to reconstruct the
  visible screen, which the compat translator doesn't currently retain
  anywhere (the grid itself has it, but nothing serializes grid state back
  to VT bytes for replay). Only matters once a client can attach mid-session
  to a pane with existing output; today's single-client-per-session flow
  doesn't exercise it. Same underlying gap as the Phase B3 scrollback item
  below — likely worth solving together.
  See: `docs/findings/2026-07-24-live-daemon-session.md` (original report),
  `docs/findings/2026-07-24-plan-implementation-audit.md#1-compattranslatorfeed-accumulates-instead-of-replacing--strong-lead-for-the-p0-rendering-bug`

## P1 — real gaps, worth fixing soon

- **`command_id`/`exit_code` in the Gateway's `exec` response are literal
  hardcoded values, and the command-history data model that should back
  them is completely unwired.** `gateway_backend.rs:111` hardcodes
  `command_id: 0` and `exit_code: None` — not a missed read of data the
  daemon already has. Separately, `malt-session::pane::CommandBlock` (a
  fully-built per-command record with exactly the fields needed:
  `command_id`, `started_at`, `finished_at`, `exit_code`) is never
  constructed or pushed anywhere outside tests — `grep` for `CommandBlock
  {` and `push_command_block` outside `tests/` returns zero hits. Sessions
  track no command history today despite having the model built for it.
  Real work on both ends: capture the exit code in the daemon's
  `RunCommand` path, and wire `CommandBlock` construction into it. See
  `docs/adr/ADR-0002-gateway-canonical-mcp-adapter.md` for the full
  context — this is Phase 3/4 of that decision.
- **`send_input`'s actual forwarding behavior is unconfirmed and suspected
  wrong.** Suspected to dispatch another `RunCommand` rather than writing
  raw bytes to an already-running process's stdin — would explain why
  simple standalone commands work in testing while genuinely interactive
  cases (REPLs, password prompts) might not. Not yet verified against the
  code. Confirm before Phase 3 work in ADR-0002 begins.
- **Agent-facing `get_output` returns the wrong representation — the human
  rendering pipeline, not something built for a program to parse.** It
  returns `StyledGrid` (character cells with RGB/bold flags, the same
  representation built for `malt-tui`/`maltty`), not plain text or
  structured stdout/stderr/status. Confirmed firsthand 2026-07-24: driving
  a session as an agent required writing a throwaway script to flatten
  grid cells back into readable text. `exec`'s response shape (plain
  `{"output": "...", "exit_code": ...}`) already gets this right for the
  synchronous case — `get_output` (used for monitoring ongoing/backgrounded
  sessions) doesn't. Fix: add a plain-text/structured variant of
  `get_output` alongside the existing grid one, not a replacement — the
  grid representation is correct and needed for the human clients.
  Distinct from the P0 rendering bug above: that's the renderer being
  *wrong*; this is the API being the *wrong shape for its audience* even
  once the renderer is correct. Also recording a considered non-decision
  here so it doesn't get re-litigated later: Agent Client Protocol (ACP)
  was evaluated as a possible fix for this and doesn't apply — ACP's
  client is the editor/terminal hosting an embedded agent's own UI
  (control flows editor→agent), whereas malt's actual relationship is an
  external agent remotely driving a persistent session (control flows
  agent→malt). Different actor model; adopting ACP wouldn't address this
  gap.
- **Compat-pane session restore is a confirmed stub.**
  `coordinator.rs:547-551` explicitly returns
  `DaemonError::RestoreFailed(id, "compat pane restore not yet
  implemented")` for any Dormant session with a Compat pane; the design's
  `spawn_compat` function was never written (zero grep hits). Shell-session
  restore, by contrast, is real and tested (26 tests). Phase B2 is further
  along than "in design" suggested — just not for Compat panes.
- **Backgrounded commands (`&`) don't appear to survive through the daemon's
  `/exec` route — confirmed to need new design, not a regression fix.** The
  process-supervisor's original design never covered detached background
  jobs surviving past their pane's lifecycle at all; it's scoped to
  interactive foreground processes tied to a PTY. `ping -n 30 127.0.0.1 &`
  returns empty output and no process survives afterward. This needs
  original design work, not "find where this broke."
  See: `docs/findings/2026-07-24-live-daemon-session.md#2-backgrounded-commands--dont-appear-to-survive-through-exec--medium-priority-not-root-caused`
- **`crates/malt-protocol/src/codec.rs` wrong constants — FIXED 2026-07-24.**
  Cross-checked every `MSG_*` constant against its real `@type(N)` in
  `schemas/*.vexil`: only Handshake, Render, and part of Input/Session were
  correct. Shell, Mux, Task, and System were substantially fabricated —
  `MSG_PING`/`MSG_PONG`/`MSG_SHUTDOWN`/`MSG_PASTE`/`MSG_TASK_FAIL`/etc. don't
  correspond to any message in any schema file at all. Confirmed via grep
  that only the already-correct constants (Hello/HelloAck, AttachSession,
  DetachSession, KeyEvent, Resize, FrameAck, RenderBatch, InitialState) are
  used by the live wire path (`malt-tui::connection`,
  `malt-daemon::vnp_listener`) — those needed zero call-site changes.
  Rewrote `codec.rs` with one constant per real schema message (renamed
  where the old name didn't match: `MSG_ERROR`→`MSG_VERSION_SKEW` for
  Handshake domain 0, `MSG_COMMAND_OUTPUT`→`MSG_OUTPUT_CHUNK`, etc.),
  dropped fabricated ones, and rewrote the test file to check every
  constant against its schema message name plus a dedicated
  `live_wire_path_constants_are_unchanged` regression test. All
  `malt-protocol`/`malt-tui`/`malt-daemon` tests green.
- **Two dead-code architecture detours, either finish or remove:**
  `crates/mash/src/builtins.rs`'s `Builtin` trait/`BuiltinRegistry` is
  never referenced anywhere (all builtins actually run through an inline
  match in `executor.rs`); `crates/malt-compat/src/parser.rs`'s second VT
  parser API (`VtEvent`/`CsiParams`/`EventCollector`, ~150 lines) is `pub`
  but unused and not re-exported, likely orphaned scaffolding for the
  never-wired Phase H out-of-process compat worker.
- **`malt-renderer`: two designed safety mechanisms aren't actually wired
  in.** `WalkConfig.max_output_bytes` (1 MiB cap) is defaulted but never
  read in `walker.rs` despite the design calling it "Enforced".
  `ClientState::should_shed()` (10s-no-ack disconnect) exists and is
  unit-tested, but `RendererHost::process_frame` never calls it.
- **`exec` responses always show `"exit_code": null`.** Observed on every
  call this session, including successful ones. Might be intentional
  (async result delivered elsewhere) — confirm before treating as a bug.
- **Test-coverage gaps on functionally-complete code.** `malt-session`'s
  `GroupManager` (create_group, add_session with max_sessions enforcement,
  on_oom) has zero tests anywhere in the crate. `malt-platform`'s `env.rs`
  has zero tests. Neither is a confirmed bug, but given that two real bugs
  in `job_objects.rs` were found specifically by writing real tests for
  under-tested code, treat these as real risk, not tidiness.

## P2 — deliberately deferred, not forgotten

- **PTY/compat supervisor spawn path has no isolation wiring.** Only
  `mash`'s own external-command spawn path (`malt_platform::process::spawn`)
  got wired to session isolation tiers today; `ProcessSupervisor::spawn` →
  `spawn_with_pty` (daemon-managed compat/legacy processes) did not.
  Deliberately scoped out to keep that day's change small and provable —
  see ADR-0001.
- **`malt-elevate` has 9 of 10 privileged operations stubbed.** Only
  `CreateSymlink` is real; `CreateNamespace`, `MountOverlay`, `SetCgroup`,
  `SetupNetns`, `ApplySeccomp`, `CreateRestrictedToken`,
  `ManageHcsContainer`, `ApplySeatbelt`, `BindPort` all return stub success
  without performing the operation.
- **HCS's real native path is unverified.** `hcs.rs` was ported from carboy
  with real Win32 bindings, but only fake-mode has actually been tested —
  the real `HcsCreateComputeSystem` path requires Windows Containers
  actually installed, which this dev machine doesn't have. Flagged
  explicitly rather than assumed working; see ADR-0001.
- **AppContainer and CRIU were not ported from carboy-isolation, on
  purpose.** Both were found to be stubs/placeholders in carboy itself
  (AppContainer's restricted-token function is a documented fallback with
  zero real isolation; CRIU is a 5-line `Err(Unsupported)` placeholder).
  Porting them would have added a false sense of security. If real
  implementations are ever wanted, they should be built as original work,
  not adopted because scaffolding already exists somewhere.
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

## Done (for context — remove once stale)

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
  call sites), with a real end-to-end test.
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
  all "agent-aware terminal" products now). Concrete finding folded into
  P1 above: `get_output` serves agents the human-rendering representation
  instead of a plain/structured one. ACP was evaluated and doesn't apply
  (wrong actor model — see the P1 entry). MCP's 2026-07-28 spec revision
  (stateless core) is a compatibility check `malt-mcp` will need soon, not
  yet added as a backlog item pending confirmation of what actually
  breaks.
- 2026-07-24: ADR-0002 — Gateway is the canonical agent control plane,
  MCP is a replaceable adapter, phased migration plan. Investigated where
  execution events should actually live: not `Bus`/`RingBuffer` (drops
  messages under pressure, destructive `drain()`, no sequence numbers —
  wrong for a durable log), but `malt-session::pane::CommandBlock`
  extended (right shape, currently unwired — see P1). Found two concrete
  bugs/gaps in the process: hardcoded `command_id`/`exit_code` in the
  Gateway, and `CommandBlock` never actually constructed anywhere outside
  tests.
