# Implementation Plan: Authenticated Raw Input with Input Authority

**Branch**: `005-raw-input-authority` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/005-raw-input-authority/spec.md`

## Summary

Close the unauthenticated VNP control plane, then make input from a client reach a command that is blocked reading — byte-for-byte, attributed to a client, and arbitrated so exactly one client types at a time.

Research turned up two things that shape the whole plan:

1. **Raw input is much cheaper than expected.** `mash`'s `read` builtin already resolves its source as `env.open_fd_read(0)` before falling back to the daemon's console. Registering a session pipe at fd 0 makes `read` take client input **with no change to the builtin** — the fall-through that audit A-07 flags simply stops being reachable (R2).
2. **Authentication has a dependency the spec did not anticipate.** Audit A-03 found the Gateway's token is derived from epoch nanoseconds rather than a CSPRNG. Authenticating VNP with a guessable token is a lock whose key can be recomputed, so **A-03 is in scope for User Story 1** rather than a parallel cleanup (R1).

## Technical Context

**Language/Version**: Rust, edition 2021

**Primary Dependencies**: `malt-platform` (`io::create_pipe`, `vfs::SharedFdRegistry`, `process::Io`, `sockets::Transport`), `mash` (`Env::register_fd`, the `read` builtin, external spawn), `malt-daemon` (VNP listener, session executor, `AuthorityTracker`), `malt-gateway` (`TokenStore`, `AuthScope`). An OS CSPRNG source is the only likely new dependency, pending R1's token fix.

**Storage**: None new. Retained type-ahead is session-lifetime only, deliberately not persisted (R6).

**Testing**: `cargo test --workspace`, plus the four gates now green (build, test, fmt, clippy). New coverage in `malt-daemon/tests/` (handshake rejection, raw input, authority), `mash` (fd-0 sourced `read`), and `malt-gateway/tests/` (token generation).

**Target Platform**: Windows native + WSL/Linux. `create_pipe` and `Transport` are already cross-platform; peer-credential authentication is *not* attempted here precisely because it is not (R1).

**Performance Goals**: Input reaches a blocked reader without perceptible delay. Authentication adds one check at connection setup, not to steady-state traffic.

**Constraints**: The control actor must never block — writes to a full input pipe must be non-blocking and refused explicitly, mirroring the `try_send` discipline features 002/004 established. Raw input must not reach `run_mash_command`, or it acquires a `command_id`, a persisted `CommandBlock`, and a published lifecycle event carrying its text (R7).

**Scale/Scope**: One shell pane per session; a handful of concurrent clients per session (a human and one or more agents).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. VT Codes Confined | ✅ Pass | Raw input is bytes to a pipe; no escape-sequence interpretation is added anywhere. |
| II. OS Calls Confined | ✅ Pass — and improves | Pipe creation goes through `malt_platform::io::create_pipe`, fd placement through `malt-platform`'s `SharedFdRegistry`. No new OS calls outside the platform crate. |
| III. Dependency-Free Foundations | ⚠️ Watch | `malt-protocol` may need extended handshake and new authority messages. Schema additions are fine (external deps only); the constraint is that no *workspace* dependency is added to it. |
| IV. Safety Is Explicit | ✅ Pass | No `unsafe` expected. Tests must drive the real VNP path — `AuthorityTracker` passes its own unit tests *today* while being unreachable, which is exactly the trap this feature exists to close (R5). |
| V. VNP Is the Only Inter-Component Protocol | ✅ Pass — and strengthens | Authentication and authority are added *to* VNP as typed frames rather than beside it. |
| VI. Shell Ships on POSIX Conformance | ⚠️ Gate | `mash` is modified (fd-0 sourced `read`, external-process stdin). Smoosh must stay 183/183 on native Windows — a regression there is a blocker, not a footnote. |
| VII. Layer Violations Are Compile Errors | ✅ Pass | All dependencies point downward. |
| VIII. Vendor, Never Depend on Unstable Siblings | ✅ N/A | No sibling-project code. |
| IX. No Silent Scope-Jumps | ⚠️ Pass, with one declared expansion | Audit A-03 (token CSPRNG) is pulled into US1, declared here with its reason: VNP authentication built on a guessable token does not satisfy its own acceptance scenario. Transport migration to peer credentials is *not* pulled in and is backlogged (R1). |
| X. Commit at Real Checkpoints | ✅ Pass | Four merge checkpoints, one per story (R8). |

**Post-Phase-1 re-check**: no new violations. Two items to keep watching: VI (Smoosh conformance, because `mash` is being modified) and IX (the A-03 expansion must not grow further — the transport migration stays out).

## Project Structure

### Documentation (this feature)

```text
specs/005-raw-input-authority/
├── spec.md
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── input-and-authority.md
├── checklists/
│   └── requirements.md  # Complete
└── tasks.md             # Phase 2 (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
crates/
├── malt-gateway/
│   └── src/auth.rs               # US1: CSPRNG token, atomic owner-only write,
│                                 #   fail startup rather than silently not persist
├── malt-bin/
│   └── src/daemon.rs             # US1: stop printing the token to stdout
├── malt-daemon/
│   ├── src/vnp_listener.rs       # US1: authenticate before disclosing the session
│   │                             #   inventory; bounded pre-identification deadline
│   │                             #   and connection cap (A-08). US3/US4: claim and
│   │                             #   authority-changed frames
│   ├── src/connection/authority.rs  # US3/US4: reachable at last — driven from the
│   │                             #   real attach path, not only from tests
│   ├── src/executor/
│   │   ├── input.rs              # NEW: session input pipe, bounded non-blocking
│   │   │                         #   write, retained-input policy
│   │   ├── session_thread.rs     # US2: own the pipe, register fd 0; US3: reject
│   │   │                         #   input from non-holders
│   │   └── coordinator.rs        # passthroughs
│   └── src/gateway_backend.rs    # US2: send_input writes raw bytes instead of
│                                 #   submitting an execution
├── mash/
│   └── src/executor.rs           # US2: external-process stdin from fd 0 when
│                                 #   registered, instead of Io::Inherit
├── malt-protocol/
│   └── (schemas/*.vexil)         # US1: handshake auth field; US3/US4: input-claim
│                                 #   and authority-changed messages
└── malt-tui/
    └── src/connection.rs         # US1: present the token; US4: surface authority
                                  #   changes to the user
```

**Structure Decision**: One new module (`executor/input.rs`) holds the session input channel and its bounded-write policy, mirroring how `executor/events.rs` holds the event log and its lag policy — same reason, that a buffer without a defined full-behavior *is* the failure mode. Everything else extends existing files. `AuthorityTracker` is **not** rewritten; it is connected.

## Complexity Tracking

| Item | Why needed | Simpler alternative rejected because |
|------|-----------|--------------------------------------|
| Audit A-03 (token CSPRNG) pulled into US1 | VNP authentication is only as strong as the credential it checks; an epoch-nanosecond token is recomputable by anyone who can approximate daemon start time | Doing A-03 afterwards means US1 ships without honestly meeting its own acceptance scenario, "a client that cannot prove its identity is refused" |
| A second input path beside command submission | A prompt answer is not a command; routing it through `run_mash_command` gives it a `command_id`, a persisted `CommandBlock`, and a lifecycle event carrying its text | Filtering input out of history and events after the fact leaves the leak one refactor away from returning; keeping it off that path is structural |
