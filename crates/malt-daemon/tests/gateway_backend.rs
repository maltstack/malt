use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_gateway::backend::GatewayBackend;
use std::sync::{Arc, Mutex};

fn make_backend() -> DaemonBackend {
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default())));
    DaemonBackend::new(coordinator)
}

#[test]
fn list_sessions_empty() {
    let backend = make_backend();
    let sessions = backend.list_sessions().unwrap();
    assert!(sessions.is_empty());
}

#[test]
fn create_and_list() {
    let backend = make_backend();
    backend
        .create_session(Some("test".to_string()), None)
        .unwrap();
    let sessions = backend.list_sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, Some("test".to_string()));
}

#[test]
fn create_and_get() {
    let backend = make_backend();
    let created = backend
        .create_session(Some("hello".to_string()), None)
        .unwrap();
    let got = backend.get_session(created.id).unwrap();
    assert_eq!(got.name, Some("hello".to_string()));
}

#[test]
fn destroy_session() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    backend.destroy_session(created.id).unwrap();
    let sessions = backend.list_sessions().unwrap();
    assert!(sessions.is_empty());
}
