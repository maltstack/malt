# Research: Persistent Command Execution History

**Feature**: 003-command-execution-history | **Date**: 2026-07-25

No `NEEDS CLARIFICATION` markers existed in the Technical Context — the codebase already answers every open question. This document records the decisions and the evidence behind them.

## R1: Where execution records come from

**Decision**: Construct `CommandBlock`s in `SessionExecutor::run_mash_command` (`crates/malt-daemon/src/executor/session_thread.rs:562`), the single choke point every execution path already flows through (Gateway `exec`, `WriteInput` line-buffered input, VNP `KeyInput` accept). Push a block with `finished_at: None, exit_code: None` *before* `execute_list`, then finalize it (set `finished_at` + `exit_code`) immediately after — including the parse-error and empty-input early returns, which already receive real `command_id`s.

**Rationale**: `run_mash_command` is where `next_command_id` is already assigned (one id per execution, all return paths), so record identity is free. Recording start-before/finalize-after is what makes FR-004 (visible while running) and FR-006 (daemon died mid-command → entry shows not-confirmed-complete) fall out of the data model naturally instead of needing special cases: a crash between push and finalize persists exactly the "started but never finished" state.

**Alternatives considered**: Recording only-on-completion (simpler, one push) — rejected because FR-006 would then require a separate "was running at shutdown" mechanism, and a mid-command crash would silently drop the entry.

## R2: Where the in-memory ring buffer lives

**Decision**: `SessionExecutor` gains a `pane_runtime: PaneRuntime` field (for its `first_pane`), reusing `malt-session::pane::PaneRuntime` exactly as built — `push_command_block` ring-buffer eviction at `DEFAULT_MAX_BLOCKS` (1000), `command_blocks()` accessor, `current_block()`.

**Rationale**: `PaneRuntime` was designed for precisely this and is unit-tested, but grep confirms zero non-test constructors in `malt-daemon` — `SessionRuntime` tracks only `Vec<PaneId>`, no runtime state per pane. Instantiating the existing L1 type in the L2 executor is the architecture-conformant closure of Gap A; a bespoke `VecDeque` in the daemon would duplicate a tested type and leave the real one dead. Finalize-after-execute mutates the back entry; `PaneRuntime` exposes `command_blocks()` (read) and `push_command_block` — a small `finalize_current_block(finished_at, exit_code)` (or equivalent mutable accessor) will be added to `PaneRuntime` itself so the invariant "only the newest entry can be open" lives next to the buffer.

**Alternatives considered**: (a) `VecDeque<CommandBlock>` directly on `SessionExecutor` — rejected, duplicates `PaneRuntime`. (b) Moving `PaneRuntime` ownership into `SessionRuntime` — rejected as a bigger refactor than the feature needs (Principle IX); the executor owns exactly one shell pane today and the per-pane schema keeps multi-pane correct later.

## R3: Persistence schema shape (Gap B)

**Decision**: Add to `schemas/persist/session.vexil`:

- a `PersistedCommandBlock` message mirroring `malt-session::pane::CommandBlock` field-for-field: `command_id: u32`, `cmd: string`, `started_at: u64` (epoch ms), `finished_at: optional<u64>`, `exit_code: optional<i32>`;
- a `command_blocks @3 : array<PersistedCommandBlock>` field on `PersistedPane` (not on the `Shell` union variant).

Recompile with `vexilc`; update `build_persisted_session` (`session_thread.rs:896`) to drain the executor's `PaneRuntime` into the field, and the restore path (`spawn_with_cwd`) to seed a fresh `PaneRuntime` from it.

**Rationale**: History is a property of a pane, not of the shell-flavored variant — `Compat` panes will eventually want the same record shape, and `PersistedPane` is where `cwd`/`title` already live. `array` (not `map`) preserves chronological order for free (FR-002). Epoch-ms `u64` matches the existing timestamp convention at `session_thread.rs:523`. An empty array is the natural zero value for pre-existing persisted sessions (no migration needed; `schema_version` stays 1 since the field is additive and decode of old files yields an empty vec — verified behavior of vexil optional/array decoding in the EnvSnapshot work, same pattern).

**Alternatives considered**: Separate history file per session — rejected: introduces a second persistence artifact with its own atomicity/quarantine story for no benefit at a 1,000-entry cap (bounded size, fine inline). Putting the field on `PersistedPaneType::Shell` — rejected per above.

## R4: Command-id monotonicity across restore

**Decision**: On restore, `next_command_id` resumes from `max(command_id)` over the restored blocks (0 if none), so post-restore executions continue the monotonic sequence instead of colliding with persisted ids.

**Rationale**: `docs/BACKLOG.md` explicitly flags that the counter "resets to 0 on restore … revisit once persistent execution history is wired, since that's the point restart-survival for this counter would actually matter." This feature is that point. Without it, restored history plus new executions would produce duplicate `command_id`s within one pane's history — violating record identity.

**Alternatives considered**: Persisting the counter separately in `PersistedSession` — rejected: derivable from the history itself; a second source of truth can only disagree.

## R5: Retrieval surface

**Decision**: Follow the established `get_output_text` pattern end-to-end:

1. `SessionCommand::GetCommandHistory { reply: mpsc::Sender<Vec<CommandBlock>> }` handled on the session thread (clone out of the ring buffer);
2. `Coordinator::get_command_history(session_id)` passthrough;
3. `GatewayBackend::get_command_history(session_id) -> Result<Vec<CommandHistoryEntry>, GatewayError>` trait method + `DaemonBackend` impl;
4. `GET /sessions/{id}/history` route, `AuthScope::Read` in `required_scope` (same scope as `/output` — history is session data of the same sensitivity class);
5. `malt history <ID>` CLI subcommand in `malt-bin`;
6. `get_command_history` MCP tool (7th tool) in `malt-mcp`, delegating to the Gateway route per ADR-0002.

Unknown session → existing `GatewayError::NotFound` → HTTP 404 (FR-009: distinguishable from `200` + empty list). Missing/insufficient token → existing middleware 401/403 (FR-008, no new auth code).

**Rationale**: This exact chain was built three times in the last work cycle (`get_output_text`, `stderr`, `command_id`) — it is the repo's sanctioned pattern, exercises existing auth uniformly, and satisfies FR-007's "equivalent data through each client" by construction since every client hits the same backend method. Scrollback/full-output-per-command is explicitly out of scope (spec assumption); the entry is metadata only.

**Alternatives considered**: Pane-scoped route (`/sessions/{id}/panes/{pane_id}/history`) — deferred: sessions have exactly one shell pane today and `list_panes` exposes the real pane id; the response includes `pane_id` so the route can grow a pane filter later without breaking the contract. Event-push (SSE/VNP lifecycle events) — separate BACKLOG item ("the Bus has zero consumers"), deliberately not folded in (Principle IX).

## R6: Interaction with in-flight feature 002 (responsive session control)

**Decision**: Plan 003 against the current committed baseline (`6ac4391`); design the record lifecycle so it is *indifferent* to 002's executor decoupling. Note the one behavioral consequence honestly: on this baseline the session thread is busy inside `execute_list` during a command, so a `GetCommandHistory` issued mid-execution waits until the command finishes (same as every other `SessionCommand` today). The "see a still-running command in history" acceptance scenario becomes *observable in practice* once 002's decoupled command workers land on main and this branch rebases — no design change needed here, because the open block is already pushed before execution starts.

**Rationale**: 002 is being implemented concurrently in the main worktree (uncommitted). Serializing 003's design on unlanded work would block for no reason; coupling to it would violate the worktree isolation this branch exists for. The pre-push design means correctness (FR-004's data model, FR-006's crash semantics) holds on both baselines — only retrieval latency during execution differs.

**Alternatives considered**: Waiting for 002 to land first — rejected: spec/plan/tasks are exactly the artifacts safe to produce in parallel; the merge point is implementation, and tasks.md will call out the rebase check.

**Outcome (2026-07-25, after 002 landed and this branch rebased onto it).** The prediction held, and the integration was cleaner than the pre-002 shape. 002 moved MASH execution to a dedicated worker thread that solely owns `Env`; the control actor keeps UI, persistence, and lifecycle state. That splits the record lifecycle across the same boundary, which turns out to be the right seam:

- The worker announces `SessionCommand::ExecutionStarted { command_id, command, started_at }` immediately before running. The control actor pushes the open block.
- The matching `ExecutionCompleted` finalizes it, committed alongside the env snapshot and sequence advance so history and shell state become visible together.
- Neither side reaches into the other: the worker owns MASH, the control actor owns the history buffer.

Two consequences worth recording. First, `command_id` assignment moved into the worker, so the restore-resume is seeded via a new `start_command_id` parameter on `spawn_command_worker` rather than a control-actor field — 002 had hardcoded it to `0`, which would have silently reintroduced id collisions after restore. Second, the "still-running command visible in history" acceptance scenario is now *actually observable*, not just structurally correct: `a_running_command_is_visible_in_history_before_it_finishes` asserts a live in-flight entry is returned in well under a second while a 1-second command runs, and that the same entry is finalized in place afterwards rather than duplicated.

The Gateway read was also restructured to match 002's `begin_*` pattern (return the receiver, drop the coordinator lock, then wait). The original implementation blocked up to 2 seconds *holding* the coordinator mutex, which under 002 would have stalled every other session's control traffic — a real defect introduced by the merge, not present in either branch alone.

## R7: Access control and auth scope

**Decision**: `GET /sessions/{id}/history` requires `AuthScope::Read`, added to the `required_scope` table in `crates/malt-gateway/src/middleware.rs`. Note the table fails closed (unrecognized route → `Admin`), so even forgetting the entry would deny, not leak — the entry makes Read-scoped clients work.

**Rationale**: History is the same sensitivity class as session output (`/output` is Read). FR-008's refusal requirement is satisfied by the existing middleware (401 missing/invalid token, 403 insufficient scope, both already tested in `malt-gateway/tests/routes.rs`).

**Alternatives considered**: `Monitor` scope — rejected: command text can contain sensitive material (paths, tokens typed into commands); `Monitor` is for liveness/inventory (`/health`, session list), not content.
