use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_gateway::backend::GatewayBackend;
use std::sync::{Arc, Mutex};

fn make_backend() -> DaemonBackend {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
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

#[test]
fn exec_reports_real_exit_code_for_success() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    let result = backend
        .exec_command(created.id, "echo hello".to_string())
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(result.output.contains("hello"));
}

#[test]
fn exec_reports_real_exit_code_for_failure() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    let result = backend.exec_command(created.id, "false".to_string()).unwrap();
    assert_eq!(
        result.exit_code,
        Some(1),
        "the `false` builtin must report a nonzero exit code, not the \
         previously-hardcoded None"
    );
}

#[test]
fn list_panes_returns_the_sessions_real_pane_id_not_always_one() {
    let backend = make_backend();
    // Create and destroy a session first so the next one's pane id is not 1,
    // proving list_panes doesn't just hardcode the first-ever pane id.
    let first = backend.create_session(None, None).unwrap();
    backend.destroy_session(first.id).unwrap();
    let second = backend.create_session(None, None).unwrap();

    let panes = backend.list_panes(second.id).unwrap();
    assert_eq!(panes.len(), 1);
    assert_ne!(
        panes[0].id, 1,
        "list_panes must return this session's real pane id, not a \
         hardcoded 1 left over from the very first session ever created"
    );
}

#[test]
fn create_session_rejects_unrecognized_isolation_string() {
    let backend = make_backend();
    let err = backend
        .create_session(None, Some("resticted".to_string()))
        .unwrap_err();
    match err {
        malt_gateway::error::GatewayError::BadRequest(msg) => {
            assert!(
                msg.contains("resticted"),
                "error message should echo back the bad value, got: {msg:?}"
            );
        }
        other => panic!(
            "expected GatewayError::BadRequest for a typo'd isolation string, got: {other:?}"
        ),
    }
}

#[test]
fn get_output_text_returns_plain_text_matching_executed_output() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    backend
        .exec_command(created.id, "echo plain-text-marker".to_string())
        .unwrap();

    let text = backend.get_output_text(created.id).unwrap();
    assert!(
        text.contains("plain-text-marker"),
        "plain-text output should contain the command's real output, got: {:?}",
        text
    );
    assert!(
        !text.contains('{') && !text.contains("\"fg\""),
        "plain-text output must not contain StyledGrid JSON span markup, got: {:?}",
        text
    );
}

#[test]
fn exec_reports_a_real_monotonic_command_id() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    let first = backend
        .exec_command(created.id, "echo one".to_string())
        .unwrap();
    let second = backend
        .exec_command(created.id, "echo two".to_string())
        .unwrap();
    let third = backend
        .exec_command(created.id, "echo three".to_string())
        .unwrap();

    assert_ne!(first.command_id, 0, "command_id must not be the old hardcoded 0");
    assert!(
        second.command_id > first.command_id,
        "command_id must increase across successive commands on the same session: {} then {}",
        first.command_id,
        second.command_id
    );
    assert!(
        third.command_id > second.command_id,
        "command_id must keep increasing: {} then {}",
        second.command_id,
        third.command_id
    );
}

#[test]
fn exec_reports_stderr_separately_from_stdout() {
    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    let result = backend
        .exec_command(created.id, "echo out-text; echo err-text 1>&2".to_string())
        .unwrap();
    assert!(
        result.output.contains("out-text"),
        "stdout should contain out-text, got: {:?}",
        result.output
    );
    assert!(
        result.stderr.contains("err-text"),
        "stderr must be captured and returned separately from stdout, not \
         silently dropped; got stderr: {:?}",
        result.stderr
    );
    assert!(
        !result.output.contains("err-text"),
        "stderr text must not leak into the stdout field"
    );
}
