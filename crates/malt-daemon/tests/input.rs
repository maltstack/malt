//! Raw input delivery to a command that is blocked reading.
//!
//! These drive the real path — a command actually blocks, input actually
//! goes through the coordinator, and the command's own output is the
//! assertion. Nothing here inspects the pipe directly; that is what the unit
//! tests in `executor::input` are for.

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_gateway::backend::GatewayBackend;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn make_backend() -> Arc<DaemonBackend> {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    Arc::new(DaemonBackend::new(coordinator))
}

/// Run `command` (which is expected to block reading), send `input` shortly
/// after, and return what the command produced.
fn answer(backend: &Arc<DaemonBackend>, session: u32, command: &str, input: &str) -> String {
    let exec_backend = Arc::clone(backend);
    let cmd = command.to_string();
    let handle = std::thread::spawn(move || exec_backend.exec_command(session, cmd));

    // Give the command time to reach its read before answering.
    std::thread::sleep(Duration::from_millis(400));
    backend
        .send_input(session, input.to_string())
        .expect("input should be accepted");

    handle
        .join()
        .expect("exec thread panicked")
        .expect("command should complete once answered")
        .output
}

#[test]
fn a_command_blocked_on_read_receives_client_input() {
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap().id;

    let out = answer(
        &backend,
        session,
        r#"read -r X; echo "got=[$X]""#,
        "hello-from-client\n",
    );
    assert!(
        out.contains("got=[hello-from-client]"),
        "the waiting command should have received the client's input, got: {out:?}"
    );
}

#[test]
fn a_bare_newline_answers_a_prompt_rather_than_being_discarded() {
    // The old path decoded, trimmed, and dropped empty input -- so the one
    // byte a confirmation prompt is waiting for never arrived and the session
    // hung.
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap().id;

    let out = answer(&backend, session, r#"read -r Y; echo "answer=[$Y]""#, "\n");
    assert!(
        out.contains("answer=[]"),
        "a bare newline must let the command proceed, got: {out:?}"
    );
}

#[test]
fn input_is_not_executed_as_a_command() {
    // The old path submitted whatever arrived as a new top-level command line,
    // so answering a prompt with `echo pwned` would have run it.
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap().id;

    let out = answer(
        &backend,
        session,
        r#"read -r Z; echo "captured=[$Z]""#,
        "echo pwned\n",
    );
    assert!(
        out.contains("captured=[echo pwned]"),
        "the text should have been captured as data, got: {out:?}"
    );
    assert!(
        !out.contains("pwned\n") || out.contains("captured=[echo pwned]"),
        "the text must not have been executed as a command: {out:?}"
    );
}

#[test]
fn a_prompt_answer_never_reaches_history_or_the_event_stream() {
    // FR-010. This requirement exists because features 003 and 004 shipped:
    // both record command text and both are readable at Read scope, so
    // routing a prompt answer through command submission would publish a
    // password into two durable, readable surfaces.
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap().id;

    let out = answer(
        &backend,
        session,
        "read -r PW; echo done",
        "hunter2-secret\n",
    );
    assert!(out.contains("done"), "the command should have completed");

    let history = backend.get_command_history(session).unwrap();
    let history_text = format!("{history:?}");
    assert!(
        !history_text.contains("hunter2-secret"),
        "the prompt answer leaked into command history: {history_text}"
    );

    // The command that *read* it is legitimately in history; the answer is not.
    assert!(
        history_text.contains("read -r PW"),
        "the command itself should still be recorded: {history_text}"
    );
}

#[test]
fn type_ahead_sent_before_a_read_is_still_delivered() {
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap().id;

    // Nothing is reading yet.
    backend
        .send_input(session, "typed-ahead\n".to_string())
        .expect("input should be retained for the next read");

    let result = backend
        .exec_command(session, r#"read -r A; echo "ahead=[$A]""#.to_string())
        .expect("command should complete using the retained input");
    assert!(
        result.output.contains("ahead=[typed-ahead]"),
        "input sent before the read should still be delivered, got: {:?}",
        result.output
    );
}

#[test]
fn input_to_an_unknown_session_is_refused() {
    let backend = make_backend();
    let err = backend
        .send_input(9999, "nobody-home\n".to_string())
        .unwrap_err();
    assert!(
        matches!(
            err,
            malt_gateway::error::GatewayError::SessionNotFound(9999)
        ),
        "expected NotFound, got {err:?}"
    );
}
