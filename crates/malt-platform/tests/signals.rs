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
