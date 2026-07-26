---

description: "Task list for feature implementation"
---

# Tasks: A Privileged Helper That Performs Privileged Operations

**Input**: Design documents from `/specs/008-privileged-helper/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/elevate-protocol.md](./contracts/elevate-protocol.md)

**Tests**: Included. The spec's success criteria are stated as things to be
verified by *doing*, not by inspection, and this feature's failure mode
(research R8) is passing tests that never cross the boundary.

**Organization**: Grouped by user story so each is independently
implementable, testable and shippable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1 / US2 / US3
- Exact file paths in every task

---

## Phase 1: Setup

**Purpose**: Make the schema and the OS surface available. Nothing here
changes behaviour.

- [X] T001 Add a build script `crates/malt-elevate/build.rs` compiling `schemas/elevate.vexil`. Follow `crates/malt-protocol/build.rs`, which **invokes the `vexilc` binary via `std::process::Command`** against `../../schemas` — it is not a `vexil-lang` build-dependency, so do not add one. CI already installs a pinned `vexilc` (`.github/actions/setup-vexilc`). **The schema exists and is complete** — this is wiring, not authoring (research R2).
- [X] T002 [P] Add `Win32_System_Services`, `Win32_System_Pipes` and `Win32_System_Threading` to the `windows-sys` features in `crates/malt-platform/Cargo.toml`. `Win32_Foundation` and `Win32_Security` are already present.
- [X] T003 [P] Confirm `malt-elevate`'s new build script runs in CI. **It is already a workspace member** (`Cargo.toml:3`, ninth entry), so `cargo build --workspace` covers it — verify the pinned `vexilc` is on PATH for it as it is for `malt-protocol`, since a build script that cannot find `vexilc` fails the whole workspace build. That is exactly how CI run 1 failed on 2026-07-26.

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ No user story work begins until this phase completes.** These change the
types every story returns.

- [X] T004 Extend `schemas/elevate.vexil`: add `session_id` and `nonce` to `ElevateRequestEnvelope`; replace `ManageHcsContainer`'s `operation : string` / `config : bytes` with a typed `ContainerOperation` union; add `OutcomeKind`, `ReasonCode`, `ElevateCapabilities`, `OperationCapability`; reshape `ElevateResponse` to carry kind/reason/detail/payload. Bump `@version`. **Do not reuse field numbers.** Full target shape in [contracts/elevate-protocol.md](./contracts/elevate-protocol.md) §1.
- [X] T005 Delete the hand-rolled `MessageTag` enum from `crates/malt-elevate/src/protocol.rs` and use the generated types. **Delete it rather than leaving it beside the generated types** — two encodings for one channel is how they drift, and `dispatch.rs:8`'s hand-maintained mirror of the same union is the evidence (research R2).
- [X] T006 Replace `ElevateResponse`'s `result: Result<Vec<u8>, String>` in `crates/malt-elevate/src/dispatch.rs` with the outcome type from [data-model.md](./data-model.md) §1. **The defect this prevents**: `Ok(vec![])` currently means both "performed" and "pretended, returning success", and no caller can tell them apart.
- [X] T007 Remove the non-test `.expect("length checked above")` at `crates/malt-elevate/src/auth.rs:30`. Constitution IV admits no exception, and this file is rewritten in US2 anyway — doing it now keeps the gate green at every checkpoint.
- [X] T008 Delete the false claims in `schemas/elevate.vexil:8` ("Single-use nonce from the nonce file. Rotated hourly with 30s overlap") and `crates/malt-elevate/src/auth.rs:44-45` ("the nonce is single-use and rotated hourly"). Neither is implemented and nothing writes the nonce file. **Do this before US2 designs the replacement**, so nobody reads them as a description of existing behaviour (research R3).

**Checkpoint**: types and schema are honest; no behaviour has changed yet.

---

## Phase 3: User Story 1 — Operations that did not happen report failure (Priority: P1) 🎯 MVP

**Goal**: Every operation the helper cannot perform returns an error naming
it. The capability surface reports what this host can actually do.

**Independent Test**: Invoke every operation where none can be performed;
assert each is `Refused` with a reason and none is `Performed`. Needs no
privilege, no installation, no container.

### Tests for User Story 1

- [X] T009 [P] [US1] **Invert** `stub_operations_return_success` in `crates/malt-elevate/src/dispatch.rs:153` into `unimplemented_operations_are_refused_not_reported_as_success`. **Invert, do not delete** (spec US1 acceptance 3): a deleted test cannot fail if the fail-open returns, and this test currently asserts the fail-open.
- [X] T010 [P] [US1] Add `crates/malt-elevate/tests/operation_outcomes.rs` asserting every operation is `Refused` or `Performed` — never a bare success payload. **Enumerate the operations from the generated schema type, not from a hand-written list in the test.** A hand-written list silently stops covering a new operation the day one is added, which is exactly how `dispatch.rs:8`'s mirror drifted from the schema it mirrors.
- [X] T011 [P] [US1] Add a test asserting the capability surface and reality agree: for each operation reported available, invoking it does not fail with `NotImplemented`/`UnsupportedPlatform`; for each reported unavailable, invoking it fails with the reason given. **The defect this prevents**: a capability answer that disagrees with reality is worse than none, because it is what a caller uses to decide what to ask for (mirrors spec 007's SC-007).

### Implementation for User Story 1

- [X] T012 [US1] Replace `stub_success` in `crates/malt-elevate/src/dispatch.rs:96-102` with a refusal carrying `ReasonCode::NotImplemented` and the operation name. Apply to all eight remaining stubs: `CreateNamespace`, `MountOverlay`, `SetCgroup`, `SetupNetns`, `ApplySeccomp`, `CreateRestrictedToken`, `ApplySeatbelt`, `BindPort`.
- [X] T013 [US1] Distinguish `UnsupportedPlatform` from `NotImplemented` in those refusals — a Linux-only operation on Windows is the former; an operation nothing implements anywhere is the latter. **The defect this prevents**: collapsing them tells a Linux caller its host is at fault when the build is.
- [X] T014 [US1] Route `CreateSymlink` through `malt_platform::fs::create_symlink` (`crates/malt-platform/src/fs.rs:282`), deleting `create_symlink_windows` and `create_symlink_unix` from `crates/malt-elevate/src/dispatch.rs:120-146`. **This removes a live Constitution II violation** (`std::os::unix::fs::symlink` at `dispatch.rs:142`) **and a worse duplicate** — the platform version handles the Windows file-vs-directory distinction more carefully than the copy (research R4).
- [X] T015 [US1] Create `crates/malt-elevate/src/capability.rs` reporting per-operation availability, basis and reason. **Availability MUST be derived from what is implemented and what the host supports — never from the protocol being able to encode the message** (FR-002).
- [X] T016 [US1] Verify no path through `dispatch_request` can now produce a success outcome without an effect, by reading every arm. Record the arms checked in the commit message. **The defect this prevents**: T012 fixes the arms someone remembered; this catches the one they did not.

**Checkpoint**: US1 ships alone. The helper is honest, still unreachable, and
gates are green.

---

## Phase 4: User Story 2 — The helper runs privileged and its state is visible (Priority: P2)

**Goal**: A privileged host process the daemon reaches over an authenticated
channel, with install/status/uninstall an operator can inspect and reverse.

**⚠️ This is the largest phase.** Research R1 found there is no server at all
— `main.rs:91-105` loads a nonce, prints two lines and exits, and `--socket`
is parsed and never used. Nothing here is adaptation.

**Independent Test**: Produce all four helper states on one host and confirm
each is reported distinctly; confirm an unauthorised local process is refused.
No container operation needed.

### Platform layer (all new OS surface — Constitution II)

- [X] T017 [P] [US2] Create `crates/malt-platform/src/ipc/` with a named-pipe server and client (`mod.rs`, `windows.rs`). Every `unsafe` block carries `// SAFETY:`. **Put this in `malt-platform`, not `malt-elevate`** — the plan's Structure Decision, and `dispatch.rs:142` is what happened last time this crate made its own OS calls.
- [X] T018 [US2] Add peer-identity query to `crates/malt-platform/src/ipc/windows.rs`: attribute a connected pipe client to an OS principal. **This is the primary authentication control** — the existing nonce proves read access to a file, not the identity of a process (research R3).
- [X] T019 [P] [US2] Create `crates/malt-platform/src/service/` with install, uninstall, and status against the Windows service manager (`mod.rs`, `windows.rs`), all `unsafe` documented.
- [X] T020 [US2] Add real-OS tests for T017–T019 in `crates/malt-platform/tests/`. **Tests that construct types without calling Win32 are the shape that hid two real `job_objects.rs` bugs** (Constitution IV) — these must open a pipe, query a peer, and register a service.

### Helper server

- [X] T021 [US2] Create `crates/malt-elevate/src/server.rs`: accept a connection, authenticate the peer, read framed requests, dispatch, respond. Bound every request with a timeout.
- [X] T022 [US2] Rewrite `crates/malt-elevate/src/main.rs` to serve. **Delete the "Phase 2 skeleton — IPC loop not yet implemented" path entirely** rather than leaving it behind a flag.
- [X] T023 [US2] Rewrite `crates/malt-elevate/src/auth.rs`: peer identity from T018 as the primary control; per-request single-use nonce with a bounded validity window for replay rejection. **A shared secret may remain as defence in depth but MUST NOT be the only control**, and no comment may claim a property the code does not implement (T008 removed the last two).
- [X] T024 [US2] Implement session-entitlement validation in `crates/malt-elevate/src/dispatch.rs` per [data-model.md](./data-model.md) §3: session ownership, pid membership checked **against the OS not against the request**, and path containment checked **after canonicalization**. **Put the path check in one function every operation calls** — guarding sites individually is how two of three tool-dispatch sites were missed.
- [X] T025 [US2] Return `Indeterminate` when a request was sent and no response arrived (FR-005). **The defect this prevents**: resolving an unknown outcome to either success or failure — the caller must be told it is unknown.

### Daemon and CLI

- [X] T026 [US2] Create `crates/malt-daemon/src/elevate_client.rs`: connect, query state, send requests. **Query `HelperState` before attempting any operation, and attempt nothing on `VersionMismatch`** (FR-014).
- [X] T027 [US2] Add `malt elevate status|install|uninstall` to `crates/malt-bin/src/`. Install and uninstall require explicit elevation and **MUST NOT run as a side effect of any other command** (FR-007).
- [X] T028 [US2] Make `status` report `reachable` **only after a round-trip response**, not because the service manager reports the service running. **The defect this prevents**: service bookkeeping is what the OS says about itself, not evidence anything answers — eliding that distinction is what this feature exists to stop.

### Tests for User Story 2

- [ ] T029 [P] [US2] Test all four helper states — `NotInstalled`, `InstalledStopped`, `Reachable`, `VersionMismatch` — are produced and reported distinctly with distinct guidance (SC-003). Skip loudly with a reason if the host cannot install a service.
- [ ] T030 [P] [US2] Test declined elevation leaves nothing installed, **verified by inspecting for each artefact installation would have created**, not by the command reporting an error (SC-010). A partial install that reports failure is still a partial install.
- [X] T031 [P] [US2] Test an unauthorised local process sending a well-formed request is refused (SC-004). **Verified by making the request**, not by reviewing the design. This is the scenario that decides whether this feature shipped a helper or a local privilege-escalation primitive.
- [X] T032 [P] [US2] Test a captured valid request replayed is refused (SC-005). Note this asserts a property two documents claimed and no code implemented before T023.
- [X] T033 [P] [US2] Test entitlement refusals (SC-006): another principal's session; a path outside `storage_root` via `..`; **a path outside it via a symlink**. The symlink case is the one a naive prefix comparison passes — test it explicitly.

**Checkpoint**: US1 and US2 both work. The boundary exists and is crossed by
authenticated requests. Gates green, commit, merge.

---

## Phase 5: User Story 3 — One operation crosses the boundary (Priority: P3)

**Goal**: The container operation that motivated the boundary works through
it. The call refused for lack of privilege from the daemon is no longer
refused for that reason through the helper.

### Isolation carrier consolidation (FR-015..017) — before the backend

- [X] T034 [US3] Extend `IsolationContext` in `crates/malt-platform/src/isolation/tier.rs` with an `Established` enum (`Nothing` / `JobObject` / `Container`) per [data-model.md](./data-model.md) §4. Do this **before** the container work so the backend has one place to attach.
- [X] T035 [US3] Make `crates/mash/src/env.rs` carry one isolation field. Remove `job_object` (`env.rs:320`) and make `isolation_context` (`env.rs:314`) the carrier that `crates/mash/src/executor.rs:5683` reads. **Remove the old field; do not deprecate it alongside** — two mechanisms six lines apart is precisely how this arose. Confirm `Env::clone()` still propagates to subshells (`env.rs:373`).
- [X] T036 [US3] Update `crates/malt-daemon/src/executor/session_thread.rs:116` and the session-status reporting to read isolation from the single carrier (FR-017), so what a session reports and what constrains it cannot drift.
- [X] T037 [US3] Assert isolation is never upgraded after session creation (FR-018). Downgrade to `Nothing` on containment lost remains spec 007's FR-017. **The defect this prevents**: spec 007 covered containment lost and nothing covered containment gained.

### Container operation

- [X] T038 [US3] Implement `ManageHcsContainer` in `crates/malt-elevate/src/dispatch.rs` against `malt_platform::isolation::hcs`. **The helper renders the container configuration document itself from typed fields plus the session's entitlement — the caller never supplies it** (FR-013, contract §1). This is the difference between a helper and an arbitrary-privileged-action service.
- [X] T039 [US3] Ensure teardown runs on every failure path, so a create that fails part-way leaves no compute system behind (contract, operation rule 3). The `run_operation` helper added to `hcs.rs` earlier today already closes operations on every path — follow that shape.
- [ ] T040 [US3] Route the daemon's contained-session path through `elevate_client` when the tier requires it, updating the session's `IsolationContext` **only on a `Performed` outcome**. `Refused` and `Indeterminate` leave it untouched.

### Tests for User Story 3

- [ ] T041 [US3] Add `crates/malt-daemon/tests/elevate_boundary.rs` making the **same call directly and through the helper in one run** and asserting the direct one is refused for lack of privilege while the routed one is not refused for that reason (SC-007). **Do not assert the routed call succeeded** — a compute system is image-backed and images are out of scope, so a different failure is a pass. Comparing both calls is what makes this falsifiable rather than a story about privilege.
- [ ] T042 [P] [US3] Test teardown leaves nothing, **verified by enumerating the compute system and finding it absent** (SC-008), not by teardown returning success — the same rule spec 007's SC-006 applies to sessions.
- [ ] T043 [P] [US3] Test that killing the helper mid-operation yields `Indeterminate` and leaves the session's isolation status unchanged (FR-005, Scenario 10).
- [X] T044 [US3] Assert `mash::Env` carries exactly one isolation field — count is 1, it is 2 today (SC-009). **Run this after T038 lands**, not before: the requirement is that adding a container backend needed no parallel path, and checking before the backend exists tests nothing.

**Checkpoint**: all three stories functional. Gates green, commit, merge.

---

## Phase 6: Polish & Cross-Cutting

- [ ] T045 Run every scenario in [quickstart.md](./quickstart.md) against a live daemon and record the outcome in a dated `docs/findings/` entry, **including what it does not establish**. Any scenario the host cannot run is recorded as skipped with its reason — **environment-blocked is not done**.
- [X] T046 [P] Resolve the divergence warning in `docs/design/architecture.md` §"Shell and Isolation". After T034–T036 the document describes what the code does; replace the warning block with a note that the consolidation happened, referencing ADR-0005.
- [X] T047 [P] Update `AGENTS.md`'s `malt-elevate` line — it currently reads "17 tests; only CreateSymlink is real, other ops stub", which stops being true at T012.
- [X] T048 [P] Update `docs/briefs/README.md` and `docs/BACKLOG.md` for anything this feature closed or opened.
- [ ] T049 Final gates: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, plus `cargo clippy -p malt-platform --features hcs --all-targets -- -D warnings`. **Smoosh does not apply** — `mash/src/env.rs` only loses a field, changing no shell semantics. Commit. **Merge to main.**

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (P1)**: no dependencies.
- **Foundational (P2)**: needs Setup. **Blocks all stories** — it changes the
  types every story returns.
- **US1 (P3)**: needs Foundational. No dependency on US2 or US3.
- **US2 (P4)**: needs Foundational. Independent of US1, though shipping US1
  first means the transport is built onto honest handlers rather than onto
  stubs that report success.
- **US3 (P5)**: needs US2 (there is no boundary to cross without it) and
  benefits from US1.
- **Polish (P6)**: needs all desired stories.

### Critical path

`T001 → T004 → T005/T006 → T017 → T018 → T021 → T023 → T026 → T038 → T041`

### Parallel opportunities

- T002, T003 together.
- T009, T010, T011 together (different files).
- T017 and T019 together — different modules, no shared state. T018 needs T017.
- T029–T033 together once T021–T028 land.
- T042, T043 together. **T044 must follow T038.**
- T046, T047, T048 together.

---

## Implementation Strategy

### MVP (User Story 1 only)

1. Phase 1 → Phase 2 → Phase 3.
2. **Stop and validate**: every operation refuses honestly; capability surface
   matches reality; gates green.
3. Ship. The helper is still unreachable, and it no longer lies — which is
   strictly better than today and worth having on its own.

### Incremental

1. Setup + Foundational → types honest.
2. US1 → helper honest → **commit, merge**.
3. US2 → boundary exists → **commit, merge** (large; commit in parts —
   platform layer, server, CLI).
4. US3 → boundary changes an outcome → **commit, merge**.

### This feature's failure mode, restated for whoever executes this

**Proving the dispatch table while proving nothing about the boundary.**

US1 can be "completed" by changing `stub_success` to return an error, after
which the same in-process tests pass in milliseconds and not one byte has
crossed a privilege boundary. That is a real improvement and it is not this
feature. No task here is complete on the strength of a test that calls a
helper function directly once US2 exists; where a test cannot cross the
boundary on the host running it, it skips loudly with the reason.

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
