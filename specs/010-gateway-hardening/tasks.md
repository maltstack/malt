---

description: "Task list for feature implementation"
---

# Tasks: Gateway Hardening

**Input**: Design documents from `/specs/010-gateway-hardening/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/limits.md](./contracts/limits.md)

**Tests**: Included. Every success criterion in this feature is stated as
something to be verified by *doing* — waiting out a window, measuring memory,
driving a client that recovers — because the easy proxy for each one passes
while the defect is fully present (research R7).

**Organization**: Grouped by user story so each is independently
implementable, testable and shippable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1 / US2 / US3
- Exact file paths in every task

## Read this before starting

The limiter is **live in `build_router` today**. There is no feature flag and
no partial rollout: whatever you write ships to every route at once. It is
also 44 lines, so the temptation is to treat this as a small change — it is
small in size and not in consequence.

**The one trap.** `429 after N requests` already works. If your tests assert
that, they pass today, before you have written anything. What is broken is
**recovery**, and only a test that advances time can see it. The same shape
applies to every other requirement here — see research R7 for the table.

---

## Phase 1: Setup

- [ ] T001 Confirm no new dependency is needed. `axum` 0.8 already provides `DefaultBodyLimit`, and the window needs only `std::time`. **If you find yourself reaching for a rate-limiting crate, stop and justify it against a `HashMap` and an `Instant`** — this is 44 lines of state, and a dependency here buys behaviour nobody has asked for.

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ Blocks all three stories** — every one of them reads or reports allowance state.

- [ ] T002 Replace the state in `crates/malt-gateway/src/rate_limit.rs` with the `Allowance` shape from [data-model.md](./data-model.md) §1: `used` plus `window_started`, evaluated on access. **Do not add a window to the existing counter** — `max_per_window` names a window that does not exist, and the file imports neither `Instant` nor `Duration`. The concept is absent, not broken.
- [ ] T003 **Delete `refill` and `refill_all`** from `rate_limit.rs`. They have zero production callers (verified 2026-07-28) and FR-002 forbids restoring allowance by an external call. **The defect this prevents**: a reset method that only works when something remembers to call it is exactly the state that produced this bug; keeping them "just in case" reproduces it.
- [ ] T004 Keep the poison-tolerant lock form `.lock().unwrap_or_else(|e| e.into_inner())` already in `rate_limit.rs:23`. **The defect this prevents**: `.unwrap()` on a poisoned mutex cascades one panic into unrelated failures — AGENTS.md records nine such call sites found in `mash`, and this file already has it right.
- [ ] T005 Update `crates/malt-gateway/tests/rate_limit.rs::refill_resets` — it exercises a method T003 deletes. **Replace it with the time-based recovery test, do not just delete it**: it is the only existing test about allowance returning, and removing it leaves that property uncovered.

**Checkpoint**: the limiter has a clock; nothing else has changed yet.

---

## Phase 3: User Story 1 — A throttled caller recovers (Priority: P1) 🎯 MVP

**Goal**: A caller that exceeds its allowance is refused for a bounded period and then served again, with no restart and no external reset.

**Independent Test**: Exhaust the allowance, wait a window, make a request, get 200.

### Tests for User Story 1

- [ ] T006 [P] [US1] Add a recovery test in `crates/malt-gateway/tests/rate_limit.rs` that exhausts the allowance, **advances past the window**, and asserts the next request succeeds (SC-001). **Assert on the request's outcome, not on internal counters** — the counter can look right while the caller is still refused.
- [ ] T007 [P] [US1] Add a sustained-rate test: a caller pacing itself under the limit is never refused across several windows (SC-002). **The defect this prevents**: a window that resets to a full allowance but also counts the triggering request against the *old* window can refuse a well-behaved caller at every boundary.
- [ ] T008 [P] [US1] Add a two-client isolation test (SC-003). **Use two distinct valid bearer tokens** — `client_id` is the token itself (research R2), so two arbitrary strings do not exercise the isolation this asserts.
- [ ] T009 [US1] Add a reclamation test (SC-008): after many distinct clients have been seen and their windows have elapsed, limiter state returns to a bounded level. **If this cannot be observed reliably, say so in the test and assert the closest honest thing** — do not fall back to counting map entries, which is the proxy that passes today.

### Implementation for User Story 1

- [ ] T010 [US1] Implement window evaluation on access in `rate_limit.rs`: if the window has elapsed, reset and count the current request against the **new** window (data-model §1's state diagram). A caller returning after a long gap is served immediately.
- [ ] T011 [US1] Implement entry reclamation for allowances whose window elapsed long ago (FR-004). **It must happen during normal operation, not in a sweep something has to call** — that is `refill`'s failure one level up. An entry with an elapsed window carries no information; it is indistinguishable from a client never seen.
- [ ] T012 [US1] Verify `crates/malt-gateway/src/middleware.rs:95` still compiles against the new signature and that the **check-before-scope order is unchanged**. Research R3 records that ordering as deliberate: reversing it lets an unauthorised caller probe routes for free. If you change it, say why in the commit.

**Checkpoint**: US1 ships alone and closes the live defect. Gates green, commit.

---

## Phase 4: User Story 2 — Oversized requests refused before buffering (Priority: P2)

**Goal**: A request body above the limit is rejected before the daemon buffers it.

**Independent Test**: Send an oversized body, get 413, and observe that daemon memory did not rise by the payload size.

### Tests for User Story 2

- [ ] T013 [P] [US2] Add a test in `crates/malt-gateway/tests/routes.rs`: a body above the limit is rejected with 413 and the documented `payload_too_large` code (contract §1).
- [ ] T014 [US2] Add the assertion that matters (SC-004): **the daemon does not buffer the oversized body**. A 413 returned *after* buffering 100 MB is the defect fully present and is indistinguishable from the fix by status code alone. If memory cannot be measured in-test, state what is asserted instead and why — research R7.
- [ ] T015 [P] [US2] Add a test that a payload at the documented ceiling for ordinary use **succeeds** (SC-005, FR-006). **The defect this prevents**: a limit tight enough to reject legitimate `exec` bodies is a regression shipped as hardening.

### Implementation for User Story 2

- [ ] T016 [US2] Apply the body limit at the **router layer** in `crates/malt-gateway/src/server.rs`, not in individual handlers (research R5). **The defect this prevents**: a per-handler check means a route added later without one is silently unprotected — the same weakness the scope map already has.
- [ ] T017 [US2] Choose the limit by **measuring real `exec` and `send` bodies**, not by guessing. The spec and contract deliberately leave the number to implementation for exactly this reason. Record the measurement in the commit message.
- [ ] T018 [US2] Ensure a request that understates its length and then exceeds the bound is refused mid-stream (FR-007). A declared-`Content-Length` rejection is easy and is only half the requirement.
- [ ] T019 [US2] Make the refusal message state the limit (contract §1). **The defect this prevents**: a caller told only "too large" has to bisect to find the boundary.

**Checkpoint**: US1 and US2 both ship. Gates green, commit.

---

## Phase 5: User Story 3 — Refusals are actionable (Priority: P3)

**Goal**: A refused caller can compute when to retry, and can tell its own quota from a system-wide ceiling.

**Independent Test**: Trigger a refusal; a client that sleeps for `Retry-After` and retries succeeds.

### Tests for User Story 3

- [ ] T020 [P] [US3] Add a test that a client sleeping for exactly `Retry-After` then retrying **succeeds** (SC-006). **The defect this prevents**: a padded constant passes a "header is present" assertion and fails this one. That is the whole distinction.
- [ ] T021 [P] [US3] Add a test that a quota refusal and a ceiling refusal are distinguishable from the response alone (SC-007).
- [ ] T022 [P] [US3] Add a test that successful responses carry `X-RateLimit-Remaining` and that it decreases (contract §2). **The defect this prevents**: a limiter that only speaks when refusing teaches clients to find the limit by hitting it.

### Implementation for User Story 3

- [ ] T023 [US3] Extend `GatewayError::RateLimited` in `crates/malt-gateway/src/error.rs` to carry `retry_after` and `cause` (data-model §2). **Keep the 429 status** — `error.rs:81` is already correct, and distinguishing the two causes by status would conflate the ceiling with `ExecutionQueueFull`'s 503.
- [ ] T024 [US3] Derive `retry_after` from real window state, never a constant (contract §1). T020 is the test that catches a constant.
- [ ] T025 [US3] Add the system-wide `Ceiling` from data-model §3, checked **in addition to** the per-caller allowance, reporting `cause: system_wide`.
- [ ] T026 [US3] Emit rate-limit headers on **every** response, not only refusals (contract §2).
- [ ] T027 [US3] Make each limit observable at runtime so an operator diagnosing a refusal can see the limit that caused it without reading source (contract §3).

**Checkpoint**: all three stories functional. Gates green, commit, merge.

---

## Phase 6: Polish & Cross-Cutting

- [ ] T028 Run every scenario in [quickstart.md](./quickstart.md) against a live daemon and record the outcome in a dated `docs/findings/` entry, **including what it does not establish**. A scenario the host cannot run is recorded as skipped with its reason — **environment-blocked is not done**.
- [ ] T029 [P] Update `docs/BACKLOG.md` and `docs/ROADMAP.md`'s "Gateway hardening (D)" section for what this closed.
- [ ] T030 [P] Mark `docs/briefs/003-rate-limiter-has-no-window.md` **RESOLVED**, keeping its original text as the record of what was wrong (the pattern used for briefs 004, 005, 006 and 007).
- [ ] T031 Final gates: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
  - **`cargo test --workspace` needs `MASH` set** or the Smoosh test fails and reads like a regression: `$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path`
  - **Linux via `bash scripts/wsl-mirror.sh`**, not `/mnt/c` — and not `/tmp` for the target dir, which is a RAM-backed tmpfs here.
  - **Smoosh is not a gate** for this feature; neither `mash` nor `malt-tools` is touched. Stated so its absence is not read as an oversight.
  - **macOS is not a target** (ADR-0006).
  - Commit. **Merge to main.**

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (P1)**: no dependencies.
- **Foundational (P2)**: needs Setup. **Blocks all stories** — it changes the type every story reads.
- **US1 (P3)**: needs Foundational. Independent of US2 and US3.
- **US2 (P4)**: needs Foundational only. Could be done before US1 if preferred; US1 is P1 because it is the live defect.
- **US3 (P5)**: needs US1 (it reports allowance state) and benefits from US2.
- **Polish (P6)**: needs all desired stories.

### Critical path

`T002 → T003 → T010 → T011 → T023 → T024 → T025`

### Parallel opportunities

- T006, T007, T008 together (same file, independent cases — coordinate edits).
- T013 and T015 together.
- T020, T021, T022 together once T023–T026 land.
- T029 and T030 together.

---

## Implementation Strategy

### MVP (User Story 1 only)

1. Phase 1 → Phase 2 → Phase 3.
2. **Stop and validate**: a client that exhausts its allowance is served again after a window, with no restart and nothing calling `refill`.
3. Ship. That alone closes the live defect — agents currently ban themselves permanently through ordinary use.

### Incremental

1. Setup + Foundational → the limiter has a clock.
2. US1 → callers recover → **commit, merge**.
3. US2 → bodies are bounded → **commit, merge**.
4. US3 → refusals are actionable → **commit, merge**.

### This feature's failure mode, restated for whoever executes this

**Asserting the status code and calling it verified.**

`429 after N requests` is already true before you change anything. `413 for a
large body` says nothing about whether it was buffered first. `a Retry-After
header exists` says nothing about whether its value works. Each easy assertion
passes while the defect it supposedly covers is fully present.

Every test here either advances time, measures memory, or drives a client that
actually recovers. Where a property genuinely cannot be observed, the test says
so — it does not quietly assert the proxy instead.

---

## MALT standing rules for task lists

*Appended by the `malt` preset. Not part of this feature.*

### A task carries the defect it prevents

Generic advice is ignored. "Add a test" is noise; "assert the session list is
empty afterwards, because an error being returned is compatible with a session
still running" is a task someone can execute and cannot misread.

### Evidence must exist

When a task cites a test name, a file path, or a line number as proof that
something is done — **verify it exists before writing it down.** A closure note
citing a test that does not exist is worse than no note, because it reads as
verification. This has happened here.

### Environment-blocked is not done

A task that cannot be truthfully completed on the available host — a platform
backend that is not installed, an OS not present, a service unavailable — is
**blocked**, not complete. Mark it so, state what is missing, and say what
would let it finish.

Do not check it off because the code "should" work. Do not quietly drop the
scenario that needed it. A checked box means verified, and the value of the
list depends on that staying true.

### One rule, one place

When a rule must hold at several call sites, put it in a function and have
every site call it. Guarding sites individually is how two of three tool
dispatch sites were missed, and how two parallel attach paths coexisted with
only one driving the authority tracker.

**Before adding a message, constant, or type: check whether it already exists
and is merely unused.**

### Checkpoints are real

Each user story ends with all applicable gates green, a commit, and a merge.
Constitution X: if a change feels too big to commit, that is a signal to
commit sooner, not later.
