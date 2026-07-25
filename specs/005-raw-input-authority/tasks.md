# Tasks: Authenticated Raw Input with Input Authority

**Input**: Design documents from `/specs/005-raw-input-authority/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/input-and-authority.md, quickstart.md

**Tests**: Included. Constitution IV and this project's history require tests that drive real paths. Two specific traps apply here and are called out at their tasks: `AuthorityTracker` passes its own unit tests *today* while being unreachable from production, and a handshake test that calls an auth function directly proves nothing about what bytes went over the socket.

**Organization**: Grouped by user story. US1 (authenticated identity) is the foundation and must land first — shipping US2 before it would make password prompts injectable by any local process, which is worse than today's inability to answer them.

**Baseline**: Branch `worktree-spec-kit-work` at `0401889`, with all four gates green (build, test, fmt, clippy).

**Standing gate for this feature**: `mash` is modified by US2. Smoosh must stay 183/183 on native Windows. A regression there blocks the story; it is not a note to file.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Verify the baseline: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`. All four must pass before starting. Record the Smoosh baseline too (`cargo test -p mash --test smoosh_runner smoosh_conformance_tests`, expect 183 passed / 3 skipped) — US2 changes `mash`, and a before-number is worthless once the change is in.
- [ ] T002 Add an OS CSPRNG dependency (`getrandom`) to `crates/malt-gateway/Cargo.toml`, needed by T003. Confirm it is not already reachable transitively before adding.

---

## Phase 2: Foundational

**Purpose**: The credential must be worth checking before anything is built to check it.

**⚠️ CRITICAL**: T003 gates User Story 1. Authenticating with a guessable token is a lock whose key can be recomputed (research R1).

- [ ] T003 Replace `generate_random_token` in `crates/malt-gateway/src/auth.rs` with OS CSPRNG bytes. It currently derives both halves from epoch nanoseconds and a fixed multiplier, so a token is recomputable by anyone who can approximate daemon start time (audit A-03). Keep the `malt_` prefix and the length so existing readers are unaffected.
- [ ] T004 Fix token persistence in `crates/malt-gateway/src/auth.rs`: directory-creation and file-write errors are currently ignored, so the daemon can come up believing it persisted a token it did not. Write atomically (temp + rename), restrict to owner-only permissions, and fail daemon startup rather than continuing with a token nothing else can read.
- [ ] T005 Stop printing the admin token to stdout in `crates/malt-bin/src/daemon.rs`. Print the path it was written to instead, so first-run discovery still works without putting the secret in scrollback, CI logs, and terminal history.
- [ ] T006 [P] Tests in `crates/malt-gateway/tests/` (new `auth.rs`): two tokens generated in the same process differ; a token generated at a known instant is not reproducible from that instant (assert against the old epoch-derived construction, so the fix cannot silently regress); a write failure surfaces as an error rather than a silent success.

**Checkpoint**: gates green. Commit. **Merge to main** — this is independently valuable even before US1.

---

## Phase 3: User Story 1 — Only an identified client can reach a session (Priority: P1) 🎯 MVP

**Goal**: The VNP transport refuses unidentified clients, discloses nothing to them, and cannot be exhausted by them.

**Independent Test**: quickstart.md Scenario 1 — connect without a credential and confirm the connection closes with no session names in the returned bytes; connect and stall and confirm the daemon closes it promptly while still serving others; confirm a legitimate client is unaffected.

### Implementation for User Story 1

- [ ] T007 [US1] Extend the handshake schema in `schemas/handshake.vexil`: add a credential field to `Hello`, and a rejection message (or reuse a refusal shape) the daemon can send before `HelloAck`. Recompile via `vexilc` and fix generated-code fallout.
- [ ] T008 [US1] Authenticate in `crates/malt-daemon/src/connection/handshake.rs`: `perform_server_handshake` takes the `TokenStore` and validates the credential, returning the resolved `AuthScope` in `HandshakeResult`. Reject before any `HelloAck` is written.
- [ ] T009 [US1] **Invert the disclosure order** in `crates/malt-daemon/src/vnp_listener.rs`. Today `list_sessions()` is called and the result passed *into* the handshake, so the inventory is assembled before anything is checked and sent inside `HelloAck`. Collect it only after authentication succeeds. This ordering is the substance of FR-002 — refusing after sending the inventory would pass a careless test.
- [ ] T010 [US1] Bound pre-identification cost in `crates/malt-daemon/src/vnp_listener.rs` (audit A-08): set the read deadline *before* the blocking handshake read rather than after it, and cap concurrent un-authenticated connections so connect-and-stall cannot consume threads and sockets without limit.
- [ ] T011 [US1] Check session access on attach in `crates/malt-daemon/src/vnp_listener.rs`: validate the caller-supplied session id against what the authenticated identity may reach, instead of honoring it because it was supplied (FR-005). Distinguish "no such session" from "not permitted".
- [ ] T012 [US1] Present the credential from `crates/malt-tui/src/connection.rs`, reading it from the same well-known token file `malt-bin` and `malt-mcp` already use. `malt attach` must keep working with no user-visible change.
- [ ] T013 [US1] Socket-level tests in `crates/malt-daemon/tests/vnp_listener.rs`: connect a real socket with no credential and assert the connection closes **and that no session name appears in the bytes received** — create named sessions first so there is something to leak. Do not call the auth function directly; the bug class here is ordering, which only a byte-level assertion catches.
- [ ] T014 [P] [US1] Deadline and capacity tests in `crates/malt-daemon/tests/vnp_listener.rs`: a connection that sends nothing is closed within the deadline; many stalled connections do not prevent a legitimate client from completing a handshake.
- [ ] T015 [P] [US1] Access test in `crates/malt-daemon/tests/vnp_listener.rs`: an authenticated client naming a session id it may not reach is refused, distinguishably from a nonexistent one.

**Checkpoint**: all four gates green; quickstart Scenario 1 verified against a live daemon. Commit. **Merge to main.**

---

## Phase 4: User Story 2 — An interactive command can be answered (Priority: P1)

**Goal**: Bytes a client sends reach a command that is blocked reading — unmodified, and without becoming a command themselves.

**Independent Test**: quickstart.md Scenario 2 — `read` blocks, `malt send` answers it, the command completes with exactly the bytes sent; whitespace, non-UTF-8, and a bare newline all survive; `cat` (external) receives input too; the answer appears in neither history nor the event stream.

### Implementation for User Story 2

- [ ] T016 [US2] Create `crates/malt-daemon/src/executor/input.rs`: a `SessionInputChannel` built on `malt_platform::io::create_pipe()`, holding the write end. Writes are **non-blocking** — a full pipe is refused with a distinct error, never waited on, because the control actor must never block on a client (the discipline features 002/004 established). Register the module in `executor/mod.rs`.
- [ ] T017 [US2] Own the channel in `crates/malt-daemon/src/executor/session_thread.rs` and register its read end at fd `0` in the session's `mash::Env` via `Env::register_fd`. This is what makes `read` take session input: the builtin already resolves `env.open_fd_read(0)` before falling back to `std::io::stdin()`, so **no change to `mash`'s `read` is needed** (research R2) — and the fall-through to the daemon's own console stops being reachable.
- [ ] T018 [US2] Add a raw-input `SessionCommand` variant carrying `Vec<u8>` (plus `client_id`, wired in US3) in `crates/malt-daemon/src/executor/session_thread.rs`, with a handler that writes to the input channel. **It must not call `run_mash_command`** — that is the only producer of `CommandBlock`s and `CommandStarted` events, so keeping raw input off that path is what satisfies FR-010 structurally rather than by filtering (research R7).
- [ ] T019 [US2] Rewrite `WriteInput` handling in `crates/malt-daemon/src/executor/session_thread.rs` to route to the input channel. Remove the `from_utf8_lossy` → `trim` → submit-as-command chain: each step independently corrupts input (mangles non-text bytes, destroys significant whitespace, discards a bare newline).
- [ ] T020 [US2] Change `send_input` in `crates/malt-daemon/src/gateway_backend.rs` to write raw bytes instead of submitting an execution and waiting 30 seconds. **This changes the meaning of an existing endpoint** (`POST /sessions/{id}/send`) — note it in the commit body, since a caller relying on `send` to run a command must switch to `exec`.
- [ ] T021 [US2] Add `Coordinator` passthrough for raw input in `crates/malt-daemon/src/executor/coordinator.rs`, following the `begin_*` pattern: do not wait while holding the coordinator lock.
- [ ] T022 [US2] Give external processes the session's stdin in `crates/mash/src/executor.rs`. Spawns currently default to `Io::Inherit` (~lines 1284, 5682, 5895), so a REPL or `ssh` prompt inherits the *daemon's* stdin. Use `Io::File` with a handle to fd 0 when one is registered, falling back to `Inherit` when it is not so non-daemon `mash` usage is unaffected.
- [ ] T023 [US2] Byte-fidelity tests in `crates/malt-daemon/tests/input.rs` (new): a blocked `read` receives exactly what was sent for three cases that the old path each broke differently — leading/trailing whitespace preserved, non-UTF-8 bytes unchanged, a bare newline delivered rather than discarded.
- [ ] T024 [US2] Confidentiality test in `crates/malt-daemon/tests/input.rs`: answer a prompt with a recognisable secret, then assert it appears in **neither** the session's command history **nor** its lifecycle event stream. Assert absence from both surfaces explicitly — "we did not call that function" is not something a later reader can verify.
- [ ] T025 [US2] Type-ahead and bound tests in `crates/malt-daemon/tests/input.rs`: input sent with nothing reading is delivered to the next read; filling the channel without a reader is refused with a clear error and the control actor keeps servicing other commands (assert by issuing another command successfully while the pipe is full).
- [ ] T026 [P] [US2] External-process test in `crates/malt-daemon/tests/input.rs`: run `cat` (or an equivalent portable external reader) and assert it receives client input rather than hanging or reading the daemon's console.
- [ ] T027 [US2] **Run the Smoosh conformance suite** and confirm 183/183 on native Windows. `read` and external-process stdin are POSIX surface; a regression here blocks this story.

**Checkpoint**: all four gates green, Smoosh 183/183, quickstart Scenario 2 verified live. Commit. **Merge to main.**

---

## Phase 5: User Story 3 — Exactly one client can type at a time (Priority: P2)

**Goal**: Input is attributed to an authenticated client, and only the authority holder's reaches the session.

**Independent Test**: quickstart.md Scenario 3 — two clients attached, both send, the command receives bytes from exactly one with no interleaving; the non-holder is told why it was refused; either can ask who holds authority.

### Implementation for User Story 3

- [ ] T028 [US3] Add `client_id` to the input-carrying `SessionCommand` variants in `crates/malt-daemon/src/executor/session_thread.rs` — `KeyInput` carries none today, which is the decisive reason authority cannot be enforced (research R4). The VNP listener already allocates a per-connection id; thread it through.
- [ ] T029 [US3] Drive `AuthorityTracker` from the real attach path in `crates/malt-daemon/src/executor/coordinator.rs` and `session_thread.rs`: `RegisterVnpClient`/`UnregisterVnpClient` must inform it. **Do not rewrite the tracker** — `attach`/`detach`/`claim`/`holder` are already implemented and unit-tested; the defect is that production never calls them (research R5).
- [ ] T030 [US3] Honour the requested authority in `crates/malt-daemon/src/vnp_listener.rs`: `wait_for_attach` currently parses `AttachSession.authority` and discards it. Apply it, defaulting an unheld session's first attacher to holding (spec Assumptions).
- [ ] T031 [US3] Enforce on input in `crates/malt-daemon/src/executor/session_thread.rs`: reject raw input from a client that does not hold authority, with a reason naming the holder. Never drop silently — that is indistinguishable from a dead connection (FR-014).
- [ ] T032 [US3] Add an authority-query path so a client can ask who holds it (`Coordinator` + `GatewayBackend` + a route, following the `begin_*` pattern), per the contract.
- [ ] T033 [US3] Arbitration test in `crates/malt-daemon/tests/input.rs`: two attached clients send distinguishable payloads concurrently to a blocked `read`; assert the command received bytes from exactly one, with **none** of the other's mixed in. Interleaving is the failure this prevents, so the payloads must be distinguishable byte-wise rather than merely counted.
- [ ] T034 [P] [US3] Rejection test: a non-holder's input is refused with a reason identifying the holder, and the non-holder still receives output and events normally (FR-020 — losing input rights must not degrade observation).
- [ ] T035 [P] [US3] Authority-through-the-real-path test in `crates/malt-daemon/tests/vnp_listener.rs`: drive attach via the actual VNP path and assert the tracker reflects it. A test that calls `AuthorityTracker` directly passes **today**, with the feature entirely absent — it must not be the evidence.

**Checkpoint**: all four gates green; quickstart Scenario 3 verified. Commit. **Merge to main.**

---

## Phase 6: User Story 4 — Authority changes hands without stranding (Priority: P2)

**Goal**: Authority transfers on claim and releases on departure, so a session is never left unanswerable.

**Independent Test**: quickstart.md Scenario 4 — B claims from A and input rights follow; then kill the holder abruptly mid-prompt and confirm another client can claim and answer without a restart or timeout.

### Implementation for User Story 4

- [ ] T036 [US4] Add claim and authority-changed messages to the schema (`schemas/input.vexil` or nearest fit) and recompile. `InputClaim`/`InputAuthorityChanged` have codec constants already but no handling.
- [ ] T037 [US4] Handle claims in `crates/malt-daemon/src/vnp_listener.rs` and the session executor: a claim from an attached client succeeds immediately and the previous holder is notified. Consent is deliberately not required — a departed or unresponsive holder would otherwise strand the session (spec Assumptions, FR-018).
- [ ] T038 [US4] Release authority on disconnect in `crates/malt-daemon/src/executor/coordinator.rs`: clean detach and abrupt disconnect both release, with no timeout or grace period, so the next attached client can claim at once.
- [ ] T039 [US4] Notify all attached clients on change in `crates/malt-daemon/src/executor/session_thread.rs`, so no client believes it can type when it cannot.
- [ ] T040 [US4] Surface authority changes in `crates/malt-tui/src/connection.rs` and the TUI, so a human whose input stopped being accepted learns why rather than typing into a void.
- [ ] T041 [US4] Handover test in `crates/malt-daemon/tests/input.rs`: B claims from A; B's input is accepted, A's is refused, both are informed.
- [ ] T042 [US4] **Abrupt-departure test** in `crates/malt-daemon/tests/input.rs`: with a command blocked at a prompt, drop the authority holder's connection without a clean detach; assert another attached client can claim and answer, and that the command proceeds. This is the scenario that turns arbitration from a feature into a hazard if it fails.
- [ ] T043 [P] [US4] Edge-case tests: claiming authority already held is a no-op with no notification to others; a session with no clients is claimable by whoever attaches next.

**Checkpoint**: all four gates green; quickstart Scenario 4 verified, including the abrupt-kill case. Commit. **Merge to main.**

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T044 Update `docs/BACKLOG.md`: mark ADR-0003 priorities 5 and 6 delivered, and audit findings A-01, A-03, A-07, A-08 closed with evidence (test names). Keep the peer-credential transport migration open and re-state why it was deferred.
- [ ] T045 [P] Update `AGENTS.md`: VNP now requires authentication; `send` writes raw input rather than executing; input authority is live. Correct the "What's Implemented" claims that this feature falsifies.
- [ ] T046 [P] Amend `docs/design/architecture.md` where it describes input handling, and note that local identification uses a shared token rather than the peer credentials the document specifies — with a pointer to the backlog item. Do not silently leave the document describing a mechanism that is not there.
- [ ] T047 Run the full quickstart manually against a live daemon — all four scenarios, including the abrupt-kill handover — and record the outcome in a dated `docs/findings/` entry. The riskiest behavior here is only observable with real clients, as it was for 003 and 004.
- [ ] T048 Final verification: all four gates plus Smoosh 183/183. Commit. **Merge to main.**

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → blocks everything.
- **Phase 2 (Foundational)** → blocks US1: T003 must land before authentication is built on the token.
- **Phase 3 (US1)** → blocks US2 as a *safety* ordering, not a technical one. US2 is technically independent, but shipping it first would make prompts injectable by any local process.
- **Phase 4 (US2)** → blocks US3 (there must be an input path to arbitrate) and US4.
- **Phase 5 (US3)** → blocks US4 (there must be arbitration before transfer means anything).
- **Phase 7 (Polish)** → after all stories.

### Within-Story Dependencies

- US1: T007 → T008 → T009 → T011; T010 independent of the ordering fix; T012 after T008; T013–T015 after their subjects.
- US2: T016 → T017 → T018 → T019/T020/T021; T022 independent of the daemon-side chain (different crate); T023–T026 after T019; T027 after T022.
- US3: T028 → T029 → T030 → T031 → T032; T033–T035 after T031.
- US4: T036 → T037 → T038 → T039 → T040; T041–T043 after T038.

### Parallel Opportunities

- T006 (gateway auth tests) alongside T004/T005 — different crates.
- T014/T015 alongside T013 once the listener changes land.
- T022 (`mash`) alongside T018–T021 (`malt-daemon`) — different crates, no shared files.
- T026 alongside T023–T025 — same file, so coordinate, but independent subjects.
- T034/T035 alongside T033.
- T045/T046 (different doc files) in parallel.

---

## Implementation Strategy

**MVP is US1, not US2.** That is deliberate and worth restating: the feature is named for raw input, but the shippable first increment is *closing the unauthenticated control plane*. It is independently valuable — it closes a Critical finding — and it is what makes the rest safe to add.

**Then US2, which is the reason the feature exists.** After it, an interactive command can actually be answered through MALT for the first time. Research found this to be much smaller than expected (`read` already consults fd 0), so the bulk of the work is external-process stdin and proving byte fidelity.

**US3 and US4 are strictly sequential**, unlike previous features where the P2 stories were independent: arbitration needs attribution, and handover needs arbitration.

**Six merge checkpoints**, one per phase after Setup — Foundational is independently mergeable because a CSPRNG token is worth having regardless of what consumes it.

---

## Notes

- The control actor must never block on a client. Input writes are non-blocking with an explicit refusal, matching `events.rs`'s `try_send` discipline.
- Raw input must never reach `run_mash_command`. That function is the sole producer of history entries and lifecycle events; structural separation is the confidentiality guarantee, not a filter.
- No `unwrap()`/`expect()` outside `#[cfg(test)]`; `thiserror` in library crates; OS calls behind `malt-platform`.
- Two tests in this feature would pass with the feature absent if written carelessly: authority driven directly against `AuthorityTracker`, and handshake auth asserted by calling the validator rather than by inspecting what crossed the socket. Both are called out at their tasks.
