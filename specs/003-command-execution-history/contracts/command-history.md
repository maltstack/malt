# Contract: Command History Retrieval

**Feature**: 003-command-execution-history | **Date**: 2026-07-25

The single source of truth for history data is `GatewayBackend::get_command_history`; every client surface below is a thin view over it, which is what makes FR-007's "equivalent data through each client" hold by construction.

## HTTP (Gateway)

### `GET /sessions/{id}/history`

- **Auth**: Bearer token, `AuthScope::Read` or higher. `401` missing/invalid token; `403` insufficient scope. (Existing middleware; new `required_scope` table entry.)
- **Rate limiting**: same per-client token bucket as all routes.

**Success — `200`** (envelope matches the Gateway's existing `{"ok": true, "data": ...}` convention):

```json
{
  "ok": true,
  "data": [
    {
      "command_id": 1,
      "cmd": "cargo test --workspace",
      "started_at": 1784070000123,
      "finished_at": 1784070042456,
      "exit_code": 0,
      "pane_id": 1
    },
    {
      "command_id": 2,
      "cmd": "false",
      "started_at": 1784070050000,
      "finished_at": 1784070050010,
      "exit_code": 1,
      "pane_id": 1
    },
    {
      "command_id": 3,
      "cmd": "sleep 300",
      "started_at": 1784070060000,
      "finished_at": null,
      "exit_code": null,
      "pane_id": 1
    }
  ]
}
```

- Entries are chronological, oldest first, capped at the retention bound (1000).
- `finished_at: null` + `exit_code: null` ⇒ the command is not confirmed complete (still running, or interrupted by a daemon stop). Never reinterpreted as success.
- A session with no executions yet returns `200` with `"data": []`.

**Errors**:

| Status | Condition | Body shape |
|--------|-----------|------------|
| `404` | Session id does not exist | existing `{"ok": false, "error": ...}` envelope |
| `401` | Missing/invalid bearer token | existing auth error envelope |
| `403` | Token scope below Read | existing auth error envelope |
| `429` | Rate limit exceeded | existing rate-limit envelope |

## CLI (`malt-bin`)

### `malt history <SESSION_ID>`

- Reads the token from `~/.config/malt/api-token` automatically (same as every subcommand).
- Calls `GET /sessions/{id}/history`; prints one line per entry: command id, start time, duration (or `running`/`incomplete` when not finalized), exit code, command text.
- Exit code 0 on success (including empty history); non-zero with the Gateway error message on 404/auth failures — same behavior class as `malt output`.

## MCP (`malt-mcp`)

### Tool: `get_command_history`

- **Input schema**: `{ "session_id": <number, required> }`
- **Behavior**: delegates to `GET /sessions/{id}/history` with the auto-read token (ADR-0002: MCP is an adapter over the Gateway, never a second backend).
- **Output**: the `data` array verbatim as the tool result content (JSON), so agents receive exactly what the HTTP client receives.
- **Errors**: Gateway error text surfaced as a JSON-RPC tool error, matching the other 6 tools.

## Internal (not a public contract, listed for completeness)

- `SessionCommand::GetCommandHistory { reply: mpsc::Sender<Vec<CommandBlock>> }` — session-thread query; on the current baseline it is serviced after any in-flight command completes (research R6).
- `Coordinator::get_command_history(session_id: &SessionId) -> Option<Vec<CommandBlock>>` — `None` for unknown session, mapped to `GatewayError::NotFound` in `DaemonBackend`.
