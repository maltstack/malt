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

/// A long-running child, spawned the way each platform actually spells it.
///
/// `ping -n 30` is Windows syntax; on Linux `-n` means "numeric output" and
/// the count flag is `-c`. Both tests below used the Windows form
/// unconditionally, which is one of the reasons they had never passed on
/// Linux -- the other being the exit-status convention, see below.
fn spawn_long_running() -> std::process::Child {
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("ping");
        c.args(["-n", "30", "127.0.0.1"]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sleep");
        c.arg("30");
        c
    };
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawning a long-running child must succeed")
}

/// Assert that `status` reflects termination by `Term`.
///
/// The two platforms report this differently and neither is wrong:
///
/// * **Unix** — a signal-killed process has *no* exit code. `code()` returns
///   `None` and the signal is read via `ExitStatusExt::signal()`. The
///   128+signum form is a *shell* convention for `$?`, not something
///   `ExitStatus` reports.
/// * **Windows** — there are no signals, so `signals/windows.rs` emulates one
///   by terminating with exit code 128+signum, making the shell convention
///   the literal process exit code.
///
/// The old assertion demanded `Some(143)` on both, so it could only ever have
/// passed on Windows.
fn assert_terminated_by_signal(status: std::process::ExitStatus, kind: SignalKind) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(signal_number(kind) as i32),
            "process should have been terminated by the signal we sent"
        );
        assert_eq!(
            status.code(),
            None,
            "a signal-killed process has no exit code on Unix"
        );
    }
    #[cfg(windows)]
    {
        assert_eq!(
            status.code(),
            Some(128 + signal_number(kind) as i32),
            "Windows emulates the signal with the 128+signum exit code documented in signals/windows.rs"
        );
    }
}

/// The success path was untested: `send_signal_nonexistent_pid` only checks
/// the error case. This spawns a real long-running process and confirms
/// `send_signal(..., Term)` actually terminates it — and that the exit
/// code follows the documented 128+signum POSIX-wait convention, not just
/// "the process died somehow".
#[test]
fn send_signal_term_actually_terminates_process_with_correct_exit_code() {
    let mut child = spawn_long_running();

    assert!(
        child
            .try_wait()
            .expect("try_wait should not error")
            .is_none(),
        "process should still be running right after spawn"
    );

    send_signal(child.id(), SignalKind::Term).expect("send_signal(Term) should succeed");

    let status = child.wait().expect("wait after signal should succeed");
    assert_terminated_by_signal(status, SignalKind::Term);
}

/// Same success-path gap for `Int` (Ctrl+C event) — the only prior coverage
/// was the nonexistent-PID error path.
#[test]
fn send_signal_int_succeeds_against_running_process() {
    let mut child = spawn_long_running();

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
