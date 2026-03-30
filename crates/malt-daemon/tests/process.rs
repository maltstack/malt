use malt_daemon::supervisor::process::{ManagedProcess, ProcessState, SpawnRequest};
use malt_protocol::common::{IsolationTier, PaneId};
use std::path::PathBuf;

#[test]
fn spawn_request_construction() {
    let req = SpawnRequest {
        program: PathBuf::from("/bin/bash"),
        args: vec!["-l".to_string()],
        cwd: PathBuf::from("/home/user"),
        pane_id: PaneId(1),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    };
    assert_eq!(req.program, PathBuf::from("/bin/bash"));
    assert_eq!(req.args.len(), 1);
    assert_eq!(req.pane_id, PaneId(1));
}

#[test]
fn process_state_default_is_running() {
    let state = ProcessState::Running;
    assert!(matches!(state, ProcessState::Running));
}

#[test]
fn process_state_exited() {
    let state = ProcessState::Exited(0);
    match state {
        ProcessState::Exited(code) => assert_eq!(code, 0),
        other => panic!("expected Exited, got {other:?}"),
    }
}

#[test]
fn restart_count_increments() {
    let mut proc = ManagedProcess::new_test(PaneId(1), 1234);
    assert_eq!(proc.restart_count(), 0);
    proc.increment_restart();
    assert_eq!(proc.restart_count(), 1);
    proc.increment_restart();
    assert_eq!(proc.restart_count(), 2);
}

#[test]
fn restart_limit_exceeded() {
    let mut proc = ManagedProcess::new_test(PaneId(1), 1234);
    for _ in 0..5 {
        proc.increment_restart();
    }
    assert!(proc.restart_limit_exceeded(5));
    assert!(!proc.restart_limit_exceeded(6));
}
