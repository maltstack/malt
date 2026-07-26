---
description: "Task list for fail-closed session isolation"
---

# Tasks: Fail-Closed Session Isolation

**Feature**: `specs/007-fail-closed-isolation/`
**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md), [data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md)

## Format: `[ID] [P?] [Story] Description`

- **[P]** — parallelisable: different files, no dependency on an incomplete task
- **[US1]–[US4]** — the user story the task serves

## Path Conventions

Multi-crate Rust workspace. Paths are repo-relative from the worktree root.

---

> **Read before starting.** Three things govern this list.
>
> 1. **This is wiring, not platform implementation.** 12 of `malt-platform`'s
>    14 isolation modules have no caller outside their own crate. Do not
>    write a backend before checking whether one exists (research R1).
> 2. **The failure mode here is different from the last three features.** It
>    is not an untested delivery path. It is **a test that asserts a
>    containment call returned `Ok` and concludes containment exists** —
>    which is exactly how the present defect survives, since the Job Object
>    call *does* succeed and nothing checks that anything is constrained by
>    it. Every tier claim needs a test that observes the constraint from
>    outside.
> 3. **Smoosh is not a gate** for this feature; `mash` is untouched. Said
>    explicitly so its absence is not read as an oversight.

---

## Phase 1: Setup

- [X] T001 Confirm the workspace baseline before changing anything: `cargo test --workspace` passed 1,448 tests / 0 failed (2026-07-26).
- [X] T002 [P] Re-verified before implementation: `apply_session_isolation` returned `()` and warned on failure; `Capped` and `Contained` had identical Job Object limits; the non-Windows branch was empty. The implementation now returns an error and refuses `Contained` rather than relabelling a Job Object.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: find out what actually works before building policy on top of it. This phase produces no user-visible change and is the phase most likely to change the rest of the plan.

- [X] T003 Added `isolation_reality.rs`; real Job Object create/query calls succeed. HCS availability is checked and the compute-system entry point confirms the default build cannot create an HCS system.
- [X] T004 Recorded the caller/test survey in `docs/findings/2026-07-26-isolation-module-reality.md`: only Job Objects are session-wired; HCS/tokens and Unix/macOS modules cannot be claimed as session containment.
- [X] T005 Added `CapabilityBasis` (`Verified`/`Assumed`/`None`) to `CapabilityReport`, preserving `CapabilityReasonCode`.
- [X] T006 Reclassified every capability facet as verified, assumed, or unavailable without converting assumptions into probes.
- [X] T007 Defined `TierRequirements`, explicit mechanisms, and distinct constraints; a unit test prevents `Contained` from aliasing `Capped`.

**Checkpoint**: it is known which modules work, and a capability report can no longer claim verification it does not have. No user-visible change yet.

---

## Phase 3: User Story 1 - Honoured or refused, never quietly downgraded (Priority: P1) 🎯 MVP

**Goal**: asking for containment produces containment or a failure.

**Independent Test**: quickstart Scenario 1 — request a tier this host cannot provide as `required`; creation fails **and `malt list` shows no session**.

### Implementation for User Story 1

- [ ] T008 [US1] Add `IsolationPolicy` (`Required`/`Preferred`/`Disabled`) to `schemas/common.vexil` and regenerate. **Check the next free `@type` in the session domain before allocating** — a duplicate message was drafted in an earlier feature because an existing definition was not checked.
- [ ] T009 [US1] Change `apply_session_isolation` in `crates/malt-daemon/src/executor/session_thread.rs` to return what was established rather than `()`. The existing warning text is honest — *"session will run without process containment"* — the defect is that it says it to a log instead of the caller.
- [ ] T010 [US1] Apply the policy at session creation in `crates/malt-daemon/src/executor/coordinator.rs`: under `Required`, a failure to establish means **no session exists afterwards**. Ensure partial establishment counts as failure, not downgrade (data-model.md).
- [ ] T011 [US1] Ensure the failure path leaves nothing behind in `crates/malt-daemon/src/executor/coordinator.rs` — no half-created session, no orphaned job or thread. A fail-closed path that leaks is a different bug traded for the first one.
- [ ] T012 [US1] Accept the policy on all three request surfaces: `crates/malt-gateway/src/routes/sessions.rs` and `types.rs`, the VNP create path, and `malt new --isolation-policy` in `crates/malt-bin/src/cli.rs`. Default to `Required` when a tier above the baseline is named (research R5).
- [ ] T013 [US1] Return 409 with `isolation_unavailable` per contracts/isolation-status.md, and **name `preferred` in the message**. A refusal that does not say how to proceed converts a silent downgrade into a dead end.
- [ ] T014 [US1] Test in `crates/malt-daemon/tests/isolation.rs`: a required request that cannot be met fails **and the session list is empty afterwards**. Assert on the absence of the session, not on the error — "an error was returned" is compatible with a session still running, which is the current defect's exact shape.
- [ ] T015 [P] [US1] Test in `crates/malt-daemon/tests/isolation.rs` that `Preferred` creates a session and reports the downgrade in the creation result itself (FR-004), and that `Disabled` attempts nothing.
- [ ] T016 [P] [US1] Test in `crates/malt-daemon/tests/isolation.rs` that a failed establishment leaves no orphaned OS resource, verified by inspection rather than by the call not erroring (SC-006 in miniature).

**Checkpoint**: the trust defect is closed on every platform, including those with no enforcement at all. Gates green. Commit. **Merge to main.**

---

## Phase 4: User Story 2 - What is in force can be seen (Priority: P2)

**Goal**: every surface reports what the session actually has.

**Independent Test**: quickstart Scenario 3 — creation response, `malt list`, and single-session query report identical status including `basis`, for granted, downgraded, and refused requests.

### Implementation for User Story 2

- [ ] T017 [US2] Add `IsolationBasis` and `IsolationStatus` to `schemas/common.vexil`, and change `SessionInfo.isolation` from `IsolationTier` to `IsolationStatus`. Bump `@revision`, keep field numbers. This is a **breaking wire change** (contracts/isolation-status.md).
- [ ] T018 [US2] Store the status on the session in `crates/malt-daemon/src/executor/coordinator.rs`, computed once at establishment. **One value read by all surfaces** — not three constructions, which is what lets FR-007 hold structurally rather than by convention.
- [ ] T019 [US2] Report it from create, get, and list in `crates/malt-gateway/src/routes/sessions.rs`, and from the VNP session-info path.
- [ ] T020 [US2] Display it in `crates/malt-bin/src/main.rs` for `malt new` and `malt list`, with a **visible** notice on downgrade. A downgrade that appears only in structured output has not been reported to a human.
- [ ] T021 [US2] Test in `crates/malt-daemon/tests/isolation.rs` that all three surfaces agree, for a granted, a downgraded, and a refused request (SC-002). Compare the whole status including `basis`, not just the tier.
- [ ] T022 [P] [US2] Test in `crates/malt-daemon/tests/isolation.rs` that a session whose basis is `Assumed` never reports `Verified` (FR-006) — the distinction added in T005/T006 must survive to the reporting surface, or it was pointless.

**Checkpoint**: US1 and US2 both work. Gates green. Commit. **Merge to main.**

---

## Phase 5: User Story 3 - Distinct levels actually differ (Priority: P2)

**Goal**: a stronger tier constrains more than a weaker one.

**Independent Test**: quickstart Scenario 4 — the same work under adjacent tiers, with a constraint binding at the stronger and not the weaker.

### Implementation for User Story 3

- [ ] T023 [US3] Replace `job_object_limits_for_tier` in `crates/malt-daemon/src/executor/session_thread.rs` with a per-tier mechanism binding driven by `TierRequirements` (T007). `Capped` and `Contained` currently return identical limits, which is the defect stated as code.
- [ ] T024 [US3] Make `Contained` use its own mechanism — HCS via `crates/malt-platform/src/isolation/hcs.rs`, which has `create_process` and today has **zero callers** (its one apparent caller is a comment at `session_thread.rs:85`). If HCS is unavailable, `Contained` is refused under `Required`, not silently resolved to a Job Object (FR-009).
- [ ] T025 [US3] Decide and record in `specs/007-fail-closed-isolation/research.md` (append an R9) whether AppContainer is a named alternative mechanism for `Contained` when HCS is absent, or out of scope. vexil-v2 uses it that way. **Acceptable only if the session reports which mechanism it got** (FR-005); a silent substitution is the defect in a new costume. Write the decision down either way (Principle IX).
- [ ] T026 [US3] Verify establishment rather than assuming it in `crates/malt-daemon/src/executor/session_thread.rs`: after applying a tier, confirm the constraint is in force where it can be observed, and set `basis` accordingly.
- [ ] T027 [US3] Test in `crates/malt-platform/tests/isolation_reality.rs` that for each adjacent tier pair, one constraint binds at the stronger and not the weaker (SC-004), by running real work — not by comparing the limit values the code passed in.
- [ ] T028 [P] [US3] Test in `crates/malt-platform/tests/isolation_reality.rs` that a process spawned inside a contained session is subject to the containment (FR-012, SC-005), observed from outside rather than inferred from the spawn succeeding.
- [ ] T029 [P] [US3] Test in `crates/malt-platform/tests/isolation_reality.rs` that teardown releases the containment and everything in it, verified by enumerating the job or container and finding it gone (SC-006).

**Checkpoint**: US1–US3 work. Gates green. Commit. **Merge to main.**

---

## Phase 6: User Story 4 - Containment beyond the primary platform (Priority: P3)

**Goal**: containment works elsewhere, or asking fails there — discoverable in advance.

**Independent Test**: quickstart Scenario 7 — `malt isolation capabilities` reports per-tier availability with an honest basis, and matches which requests then succeed.

### Implementation for User Story 4

- [ ] T030 [US4] Add `GET /isolation/capabilities` at **`Read`** scope in `crates/malt-gateway/src/routes/`, `server.rs` and `middleware.rs`, per contracts/isolation-status.md, reporting per tier availability, mechanism, and basis (FR-010).
- [ ] T031 [US4] Add `malt isolation capabilities` in `crates/malt-bin/src/{cli,client,main}.rs`.
- [ ] T032 [US4] Wire the Linux path in `crates/malt-daemon/src/executor/session_thread.rs` using the existing `namespaces`/`cgroups`/`seccomp`/`rlimit` modules — subject to what T004 found about whether they work. Do not write new backends before confirming the existing ones are absent or broken.
- [ ] T033 [US4] Wire the macOS path using the existing `crates/malt-platform/src/isolation/sandbox.rs` — 359 lines of `sandbox_init`, currently zero callers. Note this is **larger** than vexil-v2's equivalent; an earlier survey wrongly called macOS a MALT gap, so check before porting anything.
- [ ] T034 [US4] Test in `crates/malt-daemon/tests/isolation.rs` that on a platform with no enforcement path a required request fails with a clear reason, and a preferred one succeeds reporting no containment.
- [ ] T035 [P] [US4] Test in `crates/malt-daemon/tests/isolation.rs` that the capabilities report matches which requests subsequently succeed. A capabilities call that disagrees with reality is worse than none, since it is what a caller uses to decide what to ask for.

**Checkpoint**: all four stories work. Gates green. Commit. **Merge to main.**

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T036 Handle restore in `crates/malt-daemon/src/executor/coordinator.rs`: a restored session re-establishes its containment or reports that it no longer holds it (FR-014). Whether restore currently re-establishes is **unverified** (research R7) — check, do not assume. A restart is the most plausible way to produce a session claiming containment it lacks.
- [ ] T037 Surface containment lost after creation (FR-017) in `crates/malt-daemon/src/executor/session_thread.rs`, updating the stored status rather than leaving a session reporting a level it no longer has. Note the detection point is unspecified by the spec -- decide whether it is checked on query, on command execution, or not at all this cycle, and record which.
- [ ] T038 Close `specs/001-cli-isolation-flag/spec.md` — its reopened fail-closed requirement is what this feature delivers. Update its status with the evidence, citing test names that **exist**; verify each before writing it.
- [ ] T039 [P] Update `docs/BACKLOG.md`: close audit A-02 with evidence, and mark priority 7 delivered. Restate what remains open — AppContainer, and any module T004 found broken.
- [ ] T040 [P] Update `AGENTS.md`: isolation is fail-closed; sessions report verified status; the new route and CLI commands. Correct any "What's Implemented" claim this falsifies, including the isolation line.
- [ ] T041 Record the **breaking change** in release notes: `SessionInfo.isolation` changes shape, and naming a tier now defaults to `Required`, so calls that previously returned a silently uncontained session will fail. Name `preferred` as the opt-out.
- [ ] T042 Run the full quickstart manually against a live daemon — all eight scenarios — and record the outcome in a dated `docs/findings/` entry, **including what it does not establish**.
- [ ] T043 Final verification: `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`. Smoosh not required (`mash` untouched). Commit. **Merge to main.**

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → blocks everything.
- **Phase 2 (Foundational)** → blocks all stories. T003/T004 in particular may change what US3 and US4 can promise; discovering a module is broken *after* building policy on it is the expensive order.
- **Phase 3 (US1)** → blocks US2 (there must be a status to report) and US3.
- **Phase 4 (US2)** and **Phase 5 (US3)** → US3 depends on US2's status type for reporting which mechanism it used; sequence US2 first.
- **Phase 6 (US4)** → after US3.
- **Phase 7 (Polish)** → after all stories, except T036/T037 which are implementation and could land with US2.

### Within-Story Dependencies

- Foundational: T003 → T004 (findings inform each other) → T005 → T006 → T007.
- US1: T008 → T009 → T010 → T011 → T012 → T013; T014–T016 after T013.
- US2: T017 → T018 → T019 → T020; T021–T022 after T020.
- US3: T023 → T024 → T025 → T026; T027–T029 after T026.
- US4: T030 → T031 → T032 → T033; T034–T035 after T033.

### Parallel Opportunities

- T002 alongside T001.
- T015/T016 alongside T014 — same file, coordinate, independent subjects.
- T022 alongside T021; T028/T029 alongside T027; T035 alongside T034.
- T039/T040 in parallel — different documents.
- US4's two platform paths (T032 Linux, T033 macOS) are independent of each other.

---

## Implementation Strategy

**MVP is Phase 1 + Phase 2 + Phase 3 (US1).** That closes the trust defect: a
caller can no longer be told it got containment it did not get. It does so on
*every* platform, including those with no enforcement path, because there the
honest answer is a refusal.

US2 makes the guarantee auditable, US3 makes the tier choice mean something,
US4 restores capability where there is currently none.

**Do not shorten Phase 2.** It produces nothing visible, and the temptation
will be to skip to T009. But T003/T004 answer whether the modules US3 and US4
depend on actually work, and this codebase has now produced five instances of
a mechanism that exists, is tested, and does nothing. Building the policy
layer on an untested assumption about twelve of them is the one mistake that
would waste the whole feature.
