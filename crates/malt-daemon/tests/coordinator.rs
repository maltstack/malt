use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_protocol::common::{IsolationTier, SessionId};

fn make_store(dir: &tempfile::TempDir) -> DebouncedStore {
    DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
}

#[test]
fn create_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_eq!(id, SessionId(1));
}

#[test]
fn create_multiple_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id1 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let id2 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_eq!(id1, SessionId(1));
    assert_eq!(id2, SessionId(2));
    assert_eq!(coord.session_count(), 2);
}

#[test]
fn destroy_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.destroy_session(id);
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn list_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    coord.create_session(Some("alpha".to_string()), IsolationTier::Bare, None).unwrap();
    coord.create_session(Some("beta".to_string()), IsolationTier::Restricted, None).unwrap();
    let sessions = coord.list_sessions();
    assert_eq!(sessions.len(), 2);
}

#[test]
fn session_ids_never_recycled() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let id1 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.destroy_session(id1.clone());
    let id2 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_ne!(id1, id2);
    assert_eq!(id2, SessionId(2));
}

use malt_daemon::bus::BusMessage;
use malt_daemon::executor::session_thread::SessionCommand;
use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, KeyModifiers, UnicodeLevel};
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
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.send_command(
        id.clone(),
        SessionCommand::AttachClient {
            client_id: 42,
            authority: InputAuthority::Exclusive,
        },
    ).unwrap();
    coord.destroy_session(id);
}

#[test]
fn shutdown_all_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.shutdown_all();
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn register_vnp_client_returns_initial_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let initial = coord.register_vnp_client(sid, 1, caps(), render_tx).unwrap();
    assert_eq!(initial.frame_seq, 0);
}

#[test]
fn unregister_vnp_client_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let _ = coord.register_vnp_client(sid.clone(), 1, caps(), render_tx).unwrap();
    coord.unregister_vnp_client(sid, 1).unwrap();
}

#[test]
fn send_key_input_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let key = KeyEvent {
        key: KeyValue::Char { codepoint: 'x' as u32 },
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
    let sid = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.ack_frame(sid, 1, 0).unwrap();
}

#[test]
fn name_uniqueness_appends_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);

    // Create "foo" — succeeds, name is "foo"
    coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();

    // Create "foo" again — must get "foo-2"
    let id2 = coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();
    let sessions = coord.list_sessions();
    let s2 = sessions.iter().find(|s| s.session_id == id2).unwrap();
    assert_eq!(s2.name.as_deref(), Some("foo-2"), "second 'foo' should be named 'foo-2'");
}

#[test]
fn name_uniqueness_increments_past_existing() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(&dir);
    let mut coord = Coordinator::new(PoolConfig::default(), store);

    // Pre-fill foo, foo-2, foo-3
    let id1 = coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();
    let id2 = coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();
    let id3 = coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();

    // Verify the three pre-filled sessions have the expected names
    let sessions = coord.list_sessions();
    let s1 = sessions.iter().find(|s| s.session_id == id1).unwrap();
    assert_eq!(s1.name.as_deref(), Some("foo"), "first 'foo' should be named 'foo'");
    let s2 = sessions.iter().find(|s| s.session_id == id2).unwrap();
    assert_eq!(s2.name.as_deref(), Some("foo-2"), "second 'foo' should be named 'foo-2'");
    let s3 = sessions.iter().find(|s| s.session_id == id3).unwrap();
    assert_eq!(s3.name.as_deref(), Some("foo-3"), "third 'foo' should be named 'foo-3'");

    // Next one must be foo-4
    let id = coord.create_session(Some("foo".to_string()), IsolationTier::Bare, None).unwrap();
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
        coord.create_session(None, IsolationTier::Bare, None).unwrap(); // id=1
        coord.create_session(None, IsolationTier::Bare, None).unwrap(); // id=2
        store.flush_all();
    }

    // Second coordinator: restores next_session_id=3.
    let store2 = make_store(&dir);
    let mut coord2 = Coordinator::new(PoolConfig::default(), store2);
    let id = coord2.create_session(None, IsolationTier::Bare, None).unwrap();
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
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_eq!(id, SessionId(1));
}
