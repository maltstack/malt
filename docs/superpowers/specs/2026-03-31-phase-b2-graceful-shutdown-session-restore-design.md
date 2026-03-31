# Phase B2 — Graceful Shutdown + Session Restore Design

**Goal:** Implement the Active ↔ Dormant session lifecycle: sessions survive daemon restarts, last-client-detach transitions sessions to Dormant (processes stopped, state persisted), and re-attach restores processes from persisted state.

**Architecture:** The Coordinator is the single source of truth for all sessions — Active and Dormant. On startup it scans the store and loads persisted sessions as Dormant entries in-memory (no threads). On detach it saves state and kills threads. On attach to a Dormant session it re-spawns processes from persisted cwd/args. Graceful shutdown saves all Active sessions before exiting.

**Tech Stack:** Rust, `std::sync::mpsc`, `malt-platform::ProcessSupervisor::spawn_with_pty`, `mash::Env`, `vexil_store` bitpack, `DebouncedStore`.

---

## Scope

Six changes across six files. No new crate dependencies.

Out of scope for B2: scrollback (B3), daemon→client `SessionDormant` push notification (clients see EOF on shutdown — reconnect on next attach), session group lifecycle, checkpoint (Phase H).

---

## Files

| File | Action |
|------|--------|
| `schemas/common.vexil` | Modify — add `Dormant` variant to `SessionState` enum |
| `crates/malt-daemon/src/error.rs` | Modify — add `AppRestoreNotSupported`, `RestoreFailed`, `SessionDormant` |
| `crates/malt-daemon/src/executor/coordinator.rs` | Modify — `SessionLifecycle` enum, startup scan, `go_dormant`, `restore_session`, `shutdown_graceful` |
| `crates/malt-daemon/src/executor/session_thread.rs` | Modify — `SessionCommand::Snapshot`, `spawn_with_cwd`, Compat pane spawn path |
| `crates/malt-daemon/src/vnp_listener.rs` | Modify — `DetachSession` message handler |
| `crates/malt-bin/src/daemon.rs` | Modify — call `shutdown_graceful` + `flush_all` before exit |

---

## 1. Schema Change: `SessionState::Dormant`

In `schemas/common.vexil`, add `Dormant` to the `SessionState` enum:

```vexil
enum SessionState {
    Active   @0
    Dormant  @1
}
```

The generated `malt_protocol::common::SessionState` gains the `Dormant` variant. `list_sessions` returns it for sessions with no running thread. All match sites on `SessionState` must be updated (coordinator and gateway backend).

---

## 2. Error Variants (`error.rs`)

```rust
#[error("app pane restore is not supported until Phase G plugin infrastructure is implemented")]
AppRestoreNotSupported,

#[error("failed to restore session {0}: {1}")]
RestoreFailed(SessionId, String),

#[error("session {0} is dormant — attach to restore it")]
SessionDormant(SessionId),
```

`SessionDormant` is returned when an operation that requires an Active session (e.g., `send_command`, `route_to_session`) is called on a Dormant session ID.

---

## 3. Coordinator (`coordinator.rs`)

### SessionLifecycle enum

Replace the current flat `cmd_tx` + `thread` + implicit "always active" model:

```rust
enum SessionLifecycle {
    Active {
        cmd_tx: mpsc::Sender<SessionCommand>,
        thread: Option<JoinHandle<()>>,
        client_count: u32,
    },
    Dormant {
        persisted: PersistedSession,
    },
}
```

`SessionHandle` keeps `id`, `name`, `isolation` at the top level (shared between states) and gains `lifecycle: SessionLifecycle`.

### Startup scan

`Coordinator::new` loads persisted sessions after the existing counter-restore block. `daemon_state` is already loaded at that point; its `sessions` field lists the IDs to scan:

```
// after existing counter restore block (which already calls store.load_daemon_state())
// re-use the Ok(state) branch to iterate session IDs:
for id in state.sessions:   // state from the Ok(state) arm of the counter restore match
    match store.load_session(&id):
        Ok(persisted) →
            insert SessionHandle {
                id: persisted.id.clone(),
                name: persisted.name.clone(),
                isolation: persisted.isolation,
                lifecycle: Dormant { persisted },
            }
        Err(NotFound) → skip (stale ID in daemon state)
        Err(CorruptFile { .. }) → log warn, skip
        Err(other) → log warn, skip
```

Dormant entries do not increment `next_session_id` or `next_pane_id` — counters are restored separately from `DaemonState`.

### `register_vnp_client` — restore on attach

```
match session.lifecycle:
    Active { .. } → proceed as before (increment client_count, send RegisterVnpClient)
    Dormant { persisted } →
        restore_session(id, persisted)?   // transitions lifecycle to Active
        proceed as Active branch
```

### `unregister_vnp_client` — go dormant on last client

```
decrement client_count
if client_count == 0:
    go_dormant(session_id)
```

### `go_dormant(id)`

```
1. Send SessionCommand::Snapshot { reply } to cmd_tx
2. persisted = reply_rx.recv_timeout(5s)?
3. store.mark_dirty(id.clone(), persisted.clone())
4. Send SessionCommand::Shutdown to cmd_tx
5. Join thread
6. session.lifecycle = Dormant { persisted }
7. persist_daemon_state()
```

If `Snapshot` times out or `Shutdown` join fails, log error and leave session as Active (do not corrupt state with a partial transition).

### `restore_session(id, persisted)`

```
for (pane_id, pane) in &persisted.panes:
    match &pane.pane_type:
        Shell { shell_path } →
            (cmd_tx, thread) = SessionExecutor::spawn_with_cwd(
                id.clone(), pane_id.clone(), persisted.isolation, pane.cwd.clone()
            )?
        Compat { program, args } →
            (cmd_tx, thread) = SessionExecutor::spawn_compat(
                id.clone(), pane_id.clone(), persisted.isolation,
                program.clone(), args.clone(), pane.cwd.clone()
            )?
        App { .. } →
            return Err(DaemonError::AppRestoreNotSupported)

session.lifecycle = Active { cmd_tx, thread: Some(thread), client_count: 0 }
persist_daemon_state()
Ok(())
```

On any error, the session lifecycle is left as `Dormant` — no partial Active state.

### `shutdown_graceful(&mut self)`

Called from `daemon.rs` before the process exits:

```
for (_, session) in &mut self.sessions:
    if let Active { cmd_tx, thread, .. } = &mut session.lifecycle:
        // Snapshot
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = cmd_tx.send(SessionCommand::Snapshot { reply: reply_tx });
        if let Ok(persisted) = reply_rx.recv_timeout(Duration::from_secs(5)):
            store.mark_dirty(session.id.clone(), persisted)
        // Shutdown thread
        let _ = cmd_tx.send(SessionCommand::Shutdown);
        if let Some(t) = thread.take():
            let _ = t.join();

store.flush_all()   // blocks until all dirty entries are on disk
```

`Coordinator::Drop` calls `shutdown_graceful` (replacing `shutdown_all`). Sessions are saved before threads die. `flush_all` guarantees data is on disk before the OS reclaims the process.

### `list_sessions` — returns all entries

```rust
self.sessions.values().map(|h| SessionInfo {
    session_id: h.id.clone(),
    name: h.name.clone(),
    pane_count: match &h.lifecycle {
        Active { .. } => 1,
        Dormant { persisted } => persisted.panes.len() as u32,
    },
    isolation: h.isolation,
    state: match &h.lifecycle {
        Active { .. } => SessionState::Active,
        Dormant { .. } => SessionState::Dormant,
    },
    _unknown: Vec::new(),
}).collect()
```

### Operations on Dormant sessions

`route_to_session`, `send_command`, `get_session_output`, `send_key_input`, `ack_frame` all check:

```rust
match &handle.lifecycle {
    Active { cmd_tx, .. } => { /* existing logic */ }
    Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
}
```

`destroy_session` handles both: if Active, send Shutdown + join thread + call `store.delete_session(&id)`; if Dormant, call `store.delete_session(&id)` + remove from map. Both paths call `persist_daemon_state()` after removal.

---

## 4. Session Thread (`session_thread.rs`)

### `SessionCommand::Snapshot`

```rust
Snapshot { reply: mpsc::Sender<PersistedSession> }
```

Handler in the session loop:

```rust
SessionCommand::Snapshot { reply } => {
    let persisted = build_persisted_session(&session_id, &session, &env);
    let _ = reply.send(persisted);
}
```

`build_persisted_session` constructs a `PersistedSession` from:
- `session.id`, `session.name()`, `session.isolation()`
- `session.layout()` → `LayoutNode`
- `session.focused_pane()` → `PaneId`
- For each pane: `cwd` from `env.cwd()`, `pane_type: Shell { shell_path: env.shell() }`
- `schema_version: 1`

### `SessionExecutor::spawn_with_cwd`

Same as `spawn` but takes `initial_cwd: PathBuf` and calls `env.set_cwd(&initial_cwd)` before starting the session loop. If `initial_cwd` does not exist on disk, falls back to `env.cwd()` (home directory) and logs a warning — never returns an error for a missing directory (the user can navigate from wherever they land).

### `SessionExecutor::spawn_compat`

```rust
pub fn spawn_compat(
    session_id: SessionId,
    pane_id: PaneId,
    isolation: IsolationTier,
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
) -> Result<(Sender<SessionCommand>, JoinHandle<()>), DaemonError>
```

- Creates a `ProcessSupervisor` and calls `spawn_with_pty(program, args, Some(cwd))`
- Wires pty output through `CompatTranslator` → `RendererHost` (same pipeline as mash output)
- Session thread loop reads from pty fd and writes input from `KeyInput` commands
- On process exit: `spawn_compat` takes an additional `Arc<Mutex<Coordinator>>` parameter (same pattern as VNP listener). When the pty fd signals EOF (process died), the compat thread calls `coordinator.lock().unwrap().on_compat_process_exited(session_id)`. The coordinator treats this the same as `client_count` hitting 0: calls `go_dormant`. This prevents zombie Active sessions with a dead pty.

`ProcessSupervisor::spawn_with_pty` already exists in `malt-platform`. The compat thread model mirrors the mash thread model but reads from a pty fd instead of calling `execute_list`.

---

## 5. VNP Listener (`vnp_listener.rs`)

### `DetachSession` handler

In the main read loop, add handling for the `session::DetachSession` domain/type alongside `KeyEvent`, `Resize`, `FrameAck`:

```rust
(DOMAIN_SESSION, MSG_DETACH_SESSION) => {
    let msg = DetachSession::unpack(&mut BitReader::new(&payload))?;
    if msg.session_id == session_id {
        // Client explicitly detaching — same as EOF cleanup
        cleanup(&coordinator, session_id.clone(), client_id, &peer);
        return;
    }
}
```

`cleanup` calls `coordinator.unregister_vnp_client` which decrements `client_count` and may trigger `go_dormant`.

The domain/type constants for `DetachSession` must be verified against the generated `malt_protocol::codec` constants.

---

## 6. Daemon Startup (`daemon.rs`)

Replace the current implicit shutdown (Drop → shutdown_all) with an explicit graceful sequence. The coordinator is wrapped in `Arc<Mutex<Coordinator>>`; before the axum server exits, acquire the lock and call `shutdown_graceful`:

```rust
// In the shutdown_signal future, after axum completes:
{
    let mut coord = coordinator.lock().unwrap_or_else(|p| p.into_inner());
    coord.shutdown_graceful();
}
```

`shutdown_graceful` already calls `store.flush_all()` at the end, so no additional flush call is needed in `daemon.rs`. The `Drop` impl on `Coordinator` also calls `shutdown_graceful` as a safety net (idempotent — second call finds no Active sessions).

---

## Error Handling

| Scenario | Behavior |
|----------|----------|
| `Snapshot` timeout during `go_dormant` | Log error, leave session Active |
| `Snapshot` timeout during `shutdown_graceful` | Log error, skip save for that session, continue shutdown |
| `restore_session` fails (spawn error) | Return error to caller, session stays Dormant |
| `restore_session` App pane | Return `AppRestoreNotSupported`, session stays Dormant |
| Persisted cwd no longer exists | Log warn, fall back to home dir, restore succeeds |
| Corrupt PersistedSession on startup scan | Log warn, skip — stale entry stays in DaemonState until overwritten |
| Operation on Dormant session | Return `DaemonError::SessionDormant` |

---

## Testing

| Test | Location | What it verifies |
|------|----------|-----------------|
| `session_goes_dormant_on_last_client_detach` | coordinator tests | register → unregister → session Dormant in list, PersistedSession on disk |
| `dormant_session_visible_in_list_sessions` | coordinator tests | startup scan loads persisted session → list returns `SessionState::Dormant` |
| `restore_shell_session_from_dormant` | coordinator tests | dormant shell session → register_vnp_client → Active, cwd matches persisted |
| `restore_compat_session_from_dormant` | coordinator tests | dormant compat session → register_vnp_client → Active, process spawned |
| `app_restore_returns_error` | coordinator tests | dormant App pane → register_vnp_client → `AppRestoreNotSupported`, stays Dormant |
| `restore_fails_cleanly_on_bad_cwd` | coordinator tests | nonexistent cwd → restore succeeds with fallback cwd (no error) |
| `shutdown_graceful_saves_all_active_sessions` | coordinator tests | 2 active sessions → shutdown_graceful → both PersistedSessions on disk |
| `second_daemon_sees_dormant_sessions` | coordinator tests | create + detach + new coordinator → both sessions Dormant in list |
| `destroy_dormant_session_removes_from_store` | coordinator tests | destroy Dormant session → deleted from store, gone from list |
| `detach_session_vnp_message_triggers_dormant` | vnp_listener tests | client sends DetachSession frame → session goes Dormant |
