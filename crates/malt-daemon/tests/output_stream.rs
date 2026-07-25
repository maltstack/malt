//! Integration tests for streaming command output (spec 006, US1).
//!
//! Every test here starts a real command through a real `SessionExecutor`
//! and its real worker thread -- `SessionCommand::RunCommand`, the same path
//! `/exec` and the MCP `run_command` tool use -- rather than injecting
//! chunks directly into the log or a subscriber. That distinction is the
//! whole point: this feature family's recurring defect has been a delivery
//! path proven only by tests that never exercise it for real (Gateway auth,
//! `AuthorityTracker`, the tool stdin slurp).

use malt_daemon::executor::output_log::OutputEventKind;
use malt_daemon::executor::session_thread::{SessionCommand, SessionExecutor};
use malt_protocol::common::{IsolationTier, PaneId, SessionId};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn subscribe(
    cmd_tx: &mpsc::SyncSender<SessionCommand>,
) -> tokio::sync::mpsc::Receiver<malt_daemon::executor::output_log::OutputEvent> {
    let (reply, rx) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::SubscribeOutput {
            resume_from: None,
            reply,
        })
        .unwrap();
    rx.recv().expect("subscribe should reply")
}

fn expect_chunk_text(event: malt_daemon::executor::output_log::OutputEvent) -> String {
    match event.kind {
        OutputEventKind::Chunk { data, .. } => String::from_utf8_lossy(&data).to_string(),
        other => panic!("expected a Chunk, got {other:?}"),
    }
}

#[test]
fn output_arrives_and_is_retrievable_while_the_command_is_still_running() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();

    let mut output_rx = subscribe(&cmd_tx);

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo first; sleep 2; echo second".to_string(),
            reply: run_reply,
        })
        .unwrap();

    // The retrieval call must return promptly -- long before the 2s sleep
    // finishes -- proving "first" is observable before the command exits.
    let start = Instant::now();
    let first_event = output_rx
        .blocking_recv()
        .expect("subscriber should receive the first chunk while sleep is still running");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(1500),
        "retrieval blocked for close to the command's full duration: {elapsed:?}"
    );
    assert_eq!(expect_chunk_text(first_event), "first\n");

    // The run itself must still be in flight at this point -- otherwise the
    // "before it ends" claim above is untested, not merely fast.
    assert!(
        run_result.try_recv().is_err(),
        "the command must not have finished yet when its first chunk was observed"
    );

    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should eventually complete");
    assert_eq!(output.exit_code, 0);

    let second_event = output_rx
        .blocking_recv()
        .expect("subscriber should receive the second chunk too");
    assert_eq!(expect_chunk_text(second_event), "second\n");

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn output_produced_before_a_failing_commands_exit_is_delivered_and_not_replaced() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(2), PaneId(1), IsolationTier::Bare).unwrap();

    let mut output_rx = subscribe(&cmd_tx);

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo produced-before-failure; sleep 1; false".to_string(),
            reply: run_reply,
        })
        .unwrap();

    let event = output_rx
        .blocking_recv()
        .expect("output produced before the failure must still be delivered");
    assert_eq!(expect_chunk_text(event), "produced-before-failure\n");

    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should eventually complete");
    assert_ne!(
        output.exit_code, 0,
        "the command must actually have failed for this test to mean anything"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn a_command_producing_more_than_the_retained_bound_completes_and_the_log_stays_bounded() {
    // A tiny bound (not the real 4 MiB default) so the test can exceed it
    // with a handful of small commands rather than megabytes of real
    // output -- SC-004 "in miniature" (see spec 006 tasks.md T018).
    const TINY_BOUND: usize = 64;

    let spawned = SessionExecutor::spawn_with_capacity_and_output_bound(
        SessionId(3),
        PaneId(1),
        IsolationTier::Bare,
        256,
        TINY_BOUND,
        Vec::new(),
    )
    .unwrap();
    let cmd_tx = spawned.control_tx;

    // Each line is ~9 bytes ("line-N\n"); ten of them comfortably exceed the
    // 64-byte bound, forcing eviction while the command runs.
    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "for i in 1 2 3 4 5 6 7 8 9 10; do echo line-$i; done".to_string(),
            reply: run_reply,
        })
        .unwrap();
    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should complete successfully despite exceeding the retention bound");
    assert_eq!(output.exit_code, 0);

    // Resuming from the very start must report a gap: everything before the
    // bound was evicted, and a subscriber must be told, not left believing
    // it can see the whole thing.
    let (reply, rx) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::SubscribeOutput {
            resume_from: Some(0),
            reply,
        })
        .unwrap();
    let mut resumed_rx = rx.recv().expect("subscribe should reply");

    let first = resumed_rx
        .try_recv()
        .expect("a gap or chunk should already be queued from the replay");
    let mut retained_bytes: usize = 0;
    let mut saw_gap = matches!(first.kind, OutputEventKind::Gap { .. });
    if let OutputEventKind::Chunk { data, .. } = &first.kind {
        retained_bytes += data.len();
    }
    while let Ok(event) = resumed_rx.try_recv() {
        match event.kind {
            OutputEventKind::Gap { .. } => saw_gap = true,
            OutputEventKind::Chunk { data, .. } => retained_bytes += data.len(),
            _ => {}
        }
    }

    assert!(
        saw_gap,
        "resuming from before the retained window must report a gap, not silently start late"
    );
    assert!(
        retained_bytes <= TINY_BOUND * 2,
        "retained output must stay near the byte bound regardless of how much was produced, got {retained_bytes} bytes"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    spawned
        .control_thread
        .join()
        .expect("session thread panicked");
}
