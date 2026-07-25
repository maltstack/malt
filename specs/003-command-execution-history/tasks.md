# Tasks: Persistent Command Execution History

**Input**: Design documents from `/specs/003-command-execution-history/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/command-history.md, quickstart.md

**Tests**: Included. This repo's engineering standards (AGENTS.md, Constitution IV) require tests that exercise real paths — persist to disk, restore, retrieve — for every change, and all workspace tests green before any task is marked complete.

**Organization**: Tasks are grouped by user story. US1 (view history) is the MVP; US2 (restart survival) and US3 (automation surface) build on it independently of each other.

**Baseline**: Branch `worktree-spec-kit-work` at `6ac4391`. Feature 002 (responsive session control) is being implemented concurrently on main — see research R6 and task T030.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup (Baseline Verification)

**Purpose**: Session Ritual compliance — verify the baseline is actually green before building on it.

- [X] T001 Run `cargo build --workspace && cargo test --workspace` from the worktree root and confirm all green; confirm `vexilc` is on PATH (`vexilc --version`, expect ≥ 0.5.1). If anything fails, stop and fix before proceeding.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Extend `PaneRuntime`'s API surface — both US1 (finalize) and US2 (restore seeding) depend on it.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T002 Add `PaneRuntime::finalize_current_block(finished_at: u64, exit_code: i32)` to `crates/malt-session/src/pane.rs` — sets both fields on the newest block only if it is still open (`finished_at.is_none()`); document and enforce the "at most one open block, always the newest" invariant (data-model.md state transitions).
- [X] T003 Add a restore-seeding constructor path to `PaneRuntime` in `crates/malt-session/src/pane.rs` (e.g. `with_blocks(id, kind, cwd, max_blocks, blocks: Vec<CommandBlock>)`) that pre-populates the ring buffer, truncating oldest-first if given more than `max_blocks`.
- [X] T004 Add unit tests in `crates/malt-session/src/pane.rs` (`#[cfg(test)]`): finalize sets fields on the open block; finalize on an already-completed block is a no-op; seeding preserves order and respects the cap; eviction after seeding evicts the oldest seeded entry first.

**Checkpoint**: `cargo test -p malt-session` green. Commit (Constitution X).

---

## Phase 3: User Story 1 — Review recent commands in a pane (Priority: P1) 🎯 MVP

**Goal**: Every execution in a session produces a `CommandBlock`; users retrieve chronological history via `malt history <ID>` and `GET /sessions/{id}/history`.

**Independent Test**: quickstart.md Scenario 1 — exec a succeeding and a failing command, `malt history 1` shows both with correct text, timestamps, and exit codes; a fresh session shows empty history; unknown session returns not-found.

### Implementation for User Story 1

- [X] T005 [US1] Add `pane_runtime: PaneRuntime` field to `SessionExecutor` in `crates/malt-daemon/src/executor/session_thread.rs`, constructed for `first_pane` in both construction sites (`spawn` ~line 265 and `spawn_with_cwd` ~line 336; the latter gets restore seeding later in T017 — plain-empty here).
- [X] T006 [US1] Record construction in `run_mash_command` (`crates/malt-daemon/src/executor/session_thread.rs:562`): push an open `CommandBlock { command_id, cmd: input.to_string(), started_at: now_ms, finished_at: None, exit_code: None }` immediately after id assignment; finalize via `finalize_current_block` on every return path — parse error (exit 1), empty input (exit 0), and normal completion (real `exit_code`). Reuse the existing epoch-ms pattern from line ~523.
- [X] T007 [US1] Add `SessionCommand::GetCommandHistory { reply: mpsc::Sender<Vec<CommandBlock>> }` variant + Debug impl arm + session-loop handler (clones `pane_runtime.command_blocks()` into a `Vec`) in `crates/malt-daemon/src/executor/session_thread.rs`.
- [X] T008 [US1] Add `Coordinator::get_command_history(&self, session_id: &SessionId) -> Option<Vec<CommandBlock>>` passthrough in `crates/malt-daemon/src/executor/coordinator.rs`, following the existing `get_session_output_text` pattern (None for unknown session).
- [X] T009 [P] [US1] Add `CommandHistoryEntry` response struct (serde, fields per data-model.md: `command_id`, `cmd`, `started_at`, `finished_at`, `exit_code`, `pane_id`) and `fn get_command_history(&self, session_id: u32) -> Result<Vec<CommandHistoryEntry>, GatewayError>` to the `GatewayBackend` trait in `crates/malt-gateway/src/backend.rs`; update the mock backend in `crates/malt-gateway/tests/routes.rs` to implement it.
- [X] T010 [US1] Implement `get_command_history` on `DaemonBackend` in `crates/malt-daemon/src/gateway_backend.rs`: map `None` → `GatewayError::NotFound`, populate `pane_id` from `session_first_pane()` (depends on T008, T009).
- [X] T011 [US1] Add `GET /sessions/{id}/history` route handler in `crates/malt-gateway/src/routes/sessions.rs` (existing `{"ok": true, "data": [...]}` envelope), register it in `build_router` in `crates/malt-gateway/src/server.rs`, and add `(GET, "/sessions/{id}/history")` → `AuthScope::Read` to `required_scope` in `crates/malt-gateway/src/middleware.rs`.
- [X] T012 [US1] Add `malt history <SESSION_ID>` subcommand: `Command::History` variant in `crates/malt-bin/src/cli.rs`, `get_command_history()` client method in `crates/malt-bin/src/client.rs` (authed, same pattern as `get_output_text`), handler in `crates/malt-bin/src/main.rs` printing per contracts/command-history.md (id, start time, duration or `running`/`incomplete`, exit code, command text).
- [X] T013 [P] [US1] Route tests in `crates/malt-gateway/tests/routes.rs`: history route returns mock entries with correct JSON shape; unknown session → 404 (uses the T009 mock).
- [X] T014 [US1] Daemon integration tests in `crates/malt-daemon/tests/gateway_backend.rs`: (a) two execs (one success, one `false`) → history has both entries, chronological, correct text/exit codes, `started_at <= finished_at`; (b) fresh session → empty vec, not an error; (c) parse-error exec → entry present with exit code 1; (d) unknown session id → `GatewayError::NotFound`.
- [X] T015 [US1] Session-thread test in `crates/malt-daemon/tests/session_thread.rs`: `GetCommandHistory` reply reflects pushes/finalizes — after an exec completes, the entry is finalized (`finished_at`/`exit_code` are `Some`).

**Checkpoint**: `cargo build --workspace && cargo test --workspace` green; quickstart Scenario 1 passes manually. Commit. **US1 is a shippable MVP here.**

---

## Phase 4: User Story 2 — History survives a daemon restart (Priority: P2)

**Goal**: History rides through the existing persist→dormant→restore cycle; `next_command_id` resumes monotonically.

**Independent Test**: quickstart.md Scenario 2 — exec, `malt stop`, `malt start`, `malt history 1` shows identical pre-restart entries; a post-restart exec gets a strictly greater `command_id`.

### Implementation for User Story 2

- [X] T016 [US2] Extend `schemas/persist/session.vexil`: add `PersistedCommandBlock` message (fields per data-model.md, stacked single-line `@doc` annotations) and `command_blocks @3 : array<PersistedCommandBlock>` on `PersistedPane`; recompile via `vexilc` and fix any generated-code fallout (existing construction sites in `crates/malt-daemon/tests/store.rs` and `crates/malt-daemon/src/executor/session_thread.rs` need the new field, `command_blocks: vec![]` where empty).
- [X] T017 [US2] Conversion + persist wiring in `crates/malt-daemon/src/executor/session_thread.rs`: `to_persisted_command_block`/`from_persisted_command_block` beside the env-snapshot converters; `build_persisted_session` (~line 896) takes the executor's `pane_runtime` blocks and writes `command_blocks` (signature gains the blocks or the `PaneRuntime` reference — follow the `env` parameter pattern); `Snapshot` handler passes them through.
- [X] T018 [US2] Restore wiring in `crates/malt-daemon/src/executor/session_thread.rs` (`spawn_with_cwd`) and its caller in `crates/malt-daemon/src/executor/coordinator.rs` (`restore_session` Shell branch): convert persisted blocks, seed `PaneRuntime` via `with_blocks` (T003), and set `next_command_id = max(command_id)` over restored blocks (0 if none) per research R4.
- [X] T019 [P] [US2] Store round-trip test in `crates/malt-daemon/tests/store.rs`: `PersistedSession` with populated `command_blocks` (including one open/interrupted block with `finished_at: None, exit_code: None`) survives save→load byte-equal; a legacy-shaped session with empty `command_blocks` decodes fine.
- [X] T020 [US2] End-to-end restore tests in `crates/malt-daemon/tests/gateway_backend.rs`, following the existing `shell_env_state_survives_dormant_restore_via_env_snapshot` pattern: (a) exec twice → force dormant→restore → history identical before/after (full-entry equality, SC-003); (b) post-restore exec gets `command_id` strictly greater than every persisted id; (c) cap consistency — seed near `max_blocks`, restore, exec more, assert cap holds with oldest evicted (spec edge case).
- [X] T021 [US2] Interrupted-command test in `crates/malt-daemon/tests/gateway_backend.rs` or `crates/malt-daemon/tests/store.rs`: persist a session whose newest block is open (daemon "stopped mid-command"), restore, retrieve — entry present with `finished_at: null`/`exit_code: null`, never shown as succeeded (FR-006).

**Checkpoint**: full workspace green; quickstart Scenario 2 passes manually. Commit.

---

## Phase 5: User Story 3 — Retrieve history through automation and AI-agent tooling (Priority: P3)

**Goal**: MCP tool parity and verified access-control refusals for programmatic clients.

**Independent Test**: quickstart.md Scenario 3 — raw `curl` with token returns the same entries as the CLI; no token → 401; unknown session → 404; MCP `get_command_history` returns the same `data` array.

### Implementation for User Story 3

- [X] T022 [US3] Add `get_command_history` MCP tool (7th tool) in `crates/malt-mcp/src/main.rs`: tools/list schema entry (`session_id` number, required), dispatch arm delegating to `GET /sessions/{id}/history` via the existing `authed()` helper, Gateway errors surfaced as tool errors — mirror the `get_output` tool's structure.
- [X] T023 [P] [US3] Auth-scope tests for the history route in `crates/malt-gateway/tests/routes.rs`: no token → 401; Monitor-scoped token → 403; Read-scoped token → 200 (FR-008, SC-005 — same pattern as the existing `insufficient_scope_is_forbidden` test).
- [X] T024 [P] [US3] MCP tool test in `crates/malt-mcp/src/main.rs` (`#[cfg(test)]`, matching how the existing 6 tools are covered): `get_command_history` appears in tools/list with correct schema; dispatch builds the right URL/auth header.

**Checkpoint**: full workspace green; quickstart Scenario 3 passes manually. Commit.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T025 [P] Update `docs/BACKLOG.md`: mark Gap A + Gap B (persistent execution history) FIXED with evidence (test names), and update the `command_id` restore-reset caveat (now resumes monotonically); note anything discovered during implementation.
- [X] T026 [P] Update `AGENTS.md`: add `malt history ID` to CLI Commands; bump MCP tool count 6 → 7 with `get_command_history`; update the "What's Implemented" daemon/MCP bullets.
- [X] T027 [P] Update `docs/design/architecture.md` — checked: line 1325 only lists "command block ring buffer" in the layer summary, still accurate for a target-state doc. No change needed.
- [X] T028 Run full quickstart.md validation manually (all three scenarios against a real daemon) and record the outcome in a dated `docs/findings/` entry if anything surprising surfaces.
- [X] T029 Final `cargo build --workspace && cargo test --workspace` green run; final commit.
- [X] T030 Rebase/merge check against feature 002 — **done 2026-07-25**, rebased onto `06c5cb2`. 002 moved MASH execution to a worker thread that solely owns `Env`, so recording moved to the same seam: the worker announces `SessionCommand::ExecutionStarted` before running (control actor pushes the open block) and `ExecutionCompleted` finalizes it. `command_id` assignment now lives in the worker, so restore-resume is seeded via a new `start_command_id` parameter on `spawn_command_worker` (002 had hardcoded `0`, which would have silently reintroduced post-restore id collisions). The Gateway read was restructured to 002's `begin_*` pattern so it no longer holds the coordinator lock while waiting — that would have stalled unrelated sessions. New test `a_running_command_is_visible_in_history_before_it_finishes` proves FR-004 is now observable mid-execution. Full workspace green. See research R6's Outcome section.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → blocks everything.
- **Phase 2 (Foundational)** → blocks all user stories (T002 blocks T006; T003 blocks T018).
- **Phase 3 (US1)** → blocks US2 and US3 in practice: US2 persists what US1 records (T017 needs T005/T006); US3 exposes the route US1 registers (T022 needs T011). US2 and US3 are independent of each other and can proceed in parallel after US1.
- **Phase 6 (Polish)** → after all desired stories.

### Within-Story Dependencies

- US1: T005 → T006 → T007 → T008 → T010 → T011 → T012; T009 [P] anytime after Phase 2; T013 after T009+T011; T014/T015 after T011.
- US2: T016 → T017 → T018; T019 [P] after T016; T020/T021 after T018.
- US3: T022 after T011; T023 [P] after T011; T024 [P] after T022.

### Parallel Opportunities

- T009 (gateway trait + mock) in parallel with T005–T008 (daemon internals) — different crates.
- T013 (route tests) in parallel with T012 (CLI) once T011 lands.
- After US1: all of US2 (T016–T021) in parallel with all of US3 (T022–T024) — disjoint files.
- Within Phase 6: T025/T026/T027 in parallel.

---

## Implementation Strategy

**MVP first**: Phases 1–3 deliver a complete, independently shippable increment — in-memory history with CLI + HTTP retrieval — closing Gap A. **Stop and validate** with quickstart Scenario 1 before continuing.

**Incremental delivery**: Phase 4 closes Gap B (the restart-survival half, quickstart Scenario 2). Phase 5 completes client parity (Scenario 3). Each phase ends with a full-workspace green run and a commit (Constitution X) — no multi-phase uncommitted piles.

**002 coordination**: implementation happens in this worktree against `6ac4391`; T030 is the explicit merge point. Do not pull 002's uncommitted changes in mid-flight.

---

## Notes

- All schema edits require `vexilc` recompilation; hand-constructed `PersistedPane`/`PersistedSession` values in existing tests will need the new field (the EnvSnapshot change hit exactly this — see AGENTS.md Type Notes).
- `CommandBlock`/`PaneId`/`SessionId` are NOT `Copy` — `.clone()` as needed.
- No `unwrap()`/`expect()` outside `#[cfg(test)]`; `thiserror` in library crates.
- Tests must exercise the real persist→restore→retrieve path (Constitution IV) — no construct-and-assert-shape tests.
