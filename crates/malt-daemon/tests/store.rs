use malt_daemon::store::{SessionStore, StoreError};
use malt_protocol::common::{
    Direction, GroupId, IsolationTier, LayoutNode, OnEmpty, OnOom, PaneId, SessionId, SplitId,
    SplitSize,
};
use malt_protocol::persist::daemon::{DaemonState, GroupPolicy, GroupState};
use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
use tempfile::tempdir;
use vexil_runtime::{BitReader, Unpack};

fn make_session(id: u32) -> PersistedSession {
    PersistedSession {
        schema_version: 1,
        id: SessionId(id),
        name: Some(format!("session-{id}")),
        layout: LayoutNode::Leaf { pane_id: PaneId(1) },
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
                        env_snapshot: None,
                    },
                    command_blocks: Vec::new(),
                    _unknown: Vec::new(),
                },
            );
            m
        },
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        selected_image: Some(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        ),
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
    assert_eq!(loaded.selected_image, session.selected_image);
}

#[test]
fn command_history_survives_the_bitpack_round_trip() {
    use malt_protocol::persist::session::PersistedCommandBlock;

    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    let mut session = make_session(1);
    session.panes.get_mut(&1).unwrap().command_blocks = vec![
        PersistedCommandBlock {
            command_id: 1,
            cmd: "cargo test --workspace".to_string(),
            started_at: 1_784_070_000_123,
            finished_at: Some(1_784_070_042_456),
            exit_code: Some(0),
            _unknown: Vec::new(),
        },
        PersistedCommandBlock {
            command_id: 2,
            cmd: "false".to_string(),
            started_at: 1_784_070_050_000,
            finished_at: Some(1_784_070_050_010),
            exit_code: Some(1),
            _unknown: Vec::new(),
        },
        // Interrupted: the daemon stopped while this was still running. Both
        // optional fields must come back absent -- a record that says "not
        // confirmed complete" is the whole point, and a decoder that filled
        // in a default 0 here would silently report a success that never
        // happened.
        PersistedCommandBlock {
            command_id: 3,
            cmd: "sleep 300".to_string(),
            started_at: 1_784_070_060_000,
            finished_at: None,
            exit_code: None,
            _unknown: Vec::new(),
        },
    ];

    store.save_session(&SessionId(1), &session).unwrap();
    let loaded = store.load_session(&SessionId(1)).unwrap();

    let blocks = &loaded.panes.get(&1).unwrap().command_blocks;
    assert_eq!(
        blocks,
        &session.panes.get(&1).unwrap().command_blocks,
        "every field of every command block must survive encode/decode"
    );
    assert!(blocks[2].finished_at.is_none());
    assert!(blocks[2].exit_code.is_none());
}

#[test]
fn a_session_with_no_command_history_round_trips_as_empty() {
    // The field is additive: sessions persisted before it existed decode
    // with an empty list rather than failing to load at all.
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.save_session(&SessionId(1), &make_session(1)).unwrap();
    let loaded = store.load_session(&SessionId(1)).unwrap();
    assert!(loaded.panes.get(&1).unwrap().command_blocks.is_empty());
}

#[test]
fn list_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.save_session(&SessionId(1), &make_session(1)).unwrap();
    store.save_session(&SessionId(2), &make_session(2)).unwrap();
    store.save_session(&SessionId(3), &make_session(3)).unwrap();
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
    store.save_session(&SessionId(1), &make_session(1)).unwrap();
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
        layout: LayoutNode::Leaf { pane_id: PaneId(1) },
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
                    command_blocks: Vec::new(),
                    _unknown: Vec::new(),
                },
            );
            m
        },
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        selected_image: None,
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
        layout: LayoutNode::Leaf { pane_id: PaneId(1) },
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
                    command_blocks: Vec::new(),
                    _unknown: Vec::new(),
                },
            );
            m
        },
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        selected_image: None,
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
                LayoutNode::Leaf { pane_id: PaneId(1) },
                LayoutNode::Leaf { pane_id: PaneId(2) },
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
                        env_snapshot: None,
                    },
                    command_blocks: Vec::new(),
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
                        env_snapshot: None,
                    },
                    command_blocks: Vec::new(),
                    _unknown: Vec::new(),
                },
            );
            m
        },
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        selected_image: None,
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

fn make_daemon_state_with_id(next_id: u32) -> DaemonState {
    DaemonState {
        schema_version: 1,
        sessions: vec![],
        active_groups: vec![],
        next_session_id: next_id,
        next_pane_id: 1,
        _unknown: vec![],
    }
}

#[test]
fn atomic_write_creates_bak() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    // First save — no .bak yet
    store
        .save_daemon_state(&make_daemon_state_with_id(10))
        .unwrap();
    let bak = dir.path().join("daemon.vxb.bak");
    assert!(!bak.exists(), ".bak must not exist after first save");

    // Second save — .bak must hold first-save content
    store
        .save_daemon_state(&make_daemon_state_with_id(20))
        .unwrap();
    assert!(bak.exists(), ".bak must exist after second save");

    // .bak must decode as DaemonState with next_session_id=10
    let bak_bytes = std::fs::read(&bak).unwrap();
    let mut r = BitReader::new(&bak_bytes);
    let recovered = DaemonState::unpack(&mut r).unwrap();
    assert_eq!(
        recovered.next_session_id, 10,
        ".bak should contain first-save state"
    );

    // Main file must contain the second-save content
    let current = store.load_daemon_state().unwrap();
    assert_eq!(current.next_session_id, 20);
}

#[test]
fn corrupt_file_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    // Write garbage daemon state bytes directly
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("daemon.vxb"),
        b"this is not valid bitpack data!!!",
    )
    .unwrap();

    let err = store.load_daemon_state().unwrap_err();
    match &err {
        StoreError::CorruptFile {
            path,
            reason: _,
            moved_to,
        } => {
            assert!(
                path.ends_with("daemon.vxb"),
                "path should be daemon.vxb, got {path:?}"
            );
            assert!(
                moved_to.exists(),
                "corrupt file should have been moved to quarantine path"
            );
            let name = moved_to.file_name().unwrap().to_string_lossy();
            assert!(
                name.contains(".corrupt.") && name.ends_with(".vxb"),
                "quarantine filename must contain '.corrupt.' and end with '.vxb': {name}"
            );
        }
        other => panic!("expected CorruptFile, got {other:?}"),
    }

    // Original file must no longer exist
    assert!(
        !dir.path().join("daemon.vxb").exists(),
        "original file must be gone after quarantine"
    );
}

#[test]
fn corrupt_file_daemon_continues_with_defaults() {
    // Verifies that CorruptFile does NOT panic and callers can recover gracefully.
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path().join("daemon.vxb"), b"garbage").unwrap();

    let result = store.load_daemon_state();
    assert!(
        matches!(result, Err(StoreError::CorruptFile { .. })),
        "expected CorruptFile, got {result:?}"
    );
    // No panic here — the coordinator would log a warning and use defaults.
}
