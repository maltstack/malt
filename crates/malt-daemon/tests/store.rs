use malt_daemon::store::SessionStore;
use malt_protocol::common::{
    Direction, GroupId, IsolationTier, LayoutNode, OnEmpty, OnOom, PaneId, SessionId, SplitId,
    SplitSize,
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

#[test]
fn roundtrip_app_pane() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    let config_bytes = vec![0xCA, 0xFE, 0xBA, 0xBE];
    let session = PersistedSession {
        schema_version: 1,
        id: SessionId(10),
        name: Some("app-session".to_string()),
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
                    title: Some("my-app".to_string()),
                    pane_type: PersistedPaneType::App {
                        app_id: "com.example.widget".to_string(),
                        config: Some(config_bytes.clone()),
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
    };

    store.save_session(&SessionId(10), &session).unwrap();
    let loaded = store.load_session(&SessionId(10)).unwrap();

    let pane = loaded.panes.get(&1).expect("pane 1 missing");
    match &pane.pane_type {
        PersistedPaneType::App { app_id, config } => {
            assert_eq!(app_id, "com.example.widget");
            assert_eq!(config.as_deref(), Some(config_bytes.as_slice()));
        }
        other => panic!("expected App pane, got {:?}", other),
    }
}

#[test]
fn roundtrip_compat_pane() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    let session = PersistedSession {
        schema_version: 1,
        id: SessionId(11),
        name: Some("compat-session".to_string()),
        layout: LayoutNode::Leaf {
            pane_id: PaneId(1),
        },
        focus: PaneId(1),
        panes: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                1,
                PersistedPane {
                    cwd: "/tmp".to_string(),
                    title: Some("vim".to_string()),
                    pane_type: PersistedPaneType::Compat {
                        program: "/usr/bin/vim".to_string(),
                        args: vec!["--clean".to_string(), "file.txt".to_string()],
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
    };

    store.save_session(&SessionId(11), &session).unwrap();
    let loaded = store.load_session(&SessionId(11)).unwrap();

    let pane = loaded.panes.get(&1).expect("pane 1 missing");
    match &pane.pane_type {
        PersistedPaneType::Compat { program, args } => {
            assert_eq!(program, "/usr/bin/vim");
            assert_eq!(args, &["--clean", "file.txt"]);
        }
        other => panic!("expected Compat pane, got {:?}", other),
    }
}

#[test]
fn roundtrip_complex_layout() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    let session = PersistedSession {
        schema_version: 1,
        id: SessionId(12),
        name: Some("split-session".to_string()),
        layout: LayoutNode::Split {
            split_id: SplitId(100),
            direction: Direction::Vertical,
            sizes: vec![
                SplitSize::Ratio { value: 0.5 },
                SplitSize::Ratio { value: 0.5 },
            ],
            children: vec![
                LayoutNode::Leaf {
                    pane_id: PaneId(1),
                },
                LayoutNode::Leaf {
                    pane_id: PaneId(2),
                },
            ],
        },
        focus: PaneId(1),
        panes: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                1,
                PersistedPane {
                    cwd: "/home/user".to_string(),
                    title: Some("left".to_string()),
                    pane_type: PersistedPaneType::Shell {
                        shell_path: "/bin/bash".to_string(),
                    },
                    _unknown: Vec::new(),
                },
            );
            m.insert(
                2,
                PersistedPane {
                    cwd: "/home/user".to_string(),
                    title: Some("right".to_string()),
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
    };

    store.save_session(&SessionId(12), &session).unwrap();
    let loaded = store.load_session(&SessionId(12)).unwrap();

    match &loaded.layout {
        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } => {
            assert_eq!(*split_id, SplitId(100));
            assert_eq!(*direction, Direction::Vertical);
            assert_eq!(sizes.len(), 2);
            assert_eq!(children.len(), 2);
        }
        other => panic!("expected Split layout, got {:?}", other),
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
            id: GroupId(42),
            name: "dev-group".to_string(),
            policy: Some(GroupPolicy {
                min_tier: IsolationTier::Restricted,
                max_memory_mb: 2048,
                max_cpu_cores: 4,
                max_sessions: 8,
                ttl_secs: Some(3600),
                idle_timeout_secs: None,
                on_empty: OnEmpty::Keep,
                on_oom: OnOom::PauseAndNotify,
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
    let group = &loaded.active_groups[0];
    assert_eq!(group.id, GroupId(42));
    assert_eq!(group.name, "dev-group");
    assert_eq!(group.session_ids, vec![SessionId(1)]);

    let policy = group.policy.as_ref().expect("policy missing");
    assert_eq!(policy.min_tier, IsolationTier::Restricted);
    assert_eq!(policy.max_memory_mb, 2048);
    assert_eq!(policy.max_cpu_cores, 4);
    assert_eq!(policy.max_sessions, 8);
    assert_eq!(policy.ttl_secs, Some(3600));
    assert_eq!(policy.idle_timeout_secs, None);
    assert_eq!(policy.on_empty, OnEmpty::Keep);
    assert_eq!(policy.on_oom, OnOom::PauseAndNotify);
}
