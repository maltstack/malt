# Implementation Plan: CLI Isolation Flag

**Branch**: `001-cli-isolation-flag` | **Date**: 2026-07-24 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-cli-isolation-flag/spec.md`

## Summary

Expose the existing session-isolation choice through `malt new` with a typed,
canonical lower-case command-line value. Carry an explicitly selected tier
through the existing session-creation client, verify that the returned session
reports the expected tier, and include that tier in the successful command
output. Preserve the current no-flag payload and Bare default, and leave the
no-subcommand automatic workflow untouched.

## Technical Context

**Language/Version**: Rust 2021 (workspace edition)

**Primary Dependencies**: clap 4 with derive support; reqwest 0.13 blocking JSON client; serde/serde_json; anyhow

**Storage**: N/A for this feature; session persistence remains owned by the daemon

**Testing**: `cargo test -p malt-bin`; focused pure unit tests for command parsing, request construction, and creation-result validation; existing workspace suite for regression

**Target Platform**: Native Windows development environment; the command contract is platform-independent and delegates availability to the daemon

**Project Type**: Rust CLI client in a multi-crate terminal-platform workspace

**Performance Goals**: No new persistent work or round trips beyond the existing single session-creation request

**Constraints**: Preserve the existing request shape when `--isolation` is omitted; accept only four lower-case spellings; never print a successful creation result after a creation failure or reported-tier mismatch; do not change VNP, daemon authority, platform enforcement, or other CLI commands

**Scale/Scope**: One subcommand option and its client payload/result handling, confined to `crates/malt-bin`; one existing session-creation request per invocation

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Pre-research gate: PASS.**

| Constitution principle | Plan assessment |
|---|---|
| VT codes confined | No VT parsing or terminal escape handling is introduced. |
| OS calls confined | The CLI makes no OS or platform-isolation calls. |
| Dependency-free foundations | `malt-protocol` and `malt-plugin-sdk` are unchanged. |
| Safety is explicit | No unsafe code is needed; all fallible command/client paths return explicit errors. |
| VNP only | The existing gateway-facing CLI path is retained; no new inter-component channel is added. |
| Shell conformance | MASH behavior is untouched. |
| Layering | Changes remain in the L3 CLI consumer and use its existing dependencies. |
| Vendor policy | No dependency is added. |
| No silent scope-jumps | The plan exposes the backlog capability only. Direct gateway input hardening and platform enforcement gaps are recorded as pre-existing authority concerns, not folded into this CLI feature. |
| Real checkpoints | The feature has a discrete Spec Kit artifact set and focused verification boundary. |

**Post-design gate: PASS.** The data model and command contract retain the
same boundaries: no lower-layer or dependency change is required.

## Project Structure

### Documentation (this feature)

```text
specs/001-cli-isolation-flag/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
crates/malt-bin/
├── src/
│   ├── cli.rs          # Command schema and typed isolation argument
│   ├── client.rs       # Create-session request payload and response handling
│   └── main.rs         # New-command dispatch, result validation, user output
└── Cargo.toml

crates/malt-gateway/
└── src/types.rs        # Existing create-session request contract; unchanged

crates/malt-daemon/
└── src/gateway_backend.rs  # Existing session authority; unchanged
```

**Structure Decision**: This is a single-crate CLI enhancement. The gateway
and daemon continue to own session creation; their files are listed only to
make the integration boundary explicit, not as planned modification targets.
