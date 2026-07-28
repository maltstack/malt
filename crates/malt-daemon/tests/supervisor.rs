use malt_daemon::supervisor::process::{ProcessState, SpawnRequest};
use malt_daemon::supervisor::ProcessSupervisor;
use malt_platform::signals::process_exists;
use malt_protocol::common::{IsolationTier, PaneId};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

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
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let exited = loop {
        let exited = sup.check_exited();
        if !exited.is_empty() || std::time::Instant::now() >= deadline {
            break exited;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!exited.is_empty());
    let (pane_id, state) = &exited[0];
    assert_eq!(*pane_id, PaneId(1));
    assert!(
        matches!(state, ProcessState::Exited(0)),
        "/bin/echo should exit 0, got {state:?}. A code of 141 is 128+SIGPIPE and          means the child was killed writing to its stdout: `spawn_with_pty` hands          the child dups of the pty *master* while nothing holds the slave, so the          write has no reader. See docs/briefs/007-unix-pty-wired-backwards.md --          the same defect shows up on Linux as EIO when the parent reads instead."
    );
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
fn kill_terminates_the_real_process_before_removing_its_record() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    let pid = sup.get(&PaneId(1)).unwrap().pid();
    assert!(
        process_exists(pid),
        "spawned process should exist before kill"
    );

    sup.kill(&PaneId(1)).unwrap();

    assert!(
        !process_exists(pid),
        "kill must wait until the OS no longer reports pid {pid}"
    );
    assert!(sup.get(&PaneId(1)).is_none());
}

#[cfg(unix)]
fn term_ignoring_request(pane_id: u32) -> SpawnRequest {
    SpawnRequest {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_string(), "trap '' TERM; sleep 60".to_string()],
        cwd: std::env::current_dir().unwrap(),
        pane_id: PaneId(pane_id),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    }
}

#[test]
fn kill_unblocks_the_pty_reader_and_self_exits_are_still_reaped() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    let (reader, _writer) = sup.take_io(&PaneId(1)).unwrap();
    let (result_tx, result_rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        use std::io::Read;

        let mut reader = reader;
        let mut buffer = [0u8; 1];
        let _ = result_tx.send(reader.read(&mut buffer));
    });

    sup.kill(&PaneId(1)).unwrap();
    let read_result = result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("killing the process must unblock its PTY reader");
    reader_thread.join().unwrap();
    match read_result {
        Ok(0) => {}
        // Unix PTY masters report EIO when the last slave closes. The daemon
        // deliberately treats it as end-of-stream in `spawn_pty_reader`.
        Err(error) if error.raw_os_error() == Some(5) => {}
        other => panic!("expected PTY end-of-stream after kill, got {other:?}"),
    }

    sup.spawn(echo_request(2)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let exited = sup.check_exited();
        if exited.iter().any(|(pane_id, _)| *pane_id == PaneId(2)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "an independently exiting process must still be reaped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(sup.get(&PaneId(2)).is_none());
}

#[cfg(unix)]
#[test]
fn kill_escalates_to_process_group_termination_when_term_is_ignored() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(term_ignoring_request(3)).unwrap();
    let pid = sup.get(&PaneId(3)).unwrap().pid();

    sup.kill(&PaneId(3)).unwrap();

    assert!(
        !process_exists(pid),
        "forced termination must kill a TERM-ignoring process group"
    );
    assert!(sup.get(&PaneId(3)).is_none());
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
