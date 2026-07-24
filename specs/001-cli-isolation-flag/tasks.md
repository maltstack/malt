# Tasks: CLI Isolation Flag

**Input**: Design documents from `/specs/001-cli-isolation-flag/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md),
[research.md](./research.md), [data-model.md](./data-model.md), and
[contracts/malt-new.md](./contracts/malt-new.md)

**Tests**: Required. The specification and quickstart require unit coverage
for command parsing, request serialization, result validation, error handling,
and focused/workspace regression execution.

**Organization**: Tasks are grouped by user story after the shared command and
client foundations, so each user-visible behavior has its own verification
checkpoint.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: May run in parallel with other marked tasks after its stated
  dependencies are complete and without overlapping files.
- **[Story]**: Maps a task to the corresponding user story in [spec.md](./spec.md).

## Phase 1: Setup

**Purpose**: Confirm the existing single-crate boundary before changing it.

No project initialization, dependency, workspace, protocol, gateway, or daemon
setup is required. The existing `malt-bin` crate and its dependencies are the
entire implementation boundary.

---

## Phase 2: Foundational Command and Client Contract

**Purpose**: Establish the typed input and testable request mapping used by all
three user stories.

**⚠️ CRITICAL**: Complete this phase before changing `malt new` dispatch or
output behavior.

- [ ] T001 [P] Add failing parser tests for all four canonical tiers, a missing value, an invalid value, and an omitted value in `crates/malt-bin/src/cli.rs`.
- [ ] T002 [P] Add failing request-payload serialization tests for name-only, empty, tier-only, and name-plus-tier creation requests in `crates/malt-bin/src/client.rs`.
- [ ] T003 [P] Implement the typed `bare`, `restricted`, `capped`, and `contained` command argument plus canonical request/display helpers in `crates/malt-bin/src/cli.rs`.
- [ ] T004 [P] Implement a serializable create-session payload and extend `MaltClient::create_session` to accept optional canonical isolation while preserving omitted-field payloads in `crates/malt-bin/src/client.rs`.

**Checkpoint**: The CLI accepts only the contract values and the client can
prove exactly what it will send, without changing any command behavior yet.

---

## Phase 3: User Story 1 - Create a Session at a Selected Isolation Tier (Priority: P1) 🎯 MVP

**Goal**: Let an operator create a named or unnamed session using one selected
canonical tier and see that matching reported tier in the creation result.

**Independent Test**: For each of the four typed values, exercise the command
result helper with a matching returned `SessionData` and verify the request
value and success text identify the same tier. Run the focused CLI test suite.

### Tests for User Story 1

- [ ] T005 [US1] Add failing unit tests for matching selected-tier result validation and creation-line formatting across all four tiers in `crates/malt-bin/src/main.rs`.

### Implementation for User Story 1

- [ ] T006 [US1] Update `Command::New` dispatch and `handle_new` to send the selected tier, verify the matching response, and print the session identifier, name fallback, and reported tier in `crates/malt-bin/src/main.rs`.

**Checkpoint**: `malt new --isolation <tier>` satisfies the command contract
for every canonical tier when the session authority returns the matching tier.

---

## Phase 4: User Story 2 - Preserve the Existing Default Session Workflow (Priority: P2)

**Goal**: Retain the existing Bare default and no-subcommand automatic
workflow while allowing `--name` to combine with a selected tier.

**Independent Test**: Verify omitted isolation derives Bare for a new-command
result, `malt new --name <name>` still has no selected payload tier, and
parsing no subcommand still selects no explicit command.

### Tests for User Story 2

- [ ] T007 [US2] Add regression tests for omitted-isolation Bare expectation, named creation without isolation, and no-subcommand parsing in `crates/malt-bin/src/cli.rs` and `crates/malt-bin/src/main.rs`.

### Implementation for User Story 2

- [ ] T008 [US2] Update the no-subcommand and no-option `create_session` call sites to pass no isolation selection while retaining Bare expectation and all existing workflow choices in `crates/malt-bin/src/main.rs`.

**Checkpoint**: Existing `malt new`, `malt new --name <name>`, and bare
`malt` workflows retain their prior behavior, with explicit creation results
now reporting Bare where applicable.

---

## Phase 5: User Story 3 - Receive Clear Input and Creation Failures (Priority: P3)

**Goal**: Reject invalid command input locally and ensure creation errors or a
reported-tier mismatch never become a successful creation message.

**Independent Test**: Confirm parser tests reject invalid and missing values;
simulate an error envelope and a response whose tier differs from the request;
verify each produces an actionable error and no successful creation line.

### Tests for User Story 3

- [ ] T009 [P] [US3] Add failing tests that preserve a create-session error message from an unsuccessful response envelope in `crates/malt-bin/src/client.rs`.
- [ ] T010 [P] [US3] Add failing tests that reject a reported-tier mismatch and suppress the success line in `crates/malt-bin/src/main.rs`.

### Implementation for User Story 3

- [ ] T011 [P] [US3] Make `MaltClient::create_session` return the gateway-provided failure reason when its create-session response is unsuccessful in `crates/malt-bin/src/client.rs`.
- [ ] T012 [US3] Make `handle_new` return an actionable requested-versus-reported tier error without retrying or deleting a session in `crates/malt-bin/src/main.rs`.

**Checkpoint**: Invalid input, creation rejection, unreadable error response,
and reported-tier mismatch all terminate unsuccessfully without a successful
creation line.

---

## Phase 6: Polish and Cross-Cutting Validation

**Purpose**: Verify the completed command behavior without expanding the
feature into gateway or platform-isolation work.

- [ ] T013 Run formatting and the focused command-client test suite in `crates/malt-bin/` with `cargo fmt --check` and `cargo test -p malt-bin`.
- [ ] T014 Run full workspace regression in `Cargo.toml` with `cargo test --workspace` and investigate any failure before completion.
- [ ] T015 Execute the isolated daemon validation scenarios and record their result against `specs/001-cli-isolation-flag/quickstart.md`, including the stated platform-evidence boundary.

---

## Dependencies and Execution Order

### Phase Dependencies

- **Phase 1**: No work is required; the existing crate is the setup boundary.
- **Phase 2**: Starts immediately; T001 → T003 and T002 → T004 are the two
  independent test-first chains.
- **User stories**: All begin after T003 and T004 establish the typed
  selection and payload contract.
- **Polish**: Begins only after all selected user-story tasks are complete.

### User Story Dependencies

- **US1 (P1)**: Depends on T003 and T004. It is the MVP and establishes the
  selected-tier success path.
- **US2 (P2)**: Depends on T003, T004, and the shared result behavior from
  US1; it verifies compatibility rather than introducing another subsystem.
- **US3 (P3)**: Depends on T003, T004, and the US1 creation path; it adds the
  failure-path behavior around that same command boundary.

### Within Each User Story

- Write the listed tests first and confirm they fail for the intended missing
  behavior before implementing their paired task.
- Keep `main.rs` tasks sequential because command dispatch, result validation,
  and user-visible output share the same file.
- Keep direct gateway parsing, capability probing, and OS containment outside
  this task list; they are not implementation dependencies of the backlog CLI
  feature.

## Parallel Opportunities

- T001 and T002 can proceed in parallel because they modify separate CLI and
  client files.
- T003 and T004 can proceed in parallel after their corresponding tests are in
  place because they modify separate files.
- T009 and T010 can proceed in parallel after the selected-tier success path
  exists; their implementations T011 and T012 can then proceed in parallel
  if their tests are complete.

## Implementation Strategy

### MVP First

1. Complete Phase 2.
2. Complete T005 and T006 for US1.
3. Run `cargo test -p malt-bin` and manually validate a Bare selected-tier
   creation against an isolated daemon.
4. Do not report platform containment from that CLI proof.

### Incremental Delivery

1. Deliver US1 for explicit tier selection and visible matching results.
2. Deliver US2 to demonstrate existing scripts and automatic workflow remain
   compatible.
3. Deliver US3 to fail closed in command reporting and preserve service error
   reasons.
4. Finish with focused, workspace, and isolated-daemon validation.
