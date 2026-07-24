# Data Model: Persistent Command Execution History

**Feature**: 003-command-execution-history | **Date**: 2026-07-25

## Entities

### CommandBlock (in-memory, exists — `crates/malt-session/src/pane.rs:12`)

The unit of history. One per execution attempt, including parse errors and empty input (both already receive real command ids).

| Field | Type | Meaning | Constraints |
|-------|------|---------|-------------|
| `command_id` | `u32` | Monotonic per-session execution id, assigned in `run_mash_command` | ≥ 1; 0 reserved for "no real id"; strictly increasing within a pane's history, including across restore (research R4) |
| `cmd` | `String` | The command text as submitted | Stored verbatim, no truncation (spec edge case: long pasted scripts stay intelligible) |
| `started_at` | `u64` | Epoch milliseconds when execution began | Set at push time, before `execute_list` |
| `finished_at` | `Option<u64>` | Epoch milliseconds when execution completed | `None` while running, and permanently `None` for a command interrupted by daemon stop (FR-006) |
| `exit_code` | `Option<i32>` | Process/shell exit status | `None` while running / interrupted; `Some(n)` once finalized. Parse errors finalize with `Some(1)` (matches mash CLI convention) |

**State transitions**:

```
(created) --push, started_at=now, finished_at=None, exit_code=None--> OPEN
OPEN --execute_list returns; finalize(finished_at=now, exit_code=Some(n))--> COMPLETED
OPEN --daemon stops before finalize; block persisted as-is--> INTERRUPTED (permanent; never reinterpreted)
```

Invariant: at most one OPEN block per pane, and it is always the newest entry (`current_block()`). Enforced by `PaneRuntime::finalize_current_block` living next to the buffer (research R2).

### PaneRuntime (in-memory container, exists — `crates/malt-session/src/pane.rs:25`)

Ordered, bounded collection of `CommandBlock`s for one pane.

- Ring buffer: `push_command_block` evicts oldest at `max_blocks` (default `DEFAULT_MAX_BLOCKS` = 1000, satisfying FR-003 and the SC-003 1,000-entry bound).
- Chronological order is insertion order (FR-002) — never re-sorted.
- New for this feature: a seeding path for restore (construct with pre-existing blocks) and `finalize_current_block`.
- Ownership: `SessionExecutor` gains one `PaneRuntime` for its `first_pane` (research R2). `SessionRuntime` is unchanged.

### PersistedCommandBlock (schema, NEW — `schemas/persist/session.vexil`)

Field-for-field mirror of `CommandBlock` for the bitpack persistence layer:

```vexil
@doc("One command execution record. Mirrors malt-session::pane::CommandBlock.")
message PersistedCommandBlock {
    command_id  @0 : u32
    cmd         @1 : string
    @doc("epoch milliseconds")
    started_at  @2 : u64
    finished_at @3 : optional<u64>
    exit_code   @4 : optional<i32>
}
```

Conversion functions live in `session_thread.rs` beside the existing `to_persisted_env_snapshot`/`from_persisted_env_snapshot` pair, same naming pattern. Conversion is lossless in both directions.

### PersistedPane (schema, EXTENDED)

```vexil
message PersistedPane {
    cwd            @0 : string
    title          @1 : optional<string>
    pane_type      @2 : PersistedPaneType
    @doc("Chronological command history, oldest first, capped at the pane's")
    @doc("retention bound (1000). Empty for sessions persisted before this field.")
    command_blocks @3 : array<PersistedCommandBlock>
}
```

- On the message (not the `Shell` union variant): history is a pane property; `Compat` panes inherit the shape later (research R3).
- Additive: old `.vxb` files decode with an empty array; `schema_version` stays 1.
- Written by `build_persisted_session` from the executor's `PaneRuntime`; read by the restore path (`spawn_with_cwd`) to seed a new `PaneRuntime` and to resume `next_command_id = max(command_id)` (research R4).

### CommandHistoryEntry (Gateway response type, NEW — `crates/malt-gateway/src/backend.rs`)

JSON-serializable view returned by `GatewayBackend::get_command_history`, consumed by the route, CLI, and MCP tool:

| Field | JSON type | Source |
|-------|-----------|--------|
| `command_id` | number | `CommandBlock.command_id` |
| `cmd` | string | `CommandBlock.cmd` |
| `started_at` | number (epoch ms) | `CommandBlock.started_at` |
| `finished_at` | number \| null | `CommandBlock.finished_at` |
| `exit_code` | number \| null | `CommandBlock.exit_code` |
| `pane_id` | number | The pane the history belongs to (today: session's `first_pane`) — future pane-filter compatibility (research R5) |

`finished_at: null` + `exit_code: null` is the wire representation of both OPEN (still running) and INTERRUPTED (daemon stopped mid-command); clients distinguish them by whether the session/daemon is currently executing — the record itself honestly says only "not confirmed complete" (FR-004, FR-006).

## Relationships

```
PersistedSession 1--* PersistedPane 1--* PersistedCommandBlock   (persistence)
SessionExecutor  1--1 PaneRuntime   1--* CommandBlock            (runtime)
GatewayBackend::get_command_history --> Vec<CommandHistoryEntry> (retrieval view)
```

## Validation rules

- Retrieval for an unknown session id → `GatewayError::NotFound` → HTTP 404 (FR-009). Never an empty 200.
- Retrieval without a valid token → 401; with insufficient scope (< Read) → 403 (FR-008). Enforced by existing middleware; `required_scope` gains the `(GET, "/sessions/{id}/history")` → `Read` entry.
- Restore MUST preserve entry content byte-for-byte (SC-003): round-trip test asserts equality of the full entry list before/after dormant→restore.
- Eviction consistency across restart (spec edge case): persisted array is already capped (it is written from the capped ring buffer), and the restore-seeded `PaneRuntime` uses the same `max_blocks`, so no duplication or over-cap growth is possible by construction — test asserts the cap holds after restore + further executions.
