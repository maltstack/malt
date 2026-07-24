# Implementation Plan: Responsive Session Control During Execution

**Branch**: `002-responsive-session-control` | **Date**: 2026-07-25 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-responsive-session-control/spec.md`

## Summary

Keep each session's control plane responsive while a synchronous MASH command
runs by splitting the current session thread into a responsive control actor
and one serial execution worker. The worker exclusively owns the authoritative
`mash::Env`; every command-producing path uses one bounded FIFO execution
ingress, and the worker cannot start the next command until the control actor
has finalized the prior result. Attach, observation, rendering controls,
snapshots, detach, and shutdown remain on the control actor and use the last
fully finalized view while execution is active.

## Technical Context

**Language/Version**: Rust 2021; workspace MSRV 1.85 (planning environment: rustc 1.97.0)

**Primary Dependencies**: Rust standard-library threads and `std::sync::mpsc`; `mash` synchronous parser/executor; `malt-compat`; `malt-renderer`; `malt-session`; `malt-gateway`/axum 0.8; existing VNP types from `malt-protocol`

**Storage**: Existing bitpack `SessionStore` and `DebouncedStore`; no persistence-schema change

**Testing**: `cargo test` with focused daemon, coordinator, Gateway, and VNP integration tests; full workspace regression; native-Windows Smoosh POSIX conformance

**Target Platform**: Native Windows, Linux, and macOS daemon operation; primary verification on Windows

**Project Type**: Multi-crate Rust terminal platform with daemon, HTTP Gateway, VNP socket service, and native clients

**Performance Goals**: Attach and all specified non-execution controls complete within 1 second during a 60-second command; 1,000 ordered trials show exactly-once FIFO execution and no overlapping shell mutation; an indefinitely busy session does not fail responsiveness checks in another session

**Constraints**: One active command per session; 256 waiting commands by default, excluding the active command; nonblocking overload rejection; no `Env` cloning or shared lock; final result and shell state finalize before the next command starts; no intermediate output visibility; no command cancellation; existing 30-second Gateway result wait remains; no new dependency or protocol

**Scale/Scope**: Per-session executor/control architecture in `malt-daemon`, explicit HTTP error mapping in `malt-gateway`, and lifecycle integration in the daemon coordinator; no client, schema, public execution-history, raw-input, or authority-policy expansion

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Pre-research gate: PASS.**

| Constitution principle | Plan assessment |
|---|---|
| VT codes confined | Only the existing `malt-compat::CompatTranslator` handles command bytes. Bounded finalization is orchestrated by the daemon without importing `vte` or interpreting escapes. |
| OS calls confined | Execution still reaches OS functionality through MASH and `malt-platform`; no direct OS dependency is added. |
| Dependency-free foundations | `malt-protocol` and `malt-plugin-sdk` are unchanged. |
| Safety is explicit | No unsafe code is required. Admission, worker failure, channel closure, and shutdown have typed outcomes instead of panics or silent loss. |
| VNP only | Existing VNP messages and the post-handshake bitpack path remain unchanged; no side channel or schema is added. |
| Shell conformance | The original authoritative `mash::Env` moves to the worker once, and commands still use the existing synchronous MASH path. Smoosh remains a release gate. |
| Layering | Work stays in existing L2 daemon/Gateway/compat relationships and L3 daemon startup integration; no upward dependency is introduced. |
| Vendor policy | No sibling or external dependency is added. |
| No silent scope-jumps | Raw stdin, streaming, cancellation, public execution identity/history, authority policy, persistence expansion, and client work remain excluded exactly as specified. |
| Real checkpoints | The implementation can checkpoint the ingress/worker split, lifecycle handling, and external contract verification as independently testable changes. |

**Post-design gate: PASS.** The Phase 1 model and contracts retain all crate,
protocol, persistence, isolation, and client boundaries. The execution worker
owns the existing `Env`; the control actor owns presentation and lifecycle
state; no constitutional exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/002-responsive-session-control/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/
│   └── session-control.md
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
crates/malt-daemon/
├── src/
│   ├── error.rs                     # Typed queue/full/closing/unavailable outcomes
│   ├── gateway_backend.rs           # Nonblocking execution admission and result wait
│   └── executor/
│       ├── mod.rs
│       ├── command_worker.rs        # New single-owner MASH execution worker
│       ├── coordinator.rs           # Lifecycle, dormancy, shutdown, asynchronous reaping
│       ├── pools.rs                 # Pending-execution capacity semantics
│       └── session_thread.rs        # Responsive control actor and bounded finalization
└── tests/
    ├── coordinator.rs
    ├── gateway_backend.rs
    ├── session_thread.rs
    └── vnp_listener.rs

crates/malt-gateway/
├── src/error.rs                     # Stable HTTP status/error-code mapping
└── tests/                           # Route-level overload/failure contract coverage

crates/malt-bin/
└── src/daemon.rs                    # Prompt daemon shutdown without blocking on a command

schemas/
├── persist/session.vexil            # Existing persistence contract; unchanged
└── *.vexil                          # Existing VNP contracts; unchanged
```

**Structure Decision**: Add one daemon-internal worker module and keep the
existing session thread as the control actor. The daemon remains the only
session authority; the Gateway only maps typed daemon outcomes, and the binary
only ensures process shutdown does not synchronously wait on an uncancellable
command. Listed schema files are compatibility boundaries and are not planned
modification targets.
