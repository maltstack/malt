use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::executor::session_thread::ClientMessage;
use malt_daemon::executor::session_thread::SessionCommand;
use malt_daemon::executor::QueueState;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use std::sync::{Arc, Mutex};

fn make_backend() -> DaemonBackend {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    DaemonBackend::new(coordinator)
}

fn make_backend_with_store(dir: &tempfile::TempDir) -> DaemonBackend {
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    DaemonBackend::new(coordinator)
}

fn make_backend_with_config(config: PoolConfig) -> DaemonBackend {
    let dir = tempfile::tempdir().unwrap();
    // Deliberately leak this test directory for the short test lifetime: the
    // daemon owns asynchronous worker threads, so removing it early would be
    // a test-only race unrelated to execution ordering.
    let path = dir.keep();
    let store = DebouncedStore::new(SessionStore::new(path));
    DaemonBackend::new(Arc::new(Mutex::new(
        Coordinator::try_new(config, store).unwrap(),
    )))
}

/// Block until the session's execution FIFO reaches an expected state.
///
/// This replaces `thread::sleep`, which cannot *establish* a precondition --
/// it only makes a race likely to resolve one way. Under parallel test load
/// it resolved the other way often enough that this file failed 3 runs in 5,
/// which meant the FIFO guarantees it exists to verify were being reported
/// intermittently rather than proven. Polling real admission state is
/// deterministic: once the condition holds it stays held until we act on it.
fn wait_for_queue<F>(backend: &DaemonBackend, session: u32, what: &str, predicate: F) -> QueueState
where
    F: Fn(QueueState) -> bool,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut last = None;
    while std::time::Instant::now() < deadline {
        let observed = backend
            .coordinator()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execution_queue_state(&malt_protocol::common::SessionId(session));
        if let Some(state) = observed {
            if predicate(state) {
                return state;
            }
            last = Some(state);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("timed out after 10s waiting for {what}; last observed {last:?}");
}

fn test_caps() -> malt_protocol::common::ClientCapabilities {
    malt_protocol::common::ClientCapabilities {
        color_depth: malt_protocol::common::ColorDepth::TrueColor,
        unicode: malt_protocol::common::UnicodeLevel::Full,
        image_protocol: malt_protocol::common::ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
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
    let result = backend
        .exec_command(created.id, "false".to_string())
        .unwrap();
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
fn required_contained_refusal_leaves_no_session() {
    let backend = make_backend();
    let error = backend
        .create_session_with_policy(
            Some("must-not-exist".to_string()),
            Some("contained".to_string()),
            Some("required".to_string()),
        )
        .unwrap_err();
    assert!(matches!(error, GatewayError::IsolationUnavailable(_)));
    assert!(
        backend.list_sessions().unwrap().is_empty(),
        "a required failure must not leave a runnable session"
    );
}

#[test]
fn preferred_contained_request_returns_visible_bare_downgrade() {
    let backend = make_backend();
    let created = backend
        .create_session_with_policy(
            None,
            Some("contained".to_string()),
            Some("preferred".to_string()),
        )
        .unwrap();
    assert_eq!(created.isolation.requested, "contained");
    assert_eq!(created.isolation.effective, "bare");
    assert_eq!(created.isolation.basis, "none");
    assert!(created.isolation.detail.is_some());
}

#[test]
fn disabled_isolation_attempts_nothing_and_reports_none() {
    let backend = make_backend();
    let created = backend
        .create_session_with_policy(
            None,
            Some("contained".to_string()),
            Some("disabled".to_string()),
        )
        .unwrap();
    assert_eq!(created.isolation.effective, "bare");
    assert_eq!(created.isolation.requested, "contained");
    assert_eq!(created.isolation.basis, "none");
    assert!(created
        .isolation
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("disabled"));
}

#[test]
fn creation_get_and_list_share_the_same_isolation_status() {
    let backend = make_backend();
    let created = backend
        .create_session_with_policy(
            Some("status-agreement".to_string()),
            Some("contained".to_string()),
            Some("preferred".to_string()),
        )
        .unwrap();
    let fetched = backend.get_session(created.id).unwrap();
    let listed = backend
        .list_sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == created.id)
        .expect("created session must appear in list");

    for status in [&fetched.isolation, &listed.isolation] {
        assert_eq!(status.effective, created.isolation.effective);
        assert_eq!(status.requested, created.isolation.requested);
        assert_eq!(status.basis, created.isolation.basis);
        assert_eq!(status.mechanism, created.isolation.mechanism);
        assert_eq!(status.detail, created.isolation.detail);
    }
    assert_ne!(created.isolation.basis, "verified");
}

#[test]
fn capabilities_match_the_required_creation_path() {
    let backend = make_backend();
    let capabilities = backend.isolation_capabilities().unwrap();
    let contained = capabilities
        .iter()
        .find(|capability| capability.tier == "contained")
        .expect("contained capability must be reported");
    assert!(!contained.available);
    assert_eq!(contained.basis, "none");

    let result = backend.create_session_with_policy(
        None,
        Some("contained".to_string()),
        Some("required".to_string()),
    );
    assert!(matches!(result, Err(GatewayError::IsolationUnavailable(_))));
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

    assert_ne!(
        first.command_id, 0,
        "command_id must not be the old hardcoded 0"
    );
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

#[test]
fn command_history_records_each_execution_with_real_status() {
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap();

    // A fresh session has no history -- and that is an empty list, not an
    // error, so callers can tell "nothing run yet" from "no such session".
    let empty = backend.get_command_history(session.id).unwrap();
    assert!(
        empty.is_empty(),
        "a session that has run nothing must report empty history, got {empty:?}"
    );

    backend
        .exec_command(session.id, "echo history-one".to_string())
        .unwrap();
    backend
        .exec_command(session.id, "false".to_string())
        .unwrap();

    let history = backend.get_command_history(session.id).unwrap();
    assert_eq!(
        history.len(),
        2,
        "both executions must be recorded: {history:?}"
    );

    assert_eq!(history[0].cmd, "echo history-one");
    assert_eq!(history[0].exit_code, Some(0));
    assert_eq!(history[1].cmd, "false");
    assert_eq!(
        history[1].exit_code,
        Some(1),
        "a failing command's real exit code must be recorded, not flattened to 0"
    );

    // Chronological order, ascending ids, and every completed entry actually
    // finalized with a finish time at or after its start.
    assert!(history[0].command_id < history[1].command_id);
    for entry in &history {
        let finished = entry
            .finished_at
            .unwrap_or_else(|| panic!("completed command must have finished_at: {entry:?}"));
        assert!(
            finished >= entry.started_at,
            "finished_at must not precede started_at: {entry:?}"
        );
        assert_eq!(
            entry.pane_id, 1,
            "history entries must carry the session's real pane id"
        );
    }
}

#[test]
fn command_history_records_a_parse_error_as_a_failed_execution() {
    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap();

    // An unterminated quote never reaches the executor -- it still counts as
    // an execution the user attempted, so it must appear in history.
    backend
        .exec_command(session.id, "echo 'unterminated".to_string())
        .unwrap();

    let history = backend.get_command_history(session.id).unwrap();
    assert_eq!(
        history.len(),
        1,
        "a parse error is still an execution: {history:?}"
    );
    assert_eq!(history[0].cmd, "echo 'unterminated");
    assert_eq!(
        history[0].exit_code,
        Some(1),
        "a parse error must be finalized as a failure, not left open"
    );
    assert!(history[0].finished_at.is_some());
}

#[test]
fn command_history_for_unknown_session_is_not_found() {
    use malt_gateway::error::GatewayError;

    let backend = make_backend();
    let err = backend.get_command_history(9999).unwrap_err();
    assert!(
        matches!(err, GatewayError::SessionNotFound(9999)),
        "an unknown session must be NotFound, not an empty history: {err:?}"
    );
}

#[test]
fn command_history_survives_dormant_restore_and_ids_stay_monotonic() {
    use malt_protocol::common::SessionId;

    let dir = tempfile::tempdir().unwrap();

    let session_id;
    let before: Vec<_>;
    {
        let backend = make_backend_with_store(&dir);
        let created = backend.create_session(None, None).unwrap();
        session_id = created.id;

        backend
            .exec_command(session_id, "echo before-restart-one".to_string())
            .unwrap();
        backend
            .exec_command(session_id, "false".to_string())
            .unwrap();

        before = backend.get_command_history(session_id).unwrap();
        assert_eq!(before.len(), 2);

        // Detaching the only client makes the session Dormant, which
        // snapshots and persists it.
        let coord_arc = backend.coordinator().clone();
        let mut coord = coord_arc.lock().unwrap();
        let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(4);
        coord
            .register_vnp_client(
                SessionId(session_id),
                1,
                malt_protocol::common::InputAuthority::Exclusive,
                test_caps(),
                render_tx,
            )
            .unwrap();
        coord
            .unregister_vnp_client(SessionId(session_id), 1)
            .unwrap();
    }

    // Fresh Coordinator over the same on-disk store == daemon restart.
    let backend2 = make_backend_with_store(&dir);
    {
        let coord_arc = backend2.coordinator().clone();
        let mut coord = coord_arc.lock().unwrap();
        let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(4);
        coord
            .register_vnp_client(
                SessionId(session_id),
                2,
                malt_protocol::common::InputAuthority::Exclusive,
                test_caps(),
                render_tx,
            )
            .unwrap();
    }

    let after = backend2.get_command_history(session_id).unwrap();
    assert_eq!(
        after, before,
        "history must survive a daemon restart unchanged -- every field, not just the count"
    );

    // The id sequence must resume past the restored history rather than
    // restarting at 1 and colliding with it.
    let highest_before = before.iter().map(|e| e.command_id).max().unwrap();
    let result = backend2
        .exec_command(session_id, "echo after-restart".to_string())
        .unwrap();
    assert!(
        result.command_id > highest_before,
        "a post-restore execution must get an id above every persisted one \
         ({} vs {highest_before})",
        result.command_id
    );

    let final_history = backend2.get_command_history(session_id).unwrap();
    assert_eq!(final_history.len(), 3);
    assert_eq!(final_history[2].cmd, "echo after-restart");
    let ids: Vec<u32> = final_history.iter().map(|e| e.command_id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        ids, sorted,
        "command ids must stay unique and ascending: {ids:?}"
    );
}

#[test]
fn a_stalled_event_subscriber_does_not_slow_execution_or_starve_others() {
    use malt_protocol::common::SessionId;

    let backend = make_backend();
    let session = backend.create_session(None, None).unwrap();

    // Baseline: how long twenty commands take with nobody watching.
    let baseline_start = std::time::Instant::now();
    for i in 0..20 {
        backend
            .exec_command(session.id, format!("echo base-{i}"))
            .unwrap();
    }
    let baseline = baseline_start.elapsed();

    // Now attach a subscriber that never reads, plus one that does.
    let coord = backend.coordinator().clone();
    let stalled = {
        let c = coord.lock().unwrap();
        c.begin_subscribe_events(SessionId(session.id), None)
            .unwrap()
    };
    let mut healthy = {
        let c = coord.lock().unwrap();
        c.begin_subscribe_events(SessionId(session.id), None)
            .unwrap()
    };

    // Overrun the stalled subscriber's buffer several times over.
    let watched_start = std::time::Instant::now();
    for i in 0..20 {
        backend
            .exec_command(session.id, format!("echo watched-{i}"))
            .unwrap();
    }
    let watched = watched_start.elapsed();

    // The healthy subscriber must still be receiving. Drain what it has.
    let mut healthy_events = 0usize;
    while healthy.try_recv().is_ok() {
        healthy_events += 1;
    }
    assert!(
        healthy_events > 0,
        "a healthy subscriber must keep receiving while another is stalled"
    );

    // Execution must not have slowed materially. Generous multiplier: this
    // asserts "not blocked on the subscriber", not a precise budget, and CI
    // timing is noisy.
    let allowed = baseline * 5 + std::time::Duration::from_millis(500);
    assert!(
        watched <= allowed,
        "execution with a stalled subscriber took {watched:?} versus a {baseline:?} \
         baseline -- a client that stops reading must never slow the session"
    );

    drop(stalled);
}

#[test]
fn a_dormant_session_reports_a_conflict_not_an_internal_error() {
    use malt_gateway::error::GatewayError;
    use malt_protocol::common::SessionId;

    let dir = tempfile::tempdir().unwrap();
    let session_id;
    {
        let backend = make_backend_with_store(&dir);
        let created = backend.create_session(None, None).unwrap();
        session_id = created.id;
        backend
            .exec_command(session_id, "echo before-dormant".to_string())
            .unwrap();

        let coord_arc = backend.coordinator().clone();
        let mut coord = coord_arc.lock().unwrap();
        let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(4);
        coord
            .register_vnp_client(
                SessionId(session_id),
                1,
                malt_protocol::common::InputAuthority::Exclusive,
                test_caps(),
                render_tx,
            )
            .unwrap();
        coord
            .unregister_vnp_client(SessionId(session_id), 1)
            .unwrap();
    }

    // Fresh coordinator over the same store: the session is known but dormant.
    let backend = make_backend_with_store(&dir);

    // "Attach to restore it" is something the caller can act on, so it must
    // not be reported as the daemon having failed.
    let exec_err = backend
        .exec_command(session_id, "echo while-dormant".to_string())
        .unwrap_err();
    assert!(
        matches!(exec_err, GatewayError::SessionDormant(_)),
        "exec on a dormant session must be a conflict, not an internal error: {exec_err:?}"
    );

    let output_err = backend.get_output_text(session_id).unwrap_err();
    assert!(
        matches!(output_err, GatewayError::SessionDormant(_)),
        "output on a dormant session must be a conflict, not an internal error: {output_err:?}"
    );

    // Still distinct from a session that does not exist at all.
    let missing = backend
        .exec_command(9999, "echo nope".to_string())
        .unwrap_err();
    assert!(matches!(missing, GatewayError::SessionNotFound(9999)));

    // History deliberately still succeeds on a dormant session -- it answers
    // from the persisted snapshot instead of requiring a restore.
    let history = backend.get_command_history(session_id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].cmd, "echo before-dormant");
}

#[test]
fn shell_env_state_survives_dormant_restore_via_env_snapshot() {
    use malt_protocol::common::SessionId;

    let dir = tempfile::tempdir().unwrap();

    let session_id;
    {
        let backend = make_backend_with_store(&dir);
        let created = backend.create_session(None, None).unwrap();
        session_id = created.id;

        // Set state across every category EnvSnapshot is supposed to carry:
        // an exported variable, an alias, and a function.
        backend
            .exec_command(
                session_id,
                "export MALT_SNAPSHOT_TEST=hello; alias mysnapalias='echo aliased'; \
                 mysnapfunc() { echo funced; }"
                    .to_string(),
            )
            .unwrap();

        // Detaching the only attached client transitions the session to
        // Dormant, which snapshots it (build_persisted_session ->
        // to_persisted_env_snapshot) and persists it to the store.
        let coord_arc = backend.coordinator().clone();
        let mut coord = coord_arc.lock().unwrap();
        let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(4);
        coord
            .register_vnp_client(
                SessionId(session_id),
                1,
                malt_protocol::common::InputAuthority::Exclusive,
                test_caps(),
                render_tx,
            )
            .unwrap();
        coord
            .unregister_vnp_client(SessionId(session_id), 1)
            .unwrap();
    }

    // Simulate a daemon restart: fresh Coordinator/DaemonBackend reading the
    // same on-disk store. The session should be Dormant; re-attaching
    // restores it via SessionExecutor::spawn_with_cwd, which now applies the
    // persisted EnvSnapshot.
    let backend2 = make_backend_with_store(&dir);
    {
        let coord_arc = backend2.coordinator().clone();
        let mut coord = coord_arc.lock().unwrap();
        let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(4);
        coord
            .register_vnp_client(
                SessionId(session_id),
                2,
                malt_protocol::common::InputAuthority::Exclusive,
                test_caps(),
                render_tx,
            )
            .unwrap();
    }

    let result = backend2
        .exec_command(
            session_id,
            "echo \"var=$MALT_SNAPSHOT_TEST\"; mysnapalias; mysnapfunc".to_string(),
        )
        .unwrap();

    assert!(
        result.output.contains("var=hello"),
        "exported variable must survive dormant->restore via EnvSnapshot, got: {:?}",
        result.output
    );
    assert!(
        result.output.contains("aliased"),
        "alias must survive dormant->restore via EnvSnapshot, got: {:?}",
        result.output
    );
    assert!(
        result.output.contains("funced"),
        "function must survive dormant->restore via EnvSnapshot, got output: {:?}, stderr: {:?}",
        result.output,
        result.stderr
    );
}

#[test]
fn busy_session_output_remains_prompt_and_another_session_is_not_delayed() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let backend = Arc::new(make_backend());
    let busy = backend.create_session(None, None).unwrap();
    let other = backend.create_session(None, None).unwrap();
    let command_backend = Arc::clone(&backend);
    let command_session = busy.id;
    let command = std::thread::spawn(move || {
        command_backend.exec_command(command_session, "sleep 1; echo finished".to_string())
    });
    std::thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    let _ = backend.get_output_text(busy.id).unwrap();
    let _ = backend.get_output_text(other.id).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(command.join().unwrap().is_ok());
}

#[test]
fn a_running_command_is_visible_in_history_before_it_finishes() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    // The payoff of running history on the control actor rather than the
    // MASH worker: a command that is still executing is already recorded,
    // and asking for history does not wait for it to finish.
    let backend = Arc::new(make_backend());
    let session = backend.create_session(None, None).unwrap();
    let other = backend.create_session(None, None).unwrap();

    let command_backend = Arc::clone(&backend);
    let session_id = session.id;
    let command = std::thread::spawn(move || {
        command_backend.exec_command(session_id, "sleep 1; echo done".to_string())
    });

    // Wait for the record to exist rather than sleeping and hoping. Note that
    // waiting on the FIFO's `active` flag is NOT sufficient here: the worker
    // sets `active` before it sends `ExecutionStarted` to the control actor,
    // and the control actor owns the history buffer. Waiting on `active` only
    // narrows the window; waiting on the record itself closes it.
    let deadline = Instant::now() + Duration::from_secs(10);
    let during = loop {
        let history = backend.get_command_history(session.id).unwrap();
        if !history.is_empty() {
            break history;
        }
        assert!(
            Instant::now() < deadline,
            "the running command never appeared in history"
        );
        std::thread::sleep(Duration::from_millis(2));
    };

    // The command must still be running, or "visible *before* it finishes"
    // is not what was just observed.
    assert!(
        during[0].finished_at.is_none(),
        "command finished before the assertion could observe it running;          it needs to be long enough to still be in flight here"
    );

    // Now the real claim: reading history does not block behind the command.
    let started = Instant::now();
    let _ = backend.get_command_history(session.id).unwrap();
    let unrelated = backend.get_command_history(other.id).unwrap();
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "history must not block behind the running command, took {elapsed:?}"
    );
    assert!(
        unrelated.is_empty(),
        "an unrelated session must not be delayed or polluted by the busy one"
    );

    assert_eq!(
        during.len(),
        1,
        "the in-flight command must already be recorded"
    );
    let entry = &during[0];
    assert_eq!(entry.cmd, "sleep 1; echo done");
    assert!(
        entry.finished_at.is_none() && entry.exit_code.is_none(),
        "a still-running command must report as not confirmed complete, got {entry:?}"
    );

    assert!(command.join().unwrap().is_ok());

    // Once it finishes, the same entry is finalized in place -- not duplicated.
    let after = backend.get_command_history(session.id).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].command_id, entry.command_id);
    assert_eq!(after[0].exit_code, Some(0));
    assert!(after[0].finished_at.is_some());
}

#[test]
fn capacity_one_keeps_one_active_and_one_pending_in_fifo_order() {
    use std::sync::Arc;

    let config = PoolConfig {
        session_channel_size: 1,
        ..PoolConfig::default()
    };
    let backend = Arc::new(make_backend_with_config(config));
    let session = backend.create_session(None, None).unwrap();

    let first_backend = Arc::clone(&backend);
    let first_id = session.id;
    let first = std::thread::spawn(move || {
        first_backend.exec_command(
            first_id,
            "export MALT_FIFO=ready; sleep 1; echo first".to_string(),
        )
    });
    // The first command must hold the active slot before anything else is
    // submitted, or the ordering this test asserts is not the ordering under
    // test.
    wait_for_queue(&backend, session.id, "the first command to start", |s| {
        s.active
    });

    let second_backend = Arc::clone(&backend);
    let second_id = session.id;
    let second = std::thread::spawn(move || {
        second_backend.exec_command(second_id, "echo second-$MALT_FIFO".to_string())
    });
    // And the second must actually occupy the single queue slot before a third
    // can be expected to bounce off a full queue.
    let state = wait_for_queue(&backend, session.id, "the second command to queue", |s| {
        s.pending == 1
    });
    assert!(state.is_full(), "queue should now be full: {state:?}");

    let overflow = backend.exec_command(session.id, "echo overflow".to_string());
    assert!(matches!(overflow, Err(GatewayError::ExecutionQueueFull(_))));
    assert!(first.join().unwrap().unwrap().output.contains("first"));
    assert!(second
        .join()
        .unwrap()
        .unwrap()
        .output
        .contains("second-ready"));
}

#[test]
fn snapshot_during_execution_uses_the_last_finalized_env_snapshot() {
    use malt_protocol::common::{IsolationTier, SessionId};
    use malt_protocol::persist::session::PersistedPaneType;
    use std::time::Duration;

    let backend = make_backend();
    let created = backend.create_session(None, None).unwrap();
    backend
        .exec_command(created.id, "export MALT_BOUNDARY=old".to_string())
        .unwrap();
    let receiver = {
        let coordinator = backend.coordinator().lock().unwrap();
        coordinator
            .submit_execution(
                SessionId(created.id),
                "export MALT_BOUNDARY=new; sleep 1".to_string(),
            )
            .unwrap()
    };
    std::thread::sleep(Duration::from_millis(50));
    let (reply, snapshot) = std::sync::mpsc::channel();
    {
        let coordinator = backend.coordinator().lock().unwrap();
        coordinator
            .send_command(
                SessionId(created.id),
                SessionCommand::Snapshot {
                    reply,
                    name: Some("snapshot".to_string()),
                    isolation: IsolationTier::Bare,
                },
            )
            .unwrap();
    }
    let persisted = snapshot.recv_timeout(Duration::from_secs(1)).unwrap();
    let pane = persisted.panes.values().next().unwrap();
    let PersistedPaneType::Shell {
        env_snapshot: Some(env),
        ..
    } = &pane.pane_type
    else {
        panic!("snapshot must retain the shell environment");
    };
    let value = env.variables.get("MALT_BOUNDARY").unwrap();
    assert!(matches!(
        value.value,
        malt_protocol::persist::session::PersistedVarValue::Str { ref value } if value == "old"
    ));
    assert!(receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_ok());
}

#[test]
fn concurrent_exec_and_send_share_the_same_session_fifo() {
    use std::sync::Arc;

    let backend = Arc::new(make_backend());
    let created = backend.create_session(None, None).unwrap();
    let first_backend = Arc::clone(&backend);
    let id = created.id;
    let first = std::thread::spawn(move || {
        first_backend.exec_command(id, "export MALT_PRODUCER=ordered; sleep 1".to_string())
    });
    // Submitting the observing exec before the first command is active would
    // test nothing: the env it checks for is exported *by* that command.
    wait_for_queue(&backend, created.id, "the first command to start", |s| {
        s.active
    });

    // Raw input races with the exec submission on purpose. Since feature 005
    // it no longer travels the execution FIFO -- it goes to the session's
    // input pipe -- so this now asserts the two paths do not interfere,
    // rather than that they share a queue.
    let send_backend = Arc::clone(&backend);
    let sent = std::thread::spawn(move || {
        send_backend.send_input(id, "echo send-$MALT_PRODUCER".to_string())
    });

    let observed = backend
        .exec_command(created.id, "echo exec-$MALT_PRODUCER".to_string())
        .unwrap();
    assert!(first.join().unwrap().is_ok());
    assert!(sent.join().unwrap().is_ok());
    assert!(observed.output.contains("exec-ordered"));
}

/// Explicit acceptance soak from the feature quickstart. It is ignored for
/// normal developer feedback because it deliberately occupies one worker for
/// ten minutes; the implementation workflow runs it before completion.
#[test]
#[ignore = "10-minute cross-session responsiveness soak"]
fn ten_minute_cross_session_soak() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let backend = Arc::new(make_backend());
    let busy = backend.create_session(None, None).unwrap();
    let independent = backend.create_session(None, None).unwrap();
    let runner = Arc::clone(&backend);
    let busy_id = busy.id;
    let command = std::thread::spawn(move || {
        runner.exec_command(busy_id, "sleep 600; echo soak-complete".to_string())
    });
    std::thread::sleep(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        let started = Instant::now();
        let _ = backend.get_output_text(independent.id).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(100));
    }
    // The command remains accepted after the Gateway's documented 30-second
    // synchronous wait expires; caller timeout is not cancellation.
    assert!(matches!(
        command.join().unwrap(),
        Err(GatewayError::Internal(message)) if message == "command timed out"
    ));
}
