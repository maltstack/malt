use malt_daemon::supervisor::process::{ProcessState, SpawnRequest};
use malt_daemon::supervisor::ProcessSupervisor;
use malt_protocol::common::{IsolationTier, PaneId};
use std::path::PathBuf;

fn echo_request(pane_id: u32) -> SpawnRequest {
    SpawnRequest {
        #[cfg(unix)]
        program: PathBuf::from("/bin/echo"),
        #[cfg(windows)]
        program: PathBuf::from("cmd"),
        #[cfg(unix)]
        args: vec!["hello".to_string()],
        #[cfg(windows)]
        args: vec!["/c".to_string(), "echo".to_string(), "hello".to_string()],
        cwd: std::env::current_dir().unwrap(),
        pane_id: PaneId(pane_id),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    }
}

fn sleep_request(pane_id: u32) -> SpawnRequest {
    SpawnRequest {
        #[cfg(unix)]
        program: PathBuf::from("/bin/sleep"),
        #[cfg(windows)]
        program: PathBuf::from("cmd"),
        #[cfg(unix)]
        args: vec!["60".to_string()],
        #[cfg(windows)]
        args: vec![
            "/c".to_string(),
            "ping".to_string(),
            "-n".to_string(),
            "60".to_string(),
            "127.0.0.1".to_string(),
        ],
        cwd: std::env::current_dir().unwrap(),
        pane_id: PaneId(pane_id),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    }
}

#[test]
fn spawn_and_check_exit() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(echo_request(1)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let exited = sup.check_exited();
    assert!(!exited.is_empty());
    let (pane_id, state) = &exited[0];
    assert_eq!(*pane_id, PaneId(1));
    assert!(matches!(state, ProcessState::Exited(0)));
}

#[test]
fn spawn_tracks_pane_id() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(42)).unwrap();
    assert!(sup.get(&PaneId(42)).is_some());
    assert_eq!(sup.process_count(), 1);
    sup.kill(&PaneId(42)).unwrap();
}

#[test]
fn kill_process() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    sup.kill(&PaneId(1)).unwrap();
    assert!(sup.get(&PaneId(1)).is_none());
}

#[test]
fn process_count() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    sup.spawn(sleep_request(2)).unwrap();
    sup.spawn(sleep_request(3)).unwrap();
    assert_eq!(sup.process_count(), 3);
    sup.kill(&PaneId(1)).unwrap();
    sup.kill(&PaneId(2)).unwrap();
    sup.kill(&PaneId(3)).unwrap();
}

#[test]
fn take_io_returns_handles() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    let io = sup.take_io(&PaneId(1));
    assert!(io.is_some());
    let io2 = sup.take_io(&PaneId(1));
    assert!(io2.is_none());
    sup.kill(&PaneId(1)).unwrap();
}
