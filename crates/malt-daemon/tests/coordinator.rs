use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_protocol::common::{IsolationTier, SessionId, SessionState};
use std::sync::mpsc;

fn make_store(dir: &tempfile::TempDir) -> DebouncedStore {
    DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
}

#[test]
fn create_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    assert_eq!(id, SessionId(1));
}

#[test]
fn create_multiple_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id1 = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let id2 = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    assert_eq!(id1, SessionId(1));
    assert_eq!(id2, SessionId(2));
    assert_eq!(coord.session_count(), 2);
}

#[test]
fn destroy_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord.destroy_session(id);
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn list_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    coord
        .create_session(Some("alpha".to_string()), IsolationTier::Bare, None)
        .unwrap();
    coord
        .create_session(Some("beta".to_string()), IsolationTier::Restricted, None)
        .unwrap();
    let sessions = coord.list_sessions();
    assert_eq!(sessions.len(), 2);
}

#[test]
fn session_ids_never_recycled() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id1 = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord.destroy_session(id1.clone());
    let id2 = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    assert_ne!(id1, id2);
    assert_eq!(id2, SessionId(2));
}

use malt_daemon::bus::BusMessage;
use malt_daemon::executor::session_thread::SessionCommand;
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, KeyModifiers, UnicodeLevel,
};
use malt_protocol::input::{KeyEvent, KeyValue};
use malt_protocol::priority::Priority;
use malt_protocol::render::RenderBatch;

fn caps() -> ClientCapabilities {
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}

#[test]
fn route_to_nonexistent_session_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let coord = Coordinator::new(PoolConfig::default(), store);
    let msg = BusMessage {
        domain: 1,
        msg_type: 1,
        priority: Priority::Normal,
        producer_id: 0,
        payload: vec![],
    };
    let err = coord.route_to_session(SessionId(999), msg).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("session not found"), "got: {msg}");
}

#[test]
fn send_attach_command() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord
        .send_command(
            id.clone(),
            SessionCommand::AttachClient {
                client_id: 42,
                authority: InputAuthority::Exclusive,
            },
        )
        .unwrap();
    coord.destroy_session(id);
}

#[test]
fn shutdown_all_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord.shutdown_all();
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn register_vnp_client_returns_initial_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let initial = coord
        .register_vnp_client(sid, 1, caps(), render_tx)
        .unwrap();
    assert_eq!(initial.frame_seq, 0);
}

#[test]
fn unregister_vnp_client_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let _ = coord
        .register_vnp_client(sid.clone(), 1, caps(), render_tx)
        .unwrap();
    coord.unregister_vnp_client(sid, 1).unwrap();
}

#[test]
fn send_key_input_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let key = KeyEvent {
        key: KeyValue::Char {
            codepoint: 'x' as u32,
        },
        modifiers: KeyModifiers::empty(),
        _unknown: Vec::new(),
    };
    coord.send_key_input(sid, key).unwrap();
}

#[test]
fn ack_frame_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    coord.ack_frame(sid, 1, 0).unwrap();
}

#[test]
fn name_uniqueness_appends_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);

    // Create "foo" — succeeds, name is "foo"
    coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();

    // Create "foo" again — must get "foo-2"
    let id2 = coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();
    let sessions = coord.list_sessions();
    let s2 = sessions.iter().find(|s| s.session_id == id2).unwrap();
    assert_eq!(
        s2.name.as_deref(),
        Some("foo-2"),
        "second 'foo' should be named 'foo-2'"
    );
}

#[test]
fn name_uniqueness_increments_past_existing() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);

    // Pre-fill foo, foo-2, foo-3
    let id1 = coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();
    let id2 = coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();
    let id3 = coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();

    // Verify the three pre-filled sessions have the expected names
    let sessions = coord.list_sessions();
    let s1 = sessions.iter().find(|s| s.session_id == id1).unwrap();
    assert_eq!(
        s1.name.as_deref(),
        Some("foo"),
        "first 'foo' should be named 'foo'"
    );
    let s2 = sessions.iter().find(|s| s.session_id == id2).unwrap();
    assert_eq!(
        s2.name.as_deref(),
        Some("foo-2"),
        "second 'foo' should be named 'foo-2'"
    );
    let s3 = sessions.iter().find(|s| s.session_id == id3).unwrap();
    assert_eq!(
        s3.name.as_deref(),
        Some("foo-3"),
        "third 'foo' should be named 'foo-3'"
    );

    // Next one must be foo-4
    let id = coord
        .create_session(Some("foo".to_string()), IsolationTier::Bare, None)
        .unwrap();
    let sessions = coord.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == id).unwrap();
    assert_eq!(s.name.as_deref(), Some("foo-4"));
}

#[test]
fn coordinator_restores_counters_from_store() {
    let dir = tempfile::tempdir().unwrap();

    // First coordinator: create sessions to advance counters, flush, drop.
    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        coord
            .create_session(None, IsolationTier::Bare, None)
            .unwrap(); // id=1
        coord
            .create_session(None, IsolationTier::Bare, None)
            .unwrap(); // id=2
        store.flush_all();
    }

    // Second coordinator: restores next_session_id=3.
    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let id = coord2
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    assert_eq!(id, SessionId(3), "coordinator should restore counter to 3");
}

#[test]
fn coordinator_corrupt_state_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();

    // Write garbage daemon state
    std::fs::write(dir.path().join("daemon.vxb"), b"garbage").unwrap();

    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);

    // Should start with defaults — first session gets id=1
    let id = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    assert_eq!(id, SessionId(1));
}

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
        None,
        None,
        Vec::new(),
    )
    .unwrap();

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

    let pane = persisted
        .panes
        .values()
        .next()
        .expect("no pane in snapshot");
    assert_eq!(
        pane.cwd,
        cwd.to_string_lossy().to_string(),
        "persisted cwd should match the spawn_with_cwd argument"
    );

    let _ = cmd_tx.send(SessionCommand::Shutdown);
}

#[test]
fn session_goes_dormant_on_last_client_detach() {
    use malt_protocol::render::RenderBatch;

    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();

    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let _ = coord
        .register_vnp_client(sid.clone(), 1, caps(), render_tx)
        .unwrap();

    // Detach — client count hits 0 → session should go Dormant.
    coord.unregister_vnp_client(sid.clone(), 1).unwrap();

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

#[test]
fn dormant_session_visible_in_list_sessions() {
    use malt_protocol::common::SessionState;

    let dir = tempfile::tempdir().unwrap();

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord
            .create_session(Some("alpha".to_string()), IsolationTier::Bare, None)
            .unwrap();

        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid.clone(), 1, caps(), render_tx)
            .unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();
    }

    let store2 = make_store(&dir);
    let coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    assert_eq!(
        sessions.len(),
        1,
        "second coordinator should see 1 dormant session"
    );
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

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord
            .create_session(Some("beta".to_string()), IsolationTier::Bare, None)
            .unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid.clone(), 1, caps(), render_tx)
            .unwrap();
        coord.unregister_vnp_client(sid.clone(), 1).unwrap();
        store.flush_all();
    }

    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    let sid = sessions[0].session_id.clone();

    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let initial = coord2
        .register_vnp_client(sid.clone(), 2, caps(), render_tx)
        .unwrap();
    assert_eq!(
        initial.frame_seq, 0,
        "restored session should start at frame 0"
    );

    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, malt_protocol::common::SessionState::Active);

    let _ = coord2.unregister_vnp_client(sid, 2);
}

#[test]
fn shutdown_graceful_saves_all_active_sessions() {
    let dir = tempfile::tempdir().unwrap();

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid1 = coord
            .create_session(Some("s1".to_string()), IsolationTier::Bare, None)
            .unwrap();
        let sid2 = coord
            .create_session(Some("s2".to_string()), IsolationTier::Bare, None)
            .unwrap();

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
    assert!(sessions
        .iter()
        .all(|s| s.state == malt_protocol::common::SessionState::Dormant));
}

#[test]
fn destroy_dormant_session_removes_from_store() {
    let dir = tempfile::tempdir().unwrap();

    {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord
            .create_session(None, IsolationTier::Bare, None)
            .unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid.clone(), 1, caps(), render_tx)
            .unwrap();
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
        let sid1 = coord
            .create_session(Some("x".to_string()), IsolationTier::Bare, None)
            .unwrap();
        let sid2 = coord
            .create_session(Some("y".to_string()), IsolationTier::Bare, None)
            .unwrap();

        // Detach both → dormant.
        let (tx1, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid1.clone(), 1, caps(), tx1)
            .unwrap();
        coord.unregister_vnp_client(sid1, 1).unwrap();

        let (tx2, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid2.clone(), 2, caps(), tx2)
            .unwrap();
        coord.unregister_vnp_client(sid2, 2).unwrap();

        store.flush_all();
    }

    let store2 = make_store(&dir);
    let coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    assert_eq!(sessions.len(), 2);
    assert!(sessions
        .iter()
        .all(|s| s.state == malt_protocol::common::SessionState::Dormant));
}

#[test]
fn restore_fails_cleanly_on_bad_cwd() {
    use malt_protocol::render::RenderBatch;

    let dir = tempfile::tempdir().unwrap();

    // Create session, make it go dormant.
    let sid = {
        let store = make_store(&dir);
        let mut coord = Coordinator::new(PoolConfig::default(), store.clone());
        let sid = coord
            .create_session(None, IsolationTier::Bare, None)
            .unwrap();
        let (render_tx, _) = std::sync::mpsc::sync_channel(4);
        coord
            .register_vnp_client(sid.clone(), 1, caps(), render_tx)
            .unwrap();
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
    assert!(
        result.is_ok(),
        "restore with bad cwd should succeed via fallback: {:?}",
        result
    );

    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, malt_protocol::common::SessionState::Active);
}

#[test]
fn app_restore_returns_error() {
    use malt_protocol::common::{LayoutNode, PaneId, SessionState};
    use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
    use malt_protocol::render::RenderBatch;
    use std::collections::BTreeMap;

    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);

    // Manually persist a session with an App pane.
    let sid = malt_protocol::common::SessionId(99);
    let pane = PersistedPane {
        cwd: ".".to_string(),
        title: None,
        pane_type: PersistedPaneType::App {
            app_id: "test-app".to_string(),
            config: None,
        },
        command_blocks: vec![],
        _unknown: vec![],
    };
    let mut panes = BTreeMap::new();
    panes.insert(1u32, pane);
    let persisted = PersistedSession {
        schema_version: 1,
        id: sid.clone(),
        name: Some("app-session".to_string()),
        layout: LayoutNode::Leaf { pane_id: PaneId(1) },
        focus: PaneId(1),
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
        matches!(
            result,
            Err(malt_daemon::DaemonError::AppRestoreNotSupported)
        ),
        "expected AppRestoreNotSupported, got: {:?}",
        result
    );

    // Session must still be Dormant (no partial Active state).
    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(
        s.state,
        SessionState::Dormant,
        "session must remain Dormant after failed restore"
    );
}

#[test]
fn restore_compat_pane_relaunches_process_and_forwards_real_output() {
    use malt_protocol::common::{LayoutNode, PaneId, SessionState};
    use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
    use malt_protocol::render::RenderBatch;
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);

    // Manually persist a session with a Compat pane -- there is no live
    // creation path for Compat panes today (confirmed by the persistence
    // audit), so restore is the only way to exercise spawn_compat; this
    // mirrors app_restore_returns_error's hand-crafted-persisted-session
    // approach.
    let sid = malt_protocol::common::SessionId(77);
    let pane = PersistedPane {
        cwd: std::env::current_dir().unwrap().to_string_lossy().to_string(),
        title: None,
        pane_type: PersistedPaneType::Compat {
            #[cfg(unix)]
            program: "/bin/echo".to_string(),
            #[cfg(windows)]
            program: "cmd".to_string(),
            #[cfg(unix)]
            args: vec!["compat-restore-marker".to_string()],
            #[cfg(windows)]
            args: vec![
                "/c".to_string(),
                "echo".to_string(),
                "compat-restore-marker".to_string(),
            ],
        },
        command_blocks: vec![],
        _unknown: vec![],
    };
    let mut panes = BTreeMap::new();
    panes.insert(1u32, pane);
    let persisted = PersistedSession {
        schema_version: 1,
        id: sid.clone(),
        name: Some("compat-session".to_string()),
        layout: LayoutNode::Leaf { pane_id: PaneId(1) },
        focus: PaneId(1),
        panes,
        theme: None,
        group: None,
        isolation: IsolationTier::Bare,
        _unknown: vec![],
    };
    store.mark_dirty(sid.clone(), persisted);
    store.flush_all();

    let daemon_state = malt_protocol::persist::daemon::DaemonState {
        schema_version: 1,
        sessions: vec![sid.clone()],
        active_groups: vec![],
        next_session_id: 78,
        next_pane_id: 2,
        _unknown: vec![],
    };
    store.mark_dirty_daemon(daemon_state);
    store.flush_all();

    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, SessionState::Dormant);

    // Attaching triggers restore_session, which spawns the real process.
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    coord2
        .register_vnp_client(sid.clone(), 1, caps(), render_tx)
        .expect("compat pane restore should succeed, not return RestoreFailed");

    let sessions = coord2.list_sessions();
    let s = sessions.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(s.state, SessionState::Active);

    // The spawned process's output reaches the session asynchronously (via
    // the pty-reader thread -> PtyOutput -> CompatTranslator), so poll
    // rather than asserting immediately.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_output;
    loop {
        last_output = coord2
            .get_session_output_text(sid.clone())
            .unwrap_or_default();
        if last_output.contains("compat-restore-marker") || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        last_output.contains("compat-restore-marker"),
        "restored compat pane's real process output must reach the \
         session's grid within 5s, got: {:?}",
        last_output
    );

    let _ = coord2.unregister_vnp_client(sid, 1);
}

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
#[test]
fn zero_execution_capacity_is_rejected_by_checked_construction() {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let config = PoolConfig { session_channel_size: 0, ..PoolConfig::default() };
    assert!(Coordinator::try_new(config, store).is_err());
}

#[test]
fn default_execution_capacity_is_256_waiting_requests() {
    assert_eq!(PoolConfig::default().session_channel_size, 256);
}

#[test]
fn last_detach_while_execution_is_busy_keeps_the_session_active() {
    use malt_daemon::executor::session_thread::SessionCommand;
    use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let capabilities = ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    };
    let (render_tx, _render_rx) = mpsc::sync_channel(4);
    coord.register_vnp_client(id.clone(), 1, capabilities, render_tx).unwrap();
    let receiver = coord.submit_execution(id.clone(), "sleep 1; echo retained".to_string()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    coord.unregister_vnp_client(id.clone(), 1).unwrap();
    assert!(coord.list_sessions().iter().any(|session| {
        session.session_id == id && session.state == SessionState::Active
    }));
    assert!(receiver.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    let _ = SessionCommand::Shutdown;
}

#[test]
fn last_detach_after_queued_write_input_keeps_session_active_and_runs_command() {
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let (render_tx, _render_rx) = mpsc::sync_channel(4);
    coord
        .register_vnp_client(id.clone(), 1, caps(), render_tx)
        .unwrap();

    // This command is deliberately queued on the control actor immediately
    // before the final detach. Dormancy must observe its admission in FIFO
    // order rather than decide from an earlier ingress-idle sample.
    coord
        .send_command(
            id.clone(),
            SessionCommand::WriteInput {
                data: b"sleep 1; echo queued-at-detach\n".to_vec(),
            },
        )
        .unwrap();
    coord.unregister_vnp_client(id.clone(), 1).unwrap();

    assert!(coord
        .list_sessions()
        .iter()
        .any(|session| { session.session_id == id && session.state == SessionState::Active }));

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut output = String::new();
    while Instant::now() < deadline {
        output = coord.get_session_output_text(id.clone()).unwrap();
        if output.contains("queued-at-detach") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(output.contains("queued-at-detach"), "output was {output:?}");
    coord.destroy_session(id);
}

#[test]
fn destroy_closes_intake_promptly_but_finalizes_active_work_once() {
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let mut coord = Coordinator::new(PoolConfig { session_channel_size: 1, ..PoolConfig::default() }, store);
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let active = coord.submit_execution(id.clone(), "sleep 1; echo active".to_string()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let pending = coord.submit_execution(id.clone(), "echo pending".to_string()).unwrap();
    let started = Instant::now();
    coord.destroy_session(id);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(active.recv_timeout(Duration::from_secs(2)).unwrap().unwrap().output.contains("active"));
    assert!(matches!(
        pending.recv_timeout(Duration::from_secs(2)).unwrap(),
        Err(malt_daemon::DaemonError::SessionShuttingDown(_))
    ));
}
