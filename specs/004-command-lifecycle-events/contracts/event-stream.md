# Contract: Command Lifecycle Event Stream

**Feature**: 004-command-lifecycle-events | **Date**: 2026-07-25

## `GET /sessions/{id}/events`

A Server-Sent Events stream of one session's command lifecycle events.

- **Auth**: Bearer token, `AuthScope::Read` or higher — same class as `/output` and `/history`, because command text can contain secrets typed at the prompt.
- **Response**: `200` with `Content-Type: text/event-stream`, then frames until the client disconnects or the session ends.
- **Resume**: send `Last-Event-ID: <sequence>` to resume after that position. Standard SSE clients send it automatically on reconnect.

**Errors are returned before the stream opens** — an SSE client treats an established stream as success, so a failure must never be reported as a frame on an opened connection:

| Status | Condition |
|--------|-----------|
| `404` | Session does not exist |
| `401` | Missing or invalid bearer token |
| `403` | Token scope below `Read` |
| `409` | Session is dormant (cannot produce events until attached) |
| `429` | Rate limit exceeded on connection establishment |

## Frame format

Each frame sets `id` (the resume position), `event` (the type), and `data` (a JSON object).

**Command started** — emitted when execution begins, while the command is still running:

```
id: 12
event: command_started
data: {"command_id":4,"cmd":"cargo test --workspace","started_at":1784070000123}

```

**Command finished** — emitted when the result is committed and its output is observable:

```
id: 13
event: command_finished
data: {"command_id":4,"exit_code":0,"finished_at":1784070042456,"duration_us":42333000}

```

**Gap** — emitted when this connection has lost events. Describes the stream, not the shell:

```
id: 41
event: gap
data: {"missed_from":14,"missed_through":40,"reason":"retention_exceeded"}

```

`reason` is `retention_exceeded` (resumed from a position older than the retained window) or `subscriber_lagged` (this client could not keep up). A `gap` on resume is delivered **before** any replayed events, so the client learns about the hole before receiving data that would otherwise look contiguous.

## Guarantees

- **Ordering**: frames arrive in the order the underlying commands executed. A `command_finished` always follows its `command_started` and carries the same `command_id` with a strictly greater `id`.
- **Pairing**: every command produces exactly one `command_started` and, once complete, exactly one `command_finished`. A command interrupted by daemon shutdown produces only the start — the stream ends with the connection, and command history (`GET /sessions/{id}/history`) is the authority on its not-confirmed-complete status.
- **Isolation**: a subscription to one session never receives another session's events.
- **Honesty**: a subscriber is never silently given an incomplete stream. Every lost range produces a `gap`.
- **Non-interference**: a subscriber that stops reading does not slow command execution, does not affect other subscribers, and does not grow daemon memory without bound. Such a subscriber receives a terminal `gap` with `reason: "subscriber_lagged"` and the connection is closed; reconnecting with `Last-Event-ID` resumes it on the catch-up path.

## Not in this contract

- **Command output.** Events describe execution, not what it printed. Use `GET /sessions/{id}/output` or `/output/text`.
- **Session and pane lifecycle** (created, destroyed, dormant, restored). A natural extension of this stream, deliberately out of scope (spec Assumptions).
- **A daemon-wide "all sessions" stream.** Subscriptions are per-session, matching the unit of access control.
- **An MCP tool.** A long-lived stream does not fit MCP's request/response tool shape; agents consume this endpoint directly (research R8).
