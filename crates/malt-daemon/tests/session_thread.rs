use malt_daemon::bus::BusMessage;
use malt_daemon::executor::session_thread::{SessionCommand, SessionExecutor};
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, IsolationTier, KeyModifiers,
    PaneId, SessionId, UnicodeLevel,
};
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};
use malt_protocol::priority::Priority;
use malt_protocol::render::RenderBatch;

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

#[test]
fn session_executor_starts_and_stops() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_processes_input() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let msg = BusMessage {
        domain: 2,
        msg_type: 1,
        priority: Priority::Critical,
        producer_id: 0,
        payload: vec![1, 2, 3],
    };
    cmd_tx.send(SessionCommand::Deliver(msg)).unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_attach_detach() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    cmd_tx
        .send(SessionCommand::AttachClient {
            client_id: 100,
            authority: InputAuthority::Exclusive,
        })
        .unwrap();
    cmd_tx
        .send(SessionCommand::DetachClient { client_id: 100 })
        .unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_resize() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    cmd_tx
        .send(SessionCommand::Resize {
            cols: 120,
            rows: 40,
        })
        .unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn register_vnp_client_returns_initial_state() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let (initial_tx, initial_rx) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RegisterVnpClient {
            client_id: 42,
            capabilities: default_capabilities(),
            render_tx,
            initial_reply: initial_tx,
        })
        .unwrap();
    let initial_state = initial_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(initial_state.frame_seq, 0);
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn key_input_char_does_not_crash() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let key = KeyEvent {
        key: KeyValue::Char {
            codepoint: 'a' as u32,
        },
        modifiers: KeyModifiers::empty(),
        _unknown: Vec::new(),
    };
    cmd_tx.send(SessionCommand::KeyInput { key }).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn key_input_enter_triggers_render_to_client() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let (render_tx, render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let (initial_tx, initial_rx) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RegisterVnpClient {
            client_id: 1,
            capabilities: default_capabilities(),
            render_tx,
            initial_reply: initial_tx,
        })
        .unwrap();
    let _ = initial_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    for ch in "echo hi".chars() {
        let key = KeyEvent {
            key: KeyValue::Char {
                codepoint: ch as u32,
            },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        cmd_tx.send(SessionCommand::KeyInput { key }).unwrap();
    }
    let enter = KeyEvent {
        key: KeyValue::Named {
            key: NamedKey::Enter,
        },
        modifiers: KeyModifiers::empty(),
        _unknown: Vec::new(),
    };
    cmd_tx
        .send(SessionCommand::KeyInput { key: enter })
        .unwrap();
    let batch = render_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(!batch.commands.is_empty());
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

/// Confirmed by the 2026-07-24 input/concurrency audit: gateway/agent-driven
/// execution (`RunCommand`, the Gateway `/exec` path) never notified
/// attached VNP clients that anything changed, regardless of attach
/// timing -- "both see the same authoritative state" didn't hold. This
/// proves the partial fix: a `RunCommand` now pushes a render frame once
/// the command completes.
#[test]
fn run_command_triggers_render_to_attached_client() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let (render_tx, render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let (initial_tx, initial_rx) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RegisterVnpClient {
            client_id: 1,
            capabilities: default_capabilities(),
            render_tx,
            initial_reply: initial_tx,
        })
        .unwrap();
    let _ = initial_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();

    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo agent-driven".to_string(),
            reply: reply_tx,
        })
        .unwrap();
    let _ = reply_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    let batch = render_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect(
            "an attached VNP client must receive a RenderBatch after a \
             gateway-driven RunCommand completes -- previously nothing \
             ever notified it",
        );
    assert!(!batch.commands.is_empty());

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn ack_frame_does_not_crash() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    cmd_tx
        .send(SessionCommand::AckFrame {
            client_id: 99,
            frame_seq: 0,
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn unregister_vnp_client_does_not_crash() {
    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let (render_tx, _render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);
    let (initial_tx, initial_rx) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RegisterVnpClient {
            client_id: 7,
            capabilities: default_capabilities(),
            render_tx,
            initial_reply: initial_tx,
        })
        .unwrap();
    let _ = initial_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    cmd_tx
        .send(SessionCommand::UnregisterVnpClient { client_id: 7 })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn control_observation_and_snapshot_remain_prompt_during_a_long_command() {
    use std::time::{Duration, Instant};

    let (cmd_tx, handle) =
        SessionExecutor::spawn(SessionId(1), PaneId(1), IsolationTier::Bare).unwrap();
    let (reply, result) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "sleep 1; echo complete".to_string(),
            reply,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    let (styled_reply, styled) = std::sync::mpsc::channel();
    let (plain_reply, plain) = std::sync::mpsc::channel();
    let (snapshot_reply, snapshot) = std::sync::mpsc::channel();
    cmd_tx.send(SessionCommand::GetOutput { reply: styled_reply }).unwrap();
    cmd_tx
        .send(SessionCommand::GetOutputText { reply: plain_reply })
        .unwrap();
    cmd_tx
        .send(SessionCommand::Snapshot {
            reply: snapshot_reply,
            name: Some("busy".to_string()),
            isolation: IsolationTier::Bare,
        })
        .unwrap();
    let _ = styled.recv_timeout(Duration::from_secs(1)).unwrap();
    let _ = plain.recv_timeout(Duration::from_secs(1)).unwrap();
    let _ = snapshot.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));

    // A dropped initiating receiver does not cancel or poison later control.
    let (dropped, dropped_result) = std::sync::mpsc::channel();
    cmd_tx
        .send(SessionCommand::RunCommand {
            command: "echo survives-drop".to_string(),
            reply: dropped,
        })
        .unwrap();
    drop(dropped_result);
    assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().output.contains("complete"));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().unwrap();
}
