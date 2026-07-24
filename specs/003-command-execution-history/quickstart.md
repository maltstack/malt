# Quickstart: Validating Persistent Command Execution History

**Feature**: 003-command-execution-history

Runnable end-to-end scenarios proving the feature works. See [contracts/command-history.md](contracts/command-history.md) for exact response shapes and [data-model.md](data-model.md) for field semantics.

## Prerequisites

- Workspace builds green: `cargo build --workspace` (requires `vexilc` on PATH for schema compilation — `~/.cargo/bin/vexilc.exe`, v0.5.1+).
- No daemon already running on the default port.

## Scenario 1 — History records commands with status (User Story 1)

```powershell
cargo run -p malt-bin -- start
cargo run -p malt-bin -- new --name hist-demo        # note the session ID (e.g. 1)
cargo run -p malt-bin -- exec 1 "echo hello"
cargo run -p malt-bin -- exec 1 "false"
cargo run -p malt-bin -- history 1
```

**Expected**: two entries, chronological — `echo hello` with exit code 0, `false` with exit code 1, each with start/finish timestamps. A fresh session shows an empty history, not an error.

## Scenario 2 — History survives daemon restart (User Story 2)

```powershell
cargo run -p malt-bin -- exec 1 "echo before-restart"
cargo run -p malt-bin -- stop                        # graceful: persists sessions
cargo run -p malt-bin -- start
cargo run -p malt-bin -- history 1
```

**Expected**: all pre-restart entries present with identical content (ids, text, timestamps, exit codes).

Note that history reads work on the restored-but-dormant session *without* waking it — that is the intended design (research R5), verified against a real daemon.

**Post-restore id monotonicity is not verifiable from the CLI.** A session restored from disk is `Dormant`, and `malt exec` on a dormant session is refused (`attach to restore it`) — pre-existing behavior, unrelated to this feature. Only attaching wakes it. The behavior is covered by `command_history_survives_dormant_restore_and_ids_stay_monotonic` (`malt-daemon/tests/gateway_backend.rs`), which drives the restore through `register_vnp_client` the way a real attach does. See `docs/findings/2026-07-25-command-execution-history.md`.

## Scenario 3 — Automation surface parity (User Story 3)

With the daemon running and the token from `~/.config/malt/api-token`:

```bash
curl -s -H "Authorization: Bearer $(cat ~/.config/malt/api-token)" http://127.0.0.1:7700/sessions/1/history
```

**Expected**: `200` with the same entries the CLI printed, as JSON. Then verify refusals:

```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:7700/sessions/1/history          # → 401 (no token)
curl -s -H "Authorization: Bearer $(cat ~/.config/malt/api-token)" -o /dev/null -w "%{http_code}\n" http://127.0.0.1:7700/sessions/999/history   # → 404 (unknown session)
```

MCP parity: call the `get_command_history` tool with `{"session_id": 1}` through any MCP client against `malt-mcp` — result must match the HTTP `data` array.

## Automated verification

```powershell
cargo test --workspace
```

Feature-specific suites (all must pass, and each must exercise the real path — persist to disk, restore, retrieve — not struct construction):

- `malt-session` pane tests: ring-buffer cap, finalize-current-block invariant, restore seeding.
- `malt-daemon/tests/session_thread.rs`: records created for success/failure/parse-error executions.
- `malt-daemon/tests/gateway_backend.rs`: end-to-end dormant→restore history survival; post-restore id monotonicity; 404 for unknown session.
- `malt-daemon/tests/store.rs`: `PersistedCommandBlock` round-trip incl. `None` fields (interrupted command).
- `malt-gateway/tests/routes.rs`: route registered, Read scope enforced (401/403 cases), response shape.

## Known baseline caveat

On the pre-002 baseline, `history` issued while a command is executing waits for that command to finish (single session thread services all queries) — the still-running entry is recorded immediately but observable mid-flight only after feature 002 (responsive session control) lands and this branch rebases. See research R6.
