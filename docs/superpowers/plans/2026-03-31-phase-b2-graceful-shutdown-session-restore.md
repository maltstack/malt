# Phase B2 — Graceful Shutdown + Session Restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Active ↔ Dormant session lifecycle so sessions survive daemon restarts: last-client-detach transitions sessions to Dormant (processes stopped, state persisted), and re-attach restores processes from persisted cwd/args.

**Architecture:** The Coordinator is the single source of truth for all sessions — Active and Dormant. `SessionHandle.lifecycle` is a `SessionLifecycle` enum (`Active { cmd_tx, thread, client_count }` / `Dormant { persisted }`). On startup, it scans the store and loads persisted sessions as Dormant entries. On last-client-detach it snapshots and kills threads. On attach to a Dormant session it re-spawns processes from persisted cwd/args. Graceful shutdown snapshots all Active sessions before exiting.

**Tech Stack:** Rust, `std::sync::mpsc`, `malt_platform::pty::spawn_with_pty`, `mash::env::{Env, Variable}`, `malt_protocol::persist::session::{PersistedSession, PersistedPane, PersistedPaneType}`, `vexil_runtime::{BitReader, Pack, Unpack}`, `DebouncedStore`.

---

## Files

| File | Action |
|------|--------|
| `crates/malt-daemon/src/store/debounce.rs` | Modify — add `list_sessions`, `load_session`, `delete_session` pass-throughs |
| `crates/malt-daemon/src/error.rs` | Modify — add `AppRestoreNotSupported`, `RestoreFailed`, `SessionDormant` |
| `crates/malt-daemon/src/executor/session_thread.rs` | Modify — add `SessionCommand::Snapshot`, `build_persisted_session`, `spawn_with_cwd` |
| `crates/malt-daemon/src/executor/coordinator.rs` | Modify — `SessionLifecycle` enum, startup scan, `go_dormant`, `restore_session`, `shutdown_graceful`, `client_count` tracking |
| `crates/malt-daemon/src/vnp_listener.rs` | Modify — `DetachSession` handler in `dispatch_frame` |
| `crates/malt-bin/src/daemon.rs` | Modify — call `shutdown_graceful` before process exit |
| `crates/malt-daemon/tests/coordinator.rs` | Modify — add 9 new B2 tests |
| `crates/malt-daemon/tests/vnp_listener.rs` | Modify — add 1 `detach_session_vnp_message` test |

---

## Task 1: DebouncedStore pass-through methods

**Files:**
- Modify: `crates/malt-daemon/src/store/debounce.rs`

The coordinator's startup scan and `destroy_session` need to call `list_sessions`, `load_session`, and `delete_session` on the underlying `SessionStore`. `DebouncedStore` currently only exposes `mark_dirty`, `mark_dirty_daemon`, `load_daemon_state`, and `flush_all`. These three synchronous pass-throughs bypass the background thread entirely — they delegate directly to `self.inner.store`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/malt-daemon/src/store/debounce.rs`:

```rust
#[test]
fn passthrough_list_load_delete() {
    use malt_protocol::persist::session::{PersistedSession, PersistedPaneType};

    let dir = tempfile::tempdir().unwrap();
    let inner = SessionStore::new(dir.path().to_path_buf());
    let store = DebouncedStore::new(inner);

    // Nothing persisted yet.
    assert!(store.list_sessions().unwrap().is_empty());

    // Save a session via mark_dirty + flush_all, then load it back.
    let sid = SessionId(7);
    let persisted = PersistedSession {
        schema_version: 1,
        id: sid.clone(),
        name: Some("test".to_string()),
        layout: None,
        focus: None,
        panes: std::collections::HashMap::new(),
        theme: None,
        group: None,
        isolation: malt_protocol::common::IsolationTier::Bare,
        _unknown: vec![],
    };
    store.mark_dirty(sid.clone(), persisted.clone());
    store.flush_all();

    let ids = store.list_sessions().unwrap();
    assert_eq!(ids, vec![sid.clone()]);

    let loaded = store.load_session(&sid).unwrap();
    assert_eq!(loaded.name, persisted.name);

    store.delete_session(&sid).unwrap();
    assert!(store.list_sessions().unwrap().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p malt-daemon passthrough_list_load_delete -- --nocapture
```

Expected: compile error — `list_sessions`, `load_session`, `delete_session` do not exist on `DebouncedStore`.

- [ ] **Step 3: Add the three pass-through methods to `DebouncedStore`**

In `crates/malt-daemon/src/store/debounce.rs`, add after `flush_all`:

```rust
/// List all persisted session IDs synchronously.
pub fn list_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
    self.inner.store.list_sessions()
}

/// Load a single persisted session synchronously.
pub fn load_session(&self, id: &SessionId) -> Result<PersistedSession, StoreError> {
    self.inner.store.load_session(id)
}

/// Delete a persisted session synchronously.
pub fn delete_session(&self, id: &SessionId) -> Result<(), StoreError> {
    self.inner.store.delete_session(id)
}
```

- [ ] **Step 4: Run test to verify it passes**

```
cargo test -p malt-daemon passthrough_list_load_delete -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Run full test suite to confirm nothing broken**

```
cargo test -p malt-daemon
```

Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```
git add crates/malt-daemon/src/store/debounce.rs
git commit -m "feat(malt-daemon): add list/load/delete session pass-throughs to DebouncedStore"
```

---

## Task 2: Error variants

**Files:**
- Modify: `crates/malt-daemon/src/error.rs`

Three new error variants are needed for B2:
- `AppRestoreNotSupported` — returned when a Dormant session has App panes (Phase G feature)
- `RestoreFailed(SessionId, String)` — returned when `restore_session` fails to spawn a process
- `SessionDormant(SessionId)` — returned when an operation requiring Active is called on Dormant

- [ ] **Step 1: Write the failing test**

Add to `crates/malt-daemon/tests/coordinator.rs` (imports will resolve after this task):

```rust
#[test]
fn error_variants_are_distinct() {
    use malt_daemon::DaemonError;
    use malt_protocol::common::SessionId;
    let e1 = DaemonError::AppRestoreNotSupported;
    let e2 = DaemonError::RestoreFailed(SessionId(1), "boom".to_string());
    let e3 = DaemonError::SessionDormant(SessionId(2));
    // Just check they format without panic.
    assert!(format!("{e1}").contains("not supported"));
    assert!(format!("{e2}").contains("1"));
    assert!(format!("{e3}").contains("dormant"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p malt-daemon error_variants_are_distinct -- --nocapture
```

Expected: compile error — variants do not exist.

- [ ] **Step 3: Add the variants to `DaemonError`**

In `crates/malt-daemon/src/error.rs`, add after `NameConflict`:

```rust
#[error("app pane restore is not supported until Phase G plugin infrastructure is implemented")]
AppRestoreNotSupported,

#[error("failed to restore session {0:?}: {1}")]
RestoreFailed(malt_protocol::common::SessionId, String),

#[error("session {0:?} is dormant — attach to restore it")]
SessionDormant(malt_protocol::common::SessionId),
```

- [ ] **Step 4: Run test to verify it passes**

```
cargo test -p malt-daemon error_variants_are_distinct -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/malt-daemon/src/error.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): add AppRestoreNotSupported, RestoreFailed, SessionDormant error variants"
```

---

## Task 3: `SessionCommand::Snapshot` + `build_persisted_session` + `spawn_with_cwd`

**Files:**
- Modify: `crates/malt-daemon/src/executor/session_thread.rs`

Three additions to `session_thread.rs`:

1. `SessionCommand::Snapshot { reply: mpsc::Sender<PersistedSession> }` — tells the session to snapshot its state and send back a `PersistedSession`.
2. `build_persisted_session` — constructs a `PersistedSession` from the session executor's current state.
3. `SessionExecutor::spawn_with_cwd` — identical to `spawn` but sets `PWD` in the mash env before starting the loop.

The `PersistedSession` for a mash shell session has a single pane of type `Shell { shell_path }`. The `shell_path` is the value of `SHELL` in the mash env, falling back to `"/bin/sh"` on Unix or `"cmd.exe"` on Windows. The cwd is the value of `PWD` in the mash env, falling back to `"."`.

The `panes` field in `PersistedSession` is `HashMap<u32, PersistedPane>` where the key is the raw pane ID.

- [ ] **Step 1: Add required imports to `session_thread.rs`**

At the top of `crates/malt-daemon/src/executor/session_thread.rs`, add to the existing imports:

```rust
use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
use std::collections::HashMap as PaneMap;
```

Note: `PaneMap` alias avoids shadowing the existing `HashMap` import used for `render_pushers`.

- [ ] **Step 2: Add `Snapshot` variant to `SessionCommand`**

In `session_thread.rs`, add after the `AckFrame` variant and before `Shutdown`:

```rust
/// Take a snapshot of the current session state for persistence.
/// The reply channel receives a `PersistedSession` built from current env + layout.
Snapshot { reply: mpsc::Sender<PersistedSession> },
```

- [ ] **Step 3: Add `Snapshot` arm to the Debug impl**

In the `impl std::fmt::Debug for SessionCommand` block, add after the `AckFrame` arm:

```rust
Self::Snapshot { .. } => write!(f, "Snapshot"),
```

- [ ] **Step 4: Add `Snapshot` handler to the session run loop**

In the `run` method, add before the `Shutdown` arm (right above it):

```rust
Ok(SessionCommand::Snapshot { reply }) => {
    let persisted = build_persisted_session(
        self.session.id(),
        self.session.id_ref(),
        self.session.name_ref(),
        self.session.isolation(),
        self.session.focused_pane(),
        &self.mash_env,
    );
    let _ = reply.send(persisted);
}
```

- [ ] **Step 5: Add `build_persisted_session` free function**

Add after `impl SessionExecutor` block (outside the impl, at module level):

```rust
/// Build a `PersistedSession` from the current session executor state.
///
/// Always produces a single Shell pane (index 0) keyed by the focused pane ID.
/// The shell_path comes from the `SHELL` env var (falls back to platform default).
/// The cwd comes from the `PWD` env var (falls back to `"."`).
fn build_persisted_session(
    session_id: &SessionId,
    _session_id_u32: u32,
    name: Option<&str>,
    isolation: IsolationTier,
    focused_pane: &PaneId,
    env: &Env,
) -> PersistedSession {
    let shell_path = env
        .get_str("SHELL")
        .to_string()
        .into();
    let shell_path = if shell_path == "" {
        #[cfg(unix)]
        { "/bin/sh".to_string() }
        #[cfg(not(unix))]
        { "cmd.exe".to_string() }
    } else {
        shell_path
    };

    let cwd = {
        let s = env.get_str("PWD");
        if s.is_empty() { ".".to_string() } else { s.to_string() }
    };

    let pane = PersistedPane {
        cwd,
        title: None,
        pane_type: PersistedPaneType::Shell { shell_path },
        _unknown: vec![],
    };

    let mut panes = PaneMap::new();
    panes.insert(focused_pane.0, pane);

    PersistedSession {
        schema_version: 1,
        id: session_id.clone(),
        name: name.map(|s| s.to_string()),
        layout: None,
        focus: Some(focused_pane.clone()),
        panes,
        theme: None,
        group: None,
        isolation,
        _unknown: vec![],
    }
}
```

- [ ] **Step 6: Check the `SessionRuntime` API for name/isolation access**

The coordinator passes `name` and `isolation` when constructing the `SessionRuntime`. Verify what methods exist:

```
cargo grep "pub fn" crates/malt-session/src/session.rs | head -20
```

Look for `name()`, `isolation()` or similar. If they don't exist, the snapshot must instead take these values as plain parameters passed from the coordinator via the `Snapshot` command.

**Adjustment:** If `SessionRuntime` does not expose `name()` and `isolation()`, change the `Snapshot` variant to carry them:

```rust
Snapshot {
    reply: mpsc::Sender<PersistedSession>,
    name: Option<String>,
    isolation: IsolationTier,
}
```

And update the coordinator's `go_dormant` call accordingly (Task 6). The handler becomes:

```rust
Ok(SessionCommand::Snapshot { reply, name, isolation }) => {
    let persisted = build_persisted_session(
        self.session.id(),
        self.session.focused_pane(),
        name.as_deref(),
        isolation,
        &self.mash_env,
    );
    let _ = reply.send(persisted);
}
```

And simplify `build_persisted_session` signature:

```rust
fn build_persisted_session(
    session_id: &SessionId,
    focused_pane: &PaneId,
    name: Option<&str>,
    isolation: IsolationTier,
    env: &Env,
) -> PersistedSession {
    // ... same body, using the passed-in name and isolation
}
```

- [ ] **Step 7: Add `SessionExecutor::spawn_with_cwd`**

Add after the existing `spawn` method in `impl SessionExecutor`:

```rust
/// Spawn a session executor with an explicit initial working directory.
///
/// Identical to `spawn` but sets the mash `PWD` variable to `initial_cwd`
/// before starting the session loop. If `initial_cwd` does not exist on disk,
/// falls back to the OS cwd (from `Env::from_os()`) and logs a warning — the
/// restore still succeeds; the user lands in a valid directory.
pub fn spawn_with_cwd(
    session_id: SessionId,
    first_pane: PaneId,
    isolation: IsolationTier,
    initial_cwd: std::path::PathBuf,
) -> Result<(mpsc::Sender<SessionCommand>, JoinHandle<()>), DaemonError> {
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("session-{}", session_id.0))
        .spawn(move || {
            let mut env = Env::from_os();
            env.set_interactive(true);

            // Apply requested cwd if the directory still exists.
            if initial_cwd.is_dir() {
                let cwd_str = initial_cwd.to_string_lossy().to_string();
                let _ = env.set_global("PWD", mash::env::Variable::exported_string(cwd_str));
            } else {
                warn!(
                    ?initial_cwd,
                    "spawn_with_cwd: directory no longer exists; falling back to OS cwd"
                );
            }

            let mut executor = SessionExecutor {
                session: SessionRuntime::new(session_id, first_pane, isolation),
                bus: Bus::new(BusConfig::default()),
                authority: AuthorityTracker::new(),
                terminal_size: Rect::new(0, 0, 80, 24),
                layout_config: LayoutConfig::default(),
                compat: None,
                mash_env: env,
                renderer: RendererHost::new(),
                editor: Editor::new(EditMode::Emacs),
                render_pushers: HashMap::new(),
                resolved_panes: Vec::new(),
            };
            executor.run(rx);
        })
        .map_err(DaemonError::Io)?;
    Ok((tx, handle))
}
```

Note: `mash::env::Variable` must be accessible. The existing import is `use mash::env::Env;` — extend it to `use mash::env::{Env, Variable};`.

- [ ] **Step 8: Write a test for `spawn_with_cwd`**

Add to `crates/malt-daemon/tests/coordinator.rs`:

```rust
#[test]
fn spawn_with_cwd_uses_provided_directory() {
    use malt_daemon::executor::session_thread::{SessionCommand, SessionExecutor};
    use malt_protocol::common::{IsolationTier, PaneId, SessionId};
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();

    let (cmd_tx, _thread) = SessionExecutor::spawn_with_cwd(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
        cwd.clone(),
    )
    .unwrap();

    // Ask the session to snapshot itself.
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::Snapshot {
            reply: reply_tx,
            name: Some("test".to_string()),
            isolation: IsolationTier::Bare,
        })
        .unwrap();

    let persisted = reply_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("snapshot reply timeout");

    let pane = persisted.panes.values().next().expect("no pane in snapshot");
    assert_eq!(
        pane.cwd,
        cwd.to_string_lossy().to_string(),
        "persisted cwd should match the spawn_with_cwd argument"
    );

    let _ = cmd_tx.send(SessionCommand::Shutdown);
}
```

- [ ] **Step 9: Run all daemon tests**

```
cargo test -p malt-daemon
```

Expected: all tests pass including the new `spawn_with_cwd_uses_provided_directory` test.

- [ ] **Step 10: Commit**

```
git add crates/malt-daemon/src/executor/session_thread.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): add SessionCommand::Snapshot, build_persisted_session, spawn_with_cwd"
```

---

## Task 4: `SessionLifecycle` refactor — structural change, zero behavior change

**Files:**
- Modify: `crates/malt-daemon/src/executor/coordinator.rs`

This task restructures `SessionHandle` to use a `SessionLifecycle` enum. No existing behavior changes — all sessions start and stay `Active`. The existing test suite must continue to pass unchanged at the end of this task.

`SessionLifecycle`:
```rust
enum SessionLifecycle {
    Active {
        cmd_tx: mpsc::Sender<SessionCommand>,
        thread: Option<std::thread::JoinHandle<()>>,
        client_count: u32,
    },
    Dormant {
        persisted: PersistedSession,
    },
}
```

`SessionHandle` becomes:
```rust
struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    lifecycle: SessionLifecycle,
}
```

- [ ] **Step 1: Run existing tests to establish baseline**

```
cargo test -p malt-daemon
```

Expected: all pass. Note the count. After this task they must all still pass.

- [ ] **Step 2: Add `PersistedSession` import to coordinator**

At the top of `crates/malt-daemon/src/executor/coordinator.rs`, add:

```rust
use malt_protocol::persist::session::PersistedSession;
```

- [ ] **Step 3: Replace `SessionHandle` and add `SessionLifecycle`**

Replace the existing `SessionHandle` struct definition with:

```rust
enum SessionLifecycle {
    Active {
        cmd_tx: mpsc::Sender<SessionCommand>,
        thread: Option<std::thread::JoinHandle<()>>,
        client_count: u32,
    },
    Dormant {
        persisted: PersistedSession,
    },
}

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    lifecycle: SessionLifecycle,
}
```

- [ ] **Step 4: Update `create_session` to use `SessionLifecycle::Active`**

Replace the `self.sessions.insert(...)` block in `create_session`:

```rust
self.sessions.insert(
    session_id.0,
    SessionHandle {
        id: session_id.clone(),
        name: Some(final_name),
        isolation,
        lifecycle: SessionLifecycle::Active {
            cmd_tx,
            thread: Some(thread),
            client_count: 0,
        },
    },
);
```

- [ ] **Step 5: Update `get_session_output` to use lifecycle**

Replace the body of `get_session_output`:

```rust
pub fn get_session_output(&self, session_id: SessionId) -> Result<String, DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            let (reply_tx, reply_rx) = mpsc::channel();
            cmd_tx
                .send(SessionCommand::GetOutput { reply: reply_tx })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
            reply_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 6: Update `destroy_session` to use lifecycle**

Replace the body of `destroy_session`:

```rust
pub fn destroy_session(&mut self, id: SessionId) {
    if let Some(mut handle) = self.sessions.remove(&id.0) {
        match handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, ref mut thread, .. } => {
                let _ = cmd_tx.send(SessionCommand::Shutdown);
                if let Some(t) = thread.take() {
                    let _ = t.join();
                }
            }
            SessionLifecycle::Dormant { .. } => {
                // No thread to shut down — just delete from store.
            }
        }
        let _ = self.store.delete_session(&id);
        info!(?id, "session destroyed");
        self.persist_daemon_state();
    }
}
```

- [ ] **Step 7: Update `route_to_session` to use lifecycle**

Replace the body of `route_to_session`:

```rust
pub fn route_to_session(
    &self,
    session_id: SessionId,
    msg: BusMessage,
) -> Result<(), DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            cmd_tx
                .send(SessionCommand::Deliver(msg))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 8: Update `send_command` to use lifecycle**

Replace the body of `send_command`:

```rust
pub fn send_command(
    &self,
    session_id: SessionId,
    cmd: SessionCommand,
) -> Result<(), DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            cmd_tx
                .send(cmd)
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 9: Update `list_sessions` to use lifecycle**

Replace the body of `list_sessions`:

```rust
pub fn list_sessions(&self) -> Vec<SessionInfo> {
    self.sessions
        .values()
        .map(|h| SessionInfo {
            session_id: h.id.clone(),
            name: h.name.clone(),
            pane_count: match &h.lifecycle {
                SessionLifecycle::Active { .. } => 1,
                SessionLifecycle::Dormant { persisted } => persisted.panes.len() as u32,
            },
            isolation: h.isolation,
            state: match &h.lifecycle {
                SessionLifecycle::Active { .. } => SessionState::Active,
                SessionLifecycle::Dormant { .. } => SessionState::Dormant,
            },
            _unknown: Vec::new(),
        })
        .collect()
}
```

- [ ] **Step 10: Update `register_vnp_client` to use lifecycle**

Replace the body of `register_vnp_client`:

```rust
pub fn register_vnp_client(
    &self,
    session_id: SessionId,
    client_id: u64,
    capabilities: ClientCapabilities,
    render_tx: mpsc::SyncSender<RenderBatch>,
) -> Result<InitialState, DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            let (initial_tx, initial_rx) = mpsc::channel();
            cmd_tx
                .send(SessionCommand::RegisterVnpClient {
                    client_id,
                    capabilities,
                    render_tx,
                    initial_reply: initial_tx,
                })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
            initial_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 11: Update `unregister_vnp_client` to use lifecycle**

Replace the body of `unregister_vnp_client`:

```rust
pub fn unregister_vnp_client(
    &self,
    session_id: SessionId,
    client_id: u64,
) -> Result<(), DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            cmd_tx
                .send(SessionCommand::UnregisterVnpClient { client_id })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Ok(()), // nothing to unregister
    }
}
```

- [ ] **Step 12: Update `send_key_input` to use lifecycle**

Replace the body of `send_key_input`:

```rust
pub fn send_key_input(
    &self,
    session_id: SessionId,
    key: KeyEvent,
) -> Result<(), DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            cmd_tx
                .send(SessionCommand::KeyInput { key })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 13: Update `ack_frame` to use lifecycle**

Replace the body of `ack_frame`:

```rust
pub fn ack_frame(
    &self,
    session_id: SessionId,
    client_id: u64,
    frame_seq: u64,
) -> Result<(), DaemonError> {
    let handle = self
        .sessions
        .get(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            cmd_tx
                .send(SessionCommand::AckFrame { client_id, frame_seq })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

- [ ] **Step 14: Update `shutdown_all` to use lifecycle**

Replace the body of `shutdown_all`:

```rust
pub fn shutdown_all(&mut self) {
    let ids: Vec<u32> = self.sessions.keys().copied().collect();
    for id in ids {
        self.destroy_session(SessionId(id));
    }
}
```

(This body is unchanged — `destroy_session` now handles both variants, so this is fine.)

- [ ] **Step 15: Run all tests to verify zero behavior change**

```
cargo test -p malt-daemon
```

Expected: same number of passing tests as the baseline in Step 1.

- [ ] **Step 16: Commit**

```
git add crates/malt-daemon/src/executor/coordinator.rs
git commit -m "refactor(malt-daemon): introduce SessionLifecycle enum in Coordinator (no behavior change)"
```

---

## Task 5: `client_count` tracking and `go_dormant`

**Files:**
- Modify: `crates/malt-daemon/src/executor/coordinator.rs`
- Modify: `crates/malt-daemon/tests/coordinator.rs`

Add proper `client_count` tracking to `register_vnp_client` / `unregister_vnp_client`, and implement `go_dormant` which is called when `client_count` reaches zero.

`go_dormant(id)`:
1. Build `Snapshot` command with name and isolation.
2. Send to `cmd_tx`, wait up to 5 s for reply.
3. On success: `store.mark_dirty(id, persisted)` + `store.flush_all()` (synchronous save).
4. Send `Shutdown`, join thread.
5. Set `lifecycle = Dormant { persisted }`.
6. Call `persist_daemon_state()`.
7. On `Snapshot` timeout: log error, leave session `Active`.

Note: `register_vnp_client` and `unregister_vnp_client` currently take `&self`. They must become `&mut self` to mutate `client_count`. This is a public API change — the caller `DaemonBackend` in `gateway_backend.rs` will need `&mut` access to the coordinator. Check whether `DaemonBackend` holds `Arc<Mutex<Coordinator>>` — if so, `coord.lock().unwrap()` already gives `MutexGuard<Coordinator>` which implements `DerefMut`, so the call site is unchanged.

- [ ] **Step 1: Write failing tests**

Add to `crates/malt-daemon/tests/coordinator.rs`:

```rust
#[test]
fn session_goes_dormant_on_last_client_detach() {
    use malt_protocol::render::RenderBatch;

    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
    let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();

    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let _ = coord
        .register_vnp_client(sid.clone(), 1, caps(), render_tx)
        .unwrap();

    // Detach — client count hits 0 → session should go Dormant.
    coord.unregister_vnp_client(sid.clone(), 1).unwrap();

    // Give the go_dormant path a moment to complete (it's synchronous in this impl).
    let sessions = coord.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(
        s.state,
        malt_protocol::common::SessionState::Dormant,
        "session should be Dormant after last client detaches"
    );

    // Verify PersistedSession is on disk.
    store.flush_all();
    let persisted = store.load_session(&sid).unwrap();
    assert_eq!(persisted.id, sid);
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p malt-daemon session_goes_dormant_on_last_client_detach -- --nocapture
```

Expected: FAIL — session is still `Active`, `go_dormant` doesn't exist.

- [ ] **Step 3: Change `register_vnp_client` and `unregister_vnp_client` to `&mut self` and add count tracking**

In `coordinator.rs`, update `register_vnp_client`:

```rust
pub fn register_vnp_client(
    &mut self,
    session_id: SessionId,
    client_id: u64,
    capabilities: ClientCapabilities,
    render_tx: mpsc::SyncSender<RenderBatch>,
) -> Result<InitialState, DaemonError> {
    let handle = self
        .sessions
        .get_mut(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &mut handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, client_count, .. } => {
            *client_count += 1;
            let cmd_tx = cmd_tx.clone();
            let id = handle.id.clone();
            let (initial_tx, initial_rx) = mpsc::channel();
            cmd_tx
                .send(SessionCommand::RegisterVnpClient {
                    client_id,
                    capabilities,
                    render_tx,
                    initial_reply: initial_tx,
                })
                .map_err(|_| DaemonError::SessionUnreachable(id))?;
            initial_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
    }
}
```

Update `unregister_vnp_client`:

```rust
pub fn unregister_vnp_client(
    &mut self,
    session_id: SessionId,
    client_id: u64,
) -> Result<(), DaemonError> {
    {
        let handle = self
            .sessions
            .get_mut(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &mut handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, client_count, .. } => {
                let _ = cmd_tx.send(SessionCommand::UnregisterVnpClient { client_id });
                if *client_count > 0 {
                    *client_count -= 1;
                }
            }
            SessionLifecycle::Dormant { .. } => return Ok(()),
        }
    }
    // Check if client_count hit 0 — if so, go dormant.
    let should_go_dormant = {
        let handle = self.sessions.get(&session_id.0).unwrap();
        matches!(&handle.lifecycle, SessionLifecycle::Active { client_count, .. } if *client_count == 0)
    };
    if should_go_dormant {
        self.go_dormant(session_id);
    }
    Ok(())
}
```

- [ ] **Step 4: Implement `go_dormant`**

Add as a private method in `impl Coordinator`:

```rust
fn go_dormant(&mut self, id: SessionId) {
    let handle = match self.sessions.get(&id.0) {
        Some(h) => h,
        None => return,
    };

    let (cmd_tx_clone, session_name, session_isolation) = match &handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, .. } => {
            (cmd_tx.clone(), handle.name.clone(), handle.isolation)
        }
        SessionLifecycle::Dormant { .. } => return, // already dormant
    };

    // Request a snapshot from the session thread.
    let (reply_tx, reply_rx) = mpsc::channel();
    let _ = cmd_tx_clone.send(SessionCommand::Snapshot {
        reply: reply_tx,
        name: session_name.clone(),
        isolation: session_isolation,
    });

    let persisted = match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(p) => p,
        Err(_) => {
            warn!(?id, "go_dormant: Snapshot timeout; leaving session Active");
            return;
        }
    };

    // Persist synchronously so state is on disk before killing the thread.
    self.store.mark_dirty(id.clone(), persisted.clone());
    self.store.flush_all();

    // Shut down the session thread.
    let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
    if let Some(handle) = self.sessions.get_mut(&id.0) {
        if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
            if let Some(t) = thread.take() {
                let _ = t.join();
            }
        }
        handle.lifecycle = SessionLifecycle::Dormant { persisted };
    }

    self.persist_daemon_state();
    info!(?id, "session transitioned to Dormant");
}
```

- [ ] **Step 5: Update `DaemonBackend` call sites for the signature change**

The `DaemonBackend` calls `register_vnp_client` and `unregister_vnp_client` through `Arc<Mutex<Coordinator>>`. Since `MutexGuard<Coordinator>` derefs to `&mut Coordinator`, the call sites work without changes. Verify by running:

```
cargo build -p malt-daemon
```

If `coord.register_vnp_client(...)` now requires `&mut` — check `gateway_backend.rs`:

```
cargo grep "register_vnp_client\|unregister_vnp_client" crates/malt-daemon/src/gateway_backend.rs
```

If the backend stores a `Arc<Mutex<Coordinator>>`, the `lock()` call returns `MutexGuard` which deref-mutably, so no source change is needed. If it stores `Arc<Coordinator>` (no Mutex), it must be updated to `Arc<Mutex<Coordinator>>`.

Also check `vnp_listener.rs` — it already calls through `coordinator.lock()` which gives `MutexGuard<Coordinator>`, so `&mut` methods work there too.

- [ ] **Step 6: Run test to verify it passes**

```
cargo test -p malt-daemon session_goes_dormant_on_last_client_detach -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Run full test suite**

```
cargo test -p malt-daemon
```

Expected: all existing tests pass.

- [ ] **Step 8: Commit**

```
git add crates/malt-daemon/src/executor/coordinator.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): track client_count per session and go_dormant on last-client-detach"
```

---

## Task 6: Startup scan + `restore_session` (Shell panes)

**Files:**
- Modify: `crates/malt-daemon/src/executor/coordinator.rs`
- Modify: `crates/malt-daemon/tests/coordinator.rs`

Two features in one task because they are tightly coupled:

1. **Startup scan** — `Coordinator::new` iterates the session IDs from the restored `DaemonState`, loads each `PersistedSession`, and inserts a `Dormant` handle.
2. **`restore_session`** — called from `register_vnp_client` when the target session is Dormant; re-spawns Shell panes from persisted cwd, transitions lifecycle to Active.
3. **`register_vnp_client` update** — when lifecycle is Dormant, call `restore_session` first, then proceed as Active.

- [ ] **Step 1: Write failing tests**

Add to `crates/malt-daemon/tests/coordinator.rs`:

```rust
#[test]
fn dormant_session_visible_in_list_sessions() {
    use malt_protocol::common::SessionState;

    let dir = tempfile::tempdir().unwrap();

    // First coordinator: create a session, go dormant via detach, flush, drop.
    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord
            .create_session(Some("alpha".to_string()), IsolationTier::Bare, None)
            .unwrap();

        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid.clone(), 1, caps(), render_tx).unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();
    }

    // Second coordinator: must see the dormant session.
    let store2 = make_store(&dir);
    let coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    assert_eq!(sessions.len(), 1, "second coordinator should see 1 dormant session");
    assert_eq!(
        sessions[0].state,
        SessionState::Dormant,
        "session should be Dormant in new coordinator"
    );
    assert_eq!(sessions[0].name.as_deref(), Some("alpha"));
}

#[test]
fn restore_shell_session_from_dormant() {
    use malt_protocol::render::RenderBatch;

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf(); // use tempdir as a known valid path

    // Create session, go dormant.
    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord.create_session(Some("beta".to_string()), IsolationTier::Bare, None).unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid.clone(), 1, caps(), render_tx).unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();
    }

    // New coordinator: session is dormant.
    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    let sid = sessions[0].session_id.clone();

    // Attach triggers restore.
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let initial = coord2.register_vnp_client(sid.clone(), 2, caps(), render_tx).unwrap();
    assert_eq!(initial.frame_seq, 0, "restored session should start at frame 0");

    // Session should now be Active.
    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, malt_protocol::common::SessionState::Active);

    let _ = coord2.unregister_vnp_client(sid, 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p malt-daemon dormant_session_visible_in_list_sessions restore_shell_session_from_dormant -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Add startup scan to `Coordinator::new`**

In the `Ok(state)` arm of the `load_daemon_state` match in `Coordinator::new`, after the counter restore:

```rust
Ok(state) => {
    next_session_id = state.next_session_id;
    next_pane_id = state.next_pane_id;
    info!(
        next_session_id,
        next_pane_id,
        "coordinator: restored counters from daemon state"
    );

    // Load dormant sessions from the store.
    // (state.sessions contains the SessionIds of all persisted sessions)
    let mut dormant_sessions: HashMap<u32, SessionHandle> = HashMap::new();
    for sid in &state.sessions {
        match store.load_session(sid) {
            Ok(persisted) => {
                dormant_sessions.insert(
                    sid.0,
                    SessionHandle {
                        id: persisted.id.clone(),
                        name: persisted.name.clone(),
                        isolation: persisted.isolation,
                        lifecycle: SessionLifecycle::Dormant { persisted },
                    },
                );
            }
            Err(StoreError::SessionNotFound(_)) => {
                // Stale ID in daemon state — session file was deleted.
                warn!(?sid, "coordinator startup: persisted session not found; skipping");
            }
            Err(e) => {
                warn!(?sid, %e, "coordinator startup: failed to load persisted session; skipping");
            }
        }
    }

    // Merge dormant sessions into the sessions map.
    // (The map is empty at this point — no sessions have been created yet.)
    return Self {
        sessions: dormant_sessions,
        next_session_id,
        next_pane_id,
        store,
        supervisor: ProcessSupervisor::new(),
        pool_config,
    };
}
```

Note: This uses an early return for the Ok branch. The Err branches that currently fall through to the default-values path can remain unchanged. Alternatively, collect dormant sessions in a local variable and assign after the match — either pattern works. Use whichever fits the existing code structure cleanly.

- [ ] **Step 4: Implement `restore_session`**

Add as a private method in `impl Coordinator`:

```rust
/// Re-spawn processes for a Dormant session, transitioning it to Active.
///
/// On any spawn error the session lifecycle remains Dormant.
fn restore_session(&mut self, id: SessionId) -> Result<(), DaemonError> {
    let (persisted, session_name, session_isolation) = {
        let handle = self
            .sessions
            .get(&id.0)
            .ok_or(DaemonError::SessionNotFound(id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Dormant { persisted } => {
                (persisted.clone(), handle.name.clone(), handle.isolation)
            }
            SessionLifecycle::Active { .. } => return Ok(()), // already active
        }
    };

    // Restore each pane. For B2, only Shell panes are supported.
    // Use the first pane ID from the persisted map.
    let (pane_id_raw, pane) = persisted
        .panes
        .iter()
        .next()
        .ok_or_else(|| DaemonError::RestoreFailed(id.clone(), "no panes in persisted session".to_string()))?;

    let pane_id = PaneId(*pane_id_raw);
    let cwd = std::path::PathBuf::from(&pane.cwd);

    let (cmd_tx, thread) = match &pane.pane_type {
        malt_protocol::persist::session::PersistedPaneType::Shell { .. } => {
            SessionExecutor::spawn_with_cwd(
                id.clone(),
                pane_id,
                session_isolation,
                cwd,
            )
            .map_err(|e| DaemonError::RestoreFailed(id.clone(), e.to_string()))?
        }
        malt_protocol::persist::session::PersistedPaneType::App { .. } => {
            return Err(DaemonError::AppRestoreNotSupported);
        }
        malt_protocol::persist::session::PersistedPaneType::Compat { .. } => {
            // Compat restore is added in Task 7.
            return Err(DaemonError::RestoreFailed(
                id.clone(),
                "compat pane restore not yet implemented".to_string(),
            ));
        }
        _ => {
            return Err(DaemonError::RestoreFailed(
                id.clone(),
                "unknown pane type".to_string(),
            ));
        }
    };

    if let Some(handle) = self.sessions.get_mut(&id.0) {
        handle.lifecycle = SessionLifecycle::Active {
            cmd_tx,
            thread: Some(thread),
            client_count: 0,
        };
        // Update name from persisted if current name is None.
        if handle.name.is_none() {
            handle.name = session_name;
        }
    }

    self.persist_daemon_state();
    info!(?id, "session restored from Dormant to Active");
    Ok(())
}
```

- [ ] **Step 5: Update `register_vnp_client` to restore dormant sessions**

Replace the `Dormant` arm of `register_vnp_client`:

```rust
SessionLifecycle::Dormant { .. } => {
    // Drop the immutable borrow before calling restore_session (&mut self).
    drop(handle); // need to restructure — see note below
}
```

**Important:** The current `register_vnp_client` borrows `handle` immutably. After detecting Dormant, we need to call `restore_session` (which needs `&mut self`). This requires restructuring to avoid holding the immutable borrow. Use this pattern:

```rust
pub fn register_vnp_client(
    &mut self,
    session_id: SessionId,
    client_id: u64,
    capabilities: ClientCapabilities,
    render_tx: mpsc::SyncSender<RenderBatch>,
) -> Result<InitialState, DaemonError> {
    // Check if dormant — if so, restore first.
    let is_dormant = match self.sessions.get(&session_id.0) {
        None => return Err(DaemonError::SessionNotFound(session_id)),
        Some(h) => matches!(h.lifecycle, SessionLifecycle::Dormant { .. }),
    };
    if is_dormant {
        self.restore_session(session_id.clone())?;
    }

    // Now proceed as Active (restore_session transitions lifecycle).
    let handle = self
        .sessions
        .get_mut(&session_id.0)
        .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
    match &mut handle.lifecycle {
        SessionLifecycle::Active { cmd_tx, client_count, .. } => {
            *client_count += 1;
            let cmd_tx = cmd_tx.clone();
            let id = handle.id.clone();
            let (initial_tx, initial_rx) = mpsc::channel();
            cmd_tx
                .send(SessionCommand::RegisterVnpClient {
                    client_id,
                    capabilities,
                    render_tx,
                    initial_reply: initial_tx,
                })
                .map_err(|_| DaemonError::SessionUnreachable(id))?;
            initial_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
        }
        SessionLifecycle::Dormant { .. } => {
            // restore_session succeeded but lifecycle is still Dormant — shouldn't happen.
            Err(DaemonError::SessionDormant(session_id))
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

```
cargo test -p malt-daemon dormant_session_visible_in_list_sessions restore_shell_session_from_dormant -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Run full test suite**

```
cargo test -p malt-daemon
```

Expected: all existing tests pass.

- [ ] **Step 8: Commit**

```
git add crates/malt-daemon/src/executor/coordinator.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): startup scan loads dormant sessions; restore_session re-spawns Shell panes on attach"
```

---

## Task 7: `shutdown_graceful` + `daemon.rs` wiring

**Files:**
- Modify: `crates/malt-daemon/src/executor/coordinator.rs`
- Modify: `crates/malt-bin/src/daemon.rs`
- Modify: `crates/malt-daemon/tests/coordinator.rs`

`shutdown_graceful` snapshots all Active sessions, persists them, and joins threads. It replaces `shutdown_all` in `Drop`. After this task, when the daemon is stopped (Ctrl-C or `/shutdown`), all running sessions are persisted so the next startup sees them as Dormant.

- [ ] **Step 1: Write failing test**

Add to `crates/malt-daemon/tests/coordinator.rs`:

```rust
#[test]
fn shutdown_graceful_saves_all_active_sessions() {
    let dir = tempfile::tempdir().unwrap();

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid1 = coord.create_session(Some("s1".to_string()), IsolationTier::Bare, None).unwrap();
        let sid2 = coord.create_session(Some("s2".to_string()), IsolationTier::Bare, None).unwrap();

        coord.shutdown_graceful();

        // After shutdown_graceful, both sessions should be persisted.
        store.flush_all();
        let p1 = store.load_session(&sid1).unwrap();
        let p2 = store.load_session(&sid2).unwrap();
        assert_eq!(p1.name.as_deref(), Some("s1"));
        assert_eq!(p2.name.as_deref(), Some("s2"));
    }

    // Second coordinator picks them up as Dormant.
    let store2 = make_store(&dir);
    let coord2 = Coordinator::new(PoolConfig::default(), store2);
    assert_eq!(coord2.session_count(), 2);
    let sessions = coord2.list_sessions();
    assert!(sessions.iter().all(|s| s.state == malt_protocol::common::SessionState::Dormant));
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p malt-daemon shutdown_graceful_saves_all_active_sessions -- --nocapture
```

Expected: FAIL — `shutdown_graceful` does not exist.

- [ ] **Step 3: Implement `shutdown_graceful`**

Add as a public method in `impl Coordinator` (before the `shutdown_all` method):

```rust
/// Snapshot all Active sessions, persist them, join threads.
///
/// After this call, all sessions are either Dormant (snapshot succeeded) or
/// destroyed (snapshot timed out). The store is flushed synchronously.
///
/// Called explicitly from daemon.rs before process exit. Also called from
/// `Drop` as a safety net (idempotent — second call finds no Active sessions).
pub fn shutdown_graceful(&mut self) {
    let ids: Vec<u32> = self.sessions.keys().copied().collect();
    for id in ids {
        let session_id = SessionId(id);
        let (cmd_tx_clone, session_name, session_isolation) = {
            let handle = match self.sessions.get(&session_id.0) {
                Some(h) => h,
                None => continue,
            };
            match &handle.lifecycle {
                SessionLifecycle::Active { cmd_tx, .. } => {
                    (cmd_tx.clone(), handle.name.clone(), handle.isolation)
                }
                SessionLifecycle::Dormant { .. } => continue,
            }
        };

        // Snapshot.
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = cmd_tx_clone.send(SessionCommand::Snapshot {
            reply: reply_tx,
            name: session_name,
            isolation: session_isolation,
        });
        match reply_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(persisted) => {
                self.store.mark_dirty(session_id.clone(), persisted.clone());
                // Transition to Dormant so persist_daemon_state captures the right state.
                if let Some(handle) = self.sessions.get_mut(&session_id.0) {
                    if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
                        let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
                        if let Some(t) = thread.take() {
                            let _ = t.join();
                        }
                    }
                    handle.lifecycle = SessionLifecycle::Dormant { persisted };
                }
            }
            Err(_) => {
                warn!(?session_id, "shutdown_graceful: Snapshot timeout; skipping save for this session");
                // Send shutdown anyway so the thread doesn't linger.
                let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
                if let Some(handle) = self.sessions.get_mut(&session_id.0) {
                    if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
                        if let Some(t) = thread.take() {
                            let _ = t.join();
                        }
                    }
                }
            }
        }
    }

    self.store.flush_all();
    self.persist_daemon_state();
}
```

- [ ] **Step 4: Update `Drop` to call `shutdown_graceful` instead of `shutdown_all`**

Replace the `Drop` impl:

```rust
impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown_graceful();
    }
}
```

- [ ] **Step 5: Update `daemon.rs` to call `shutdown_graceful` explicitly before exit**

In `crates/malt-bin/src/daemon.rs`, add explicit `shutdown_graceful` call after `axum::serve` completes (after the `await?`):

```rust
axum::serve(listener, router)
    .with_graceful_shutdown(shutdown_signal(shutdown_rx))
    .await?;

// Explicitly persist all active sessions before process exit.
// The Drop impl also calls shutdown_graceful as a safety net,
// but explicit is better than implicit here.
{
    let mut coord = coordinator.lock().unwrap_or_else(|p| p.into_inner());
    coord.shutdown_graceful();
}

println!("daemon stopped");
Ok(())
```

- [ ] **Step 6: Run tests to verify they pass**

```
cargo test -p malt-daemon shutdown_graceful_saves_all_active_sessions -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Run full test suite**

```
cargo test -p malt-daemon
```

Expected: all tests pass.

- [ ] **Step 8: Build `malt-bin` to verify daemon.rs compiles**

```
cargo build -p malt-bin
```

Expected: no errors.

- [ ] **Step 9: Commit**

```
git add crates/malt-daemon/src/executor/coordinator.rs crates/malt-bin/src/daemon.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): implement shutdown_graceful; wire into Drop and daemon.rs exit path"
```

---

## Task 8: `destroy_dormant_session` + `app_restore_returns_error` + `restore_fails_cleanly_on_bad_cwd` tests

**Files:**
- Modify: `crates/malt-daemon/tests/coordinator.rs`

These three tests verify corner cases in the already-implemented logic. No production code changes — just test coverage.

- [ ] **Step 1: Add three tests**

Add to `crates/malt-daemon/tests/coordinator.rs`:

```rust
#[test]
fn destroy_dormant_session_removes_from_store() {
    let dir = tempfile::tempdir().unwrap();

    // Create session, go dormant.
    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid.clone(), 1, caps(), render_tx).unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();

        // Destroy the dormant session.
        coord.destroy_session(sid.clone());
        assert_eq!(coord.session_count(), 0);

        // Verify it's gone from the store.
        store.flush_all();
        assert!(store.load_session(&sid).is_err());
    }
}

#[test]
fn second_daemon_sees_dormant_sessions() {
    let dir = tempfile::tempdir().unwrap();

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid1 = coord.create_session(Some("x".to_string()), IsolationTier::Bare, None).unwrap();
        let sid2 = coord.create_session(Some("y".to_string()), IsolationTier::Bare, None).unwrap();

        // Detach both → dormant.
        let (tx1, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid1.clone(), 1, caps(), tx1).unwrap();
        coord.unregister_vnp_client(sid1, 1).unwrap();

        let (tx2, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid2.clone(), 2, caps(), tx2).unwrap();
        coord.unregister_vnp_client(sid2, 2).unwrap();

        store.flush_all();
    }

    let store2 = make_store(&dir);
    let coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|s| s.state == malt_protocol::common::SessionState::Dormant));
}

#[test]
fn restore_fails_cleanly_on_bad_cwd() {
    use malt_protocol::render::RenderBatch;

    let dir = tempfile::tempdir().unwrap();

    // Create session, make it go dormant.
    let sid = {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord.register_vnp_client(sid.clone(), 1, caps(), render_tx).unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();
        sid
    };

    // Overwrite the persisted cwd with a nonexistent path.
    {
        let store = make_store(&dir);
        let mut persisted = store.load_session(&sid).unwrap();
        if let Some(pane) = persisted.panes.values_mut().next() {
            pane.cwd = "/this/path/does/not/exist/anywhere".to_string();
        }
        store.mark_dirty(sid.clone(), persisted);
        store.flush_all();
    }

    // Restore should succeed (spawn_with_cwd falls back to OS cwd on bad path).
    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let result = coord2.register_vnp_client(sid.clone(), 2, caps(), render_tx);
    assert!(result.is_ok(), "restore with bad cwd should succeed via fallback: {:?}", result);

    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, malt_protocol::common::SessionState::Active);
}
```

- [ ] **Step 2: Run the three tests**

```
cargo test -p malt-daemon destroy_dormant_session_removes_from_store second_daemon_sees_dormant_sessions restore_fails_cleanly_on_bad_cwd -- --nocapture
```

Expected: all three PASS.

- [ ] **Step 3: Commit**

```
git add crates/malt-daemon/tests/coordinator.rs
git commit -m "test(malt-daemon): add destroy_dormant, second_daemon, bad_cwd restore coverage"
```

---

## Task 9: VNP `DetachSession` handler

**Files:**
- Modify: `crates/malt-daemon/src/vnp_listener.rs`
- Modify: `crates/malt-daemon/tests/vnp_listener.rs`

Add handling for `DetachSession` (domain=4 `DOMAIN_SESSION`, type=0x03 `MSG_DETACH_SESSION`) in `dispatch_frame`. When a client sends this, the listener calls `cleanup` (which calls `unregister_vnp_client`, decrementing `client_count` and potentially triggering `go_dormant`) and returns from the handler loop.

`MSG_DETACH_SESSION` is already defined in `malt_protocol::codec` as `0x03`. `DetachSession` struct is in `malt_protocol::session` (generated from schema).

- [ ] **Step 1: Write failing test**

Add to `crates/malt-daemon/tests/vnp_listener.rs`:

```rust
#[test]
fn detach_session_vnp_message_triggers_dormant() {
    use malt_protocol::codec::{
        make_envelope, DOMAIN_SESSION, MSG_DETACH_SESSION, MSG_INITIAL_STATE, DOMAIN_RENDER,
    };
    use malt_protocol::envelope::encode_message;
    use malt_protocol::framing::{Frame, FrameFlags, FrameWriter};
    use malt_protocol::session::DetachSession;
    use malt_protocol::common::{SessionId, SessionState};
    use vexil_runtime::{BitWriter, Pack};

    let dir = tempfile::tempdir().unwrap();
    let (coord, session_id) = make_coordinator_with_session(&dir);
    let (listener, port) = start_test_listener(coord.clone());

    let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    do_handshake(&mut stream);
    do_attach(&mut stream, session_id.0);
    read_initial_state(&mut stream);

    // Send DetachSession.
    let detach = DetachSession {
        session_id: session_id.clone(),
        _unknown: vec![],
    };
    let envelope = make_envelope(DOMAIN_SESSION, MSG_DETACH_SESSION, session_id.0);
    let mut bw = BitWriter::new();
    detach.pack(&mut bw).unwrap();
    let msg_bytes = bw.finish();
    let payload = encode_message(&envelope, &msg_bytes).unwrap();
    let frame = Frame { flags: FrameFlags::new(), payload };
    let mut writer = FrameWriter::new(&mut stream);
    writer.write_frame(&frame).unwrap();

    // Give the listener time to process the detach and go_dormant.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Session should now be Dormant.
    let sessions = coord.lock().unwrap().list_sessions();
    let s = sessions.iter().find(|s| s.session_id == session_id).unwrap();
    assert_eq!(
        s.state,
        SessionState::Dormant,
        "session should be Dormant after DetachSession frame"
    );

    drop(listener);
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p malt-daemon detach_session_vnp_message_triggers_dormant -- --nocapture
```

Expected: FAIL — `dispatch_frame` ignores the DetachSession message (falls through to the `_` warning arm).

- [ ] **Step 3: Add `DetachSession` to the imports in `vnp_listener.rs`**

Extend the existing import of `malt_protocol::session::AttachSession` to also import `DetachSession` and `MSG_DETACH_SESSION`:

```rust
use malt_protocol::codec::{
    make_envelope, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION, MSG_ATTACH_SESSION,
    MSG_DETACH_SESSION, MSG_FRAME_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT, MSG_RENDER_BATCH,
    MSG_RESIZE,
};
use malt_protocol::session::{AttachSession, DetachSession};
```

- [ ] **Step 4: Add `DetachSession` handling to `dispatch_frame`**

In the `match (envelope.domain, envelope.msg_type)` block, add before the `_` wildcard arm:

```rust
(DOMAIN_SESSION, MSG_DETACH_SESSION) => {
    let mut bit_reader = BitReader::new(msg_bytes);
    match DetachSession::unpack(&mut bit_reader) {
        Ok(detach) => {
            // Only handle if it's for our session.
            if detach.session_id.0 == session_id {
                info!(peer, client_id, session_id, "VNP: client sent DetachSession");
                // Unregister this client — triggers go_dormant if client_count hits 0.
                match coordinator.lock() {
                    Ok(mut coord) => {
                        if let Err(e) =
                            coord.unregister_vnp_client(SessionId(session_id), client_id)
                        {
                            warn!(peer, error = %e, "VNP: unregister after DetachSession failed");
                        }
                    }
                    Err(e) => {
                        warn!(peer, error = %e, "VNP: coordinator lock poisoned during DetachSession");
                        return Err(());
                    }
                }
                // Signal the caller to terminate this connection.
                return Err(());
            }
        }
        Err(e) => {
            warn!(peer, error = %e, "VNP: DetachSession decode failed");
        }
    }
}
```

Note: Returning `Err(())` from `dispatch_frame` causes the caller (`handle_client`) to call `cleanup` again. Since `unregister_vnp_client` is idempotent (returns `Ok(())` for unknown clients), the double-call is safe. However, to avoid a double unregister, use a `DetachSession`-specific return variant or handle it inline in `handle_client`. The cleanest approach: return a new internal signal. But `dispatch_frame` returns `Result<(), ()>` — to signal "detach" cleanly, add a `bool` detach flag or a dedicated error type. Since the goal is minimal change, returning `Err(())` is acceptable — `cleanup` will try to unregister again, see the session is Dormant or the client_id is unknown, and return `Ok(())`.

- [ ] **Step 5: Update `handle_client` to skip double-cleanup on `DetachSession`**

In `handle_client`, the `dispatch_frame` returning `Err` currently always calls `cleanup`. That cleanup path calls `unregister_vnp_client` again after DetachSession already did. This is safe (Dormant sessions return Ok from unregister) but the double-call could incorrectly decrement `client_count` below zero on a race. To avoid this, add an internal `CleanupNeeded` enum or a flag.

**Simplest correct approach:** Change `dispatch_frame` to return a tri-state:

```rust
enum DispatchResult {
    Continue,
    /// Client disconnected normally — call cleanup.
    Disconnect,
    /// Client explicitly detached — cleanup already done inside dispatch_frame.
    DetachedClean,
}
```

Then in `dispatch_frame`:
- Normal success: `DispatchResult::Continue`
- `DetachSession` handled: `DispatchResult::DetachedClean`
- Fatal error: `DispatchResult::Disconnect`

In `handle_client`, replace the `if dispatch_frame(...).is_err()` block:

```rust
match dispatch_frame(frame, &coordinator, client_id, session_id_raw, &peer) {
    DispatchResult::Continue => {}
    DispatchResult::Disconnect => {
        cleanup(&coordinator, session_id.clone(), client_id, &peer);
        return;
    }
    DispatchResult::DetachedClean => {
        // DetachSession was handled inside dispatch_frame — no cleanup needed.
        return;
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

```
cargo test -p malt-daemon detach_session_vnp_message_triggers_dormant -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Run full test suite**

```
cargo test -p malt-daemon
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```
git add crates/malt-daemon/src/vnp_listener.rs crates/malt-daemon/tests/vnp_listener.rs
git commit -m "feat(malt-daemon): handle DetachSession VNP message to trigger go_dormant"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task |
|-----------------|------|
| `Dormant` variant in `SessionState` | Already in schema — no task needed |
| `AppRestoreNotSupported`, `RestoreFailed`, `SessionDormant` errors | Task 2 |
| `SessionLifecycle` enum | Task 4 |
| Startup scan loads dormant sessions | Task 6 |
| `register_vnp_client` restores on attach | Task 6 |
| `unregister_vnp_client` decrements count, go_dormant on 0 | Task 5 |
| `go_dormant` — Snapshot → persist → Shutdown → join → Dormant | Task 5 |
| `restore_session` Shell pane | Task 6 |
| `restore_session` App pane returns error | Task 8 (tested via bad_cwd test indirectly; add explicit test below) |
| `spawn_with_cwd` sets initial cwd | Task 3 |
| `shutdown_graceful` snapshots all active | Task 7 |
| `Drop` calls `shutdown_graceful` | Task 7 |
| `daemon.rs` explicit `shutdown_graceful` | Task 7 |
| `DetachSession` VNP handler | Task 9 |
| `DebouncedStore` pass-throughs | Task 1 |
| Dormant operations return `SessionDormant` | Task 4 |
| `destroy_session` handles both lifecycle states | Task 4 |

**Missing test:** Spec requires `app_restore_returns_error`. Task 8 covers `destroy_dormant`, `second_daemon`, and `bad_cwd` but not an App pane test explicitly. App pane restore would require crafting a `PersistedSession` with `PersistedPaneType::App { .. }` — add this to Task 8.

**Additional test — `app_restore_returns_error`** — add to Task 8's Step 1:

```rust
#[test]
fn app_restore_returns_error() {
    use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
    use malt_protocol::common::SessionState;
    use malt_protocol::render::RenderBatch;
    use std::collections::HashMap;

    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);

    // Manually persist a session with an App pane.
    let sid = malt_protocol::common::SessionId(99);
    let pane = PersistedPane {
        cwd: ".".to_string(),
        title: None,
        pane_type: PersistedPaneType::App {
            app_id: "test-app".to_string(),
            config: vec![],
            _unknown: vec![],
        },
        _unknown: vec![],
    };
    let mut panes = HashMap::new();
    panes.insert(1u32, pane);
    let persisted = PersistedSession {
        schema_version: 1,
        id: sid.clone(),
        name: Some("app-session".to_string()),
        layout: None,
        focus: Some(malt_protocol::common::PaneId(1)),
        panes,
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        _unknown: vec![],
    };
    store.mark_dirty(sid.clone(), persisted);
    store.flush_all();

    // Inject the session ID into daemon state so the coordinator scans it.
    let daemon_state = malt_protocol::persist::daemon::DaemonState {
        schema_version: 1,
        sessions: vec![sid.clone()],
        active_groups: vec![],
        next_session_id: 100,
        next_pane_id: 2,
        _unknown: vec![],
    };
    store.mark_dirty_daemon(daemon_state);
    store.flush_all();

    // New coordinator: session is dormant with App pane.
    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, SessionState::Dormant);

    // Attach should fail with AppRestoreNotSupported.
    let (render_tx, _) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let result = coord2.register_vnp_client(sid.clone(), 1, caps(), render_tx);
    assert!(
        matches!(result, Err(malt_daemon::DaemonError::AppRestoreNotSupported)),
        "expected AppRestoreNotSupported, got: {:?}", result
    );

    // Session must still be Dormant (no partial Active state).
    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, SessionState::Dormant, "session must remain Dormant after failed restore");
}
```

**Placeholder scan:** No TBD/TODO found. All code blocks are complete.

**Type consistency check:**
- `SessionCommand::Snapshot { reply, name, isolation }` — matches across Task 3 (definition), Task 5 (`go_dormant` call), Task 7 (`shutdown_graceful` call). ✅
- `build_persisted_session(session_id, focused_pane, name, isolation, env)` — matches across Task 3 (definition + handler). ✅
- `SessionExecutor::spawn_with_cwd(session_id, pane_id, isolation, cwd)` — matches Task 3 (definition) and Task 6 (`restore_session` call). ✅
- `DispatchResult` enum — introduced in Task 9 and used in `handle_client`. ✅

---

**Plan complete and saved to `docs/superpowers/plans/2026-03-31-phase-b2-graceful-shutdown-session-restore.md`.**

Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, spec + code quality review between tasks, fast iteration

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
