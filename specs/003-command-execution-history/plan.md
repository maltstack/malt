# Implementation Plan: Persistent Command Execution History

**Branch**: `003-command-execution-history` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/003-command-execution-history/spec.md`

## Summary

Every command executed in a session pane gets a durable execution record (command text, start time, finish time, exit status). The mechanism already half-exists: `malt-session::pane::CommandBlock` and its ring-buffered `PaneRuntime` container are fully built and unit-tested but have zero non-test constructors anywhere in the daemon (Gap A), and the persistence schema has no field to carry history across a daemon restart (Gap B). This plan closes both gaps as one coordinated change — record construction in `run_mash_command`, a `PersistedCommandBlock` schema addition with persist/restore wiring, and retrieval surfaced through a new `SessionCommand`, a Gateway endpoint, the CLI, and the MCP server. As a deliberate side effect, restoring history lets `next_command_id` resume from the highest persisted id instead of resetting to 0 on restore (a known BACKLOG caveat).

## Technical Context

**Language/Version**: Rust, edition 2021 (workspace-wide)

**Primary Dependencies**: `malt-session` (L1, owns `CommandBlock`/`PaneRuntime`), `malt-protocol` (generated schema types), `malt-daemon` (L2, session executor + persistence), `malt-gateway` (L2, axum 0.8 HTTP), `malt-bin`/`malt-mcp` (L3 clients), `vexilc` CLI (schema compilation, installed at `~/.cargo/bin/vexilc.exe` v0.5.1)

**Storage**: Existing session persistence path — bitpack `.vxb` via `SessionStore`/`DebouncedStore` (atomic write + `.bak` + corruption quarantine). History rides inside `PersistedPane`; no new storage system.

**Testing**: `cargo test --workspace` (1,200+ tests green baseline). New tests in `malt-session` (ring-buffer edge cases already covered; add restore-seeding), `malt-daemon/tests/{session_thread,gateway_backend,store,coordinator}.rs`, `malt-gateway/tests/routes.rs`.

**Target Platform**: Windows native + WSL/Linux (same as workspace; no platform-specific code in this feature)

**Project Type**: Multi-crate Rust workspace — daemon-authoritative terminal platform

**Performance Goals**: History retrieval at the 1,000-entry retention bound returns in well under 1 s (SC-004); record construction adds negligible overhead per command (two timestamps + one ring-buffer push).

**Constraints**: Retention bound 1,000 entries/pane (existing `DEFAULT_MAX_BLOCKS`); persisted size stays bounded by the same cap; no history data loss across dormant→restore for retained entries; access control reuses existing `AuthScope` model (Read scope for retrieval).

**Scale/Scope**: One shell pane per session today (`first_pane`); schema and data model are per-pane so multi-pane sessions inherit correctness later without schema change.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. VT Codes Confined | ✅ Pass | No VT/escape handling anywhere in this feature. |
| II. OS Calls Confined | ✅ Pass | Only `std::time::SystemTime` (already used in `session_thread.rs:523`), no `nix`/`windows-sys`/`libc`. |
| III. Dependency-Free Foundations | ✅ Pass | `malt-protocol` gains only vexilc-generated types from the schema (external deps only, unchanged). `malt-plugin-sdk` untouched. |
| IV. Safety Is Explicit | ✅ Pass | No `unsafe`; no `unwrap()`/`expect()` outside tests. Tests exercise the real persist→restore→retrieve path, not struct construction. |
| V. VNP Only Inter-Component Protocol | ✅ Pass | Persistence uses the bitpack schema; retrieval uses the pre-existing, sanctioned Gateway HTTP surface (same boundary as `get_output`). No new side-channels. |
| VI. Shell Ships on POSIX Conformance | ✅ N/A | `mash` semantics untouched — the executor records around `execute_list`, not inside it. |
| VII. Layer Violations Are Compile Errors | ✅ Pass | All deps point downward: L2 `malt-daemon` consumes L1 `malt-session` + L0 `malt-protocol`; L3 clients consume the Gateway. |
| VIII. Vendor, Never Depend on Unstable Siblings | ✅ N/A | No sibling-project code involved. |
| IX. No Silent Scope-Jumps | ✅ Pass | Scope is exactly BACKLOG's Gap A + Gap B + retrieval surface. Anything bigger (e.g. full scrollback-per-command, event streaming) is explicitly out of scope (see research.md R5). |
| X. Commit at Real Checkpoints | ✅ Pass | tasks.md will sequence commits per user story (record → persist → retrieve surfaces). |

**Post-Phase-1 re-check**: design artifacts below introduce no new violations — schema additions are additive (`@non_exhaustive`-compatible optional field), no layering changes, no new dependencies. Gate still passes.

## Project Structure

### Documentation (this feature)

```text
specs/003-command-execution-history/
├── spec.md              # Feature specification
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── command-history.md   # Gateway/CLI/MCP retrieval contract
├── checklists/
│   └── requirements.md  # Spec quality checklist (complete)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
schemas/
└── persist/session.vexil            # + PersistedCommandBlock message; + command_blocks field on PersistedPane

crates/
├── malt-session/
│   └── src/pane.rs                  # CommandBlock/PaneRuntime (exists; + seed-from-restore constructor path)
├── malt-daemon/
│   ├── src/executor/
│   │   ├── session_thread.rs        # Record construction in run_mash_command; PaneRuntime ownership;
│   │   │                            #   SessionCommand::GetCommandHistory; persist/restore wiring;
│   │   │                            #   next_command_id resume-from-history
│   │   └── coordinator.rs           # get_command_history() passthrough
│   ├── src/gateway_backend.rs       # GatewayBackend::get_command_history impl
│   └── tests/
│       ├── session_thread.rs        # Record-construction tests
│       ├── gateway_backend.rs       # End-to-end incl. dormant→restore survival
│       └── store.rs                 # PersistedCommandBlock round-trip
├── malt-gateway/
│   ├── src/backend.rs               # Trait method + CommandHistoryEntry response type
│   ├── src/routes/sessions.rs       # GET /sessions/{id}/history handler
│   ├── src/server.rs                # Route registration
│   ├── src/middleware.rs            # required_scope entry (Read)
│   └── tests/routes.rs              # Route + auth-scope tests
├── malt-bin/
│   ├── src/cli.rs / main.rs         # `malt history ID` subcommand
│   └── src/client.rs                # get_command_history() client call
└── malt-mcp/
    └── src/main.rs                  # get_command_history MCP tool (7th tool)
```

**Structure Decision**: No new crates, no new modules — every change lands in an existing file of an existing crate, following the exact pattern the codebase already uses for `get_output_text` (SessionCommand variant → Coordinator passthrough → GatewayBackend method → route → CLI/MCP client). The only schema-bearing change is additive in `schemas/persist/session.vexil`.

## Complexity Tracking

No constitution violations to justify — table intentionally omitted.
