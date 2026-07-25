# Implementation Plan: Command Lifecycle Event Delivery

**Branch**: `004-command-lifecycle-events` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/004-command-lifecycle-events/spec.md`

## Summary

Clients subscribe to a session and receive `CommandStarted`/`CommandFinished` events as they happen, with resume-after-disconnect and a defined slow-consumer policy. The producer side already exists: the command worker announces `SessionCommand::ExecutionStarted` before running and `ExecutionCompleted` with the result, and the control actor handles both (that pair drives command history from feature 003). This feature is the delivery path.

Two decisions dominate and are settled in [research.md](research.md):

1. **Transport: a Gateway SSE endpoint** (`GET /sessions/{id}/events`), not a VNP channel. ADR-0002 makes the Gateway canonical and the spec's P1 story is an agent. VNP forwarding for `malt-tui` is a deliberate follow-up, not a silent omission (R1).
2. **A first-party consumer ships with the endpoint.** `malt-bin` gains `malt watch <ID>`, which does the real reconnect-with-`Last-Event-ID` loop. Without it the resume path would ship exercised only by `curl` and by tests driving the backend directly — the "built, unit-tested, never actually driven" pattern this project keeps repeating. The full `malt-gateway-sdk` crate is explicitly *not* built here; the decisive test was whether designing for it would change this contract, and it does not (R10).
3. **Delivery does *not* go through the existing `Bus`.** The Bus's `Reliable` priority is documented and implemented to grow its ring buffer beyond capacity rather than evict — directly contradicting FR-009/FR-010, which require a stalled subscriber to cause no unbounded growth. Since `CommandStarted`/`CommandFinished` are specified as `Reliable`, routing them through the Bus would build the exact failure this spec exists to prevent. Delivery instead uses a bounded per-subscriber channel following the established `render_pushers` precedent (R2). **This changes the framing recorded in `docs/BACKLOG.md`** ("give the Bus its first real consumer") and is called out rather than quietly re-scoped.

## Technical Context

**Language/Version**: Rust, edition 2021

**Primary Dependencies**: `axum` 0.8 (SSE via `axum::response::sse`), `tokio` 1.x (`sync` feature — already a `malt-daemon` dependency), `malt-protocol` (`CommandStarted`/`CommandFinished` already schema-defined in `schemas/shell.vexil` with domain/type constants), `malt-daemon` executor, `malt-gateway`.

**Storage**: None persistent. The catch-up window is an in-memory bounded ring per session, deliberately not durable — command history (feature 003) already covers durability (spec Assumptions).

**Testing**: `cargo test --workspace`. New coverage in `malt-daemon/tests/` (subscription lifecycle, catch-up, lag policy) and `malt-gateway/tests/routes.rs` (endpoint shape, auth scope, SSE framing).

**Target Platform**: Windows native + WSL/Linux; no platform-specific code.

**Performance Goals**: Event reaches a connected subscriber in well under 1 s from the underlying transition (SC-004). Publishing must be non-blocking on the session control actor — a subscriber must never add measurable latency to command execution (SC-006).

**Constraints**: Bounded memory per subscriber (default 256 buffered events) and a bounded per-session catch-up window (default 1024 events). No blocking of the control actor. No blocking of a tokio worker for the lifetime of a stream — which rules out reusing the `recv_timeout`-on-`std::mpsc` idiom `exec_command` uses (R3).

**Scale/Scope**: Tens of concurrent subscribers per daemon is the realistic target (agents plus a human client), not thousands.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. VT Codes Confined | ✅ Pass | No VT/escape handling. Events carry command text and status, never terminal output. |
| II. OS Calls Confined | ✅ Pass | No direct OS calls; timestamps via `std::time`, already used. |
| III. Dependency-Free Foundations | ✅ Pass | `malt-protocol` gains nothing new — the two messages already exist. `malt-plugin-sdk` untouched. |
| IV. Safety Is Explicit | ✅ Pass | No `unsafe`; no `unwrap()`/`expect()` outside tests. Tests must exercise real subscribe → run → receive → disconnect → resume paths, not construct event structs and assert on their fields. |
| V. VNP Is the Only Inter-Component Protocol | ✅ Pass — with reasoning | The Gateway's HTTP+JSON surface is the *external* API boundary, made canonical by ADR-0002, not an ad-hoc side-channel between components behind the protocol boundary. SSE is that same sanctioned surface in streaming form, carrying the schema-defined messages rendered to JSON exactly as `get_output`/`exec` already render their payloads. The VNP client path is not bypassed — it is deferred (R1), and the daemon-side event source is transport-neutral precisely so VNP forwarding can be added without redesign. |
| VI. Shell Ships on POSIX Conformance | ✅ N/A | `mash` untouched. |
| VII. Layer Violations Are Compile Errors | ✅ Pass | Dependencies point downward; no upward edges. |
| VIII. Vendor, Never Depend on Unstable Siblings | ✅ N/A | No sibling-project code. |
| IX. No Silent Scope-Jumps | ⚠️ Pass, with one flagged deviation | The Bus decision (R2) contradicts the backlog's stated framing of this work. It is written down here and in research.md rather than absorbed silently, and the Bus's own unresolved status becomes a separate backlog item rather than being fixed opportunistically inside this feature. |
| X. Commit at Real Checkpoints | ✅ Pass | tasks.md sequences a commit per user story. |

**Post-Phase-1 re-check**: the design introduces no new violations. The item needing continued attention is IX — the Bus's `Reliable`-grows-unbounded policy is now a known, documented defect that this feature routes around rather than repairs. Recording it in `docs/BACKLOG.md` is part of this work, not a follow-up someone might forget.

## Project Structure

### Documentation (this feature)

```text
specs/004-command-lifecycle-events/
├── spec.md
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── event-stream.md  # SSE endpoint + event payload contract
├── checklists/
│   └── requirements.md  # Complete
└── tasks.md             # Phase 2 (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
crates/
├── malt-daemon/
│   ├── src/executor/
│   │   ├── events.rs            # NEW: LifecycleEvent, EventLog (bounded ring +
│   │   │                        #   monotonic sequence), SubscriberSink, lag policy
│   │   ├── session_thread.rs    # Publish on ExecutionStarted/ExecutionCompleted;
│   │   │                        #   own the EventLog + sinks; Subscribe/Unsubscribe
│   │   └── coordinator.rs       # begin_subscribe_events() passthrough
│   ├── src/gateway_backend.rs   # GatewayBackend::subscribe_events impl
│   └── tests/
│       ├── events.rs            # NEW: subscription lifecycle, catch-up, lag
│       └── gateway_backend.rs   # End-to-end via the backend
├── malt-gateway/
│   ├── src/backend.rs           # Trait method + EventSubscription/LifecycleEventDto
│   ├── src/routes/sessions.rs   # GET /sessions/{id}/events (SSE)
│   ├── src/server.rs            # Route registration
│   ├── src/middleware.rs        # required_scope entry (Read)
│   └── tests/routes.rs          # Endpoint + auth-scope tests
├── malt-bin/
│   ├── src/events.rs            # NEW: SSE frame parser + reconnect/resume loop.
│   │                            #   Deliberately free of CLI assumptions -- this is
│   │                            #   the nucleus malt-gateway-sdk later extracts (R10)
│   ├── src/cli.rs / main.rs     # `malt watch <ID>` subcommand
│   └── src/client.rs            # Streaming request (reqwest blocking Response: Read)
└── malt-mcp/                    # NOT touched — see research R8
```

**Structure Decision**: Two new modules. `executor/events.rs` holds the event log, subscriber sinks, and lag policy as a unit, because those three are only correct together — a bounded log without a lag policy is exactly the failure mode FR-010 describes. `malt-bin/src/events.rs` holds the client-side frame parser and reconnect loop, kept free of CLI-specific assumptions so the SDK extraction later is a move rather than a rewrite. Everything else extends existing files along the path features 002/003 established (`SessionCommand` variant → `Coordinator` `begin_*` passthrough → `GatewayBackend` method → route → middleware scope entry).

## Complexity Tracking

No constitution violations requiring justification. The single flagged item (IX, the Bus deviation) is a documented decision with its reasoning in research.md R2, not an unjustified violation.
