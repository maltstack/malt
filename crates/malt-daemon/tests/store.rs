use malt_daemon::store::SessionStore;
use malt_protocol::common::{IsolationTier, LayoutNode, PaneId, SessionId};
use malt_protocol::persist::daemon::DaemonState;
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
    store.save_session(&SessionId(1), &session).unwrap();
    let loaded = store.load_session(&SessionId(1)).unwrap();
    assert_eq!(loaded.id, SessionId(1));
    assert_eq!(loaded.name, Some("session-1".to_string()));
    assert_eq!(loaded.schema_version, 1);
}

#[test]
fn list_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store
        .save_session(&SessionId(1), &make_session(1))
        .unwrap();
    store
        .save_session(&SessionId(2), &make_session(2))
        .unwrap();
    store
        .save_session(&SessionId(3), &make_session(3))
        .unwrap();
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
    store
        .save_session(&SessionId(1), &make_session(1))
        .unwrap();
    store.delete_session(&SessionId(1)).unwrap();
    let result = store.load_session(&SessionId(1));
    assert!(result.is_err());
}

#[test]
fn atomic_write_creates_file() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store
        .save_session(&SessionId(42), &make_session(42))
        .unwrap();
    let path = dir.path().join("sessions").join("42.vxb");
    assert!(path.exists());
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
    let result = store.load_session(&SessionId(999));
    assert!(result.is_err());
}
