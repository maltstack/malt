# Phase 3D: Session Store + Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build session persistence using vexil-runtime's Pack/Unpack traits (bitpack format) with atomic file writes.

**Architecture:** The generated persist types (PersistedSession, DaemonState) already implement `Pack`/`Unpack` via vexil-runtime. We serialize directly to bitpack bytes and write to files with atomic temp+rename. No Value conversion or CompiledSchema needed.

**Tech Stack:** Rust, vexil-runtime (Pack/Unpack/BitWriter/BitReader), malt-protocol persist types, tempfile (for tests)

---

## File Structure

```
malt-daemon/
  src/
    store/
      mod.rs            — SessionStore: save/load/list/delete with atomic writes
      error.rs          — StoreError enum
  ...existing modules...
```

---

### Task 1: StoreError and Module Setup

**Files:**
- Create: `crates/malt-daemon/src/store/mod.rs`
- Create: `crates/malt-daemon/src/store/error.rs`
- Modify: `crates/malt-daemon/src/lib.rs`

- [ ] **Step 1: Create store/error.rs**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("session not found: {0:?}")]
    SessionNotFound(malt_protocol::common::SessionId),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("corrupt file: {path}")]
    CorruptFile { path: String },
}
```

- [ ] **Step 2: Create store/mod.rs stub**

```rust
pub mod error;

pub use error::StoreError;
```

- [ ] **Step 3: Add store module to lib.rs**

Add `pub mod store;` to `crates/malt-daemon/src/lib.rs` (after the existing module declarations).

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p malt-daemon`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add crates/malt-daemon/src/store/
git commit -m "feat(malt-daemon): store module scaffolding with StoreError"
```

---

### Task 2: SessionStore Implementation

**Files:**
- Modify: `crates/malt-daemon/src/store/mod.rs`
- Create: `crates/malt-daemon/tests/store.rs`

- [ ] **Step 1: Write failing tests**

First, add `tempfile` as a dev-dependency in `crates/malt-daemon/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

Create `crates/malt-daemon/tests/store.rs`:
```rust
use malt_daemon::store::{SessionStore, StoreError};
use malt_protocol::common::{
    GroupId, IsolationTier, LayoutNode, PaneId, SessionId,
};
use malt_protocol::persist::daemon::{DaemonState, GroupPolicy, GroupState};
use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
use tempfile::tempdir;

fn make_session(id: u32) -> PersistedSession {
    PersistedSession {
        schema_version: 1,
        id: SessionId(id),
        name: Some(format!("session-{id}")),
        layout: LayoutNode::Leaf {
            pane_id: PaneId(1),
        },
        focus: PaneId(1),
        panes: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                1,
                PersistedPane {
                    cwd: "/home/user".to_string(),
                    title: Some("bash".to_string()),
                    pane_type: PersistedPaneType::Shell {
                        shell_path: "/bin/bash".to_string(),
                    },
                    _unknown: Vec::new(),
                },
            );
            m
        },
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        _unknown: Vec::new(),
    }
}

fn make_daemon_state() -> DaemonState {
    DaemonState {
        schema_version: 1,
        sessions: vec![SessionId(1), SessionId(2)],
        active_groups: vec![],
        next_session_id: 3,
        next_pane_id: 5,
        _unknown: Vec::new(),
    }
}

#[test]
fn save_and_load_session() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = make_session(1);
    store.save_session(SessionId(1), &session).unwrap();
    let loaded = store.load_session(SessionId(1)).unwrap();
    assert_eq!(loaded.id, SessionId(1));
    assert_eq!(loaded.name, Some("session-1".to_string()));
    assert_eq!(loaded.schema_version, 1);
}

#[test]
fn list_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.save_session(SessionId(1), &make_session(1)).unwrap();
    store.save_session(SessionId(2), &make_session(2)).unwrap();
    store.save_session(SessionId(3), &make_session(3)).unwrap();
    let mut ids = store.list_sessions().unwrap();
    ids.sort_by_key(|id| id.0);
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], SessionId(1));
    assert_eq!(ids[2], SessionId(3));
}

#[test]
fn delete_session() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.save_session(SessionId(1), &make_session(1)).unwrap();
    store.delete_session(SessionId(1)).unwrap();
    let result = store.load_session(SessionId(1));
    assert!(result.is_err());
}

#[test]
fn atomic_write_creates_file() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.save_session(SessionId(42), &make_session(42)).unwrap();
    let path = dir.path().join("sessions").join("42.vxb");
    assert!(path.exists());
    // Temp file should NOT exist
    let tmp_path = dir.path().join("sessions").join("42.vxb.tmp");
    assert!(!tmp_path.exists());
}

#[test]
fn save_and_load_daemon_state() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let state = make_daemon_state();
    store.save_daemon_state(&state).unwrap();
    let loaded = store.load_daemon_state().unwrap();
    assert_eq!(loaded.next_session_id, 3);
    assert_eq!(loaded.next_pane_id, 5);
    assert_eq!(loaded.sessions.len(), 2);
}

#[test]
fn load_nonexistent_returns_error() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let result = store.load_session(SessionId(999));
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test store`
Expected: FAIL — SessionStore not implemented

- [ ] **Step 3: Implement SessionStore**

Write `crates/malt-daemon/src/store/mod.rs`:
```rust
pub mod error;

pub use error::StoreError;

use malt_protocol::common::SessionId;
use malt_protocol::persist::daemon::DaemonState;
use malt_protocol::persist::session::PersistedSession;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

/// Persistent storage for sessions and daemon state.
///
/// Uses vexil-runtime bitpack format (.vxb files) with atomic writes.
/// File layout:
/// ```text
/// base_dir/
///   sessions/
///     {session_id}.vxb
///   daemon.vxb
/// ```
pub struct SessionStore {
    base_dir: PathBuf,
}

impl SessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    fn session_path(&self, id: SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{}.vxb", id.0))
    }

    fn daemon_state_path(&self) -> PathBuf {
        self.base_dir.join("daemon.vxb")
    }

    /// Save a session to disk (atomic write).
    pub fn save_session(
        &self,
        id: SessionId,
        session: &PersistedSession,
    ) -> Result<(), StoreError> {
        let dir = self.sessions_dir();
        fs::create_dir_all(&dir)?;

        let bytes = pack_to_bytes(session)?;
        atomic_write(&self.session_path(id), &bytes)?;

        info!(?id, "session saved");
        Ok(())
    }

    /// Load a session from disk.
    pub fn load_session(&self, id: SessionId) -> Result<PersistedSession, StoreError> {
        let path = self.session_path(id);
        if !path.exists() {
            return Err(StoreError::SessionNotFound(id));
        }
        let bytes = fs::read(&path)?;
        unpack_from_bytes(&bytes, &path)
    }

    /// List all persisted session IDs.
    pub fn list_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        let dir = self.sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".vxb") {
                if let Ok(id) = stem.parse::<u32>() {
                    ids.push(SessionId(id));
                }
            }
        }
        Ok(ids)
    }

    /// Delete a persisted session.
    pub fn delete_session(&self, id: SessionId) -> Result<(), StoreError> {
        let path = self.session_path(id);
        if path.exists() {
            fs::remove_file(&path)?;
            info!(?id, "session deleted");
        }
        Ok(())
    }

    /// Save daemon state to disk (atomic write).
    pub fn save_daemon_state(&self, state: &DaemonState) -> Result<(), StoreError> {
        fs::create_dir_all(&self.base_dir)?;
        let bytes = pack_to_bytes(state)?;
        atomic_write(&self.daemon_state_path(), &bytes)?;
        info!("daemon state saved");
        Ok(())
    }

    /// Load daemon state from disk.
    pub fn load_daemon_state(&self) -> Result<DaemonState, StoreError> {
        let path = self.daemon_state_path();
        if !path.exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "daemon.vxb not found",
            )));
        }
        let bytes = fs::read(&path)?;
        unpack_from_bytes(&bytes, &path)
    }
}

/// Serialize a type to bitpack bytes.
fn pack_to_bytes<T: Pack>(value: &T) -> Result<Vec<u8>, StoreError> {
    let mut w = BitWriter::new();
    value
        .pack(&mut w)
        .map_err(|e| StoreError::Encode(format!("{e}")))?;
    Ok(w.finish())
}

/// Deserialize a type from bitpack bytes.
fn unpack_from_bytes<T: Unpack>(bytes: &[u8], path: &Path) -> Result<T, StoreError> {
    let mut r = BitReader::new(bytes);
    T::unpack(&mut r).map_err(|e| StoreError::Decode(format!("{}: {e}", path.display())))
}

/// Write bytes to a file atomically (temp + rename).
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), StoreError> {
    let tmp_path = path.with_extension("vxb.tmp");
    fs::write(&tmp_path, data)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test store`
Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-daemon/src/store/ crates/malt-daemon/tests/store.rs crates/malt-daemon/Cargo.toml
git commit -m "feat(malt-daemon): session store — bitpack persistence with atomic writes"
```

---

### Task 3: Pane Type Roundtrip Tests

**Files:**
- Modify: `crates/malt-daemon/tests/store.rs`

- [ ] **Step 1: Add roundtrip tests for different pane types**

Append to `crates/malt-daemon/tests/store.rs`:
```rust
#[test]
fn roundtrip_app_pane() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = make_session(1);
    session.panes.insert(
        2,
        PersistedPane {
            cwd: "/tmp".to_string(),
            title: Some("my-app".to_string()),
            pane_type: PersistedPaneType::App {
                app_id: "com.example.app".to_string(),
                config: Some(vec![1, 2, 3, 4]),
            },
            _unknown: Vec::new(),
        },
    );
    store.save_session(SessionId(1), &session).unwrap();
    let loaded = store.load_session(SessionId(1)).unwrap();
    let pane = loaded.panes.get(&2).unwrap();
    match &pane.pane_type {
        PersistedPaneType::App { app_id, config } => {
            assert_eq!(app_id, "com.example.app");
            assert_eq!(config, &Some(vec![1, 2, 3, 4]));
        }
        other => panic!("expected App, got {other:?}"),
    }
}

#[test]
fn roundtrip_compat_pane() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = make_session(1);
    session.panes.insert(
        3,
        PersistedPane {
            cwd: "/home/user".to_string(),
            title: None,
            pane_type: PersistedPaneType::Compat {
                program: "vim".to_string(),
                args: vec!["file.txt".to_string()],
            },
            _unknown: Vec::new(),
        },
    );
    store.save_session(SessionId(1), &session).unwrap();
    let loaded = store.load_session(SessionId(1)).unwrap();
    let pane = loaded.panes.get(&3).unwrap();
    match &pane.pane_type {
        PersistedPaneType::Compat { program, args } => {
            assert_eq!(program, "vim");
            assert_eq!(args, &vec!["file.txt".to_string()]);
        }
        other => panic!("expected Compat, got {other:?}"),
    }
}

#[test]
fn roundtrip_complex_layout() {
    use malt_protocol::common::{Direction, SplitId, SplitSize};
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = make_session(1);
    session.layout = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Box::new(Direction::Vertical),
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    session.panes.insert(
        2,
        PersistedPane {
            cwd: "/tmp".to_string(),
            title: None,
            pane_type: PersistedPaneType::Shell {
                shell_path: "/bin/zsh".to_string(),
            },
            _unknown: Vec::new(),
        },
    );
    store.save_session(SessionId(1), &session).unwrap();
    let loaded = store.load_session(SessionId(1)).unwrap();
    match &loaded.layout {
        LayoutNode::Split { children, .. } => {
            assert_eq!(children.len(), 2);
        }
        other => panic!("expected Split, got {other:?}"),
    }
}

#[test]
fn roundtrip_daemon_state_with_groups() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let state = DaemonState {
        schema_version: 1,
        sessions: vec![SessionId(1)],
        active_groups: vec![GroupState {
            id: GroupId(1),
            name: "dev".to_string(),
            policy: Some(GroupPolicy {
                min_tier: IsolationTier::Restricted,
                max_memory_mb: 512,
                max_cpu_cores: 4,
                max_sessions: 10,
                ttl_secs: Some(3600),
                idle_timeout_secs: Some(300),
                on_empty: malt_protocol::common::OnEmpty::Keep,
                on_oom: malt_protocol::common::OnOom::PauseAndNotify,
                _unknown: Vec::new(),
            }),
            session_ids: vec![SessionId(1)],
            _unknown: Vec::new(),
        }],
        next_session_id: 2,
        next_pane_id: 3,
        _unknown: Vec::new(),
    };
    store.save_daemon_state(&state).unwrap();
    let loaded = store.load_daemon_state().unwrap();
    assert_eq!(loaded.active_groups.len(), 1);
    assert_eq!(loaded.active_groups[0].name, "dev");
    let policy = loaded.active_groups[0].policy.as_ref().unwrap();
    assert_eq!(policy.max_memory_mb, 512);
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p malt-daemon --test store`
Expected: all 10 tests PASS

- [ ] **Step 3: Commit**

```bash
git add crates/malt-daemon/tests/store.rs
git commit -m "test(malt-daemon): store roundtrip tests for pane types, layout, groups"
```

---

### Task 4: Final Verification

- [ ] **Step 1: Run all malt-daemon tests**

Run: `cargo test -p malt-daemon`
Expected: all tests PASS (47 existing + 10 new = 57 total)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p malt-daemon -- -W clippy::all -A unused-imports`
Expected: no warnings from malt-daemon code

- [ ] **Step 3: Fix any issues**

- [ ] **Step 4: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all PASS

- [ ] **Step 5: Commit if fixes needed**

```bash
git add -u
git commit -m "fix(malt-daemon): clippy fixes for store module"
```
