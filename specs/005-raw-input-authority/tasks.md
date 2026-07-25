# Tasks: Authenticated Raw Input with Input Authority

**Input**: Design documents from `/specs/005-raw-input-authority/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/input-and-authority.md, quickstart.md

**Tests**: Included. Constitution IV and this project's history require tests that drive real paths. Two specific traps apply here and are called out at their tasks: `AuthorityTracker` passes its own unit tests *today* while being unreachable from production, and a handshake test that calls an auth function directly proves nothing about what bytes went over the socket.

**Organization**: Grouped by user story. US1 (authenticated identity) is the foundation and must land first — shipping US2 before it would make password prompts injectable by any local process, which is worse than today's inability to answer them.

**Baseline**: Branch `worktree-spec-kit-work` at `0401889`, with all four gates green (build, test, fmt, clippy).

**Standing gate for this feature**: `mash` is modified by US2. Smoosh must stay 183/183 on native Windows. A regression there blocks the story; it is not a note to file.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [X] T001 Verify the baseline: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`. All four must pass before starting. Record the Smoosh baseline too (`cargo test -p mash --test smoosh_runner smoosh_conformance_tests`, expect 183 passed / 3 skipped) — US2 changes `mash`, and a before-number is worthless once the change is in.
- [X] T002 Add an OS CSPRNG dependency (`getrandom`) to `crates/malt-gateway/Cargo.toml`, needed by T003. Confirm it is not already reachable transitively before adding.

---

## Phase 2: Foundational

**Purpose**: The credential must be worth checking before anything is built to check it.

**⚠️ CRITICAL**: T003 gates User Story 1. Authenticating with a guessable token is a lock whose key can be recomputed (research R1).

- [X] T003 Replace `generate_random_token` in `crates/malt-gateway/src/auth.rs` with OS CSPRNG bytes. It currently derives both halves from epoch nanoseconds and a fixed multiplier, so a token is recomputable by anyone who can approximate daemon start time (audit A-03). Keep the `malt_` prefix and the length so existing readers are unaffected.
- [X] T004 Fix token persistence in `crates/malt-gateway/src/auth.rs`: directory-creation and file-write errors are currently ignored, so the daemon can come up believing it persisted a token it did not. Write atomically (temp + rename), restrict to owner-only permissions, and fail daemon startup rather than continuing with a token nothing else can read.
- [X] T005 Stop printing the admin token to stdout in `crates/malt-bin/src/daemon.rs`. Print the path it was written to instead, so first-run discovery still works without putting the secret in scrollback, CI logs, and terminal history.
- [X] T006 [P] Tests in `crates/malt-gateway/tests/` (new `auth.rs`): two tokens generated in the same process differ; a token generated at a known instant is not reproducible from that instant (assert against the old epoch-derived construction, so the fix cannot silently regress); a write failure surfaces as an error rather than a silent success.

**Checkpoint**: gates green. Commit. **Merge to main** — this is independently valuable even before US1.

---

## Phase 3: User Story 1 — Only an identified client can reach a session (Priority: P1) 🎯 MVP

**Goal**: The VNP transport refuses unidentified clients, discloses nothing to them, and cannot be exhausted by them.

**Independent Test**: quickstart.md Scenario 1 — connect without a credential and confirm the connection closes with no session names in the returned bytes; connect and stall and confirm the daemon closes it promptly while still serving others; confirm a legitimate client is unaffected.

### Implementation for User Story 1

- [X] T007 [US1] Extend the handshake schema in `schemas/handshake.vexil`: add a credential field to `Hello`, and a rejection message (or reuse a refusal shape) the daemon can send before `HelloAck`. Recompile via `vexilc` and fix generated-code fallout.
- [X] T008 [US1] Authenticate in `crates/malt-daemon/src/connection/handshake.rs`: `perform_server_handshake` takes the `TokenStore` and validates the credential, returning the resolved `AuthScope` in `HandshakeResult`. Reject before any `HelloAck` is written.
- [X] T009 [US1] **Invert the disclosure order** in `crates/malt-daemon/src/vnp_listener.rs`. Today `list_sessions()` is called and the result passed *into* the handshake, so the inventory is assembled before anything is checked and sent inside `HelloAck`. Collect it only after authentication succeeds. This ordering is the substance of FR-002 — refusing after sending the inventory would pass a careless test.
- [X] T010 [US1] Bound pre-identification cost in `crates/malt-daemon/src/vnp_listener.rs` (audit A-08): set the read deadline *before* the blocking handshake read rather than after it, and cap concurrent un-authenticated connections so connect-and-stall cannot consume threads and sockets without limit.
- [ ] T011 [US1] Check session access on attach in `crates/malt-daemon/src/vnp_listener.rs`: validate the caller-supplied session id against what the authenticated identity may reach, instead of honoring it because it was supplied (FR-005). Distinguish "no such session" from "not permitted".
- [X] T012 [US1] Present the credential from `crates/malt-tui/src/connection.rs`, reading it from the same well-known token file `malt-bin` and `malt-mcp` already use. `malt attach` must keep working with no user-visible change.
- [X] T013 [US1] Socket-level tests in `crates/malt-daemon/tests/vnp_listener.rs`: connect a real socket with no credential and assert the connection closes **and that no session name appears in the bytes received** — create named sessions first so there is something to leak. Do not call the auth function directly; the bug class here is ordering, which only a byte-level assertion catches.
- [X] T014 [P] [US1] Deadline and capacity tests in `crates/malt-daemon/tests/vnp_listener.rs`: a connection that sends nothing is closed within the deadline; many stalled connections do not prevent a legitimate client from completing a handshake.
- [ ] T015 [P] [US1] Access test in `crates/malt-daemon/tests/vnp_listener.rs`: an authenticated client naming a session id it may not reach is refused, distinguishably from a nonexistent one.

**Checkpoint**: all four gates green; quickstart Scenario 1 verified against a live daemon. Commit. **Merge to main.**

---

## Phase 4: User Story 2 — An interactive command can be answered (Priority: P1)

**Goal**: Bytes a client sends reach a command that is blocked reading — unmodified, and without becoming a command themselves.

**Independent Test**: quickstart.md Scenario 2 — `read` blocks, `malt send` answers it, the command completes with exactly the bytes sent; whitespace, non-UTF-8, and a bare newline all survive; `cat` (external) receives input too; the answer appears in neither history nor the event stream.

### Implementation for User Story 2

> **US2 complete 2026-07-25.** Client input reaches a waiting command --
> the `read` builtin, an external process, and in-process tools alike.
>
> Verified live: an external program's password prompt answered by a client;
> `cat` echoing streamed lines and terminating on end-of-input; `head -n1`
> returning on its first line with no EOF needed; `wc -l` counting 3; `grep`
> matching; and the session still accepting input afterwards.
>
> Three things worth carrying forward:
>
> - **`ToolFn` is reader-based now.** It took a finished `&[u8]`, so dispatch
>   had to read fd 0 to EOF before calling a tool -- which a session's stdin
>   never reaches. The earlier workaround (hand tools an empty buffer) is gone
>   along with the whole `endless_fds` mechanism it needed.
> - **Streaming input required end-of-input to ship with it.** A tool reading
>   for real hangs without a way to end the stream, which would have been the
>   same hang, reintroduced. `malt eof` / `POST /sessions/{id}/eof` ends the
>   read and installs a fresh pipe, so the session stays usable.
> - **`head` had a latent off-by-one**: enumerate-and-break pulled one line
>   past the limit. Equivalent on a finished buffer, but on a live stream it
>   means blocking for a line the user has no reason to type.
>
> Remaining, deliberately not done here: tool *output* is still buffered until
> the command finishes, so an interactive `cat` shows its echo at the end
> rather than line by line. Input was the gap that made tools unusable;
> output streaming needs a writer-based `ToolFn` and a different result type.

## Phase 5: User Story 3 — Exactly one client can type at a time (Priority: P2)

**Goal**: Input is attributed to an authenticated client, and only the authority holder's reaches the session.

**Independent Test**: quickstart.md Scenario 3 — two clients attached, both send, the command receives bytes from exactly one with no interleaving; the non-holder is told why it was refused; either can ask who holds authority.

### Implementation for User Story 3

- [X] T028 [US3] Add `client_id` to the input-carrying `SessionCommand` variants in `crates/malt-daemon/src/executor/session_thread.rs` — `KeyInput` carries none today, which is the decisive reason authority cannot be enforced (research R4). The VNP listener already allocates a per-connection id; thread it through.
- [X] T029 [US3] Drive `AuthorityTracker` from the real attach path in `crates/malt-daemon/src/executor/coordinator.rs` and `session_thread.rs`: `RegisterVnpClient`/`UnregisterVnpClient` must inform it. **Do not rewrite the tracker** — `attach`/`detach`/`claim`/`holder` are already implemented and unit-tested; the defect is that production never calls them (research R5).
- [X] T030 [US3] Honour the requested authority in `crates/malt-daemon/src/vnp_listener.rs`: `wait_for_attach` currently parses `AttachSession.authority` and discards it. Apply it, defaulting an unheld session's first attacher to holding (spec Assumptions).
- [X] T031 [US3] Enforce on input in `crates/malt-daemon/src/executor/session_thread.rs`: reject raw input from a client that does not hold authority, with a reason naming the holder. Never drop silently — that is indistinguishable from a dead connection (FR-014).
- [X] T032 [US3] Add an authority-query path so a client can ask who holds it (`Coordinator` + `GatewayBackend` + a route, following the `begin_*` pattern), per the contract.
- [X] T033 [US3] Arbitration test in `crates/malt-daemon/tests/input.rs`: two attached clients send distinguishable payloads concurrently to a blocked `read`; assert the command received bytes from exactly one, with **none** of the other's mixed in. Interleaving is the failure this prevents, so the payloads must be distinguishable byte-wise rather than merely counted.
- [X] T034 [P] [US3] Rejection test: a non-holder's input is refused with a reason identifying the holder, and the non-holder still receives output and events normally (FR-020 — losing input rights must not degrade observation).
- [X] T035 [P] [US3] Authority-through-the-real-path test in `crates/malt-daemon/tests/vnp_listener.rs`: drive attach via the actual VNP path and assert the tracker reflects it. A test that calls `AuthorityTracker` directly passes **today**, with the feature entirely absent — it must not be the evidence.

**Checkpoint**: all four gates green; quickstart Scenario 3 verified. Commit. **Merge to main.**

> **US3 complete 2026-07-25.** The finding that mattered was sharper than the
> task text: `AuthorityTracker` was not merely uncalled. There were **two
> parallel attach paths** — `AttachClient`/`DetachClient`, which drove the
> tracker but were sent only by a test, and `RegisterVnpClient`, the path
> production actually uses, which ignored authority entirely. So the tracker
> looked wired, and a test proved it was, while no real client ever reached
> it. The vestigial pair is deleted; there is one attach path now.
>
> Input is attributed via `InputOrigin`. `Unattributed` is deliberately not a
> fake client id: the HTTP surface has no per-connection identity, and
> inventing one would put a lie in the type. It is accepted only while nobody
> holds authority — so an unattached session behaves exactly as before (which
> is why every pre-existing input test still passes), and an agent cannot use
> the HTTP door to type over a human holding the keyboard.
>
> Refusals name the holder rather than being flattened to "unreachable": a
> client that cannot see who has the keyboard cannot decide whether to claim
> it. `GET /sessions/{id}/authority` is at **Read** scope, not Interact —
> knowing who holds input is observation, and FR-020 keeps observation
> available to clients that cannot type.
>
> Gates: workspace 1411 passed / 0 failed, fmt and clippy clean. Smoosh not
> re-run: `mash` is untouched by this story.

---

## Phase 6: User Story 4 — Authority changes hands without stranding (Priority: P2)

**Goal**: Authority transfers on claim and releases on departure, so a session is never left unanswerable.

**Independent Test**: quickstart.md Scenario 4 — B claims from A and input rights follow; then kill the holder abruptly mid-prompt and confirm another client can claim and answer without a restart or timeout.

### Implementation for User Story 4

- [X] T036 [US4] Add claim and authority-changed messages to the schema (`schemas/input.vexil` or nearest fit) and recompile. `InputClaim`/`InputAuthorityChanged` have codec constants already but no handling.
- [X] T037 [US4] Handle claims in `crates/malt-daemon/src/vnp_listener.rs` and the session executor: a claim from an attached client succeeds immediately and the previous holder is notified. Consent is deliberately not required — a departed or unresponsive holder would otherwise strand the session (spec Assumptions, FR-018).
- [X] T038 [US4] Release authority on disconnect in `crates/malt-daemon/src/executor/coordinator.rs`: clean detach and abrupt disconnect both release, with no timeout or grace period, so the next attached client can claim at once.
- [X] T039 [US4] Notify all attached clients on change in `crates/malt-daemon/src/executor/session_thread.rs`, so no client believes it can type when it cannot.
- [X] T040 [US4] Surface authority changes in `crates/malt-tui/src/connection.rs` and the TUI, so a human whose input stopped being accepted learns why rather than typing into a void.
- [X] T041 [US4] Handover test in `crates/malt-daemon/tests/input.rs`: B claims from A; B's input is accepted, A's is refused, both are informed.
- [X] T042 [US4] **Abrupt-departure test** in `crates/malt-daemon/tests/input.rs`: with a command blocked at a prompt, drop the authority holder's connection without a clean detach; assert another attached client can claim and answer, and that the command proceeds. This is the scenario that turns arbitration from a feature into a hazard if it fails.
- [X] T043 [P] [US4] Edge-case tests: claiming authority already held is a no-op with no notification to others; a session with no clients is claimable by whoever attaches next.

**Checkpoint**: all four gates green; quickstart Scenario 4 verified, including the abrupt-kill case. Commit. **Merge to main.**

> **US4 complete 2026-07-25.**
>
> T036 needed no schema work: `InputClaim` and `InputAuthorityChanged` were
> already defined in `schemas/session.vexil` and already generated. What was
> missing was handling, exactly as the task predicted.
>
> T038 was already satisfied by US3 — `cleanup()` runs
> `unregister_vnp_client` on every disconnect path, and unregister releases
> authority. It is now *proved* rather than assumed, by the abrupt-departure
> test rather than by reading the code.
>
> **Per-client delivery became one ordered stream.** Authority changes travel
> the same channel as frames (`ClientMessage`) rather than a second channel,
> because a client must not learn it lost authority *after* rendering frames
> that arrived later — and two channels cannot promise that ordering.
>
> A claim requires the claimant to be attached. Otherwise a departed client
> could take the keyboard for a session it is no longer listening to, which
> is the stranding FR-018 exists to prevent, arrived at from the other side.
> Re-claiming what you already hold notifies nobody: FR-019 is about actual
> changes.
>
> The TUI now draws a notice when authority moves. Without it the human
> symptom of losing input is indistinguishable from a hung terminal.
>
> Gates: workspace 1415 passed / 0 failed, fmt and clippy clean. Smoosh not
> re-run — `mash` is untouched by this story. Live: a session with nobody
> attached reports no holder and accepts input, confirming an unattached
> session stays answerable.

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
