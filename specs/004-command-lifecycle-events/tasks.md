# Tasks: Command Lifecycle Event Delivery

**Input**: Design documents from `/specs/004-command-lifecycle-events/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/event-stream.md, quickstart.md

**Tests**: Included. AGENTS.md and Constitution IV require tests that exercise real paths — subscribe, execute, receive, disconnect, resume — never construct-and-assert-shape. Full workspace green before any task is marked complete.

**Organization**: Grouped by user story. US1 (live delivery) is the MVP; US2 (catch-up) and US3 (slow-consumer honesty) are independent of each other and can proceed in parallel after US1.

**Baseline**: Branch `004-command-lifecycle-events` on top of `d964be8` (features 002 + 003 landed).

**Critical sequencing note**: the bounded, non-blocking send path lands in Phase 2, *before* any subscriber exists. There must never be a commit in which a stalled subscriber can grow daemon memory without bound — not even transiently. US3 adds the *honesty* layer (gap signalling, explicit close) on top of a send path that is already bounded.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

**Purpose**: Verify the baseline and settle the one open dependency question.

- [X] T001 Run `cargo build --workspace && cargo test --workspace` from the worktree root and confirm all green. Note the known pre-existing `mash` shared-process-state flake (AGENTS.md) — if a `mash` executor test fails, re-run it in isolation to confirm it is that flake and not a regression.
- [X] T002 Add `tokio-stream = "0.1"` to `crates/malt-gateway/Cargo.toml`. It is not currently in `Cargo.lock`, so this is a genuinely new (tokio-team) dependency, needed for `ReceiverStream` to adapt a `tokio::sync::mpsc::Receiver` into the `Stream` that `axum::response::Sse` requires. Considered and rejected: hand-rolling a ~15-line `Stream` impl over `Receiver::poll_recv` to avoid the dependency — it works, but it is exactly the kind of cleverness a reviewer then has to verify, and this crate is not one of the dependency-free foundations Constitution III protects.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The event core — types, the bounded log, and the bounded non-blocking sink. All three user stories depend on it.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T003 Create `crates/malt-daemon/src/executor/events.rs` with `LifecycleEvent { sequence, kind }` and `LifecycleEventKind::{CommandStarted, CommandFinished, Gap}` plus `GapReason::{RetentionExceeded, SubscriberLagged}`, per data-model.md. Field names must mirror `schemas/shell.vexil`'s `CommandStarted`/`CommandFinished` so the deferred VNP path (research R1) carries the same information under the same names. Register the module in `crates/malt-daemon/src/executor/mod.rs`.
- [X] T004 Implement `EventLog` in `crates/malt-daemon/src/executor/events.rs`: `VecDeque<LifecycleEvent>` capped at `MAX_RETAINED_EVENTS = 1024`, `next_sequence: u64` starting at 1, `publish(kind) -> LifecycleEvent` (assigns sequence, appends, evicts oldest at cap), and `replay_after(seq) -> (Vec<LifecycleEvent>, bool)` where the bool reports whether `seq` predates the oldest retained entry.
- [X] T005 Implement `SubscriberSink` in `crates/malt-daemon/src/executor/events.rs`: `{ id, tx: tokio::sync::mpsc::Sender<LifecycleEvent>, last_sent: u64 }` with `SUBSCRIBER_BUFFER = 256`. Delivery uses `try_send` **only** — never `send().await`, never `blocking_send`. A `Full` or `Closed` result marks the sink for removal and never grows the channel. This is the bound that must exist before any subscriber can be created.
- [X] T006 [P] Unit tests in `crates/malt-daemon/src/executor/events.rs` (`#[cfg(test)]`): sequence starts at 1 and increases monotonically; the log evicts oldest at the cap; `replay_after` returns only later events and correctly reports the predates-retention case; `replay_after` on an empty log is empty, not an error.
- [X] T007 [P] Unit tests for `SubscriberSink` in the same file: `try_send` succeeds until the buffer is full then reports `Full` without growing; a sink whose receiver was dropped reports `Closed`; `last_sent` advances only on success.

**Checkpoint**: `cargo test -p malt-daemon` green. Commit (Constitution X).

---

## Phase 3: User Story 1 — Live start and finish delivery (Priority: P1) 🎯 MVP

**Goal**: A client subscribes to a session over SSE and receives `command_started` while a command is still running, then `command_finished` with its real exit status.

**Independent Test**: quickstart.md Scenario 1 — subscribe, run `sleep 3; echo done`, observe the start frame ~3s before the finish frame, both sharing a `command_id`; then run `false` and confirm `exit_code: 1`.

### Implementation for User Story 1

- [X] T008 [US1] Add the session-side subscription state to `SessionExecutor` in `crates/malt-daemon/src/executor/session_thread.rs`: an `EventLog` and a `Vec<SubscriberSink>`, plus a `publish_lifecycle(kind)` helper that appends to the log, fans out to every sink with `try_send`, and removes sinks reporting `Full`/`Closed` (crude removal here; US3 makes it honest).
- [X] T009 [US1] Publish from the two existing handlers in `crates/malt-daemon/src/executor/session_thread.rs`: `SessionCommand::ExecutionStarted` emits `CommandStarted { command_id, cmd, started_at }`; the `ExecutionCompleted` commit point in `advance_finalization` (where history is finalized) emits `CommandFinished { command_id, exit_code, finished_at, duration_us }`, deriving `duration_us` from the matching open history block. Publishing at the commit point — not when the worker returns — is what guarantees a finish event never precedes visibility of the output that produced it (research R9).
- [X] T010 [US1] Add `SessionCommand::SubscribeEvents { reply: mpsc::Sender<tokio::sync::mpsc::Receiver<LifecycleEvent>>, resume_from: Option<u64> }` and `UnsubscribeEvents { id: u64 }` variants plus Debug arms and loop handlers in `crates/malt-daemon/src/executor/session_thread.rs`. The handler creates the bounded channel, registers a sink, and returns the receiver. (`resume_from` is accepted now but only honored in US2 — wiring the parameter here avoids reshaping the command later.)
- [X] T011 [US1] Add `Coordinator::begin_subscribe_events(session_id, resume_from) -> Result<tokio::sync::mpsc::Receiver<LifecycleEvent>, DaemonError>` in `crates/malt-daemon/src/executor/coordinator.rs`, following the `begin_get_session_command_history` pattern: return without waiting under the coordinator lock. Active → send the command; Dormant → `DaemonError::SessionDormant` (a dormant session cannot produce events); missing → `SessionNotFound`.
- [X] T012 [P] [US1] Add `LifecycleEventDto` (serde, per contracts/event-stream.md) and `fn subscribe_events(&self, session_id: u32, resume_from: Option<u64>) -> Result<tokio::sync::mpsc::Receiver<LifecycleEventDto>, GatewayError>` to the `GatewayBackend` trait in `crates/malt-gateway/src/backend.rs`; implement it in the mock backend in `crates/malt-gateway/tests/routes.rs`.
- [X] T013 [US1] Implement `subscribe_events` on `DaemonBackend` in `crates/malt-daemon/src/gateway_backend.rs`: resolve the session first (existence check → 404 before any stream opens), call the coordinator, map errors via `map_execution_error` so dormant is 409, and convert `LifecycleEvent` → `LifecycleEventDto`.
- [X] T014 [US1] Add the SSE route in `crates/malt-gateway/src/routes/sessions.rs`: `events()` handler returning `Sse<impl Stream<Item = Result<Event, Infallible>>>` built from `ReceiverStream`, setting SSE `id` from `sequence`, `event` from the kind (`command_started`/`command_finished`/`gap`), and `data` from the JSON payload. Attach a keep-alive so idle connections are not reaped by intermediaries. Register `GET /sessions/{id}/events` in `crates/malt-gateway/src/server.rs` (inside `build_router`, so `with_auth` covers it) and add `(GET, "/sessions/{id}/events")` → `AuthScope::Read` to `required_scope` in `crates/malt-gateway/src/middleware.rs`.
- [X] T015 [US1] Integration test in `crates/malt-daemon/tests/events.rs` (new file): subscribe via the backend, exec a command, and assert exactly one `CommandStarted` then one `CommandFinished` arrive with the same `command_id`, ascending `sequence`, correct `cmd` text, and a real exit code for both a succeeding and a failing command.
- [X] T016 [US1] Per-session isolation test in `crates/malt-daemon/tests/events.rs`: two sessions, a subscriber on one, commands run in both — assert the subscriber receives only its own session's events (FR-004).
- [X] T017 [P] [US1] Route tests in `crates/malt-gateway/tests/routes.rs`: the events route returns `200` with `content-type: text/event-stream`; an unknown session returns `404` **before** any stream is opened; a dormant session returns `409`.
- [X] T018 [US1] Create `crates/malt-bin/src/events.rs`: an SSE frame parser over a `BufReader` line loop (`reqwest`'s blocking `Response` implements `std::io::Read`, so no new dependency), producing typed `{ sequence, kind, payload }` values, plus a `stream_events(client, session_id, from)` entry point. **Keep this module free of CLI-specific assumptions** — no printing, no `clap` types, no `process::exit` — because it is the nucleus `malt-gateway-sdk` later extracts (research R10); that extraction should be a move, not a rewrite.
- [X] T019 [US1] Add the streaming request method to `crates/malt-bin/src/client.rs` (authed, no `.json()` — return the raw `Response` so the caller reads it incrementally), and the `malt watch <SESSION_ID> [--resume-from <SEQUENCE>]` subcommand: `Command::Watch` in `crates/malt-bin/src/cli.rs`, handler in `crates/malt-bin/src/main.rs` printing one line per event per contracts/event-stream.md. Non-zero exit with the Gateway's message on 401/403/404/409.
- [X] T020 [P] [US1] Frame-parser unit tests in `crates/malt-bin/src/events.rs` (`#[cfg(test)]`): a well-formed `id`/`event`/`data` frame parses; blank-line-separated frames split correctly; an unknown `event` type is skipped rather than aborting the stream (forward compatibility — a future event type must not break existing clients).

**Checkpoint**: `cargo build --workspace && cargo test --workspace` green; quickstart Scenario 1 passes manually against a live daemon. Commit. **US1 is a shippable MVP.**

---

## Phase 4: User Story 2 — Resume after disconnect (Priority: P2)

**Goal**: A reconnecting subscriber receives what it missed, or is explicitly told there is a gap.

**Independent Test**: quickstart.md Scenario 2 — note an `id`, disconnect, run commands, resume with `Last-Event-ID` and see the missed frames replay in order; then resume from a position older than the retained window and see a `gap` frame arrive first.

### Implementation for User Story 2

- [X] T021 [US2] Honor `resume_from` in the `SubscribeEvents` handler in `crates/malt-daemon/src/executor/session_thread.rs`: call `EventLog::replay_after`, and if the requested position predates retention, push a synthesized `Gap { missed_from, missed_through, reason: RetentionExceeded }` into the new sink **before** the replayed events, then the replay, then live events. Ordering is the requirement, not an implementation detail — a client must learn about a hole before receiving data that would otherwise look contiguous (FR-008).
- [X] T022 [US2] Parse the `Last-Event-ID` header in the SSE handler in `crates/malt-gateway/src/routes/sessions.rs` and pass it through as `resume_from`. A malformed or absent header means "start from now", not an error — a first-time subscriber must not be required to process the session's past (FR-007 scenario 3).
- [X] T023 [US2] Replay test in `crates/malt-daemon/tests/events.rs`: subscribe, run a command, record the last sequence, drop the subscription, run two more commands, resubscribe with `resume_from` — assert the four missed events replay in order before any live event, with no duplicates of the already-seen one.
- [X] T024 [US2] Retention-overrun test in `crates/malt-daemon/tests/events.rs`: use a small `EventLog` cap (inject a test-only capacity rather than generating 1024 real commands), overrun it, then resume from position 1 — assert a `Gap` with `reason: RetentionExceeded` arrives **first**, naming a range consistent with what was actually evicted, followed by the retained events.
- [X] T025 [US2] Implement the reconnect loop in `crates/malt-bin/src/events.rs`: on transport failure, reconnect automatically sending `Last-Event-ID` with the highest sequence seen so far, with a bounded backoff. This is the behavior that would otherwise ship unexercised (research R10) — it is the reason a first-party client is in scope at all.
- [X] T026 [P] [US2] Fresh-subscription test in `crates/malt-daemon/tests/events.rs`: subscribing with no `resume_from` to a session that has already run commands yields no replay — only events from that moment forward.

**Checkpoint**: full workspace green; quickstart Scenario 2 passes manually. Commit.

---

## Phase 5: User Story 3 — A stalled subscriber cannot degrade the session (Priority: P2)

**Goal**: A subscriber that stops reading is told it fell behind and closed, while execution speed, other subscribers, and daemon memory are unaffected.

**Independent Test**: quickstart.md Scenario 3 — a `SIGSTOP`ped subscriber alongside a healthy one; the healthy one receives everything, execution timing matches a no-subscriber baseline, and the stalled one sees a terminal `gap` with `reason: subscriber_lagged`.

### Implementation for User Story 3

- [X] T027 [US3] Implement the lag policy in `crates/malt-daemon/src/executor/events.rs` and the fan-out in `crates/malt-daemon/src/executor/session_thread.rs`: on `try_send` returning `Full`, mark the sink lagged, make a best-effort delivery of a terminal `Gap { missed_from: last_sent + 1, missed_through: current_sequence, reason: SubscriberLagged }`, then drop the sink so the receiver closes and the SSE stream ends. Replaces US1's crude silent removal (T008).
- [X] T028 [US3] Ensure sinks whose receiver has closed (unclean client disconnect) are removed on the next publish in `crates/malt-daemon/src/executor/session_thread.rs`, and that `UnsubscribeEvents` removes cleanly — no sink may outlive its client (FR-011).
- [X] T029 [US3] Lag test in `crates/malt-daemon/tests/events.rs`: create a subscriber and never read it, publish more than `SUBSCRIBER_BUFFER` events, then read — assert the final event received is a `Gap` with `reason: SubscriberLagged` and a range matching what was skipped, and that the channel is then closed. Assert the sink is gone from the session (subscriber count returns to zero).
- [X] T030 [US3] Non-interference test in `crates/malt-daemon/tests/gateway_backend.rs`: with one fully stalled subscriber attached, assert a healthy second subscriber receives every event, and that command execution wall-clock time is within tolerance of a no-subscriber baseline measured in the same test (the SC-006 assertion — measure both, do not assert an absolute number).
- [X] T031 [US3] Surface `gap` events prominently in `malt watch` (`crates/malt-bin/src/main.rs`): a gap means this client's view is incomplete, so it must be visually distinct from ordinary frames and name the missed range and reason — not printed as just another line the user scrolls past.
- [X] T032 [P] [US3] Bounded-memory test in `crates/malt-daemon/tests/events.rs`: publish far more events than the buffer to a stalled sink and assert the sink's queued count never exceeds `SUBSCRIBER_BUFFER` — the property that makes the Bus unusable here (research R2) and must be proven, not assumed.

**Checkpoint**: full workspace green; quickstart Scenario 3 passes manually. Commit.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T033 Update `docs/BACKLOG.md`: mark ADR-0003 priority 3 (command lifecycle events) delivered with evidence (test names). **Critically, record that the Bus still has zero consumers after this feature** — the backlog framed this work as "giving the Bus its first real consumer" and that is no longer what happened (research R2, Constitution IX). Add the follow-up it exposes: a bus whose `Reliable` tier grows without bound has no safe consumer, so it needs either a bounded `Reliable` policy with gap signalling or an honest re-scoping to in-daemon trusted consumers.
- [ ] T034 [P] Add the two deferred follow-ups to `docs/BACKLOG.md`: VNP lifecycle-event forwarding to `malt-tui` (research R1) and an MCP-native subscription pending a real notifications design (research R8). Both are deliberate deferrals from this feature, not oversights.
- [ ] T035 [P] Update `AGENTS.md`: add `GET /sessions/{id}/events` to the Gateway description and a "What's Implemented" bullet for lifecycle event delivery, noting the SSE transport and the per-session bounded event log.
- [ ] T036 Run the full quickstart.md validation manually against a live daemon (all four scenarios, including the `SIGSTOP` slow-consumer case) and record the outcome in a dated `docs/findings/` entry — this feature's riskiest behavior is only observable against a real client, exactly like the 003 in-flight-history validation.
- [ ] T037 Final `cargo build --workspace && cargo test --workspace` green run; final commit.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → blocks everything.
- **Phase 2 (Foundational)** → blocks all user stories. T003 → T004/T005; T006/T007 after their subjects.
- **Phase 3 (US1)** → blocks US2 and US3 in practice: US2 extends the subscribe handler US1 creates (T018 needs T010), and US3 replaces the fan-out US1 writes (T023 needs T008).
- **Phase 4 (US2)** and **Phase 5 (US3)** are independent of each other — disjoint concerns, and after US1 they touch mostly different functions. Either may ship first.
- **Phase 6 (Polish)** → after all desired stories.

### Within-Story Dependencies

- US1: T008 → T009 → T010 → T011 → T013 → T014; T012 [P] any time after Phase 2. Tests T015/T016 after T013, T017 after T012 + T014. Client: T018 → T019, T020 [P] after T018 — the client needs the route (T014) live to be exercised end to end.
- US2: T021 → T022 (server replay before the client reconnect loop that drives it); T023–T025 after T021.
- US3: T026 → T027 → T028; T029/T030 after T026.

### Parallel Opportunities

- T006 and T007 (unit tests for different types) in parallel.
- T012 (gateway trait + mock, different crate) in parallel with T008–T011 (daemon internals).
- T017 (route tests) in parallel with T015/T016 (daemon tests) once prerequisites land.
- T018–T020 (`malt-bin` client) in parallel with T015–T017 (daemon and gateway tests) — different crates, and the client only needs the route to exist.
- After US1: all of US2 (T021–T025) in parallel with all of US3 (T026–T030).
- T033 and T034 (different doc files) in parallel.

---

## Implementation Strategy

**MVP first**: Phases 1–3 deliver a complete, shippable increment — live SSE delivery of start and finish events *plus a real client that consumes them* (`malt watch`), which is the entire P1 story and the reason the feature exists. **Stop and validate** with quickstart Scenario 1 against a real daemon before continuing. That the endpoint ships with a first-party consumer is deliberate (research R10): an event stream nobody has written a client for is an event stream whose awkward parts are still undiscovered.

**Then, honestly**: US1 ships with a *bounded but crude* slow-consumer behavior (a full sink is silently removed). That is safe — memory is bounded from Phase 2 onward, by design, so no commit ever contains the unbounded-growth failure. But it is not yet *honest*: a dropped subscriber is not told. US3 is what closes that, and it should land before any real agent depends on the stream. Do not treat US1's checkpoint as "done" in a way that lets US3 slip.

**Ordering flexibility**: US2 and US3 are genuinely independent. If a reconnecting agent is the more pressing need, take US2 first; if the stream is about to be exposed to an unattended client, take US3 first.

**On the SDK**: `malt-gateway-sdk` is deliberately *not* built here (research R10) — the contract does not change for it, so building it now buys no design safety. But `crates/malt-bin/src/events.rs` is written as its nucleus: CLI-free, self-contained stream handling. When the SDK is specced, that module should move out, not be rewritten. Keeping it clean is a real constraint on T018/T022, not a stylistic note.

---

## Notes

- The publish path must only ever `try_send`. Any `await` or blocking send on the control actor reintroduces the exact coupling features 002 and 003 removed — a client able to slow execution.
- `LifecycleEvent` carries `String` command text; sinks clone per subscriber. Acceptable at the expected scale (tens of subscribers); if that ever changes, `Arc<str>` is the escape hatch — noted, not pre-optimized.
- No `unwrap()`/`expect()` outside `#[cfg(test)]`; `thiserror` in library crates.
- Tests must drive the real path. A test that constructs a `LifecycleEvent` and asserts on its fields proves nothing about delivery — the exact failure mode AGENTS.md documents for `job_objects.rs`.
