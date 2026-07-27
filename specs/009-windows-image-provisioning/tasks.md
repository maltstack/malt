# Tasks: Windows contained-image provisioning

**Input**: Design documents from `/specs/009-windows-image-provisioning/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md),
[research.md](research.md), [data-model.md](data-model.md), and
[contracts/](contracts/)

**Tests**: Tests are required by the feature specification and the project
constitution. Every Windows FFI path must have focused real-API evidence on a
Windows host in addition to deterministic fake/rollback coverage.

**Organization**: Tasks are grouped by user story. The foundational phase is
intentionally small and security-critical because every story crosses the
existing authenticated elevation boundary.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the owned crate, protocol generation surface, and test
fixtures without adding a direct daemon-to-HCS route.

- [X] T001 Add the ADR-required `malt-image` L1 member and its minimal crate skeleton to `Cargo.toml` and `crates/malt-image/Cargo.toml` after explicit dependency approval.
- [X] T002 Define public image-domain error, digest, platform, descriptor, and immutable manifest types in `crates/malt-image/src/lib.rs` and `crates/malt-image/src/model.rs`.
- [ ] T003 [P] Add feature-local fixture manifests and intentionally corrupt descriptor samples in `crates/malt-image/tests/fixtures/`.
- [ ] T004 [P] Add typed opaque image-ID, readiness, and image-operation messages to `schemas/elevate.vexil` and regenerate/update `malt-protocol` exports in `crates/malt-protocol/src/`.
- [ ] T005 Add VNP round-trip and rejection tests for the new opaque image messages in `crates/malt-protocol/tests/roundtrip.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the verified content and helper-owned HCS substrate that
every image command and contained session needs.

**⚠️ CRITICAL**: No contained session may use an image record before all
verification, ownership, and HCS transaction tests in this phase pass.

- [X] T006 Implement immutable SHA-256 digest parsing, canonical formatting, and streaming verification in `crates/malt-image/src/digest.rs` with tests in `crates/malt-image/src/digest.rs`.
- [X] T007 [P] Implement Windows/amd64 manifest-list selection and host-version-policy inputs in `crates/malt-image/src/manifest.rs` with malformed, non-Windows, wrong-architecture, and ambiguous-selection tests.
- [ ] T008 [P] Implement bounded public-registry manifest/blob retrieval and size-aware descriptor verification in `crates/malt-image/src/registry.rs` with local fixture-server tests in `crates/malt-image/tests/registry.rs`.
- [ ] T009 Implement safe OCI layer archive validation/extraction that rejects traversal, duplicate entries, symbolic links, and special files while allowing only hard links to verified regular files in the same layer after complete archive validation in `crates/malt-image/src/archive.rs` with adversarial tests in `crates/malt-image/src/archive.rs`.
- [ ] T010 Implement atomic helper-owned blob/image-record publication and transaction rollback in `crates/malt-image/src/store.rs` with interrupted/corrupt-state tests in `crates/malt-image/src/store.rs`.
- [ ] T011 Add owned-root, owner-marker, and canonical-containment validation in `crates/malt-platform/src/isolation/layers.rs` with unit tests beside the module.
- [ ] T012 Implement Windows HCS parent-layer import/materialization, asynchronous operation-result checking, and rollback in `crates/malt-platform/src/isolation/layers.rs` with a deterministic fake backend test seam in that module.
- [ ] T013 Implement session-scoped writable-layer initialization, storage-filter attach/detach, and owner-checked cleanup in `crates/malt-platform/src/isolation/layers.rs` with ordered rollback tests beside the module.
- [ ] T014 Add Windows-only real HCS layer/scratch lifecycle tests, skipped with an explicit capability reason when Containers is unavailable, in `crates/malt-platform/tests/hcs_layers.rs`.
- [ ] T015 Extend `crates/malt-elevate/src/dispatch.rs` and `crates/malt-elevate/src/protocol.rs` to resolve only helper-owned opaque image IDs and dispatch provision/list/inspect/remove/workspace operations.
- [ ] T016 Add authenticated-helper tests proving raw paths, raw HCS storage JSON, unknown IDs, and unowned removal targets are refused in `crates/malt-elevate/src/dispatch.rs`.
- [ ] T017 Integrate helper-owned image readiness assessment, including host-change re-evaluation and sanitized reasons, in `crates/malt-elevate/src/dispatch.rs` with tests in `crates/malt-elevate/src/dispatch.rs`.

**Checkpoint**: A verified helper-owned image record and prepared parent chain
can be represented and rolled back, but no daemon or CLI claim yet says a
session is contained.

---

## Phase 3: User Story 1 - Provision a Windows base image (Priority: P1) 🎯 MVP

**Goal**: An operator provisions and inspects a public Windows image by
immutable digest and sees an honest readiness assessment.

**Independent Test**: Provision a supported public Windows reference against
the helper, inspect its manifest/layers/platform/readiness, then prove
non-Windows, tampered, and failed preparations do not create a selectable
record.

- [ ] T018 [P] [US1] Add daemon image client methods for provision/list/inspect and typed helper-result mapping in `crates/malt-daemon/src/elevate_client.rs`.
- [ ] T019 [US1] Implement daemon-side immutable image views and persistent display records in `crates/malt-daemon/src/image_manager.rs` and export them from `crates/malt-daemon/src/lib.rs`.
- [ ] T020 [US1] Add authenticated image-management gateway backend methods in `crates/malt-daemon/src/gateway_backend.rs` without exposing privileged paths.
- [ ] T021 [US1] Add image-management routes and request validation in `crates/malt-gateway/src/routes/images.rs` and register them from `crates/malt-gateway/src/lib.rs`.
- [ ] T022 [US1] Add the `malt image provision`, `list`, and `inspect` command grammar and output formatting in `crates/malt-bin/src/cli.rs` and `crates/malt-bin/src/main.rs` per `contracts/image-cli.md`.
- [ ] T023 [US1] Add daemon/gateway integration tests for immutable display identity, host-readiness reasons, and failed-provisioning invisibility in `crates/malt-daemon/tests/image_provisioning.rs`.
- [ ] T024 [US1] Add CLI parsing/output tests for provision/list/inspect and no-privileged-path disclosure in `crates/malt-bin/src/main.rs`.

**Checkpoint**: User Story 1 is independently complete when an operator can
obtain and inspect a verified, helper-owned image record without Docker.

---

## Phase 4: User Story 2 - Start an actually contained Windows session (Priority: P1)

**Goal**: A required contained session selects a ready image, owns a private
writable layer, and launches external commands through the existing HCS
process handoff.

**Independent Test**: Create a required contained session with a ready image,
execute `cmd /c ver` through `malt exec`, observe containment established and
the selected digest, then inject failure after workspace creation and prove no
session, compute system, or scratch state remains.

- [ ] T025 [US2] Add opaque contained-image selection to session creation requests in `crates/malt-protocol/src/` and `crates/malt-gateway/src/routes/sessions.rs`.
- [ ] T026 [US2] Add `--image` selection, single-ready-image resolution, and selected-digest reporting to `crates/malt-bin/src/cli.rs` and `crates/malt-bin/src/main.rs`.
- [ ] T027 [US2] Extend coordinator session records with selected image and active workspace reference state in `crates/malt-daemon/src/executor/coordinator.rs`.
- [ ] T028 [US2] Call helper readiness/workspace construction before reporting a contained session created in `crates/malt-daemon/src/executor/session_thread.rs`.
- [ ] T029 [US2] Replace the empty parent-layer HCS configuration in `crates/malt-elevate/src/dispatch.rs` with helper-resolved verified layers and a session-scoped writable workspace.
- [ ] T030 [US2] Preserve the existing `HcsProcessSpawner` process-only handoff while binding it to the prepared helper workspace in `crates/malt-daemon/src/executor/session_thread.rs` and `crates/malt-elevate/src/dispatch.rs`.
- [ ] T031 [US2] Implement rollback that tears down helper compute/workspace state before coordinator session publication in `crates/malt-daemon/src/executor/coordinator.rs` and `crates/malt-daemon/src/executor/session_thread.rs`.
- [ ] T032 [US2] Tear down contained compute/workspace state on session destroy and dormancy before clearing isolation context in `crates/malt-daemon/src/executor/coordinator.rs`.
- [ ] T033 [US2] Add daemon integration tests for required refusal, explicit preferred downgrade, selected-image reporting, process-spawner routing, and injected-construction rollback in `crates/malt-daemon/tests/elevate_boundary.rs`.
- [ ] T034 [US2] Add Windows-only real HCS contained external-command and cleanup test in `crates/malt-daemon/tests/contained_image_session.rs`.

**Checkpoint**: User Story 2 is complete only with a real HCS-contained
external command and verified cleanup; a green fake test alone is insufficient.

---

## Phase 5: User Story 3 - Operate and retire provisioned images safely (Priority: P2)

**Goal**: Operators list, inspect, and remove helper-owned images safely,
including in-use protection and post-host-change diagnosis.

**Independent Test**: Provision two records, start a contained session from
one, prove the in-use image is refused, then destroy the session and remove
only its record without affecting the other.

- [ ] T035 [P] [US3] Implement helper-side active-workspace/reference checks and owner-marked image removal transaction in `crates/malt-elevate/src/dispatch.rs`.
- [ ] T036 [US3] Implement daemon image-use reference accounting and reconciliation after uncertain helper results in `crates/malt-daemon/src/image_manager.rs`.
- [ ] T037 [US3] Add authenticated remove route/backend and `malt image remove` output/error behavior in `crates/malt-gateway/src/routes/images.rs`, `crates/malt-daemon/src/gateway_backend.rs`, and `crates/malt-bin/src/main.rs`.
- [ ] T038 [US3] Add integration tests for list/inspect active-use counts, in-use removal refusal, last-reference release, idempotent missing-record reporting, and stale-host reassessment in `crates/malt-daemon/tests/image_provisioning.rs`.
- [ ] T039 [US3] Add Windows-only removal test that verifies owned prepared state is gone and unrelated directories are untouched in `crates/malt-elevate/tests/image_cleanup.rs`.

**Checkpoint**: Every image lifecycle state is observable, and only unused,
MALT-owned artifacts can be removed.

---

## Phase 6: Polish, Live Evidence, and Spec 008 Return

**Purpose**: Prove the complete feature under its real privilege boundary,
preserve cross-platform honesty, and close only the Spec 008 tasks that the
new evidence actually satisfies.

- [ ] T040 [P] Add cross-platform unavailable responses and Docker-independent assertions to `crates/malt-daemon/tests/image_provisioning.rs` and `crates/malt-bin/src/main.rs`.
- [ ] T041 [P] Add negative protocol/fuzz-style bounds tests for malformed image references, response sizes, and archive metadata in `crates/malt-image/tests/` and `crates/malt-elevate/src/dispatch.rs`.
- [ ] T042 Update capability/status documentation and command help in `docs/BACKLOG.md`, `docs/design/architecture.md`, and `crates/malt-bin/src/main.rs` to distinguish acquired, prepared, and live-proven containment.
- [ ] T043 Run the complete Windows validation from `specs/009-windows-image-provisioning/quickstart.md`, capture exact host/image/helper/session/cleanup evidence in `docs/findings/2026-07-27-windows-image-provisioning.md`, and prove Docker engine independence.
- [ ] T044 Use the successful live evidence to check only the satisfied HCS/quickstart tasks in `specs/008-privileged-helper/tasks.md`; retain T030 unchecked unless a human has actually declined UAC.
- [X] T045 Run `cargo fmt --check`, strict clippy, the exact full `cargo test --workspace` suite, and the affected real Windows HCS tests; record fresh command outcomes in `docs/findings/2026-07-27-windows-image-provisioning.md`.
- [X] T046 Re-run `speckit-converge` for `specs/009-windows-image-provisioning/`, append any genuinely unbuilt work as new unchecked tasks, and only then mark the feature tasks complete.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: starts immediately after dependency approval.
- **Foundational (Phase 2)**: depends on the generated types; blocks all user
  stories because it owns integrity and privilege boundaries.
- **US1 (Phase 3)**: depends on Phase 2; delivers inspectable safe image
  provisioning.
- **US2 (Phase 4)**: depends on US1 because it consumes a ready image record
  and includes the actual HCS session proof.
- **US3 (Phase 5)**: depends on US2 because active-use protection consumes
  workspace lifecycle/reference accounting.
- **Polish (Phase 6)**: depends on all desired stories and is the only phase
  allowed to close matching Spec 008 tasks.

### User Story Dependencies

- **US1 (P1)**: Starts after Phase 2; independently testable without a
  contained session.
- **US2 (P1)**: Starts after US1; requires a ready selected image.
- **US3 (P2)**: Starts after US2; requires image-to-session references.

### Parallel Opportunities

- T003 and T004 can proceed after T001.
- T007 and T008 can proceed after T002; T011 can proceed once the platform
  module exists.
- T018 and T019 can proceed after the helper contract stabilizes.
- T035 can proceed alongside daemon reference-accounting design T036 once US2
  teardown semantics are known.
- T040 and T041 can proceed after US3 completes.

## Parallel Example: User Story 1

```text
Task: "Add daemon image client methods in crates/malt-daemon/src/elevate_client.rs"
Task: "Add CLI image grammar in crates/malt-bin/src/cli.rs"
```

These tasks still converge through the gateway/backend registration task and
must share the final typed helper protocol, not invent a second transport.

## Implementation Strategy

### MVP First

1. Complete Setup and Foundational phases.
2. Deliver US1 with real digest verification and helper-owned preparation.
3. Validate US1 against a public Windows image before adding session selection.
4. Deliver US2 and stop only after its live HCS external-command/cleanup
   proof exists.

### Incremental Delivery

1. Verified immutable image records.
2. Ready-image contained process execution.
3. Safe lifecycle/removal and host-change diagnosis.
4. Live evidence and exact return to Spec 008.

---

## Phase 7: Convergence

- [X] T047 Implement authoritative image-to-session reference accounting and expose it in image views; refuse removal with the dependent session identity rather than the current hard-coded `active_sessions: 0`, per FR-008/FR-009 and US3/AC1-2.
- [X] T048 Re-evaluate a prepared image against the current HCS/host version policy immediately before contained-session construction, preserving a concrete unavailable reason instead of accepting `record.prepared` as permanent readiness, per FR-003/FR-011.
- [X] T049 Make contained HCS process stdin lifecycle complete one-shot commands without a manual `malt eof`, while preserving explicit raw-input/EOF semantics for interactive commands, per US2/AC2 and SC-002.
- [x] T050 Persist and report the selected immutable image digest on a contained session creation/result surface, per US2/AC1 and SC-002.
- [X] T051 Add Windows-only real HCS layer, contained-command, and post-destroy cleanup tests plus dated Docker-independent live evidence, per FR-010/FR-012/FR-013 and SC-003/SC-005 (missing).
- [X] T052 Model and report acquired, HCS-prepared, and live-proven containment evidence as distinct image readiness states, per FR-003 and plan: lifecycle evidence decision.

---

## Phase 8: Convergence

- [X] T053 [US3] Diagnose and remediate the HCS prepared-layer removal sharing violation so an unused image removes its MALT-owned prepared state and record without restarting unrelated host services; add a Windows-real removal test that proves no prepared state remains after the last contained session. Evidence: `malt image remove sha256:852bbe55ef9eddac52f2e11b90d24d0d5b0d2518344ec813cf14891f76a8d47f` returned `DestroyLayer HRESULT=0x80070020` after helper restart, daemon restart, `active: 0`, and no MALT system in `hcsdiag list`, per FR-008/FR-010, US3/AC3, and SC-006 (partial).

---

## Phase 9: Convergence

- [ ] T054 [US3] Add authenticated integration coverage for two distinct helper-owned image records: hold one in an active contained session, prove its removal names that session, and prove removal of the other record does not remove or alter the first record or workspace, per US3/AC1-3 and SC-006 (partial).
- [ ] T055 [US2] Add a deterministic post-workspace HCS-construction failure seam and integration proof that required containment publishes no session and leaves no compute system or helper-owned writable workspace, per US2/AC3, FR-006, FR-010, and SC-003 (partial).
- [X] T056 [US1] Record a repeatable quickstart refusal check for a non-Windows/wrong-platform image reference and an invalid immutable reference, proving neither leaves a selectable helper-owned image record, per US1/AC2, FR-002, and SC-004 (partial).
