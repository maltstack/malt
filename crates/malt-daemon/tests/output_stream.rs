//! Integration tests for streaming command output (spec 006, US1/US3/US2).
//!
//! Every test here starts a real command through a real `SessionExecutor`
//! and its real worker thread -- `SessionCommand::RunCommand`, the same path
//! `/exec` and the MCP `run_command` tool use -- rather than injecting
//! chunks directly into the log or a subscriber. That distinction is the
//! whole point: this feature family's recurring defect has been a delivery
//! path proven only by tests that never exercise it for real (Gateway auth,
//! `AuthorityTracker`, the tool stdin slurp).

use malt_daemon::executor::output_log::OutputEventKind;
use malt_daemon::executor::session_thread::{ClientMessage, SessionCommand, SessionExecutor};
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, IsolationTier, PaneId,
    SessionId, UnicodeLevel,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn default_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}

/// Register a VNP client and wait for its initial state, returning the
/// receiver its `Render`/`OutputChunk` messages arrive on.
fn register_vnp_client(
    cmd_tx: &mpsc::SyncSender<SessionCommand>,
    client_id: u64,
) -> std::sync::mpsc::Receiver<ClientMessage> {
    let (render_tx, render_rx) = std::sync::mpsc::sync_channel::<ClientMessage>(64);
    let (initial_tx, initial_rx) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RegisterVnpClient {
            client_id,
            authority: InputAuthority::Observe,
            capabilities: default_capabilities(),
            render_tx,
            initial_reply: initial_tx,
        })
        .unwrap();
    initial_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("RegisterVnpClient should reply with initial state");
    render_rx
}

fn subscribe(
    cmd_tx: &mpsc::SyncSender<SessionCommand>,
) -> tokio::sync::mpsc::Receiver<malt_daemon::executor::output_log::OutputEvent> {
    subscribe_from(cmd_tx, None)
}

fn subscribe_from(
    cmd_tx: &mpsc::SyncSender<SessionCommand>,
    resume_from: Option<u64>,
) -> tokio::sync::mpsc::Receiver<malt_daemon::executor::output_log::OutputEvent> {
    let (reply, rx) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::SubscribeOutput { resume_from, reply })
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
        malt_daemon::executor::output_log::SUBSCRIBER_BUFFER,
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

/// T024 (US3): a subscriber disconnects mid-command and resumes from the
/// last sequence it saw; the two runs' content, concatenated, must
/// reproduce the command's full output exactly once each -- verified by
/// content, not by count (SC-003).
#[test]
fn resuming_after_disconnect_reproduces_the_full_output_byte_for_byte() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(4), PaneId(1), IsolationTier::Bare).unwrap();

    let mut first_rx = subscribe(&cmd_tx);

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo one; sleep 1; echo two; sleep 1; echo three".to_string(),
            reply: run_reply,
        })
        .unwrap();

    // Read exactly one chunk, then simulate a disconnect by dropping the
    // receiver while the command is still running.
    let first_event = first_rx
        .blocking_recv()
        .expect("subscriber should receive the first chunk");
    let mut received = match first_event.kind {
        OutputEventKind::Chunk { data, .. } => data,
        other => panic!("expected a Chunk, got {other:?}"),
    };
    let last_seen = first_event.sequence;
    drop(first_rx);

    // Resume from the last sequence the (now-gone) first subscriber saw.
    // The daemon must not care that a different logical subscriber is
    // asking -- resumption is keyed on the sequence, not on connection
    // identity.
    let mut resumed_rx = subscribe_from(&cmd_tx, Some(last_seen));

    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should eventually complete");
    assert_eq!(output.exit_code, 0);

    // Drain whatever the resumed subscriber has -- the command already
    // finished, so everything after `last_seen` should already be queued.
    while let Ok(event) = resumed_rx.try_recv() {
        if let OutputEventKind::Chunk { data, .. } = event.kind {
            received.extend_from_slice(&data);
        }
    }

    assert_eq!(
        received,
        output.output.as_bytes(),
        "concatenating both runs must reproduce the command's full output byte-for-byte"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

/// T025 (US3): a subscriber that stops reading is told it lagged and is
/// dropped -- the command's duration and other subscribers are unaffected
/// (SC-005). A tiny subscriber buffer (not the real 256-deep default) so a
/// handful of real chunks are enough to overflow it deterministically. The
/// command paces itself with short sleeps between chunks -- enough real
/// wall-clock time for a genuinely-draining reader thread to be scheduled
/// and keep up under parallel test-suite load, which a back-to-back burst
/// with a very tight buffer does not reliably leave room for.
#[test]
fn a_stalled_subscriber_is_told_it_lagged_and_dropped_without_affecting_others() {
    let spawned = SessionExecutor::spawn_with_capacity_and_output_bound(
        SessionId(5),
        PaneId(1),
        IsolationTier::Bare,
        256,
        malt_daemon::executor::output_log::MAX_RETAINED_BYTES,
        4,
        Vec::new(),
    )
    .unwrap();
    let cmd_tx = spawned.control_tx;

    // The stalled subscriber: never read from again after subscribing.
    let mut stalled_rx = subscribe(&cmd_tx);
    // A healthy subscriber that keeps draining throughout -- on its own
    // thread, actively reading the whole time, so it cannot itself lag
    // behind the same tiny buffer while this test waits for the command.
    let mut healthy_rx = subscribe(&cmd_tx);
    let healthy_handle = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        while let Some(event) = healthy_rx.blocking_recv() {
            match event.kind {
                OutputEventKind::Chunk { data, .. } => bytes.extend_from_slice(&data),
                OutputEventKind::Gap { .. } => {
                    panic!("the healthy subscriber must not lag behind a stalled one")
                }
                _ => {}
            }
            if bytes.ends_with(b"six\n") {
                break;
            }
        }
        bytes
    });

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo one; sleep 0.1; echo two; sleep 0.1; echo three; sleep 0.1; \
                      echo four; sleep 0.1; echo five; sleep 0.1; echo six"
                .to_string(),
            reply: run_reply,
        })
        .unwrap();

    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("the command must complete at normal speed despite the stalled subscriber");
    assert_eq!(output.exit_code, 0);

    // The healthy subscriber must see everything, unaffected by the other
    // subscriber's stall.
    let healthy_bytes = healthy_handle
        .join()
        .expect("healthy-subscriber thread panicked");
    assert_eq!(healthy_bytes, output.output.as_bytes());

    // The stalled subscriber must have a gap queued -- its absence is
    // exactly how this defect hid in feature 004.
    let mut saw_gap = false;
    while let Ok(event) = stalled_rx.try_recv() {
        if matches!(event.kind, OutputEventKind::Gap { .. }) {
            saw_gap = true;
        }
    }
    assert!(
        saw_gap,
        "a stalled subscriber must be told it lagged, not dropped silently"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    spawned
        .control_thread
        .join()
        .expect("session thread panicked");
}

/// T026 (US3): invalid UTF-8 and a multi-byte character survive the round
/// trip, including base64 (SC-006).
///
/// Uses a real file with deliberately unusual bytes and a real `cat`
/// (malt-tools, dispatched through the same sink-forwarding path as any
/// other command) rather than `printf` escapes: mash's own `\NNN` octal
/// escape interpreter casts the byte value directly to `char` before
/// pushing it into a `String`, so `printf '\377'` does not actually
/// produce a raw 0xFF byte on the wire -- it produces the UTF-8 encoding
/// of U+00FF. That is a pre-existing, separate quirk of `printf` (not
/// touched by this feature), not something to route around by injecting
/// bytes directly into the pipeline.
#[test]
fn invalid_utf8_and_a_split_multibyte_character_survive_the_round_trip_including_base64() {
    use base64::Engine;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bytes.bin");
    let mut original = Vec::new();
    original.extend_from_slice("caf".as_bytes());
    original.extend_from_slice(&[0xC3, 0xA9]); // 'é', split across the boundary below
    original.extend_from_slice(b"\n");
    original.extend_from_slice(&[0xFF, 0xFE]); // not valid UTF-8 in any encoding
    original.extend_from_slice(b" binary\n");
    std::fs::write(&file, &original).unwrap();

    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(6), PaneId(1), IsolationTier::Bare).unwrap();
    let mut output_rx = subscribe(&cmd_tx);

    let cmd = format!("cat '{}'", file.to_string_lossy());
    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: cmd,
            reply: run_reply,
        })
        .unwrap();

    let output = run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should complete");
    assert_eq!(output.exit_code, 0);

    let mut received = Vec::new();
    while let Ok(event) = output_rx.try_recv() {
        if let OutputEventKind::Chunk { data, .. } = event.kind {
            received.extend_from_slice(&data);
        }
    }

    assert_eq!(
        received, original,
        "raw bytes must survive the sink/log/subscriber path exactly, invalid UTF-8 included"
    );

    // The HTTP transport encodes chunks as base64 (research R6) -- prove
    // that step is lossless for this exact content too.
    let encoded = base64::engine::general_purpose::STANDARD.encode(&received);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .expect("valid base64 must always decode");
    assert_eq!(
        decoded, original,
        "base64 round-trip must reproduce the exact bytes, not a lossy text approximation"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

/// T030 (US2): an attached client receives more than one `Render` during a
/// command that produces output over several seconds (FR-007), driven by a
/// real command -- not by calling `dispatch_render` directly.
#[test]
fn an_attached_client_receives_more_than_one_render_during_a_running_command() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(7), PaneId(1), IsolationTier::Bare).unwrap();
    let render_rx = register_vnp_client(&cmd_tx, 1);

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo one; sleep 0.2; echo two; sleep 0.2; echo three".to_string(),
            reply: run_reply,
        })
        .unwrap();

    let mut render_count = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && render_count < 2 {
        if let Ok(ClientMessage::Render(_)) = render_rx.recv_timeout(Duration::from_millis(500)) {
            render_count += 1;
        }
    }
    assert!(
        render_count >= 2,
        "an attached client must see the view update more than once during the command, saw {render_count}"
    );

    run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should complete");

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

/// T031 (US2): two attached clients converge on identical content; neither
/// sees output the other does not (SC-007).
#[test]
fn two_attached_clients_converge_on_identical_content() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(8), PaneId(1), IsolationTier::Bare).unwrap();
    let render_rx_a = register_vnp_client(&cmd_tx, 1);
    let render_rx_b = register_vnp_client(&cmd_tx, 2);

    let (run_reply, run_result) = mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo one; echo two; echo three".to_string(),
            reply: run_reply,
        })
        .unwrap();
    run_result
        .recv_timeout(Duration::from_secs(10))
        .expect("command should complete");

    // Drain each client's queue for a moment after completion; every Render
    // reflects the whole grid, so the *last* one each client saw is what
    // matters for convergence.
    let last_frame = |rx: &std::sync::mpsc::Receiver<ClientMessage>| -> Option<malt_protocol::render::RenderBatch> {
        let mut last = None;
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(ClientMessage::Render(batch)) => last = Some(batch),
                Ok(_) => {}
                Err(_) => continue,
            }
        }
        last
    };

    let last_a = last_frame(&render_rx_a).expect("client A must have received at least one render");
    let last_b = last_frame(&render_rx_b).expect("client B must have received at least one render");
    assert_eq!(
        last_a.commands, last_b.commands,
        "two attached clients must converge on identical content"
    );

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}
