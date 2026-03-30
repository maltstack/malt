use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_protocol::common::{IsolationTier, SessionId};

#[test]
fn create_session() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_eq!(id, SessionId(1));
}

#[test]
fn create_multiple_sessions() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id1 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    let id2 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_eq!(id1, SessionId(1));
    assert_eq!(id2, SessionId(2));
    assert_eq!(coord.session_count(), 2);
}

#[test]
fn destroy_session() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.destroy_session(id);
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn list_sessions() {
    let mut coord = Coordinator::new(PoolConfig::default());
    coord.create_session(Some("alpha".to_string()), IsolationTier::Bare, None).unwrap();
    coord.create_session(Some("beta".to_string()), IsolationTier::Restricted, None).unwrap();
    let sessions = coord.list_sessions();
    assert_eq!(sessions.len(), 2);
}

#[test]
fn session_ids_never_recycled() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id1 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.destroy_session(id1.clone());
    let id2 = coord.create_session(None, IsolationTier::Bare, None).unwrap();
    assert_ne!(id1, id2);
    assert_eq!(id2, SessionId(2));
}

use malt_daemon::bus::BusMessage;
use malt_daemon::executor::session_thread::SessionCommand;
use malt_protocol::common::InputAuthority;
use malt_protocol::priority::Priority;

#[test]
fn route_to_nonexistent_session_errors() {
    let coord = Coordinator::new(PoolConfig::default());
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
    let mut coord = Coordinator::new(PoolConfig::default());
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
    let mut coord = Coordinator::new(PoolConfig::default());
    coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.create_session(None, IsolationTier::Bare, None).unwrap();
    coord.shutdown_all();
    assert_eq!(coord.session_count(), 0);
}
