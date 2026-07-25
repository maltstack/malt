# Implementation Plan: Streaming Command Output

**Branch**: `006-streaming-command-output` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/006-streaming-command-output/spec.md`

## Summary

A command's output is buffered inside `mash` until the command ends, so
nothing downstream can be incremental. This feature makes the top-level
command's stdout and stderr flow as they are produced: `mash` writes them to
an output sink instead of accumulating, the worker forwards chunks to the
session's control actor over the existing ordered channel, and the control
actor feeds the terminal grid and publishes to output subscribers using the
same bounded, lag-reporting policy that command lifecycle events already use.

The prerequisite — command execution no longer occupying the session's control
path — was delivered by feature 002, so the control actor is free to do this
work while a command runs.

## Technical Context

**Language/Version**: Rust, edition 2021

**Primary Dependencies**: existing workspace only — `vte` 0.15 (via
`malt-compat`), `axum` 0.8 (Gateway), `vexil-runtime` (VNP bitpack). No new
external dependencies: `base64` is already in `Cargo.lock` as a transitive
dependency, so the HTTP transport encoding (R6) needs only a direct
declaration in `malt-gateway`, not a new third-party package.

**Storage**: none. Retained output is an in-memory bounded ring; this feature
adds nothing to the session store and does not make output durable.

**Testing**: `cargo test --workspace`; Smoosh POSIX conformance
(`cargo test -p mash --test smoosh_runner`) is a **gate**, not a note,
because `mash`'s executor is modified.

**Target Platform**: Windows-native primary; must not regress Linux/macOS.

**Project Type**: Multi-crate Rust workspace (daemon + shell + clients).

**Performance Goals**: first output observable within 1 s of being produced
(SC-001); at least 10 updates over a 30 s output-producing command (SC-002).

**Constraints**: bounded daemon memory regardless of output volume (SC-004,
100 MB command); byte-for-byte fidelity including split multi-byte characters
(SC-006); a stalled subscriber must not delay the command or other
subscribers (SC-005).

**Scale/Scope**: one command at a time per session; a handful of subscribers
per session.

## Constitution Check

*GATE: checked before Phase 0 research, re-checked after Phase 1 design.*

| Principle | Assessment |
|---|---|
| **I. VT Codes Confined** | PASS. Chunks are opaque bytes everywhere except `malt-compat`, which already owns `feed()`. No new crate parses escape sequences. |
| **II. OS Calls Confined** | PASS. Reading a child's output incrementally happens through `mash`'s existing `malt-platform` process abstractions. If the external branch needs a non-blocking read, that primitive belongs in `malt-platform`, not in `mash`. |
| **III. Dependency-Free Foundations** | PASS, with care. New message shapes go in `malt-protocol` and must use only external deps. A base64 dependency belongs in the Gateway, **not** in `malt-protocol`. |
| **IV. Safety Is Explicit** | PASS. No `unsafe` anticipated. No `unwrap()`/`expect()` outside tests. Note the principle's own warning: tests must exercise real paths — a streaming test that feeds a translator directly proves nothing about whether a running command's output reaches anyone. |
| **V. VNP Only** | PASS. Chunks to attached clients travel as typed VNP messages. The HTTP stream is a Gateway surface, consistent with the existing events stream, not a component-to-component side channel. |
| **VI. Shell Ships on POSIX Conformance** | **Gate applies.** `mash`'s executor is being changed. Smoosh 183/183 (Windows) must hold, and it is the check most likely to catch a mistake in the capture/stream distinction (R2) — command substitution and pipelines are heavily covered there. |
| **VII. Layering** | PASS. `mash` (L1) gains a sink abstraction it owns; the daemon (L2) provides the implementation. No upward dependency. |
| **VIII. Vendor, Never Depend on Unstable Siblings** | N/A. |
| **IX. No Silent Scope-Jumps** | Watch item. Two adjacent temptations: fixing the known grid "staircase" defect that streaming will make more visible (R7), and converting `ToolFn` to a writer beyond what US4 needs. Both are out of scope; if either looks necessary, it gets written down, not absorbed. |
| **X. Commit at Real Checkpoints** | Each user story is a checkpoint with its own gates and merge, as in 003–005. |

**Result: PASS.** No violations to justify; the Complexity Tracking table is
therefore omitted.

**Post-Phase-1 re-check: PASS.** The design adds one sink trait in `mash`, one
`SessionCommand` variant, one retained-output structure modelled on the
existing event log, one VNP message, and one HTTP route. No new crate, no new
layer, no new protocol.

## Project Structure

### Documentation (this feature)

```text
specs/006-streaming-command-output/
├── spec.md              # Feature specification
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── output-stream-http.md
│   └── output-chunk-vnp.md
├── checklists/
│   └── requirements.md
└── tasks.md             # Created by /speckit-tasks, not here
```

### Source Code (repository root)

```text
crates/
├── mash/src/
│   ├── env.rs                    # output sink handle on Env; cleared for
│   │                             #   command substitution and subshells
│   └── executor.rs               # write top-level stdout/stderr to the sink
│                                 #   instead of accumulating; capture and
│                                 #   pipeline paths unchanged
├── malt-protocol/                # regenerated OutputChunk (already exists)
├── malt-daemon/src/executor/
│   ├── command_worker.rs         # sink impl; forward chunks to control
│   ├── session_thread.rs         # OutputChunk SessionCommand; feed compat
│   │                             #   incrementally; dispatch_render per chunk
│   ├── output_log.rs             # NEW: retained ring + subscriber sinks,
│   │                             #   modelled on events.rs (share where the
│   │                             #   shapes are genuinely identical)
│   └── coordinator.rs            # subscribe/resume entry points
├── malt-gateway/src/
│   ├── routes/sessions.rs        # GET /sessions/{id}/output/stream (SSE)
│   ├── server.rs                 # route registration
│   └── middleware.rs             # Read scope for the new route
├── malt-tui/src/                 # consumes chunks via existing ClientMessage
└── malt-bin/src/                 # first-party stream consumer

schemas/
└── shell.vexil                   # EXTEND the existing OutputChunk (0x04) --
                                  #   do not add a second output message
```

**Structure Decision**: No new crate. The work lands in the crates that
already own each concern: `mash` owns command execution and therefore where
output originates; `malt-daemon` owns session state, retention, and
subscribers; `malt-gateway` owns the HTTP surface; `malt-compat` is untouched
because it is already incremental.

## Implementation Order and Risk

The order is dictated by R1: nothing downstream can be tested until `mash`
emits incrementally, so the sink lands first even though it is the least
visible part.

1. **`mash` output sink** — the capture/stream distinction (R2) is the
   highest-risk change in the feature, because getting it wrong corrupts
   command substitution and pipelines rather than merely failing to stream.
   Smoosh is the check that catches this.
2. **Worker → control chunk transport** — bounded, ordered, blocking-on-full
   (R4).
3. **Retained ring + subscribers** (R5) and the HTTP stream (R6, R8) — US1
   and US3 become observable here.
4. **Incremental compat feed + render dispatch** (R7) — US2.
5. **In-process tool streaming** (US4) — the `ToolFn` writer change, last
   because it is the narrowest slice and depends on everything above.

**The known failure mode for this feature**, based on 003–005: building the
delivery path and verifying it with tests that inject chunks directly, while
no real command's output ever reaches it. Every story's acceptance evidence
must start from a command actually running.
