use malt_platform::signals::{
    process_exists, send_signal, signal_by_name, signal_name, signal_number, SignalError,
    SignalKind,
};

#[test]
fn signal_by_name_bare() {
    assert_eq!(signal_by_name("TERM"), Some(SignalKind::Term));
}

#[test]
fn signal_by_name_with_sig_prefix() {
    assert_eq!(signal_by_name("SIGTERM"), Some(SignalKind::Term));
}

#[test]
fn signal_by_name_by_number() {
    assert_eq!(signal_by_name("15"), Some(SignalKind::Term));
}

#[test]
fn signal_name_returns_canonical() {
    assert_eq!(signal_name(SignalKind::Term), "TERM");
}

#[test]
fn signal_number_returns_nonzero_for_known() {
    assert_ne!(signal_number(SignalKind::Int), 0);
    assert_ne!(signal_number(SignalKind::Term), 0);
    assert_ne!(signal_number(SignalKind::Hup), 0);
    assert_ne!(signal_number(SignalKind::Quit), 0);
    assert_ne!(signal_number(SignalKind::Usr1), 0);
    assert_ne!(signal_number(SignalKind::Usr2), 0);
    assert_ne!(signal_number(SignalKind::Tstp), 0);
}

#[test]
fn send_signal_nonexistent_pid() {
    let result = send_signal(99_999_999, SignalKind::Term);
    assert!(result.is_err());
    match result.unwrap_err() {
        SignalError::NoSuchProcess { pid } => assert_eq!(pid, 99_999_999),
        other => panic!("expected NoSuchProcess, got: {other:?}"),
    }
}

#[test]
fn process_exists_self() {
    assert!(process_exists(std::process::id()));
}

#[test]
fn process_exists_nonexistent() {
    assert!(!process_exists(99_999_999));
}

#[test]
fn signal_by_name_unknown_returns_none() {
    assert_eq!(signal_by_name("NONEXISTENT"), None);
    assert_eq!(signal_by_name("999"), None);
}

/// The success path was untested: `send_signal_nonexistent_pid` only checks
/// the error case. This spawns a real long-running process and confirms
/// `send_signal(..., Term)` actually terminates it — and that the exit
/// code follows the documented 128+signum POSIX-wait convention, not just
/// "the process died somehow".
#[test]
fn send_signal_term_actually_terminates_process_with_correct_exit_code() {
    use std::process::{Command, Stdio};

    let mut child = Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ping (must be present on any Windows host)");

    assert!(
        child
            .try_wait()
            .expect("try_wait should not error")
            .is_none(),
        "process should still be running right after spawn"
    );

    send_signal(child.id(), SignalKind::Term).expect("send_signal(Term) should succeed");

    let status = child.wait().expect("wait after signal should succeed");
    let expected_code = 128 + signal_number(SignalKind::Term) as i32;
    assert_eq!(
        status.code(),
        Some(expected_code),
        "exit code should follow the 128+signum convention documented in signals/windows.rs"
    );
}

/// Same success-path gap for `Int` (Ctrl+C event) — the only prior coverage
/// was the nonexistent-PID error path.
#[test]
fn send_signal_int_succeeds_against_running_process() {
    use std::process::{Command, Stdio};

    let mut child = Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ping (must be present on any Windows host)");

    // GenerateConsoleCtrlEvent targets a process *group*; ping isn't in our
    // console's process group by default, so the realistic, honest thing to
    // check is that the call itself succeeds against a real PID (doesn't
    // error as NoSuchProcess), not that ping necessarily dies from Ctrl+C —
    // asserting the latter would be testing console group semantics we
    // don't control here, not this function.
    let result = send_signal(child.id(), SignalKind::Int);
    assert!(
        result.is_ok(),
        "send_signal(Int) against a real PID should not error: {result:?}"
    );

    // Clean up regardless of whether Ctrl+C actually killed it.
    let _ = child.kill();
    let _ = child.wait();
}
