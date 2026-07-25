//! Integration tests for the VNP listener using typed bitpack framing.
//!
//! Each test connects to a real TCP listener, performs the VNP handshake,
//! and verifies the typed protocol messages using real bitpack encode/decode.
//! No JSON is used post-handshake.

use std::io::BufReader;
use std::net::TcpStream;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use malt_protocol::codec::{
    make_envelope, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION, MSG_ATTACH_SESSION, MSG_FRAME_ACK,
    MSG_HELLO, MSG_HELLO_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT, MSG_RENDER_BATCH,
};
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, IsolationTier, KeyModifiers,
    SessionId, UnicodeLevel,
};
use malt_protocol::envelope::{decode_envelope, encode_message};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck};
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};
use malt_protocol::render::{FrameAck, InitialState, RenderBatch};
use malt_protocol::session::AttachSession;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_daemon::vnp_listener::accept_vnp_connections;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_coordinator_with_session() -> (Arc<Mutex<Coordinator>>, u32) {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let session_id = sid.0;
    (Arc::new(Mutex::new(coord)), session_id)
}

fn start_test_listener(coordinator: Arc<Mutex<Coordinator>>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let counter = Arc::new(AtomicU64::new(1));
    std::thread::spawn(move || {
        accept_vnp_connections(listener, coordinator, counter);
    });
    addr
}

/// Build a Hello message with test capabilities.
fn make_hello() -> Hello {
    Hello {
        version: 1,
        client_type: "test".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::TrueColor,
            unicode: UnicodeLevel::Full,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: true,
            max_fps: 60,
            _unknown: Vec::new(),
        },
        _unknown: Vec::new(),
    }
}

/// Perform the VNP handshake using the correct DOMAIN_HANDSHAKE (0) constant.
fn do_handshake(
    writer: &mut FrameWriter<TcpStream>,
    reader: &mut FrameReader<BufReader<TcpStream>>,
) -> HelloAck {
    let hello = make_hello();
    // domain = 0 (DOMAIN_HANDSHAKE), msg_type = MSG_HELLO = 0x01
    let env = make_envelope(0, MSG_HELLO, 0);
    let mut w = BitWriter::new();
    hello.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&frame).unwrap();

    let recv_frame = reader.read_frame().unwrap();
    let (env, msg_bytes) = decode_envelope(&recv_frame.payload).unwrap();
    assert_eq!(env.domain, 0, "expected DOMAIN_HANDSHAKE for HelloAck");
    assert_eq!(env.msg_type, MSG_HELLO_ACK, "expected HelloAck");
    let mut r = BitReader::new(msg_bytes);
    HelloAck::unpack(&mut r).unwrap()
}

/// Send AttachSession for the given session_id.
fn do_attach(writer: &mut FrameWriter<TcpStream>, session_id: u32) {
    let attach = AttachSession {
        session_id: SessionId(session_id),
        authority: InputAuthority::Exclusive,
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_SESSION, MSG_ATTACH_SESSION, session_id);
    let mut w = BitWriter::new();
    attach.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&frame).unwrap();
}

/// Read and decode the next InitialState frame, asserting correct domain/type.
fn read_initial_state(reader: &mut FrameReader<BufReader<TcpStream>>) -> InitialState {
    let frame = reader.read_frame().unwrap();
    let (env, msg_bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(
        env.domain, DOMAIN_RENDER,
        "expected DOMAIN_RENDER for InitialState"
    );
    assert_eq!(
        env.msg_type, MSG_INITIAL_STATE,
        "expected MSG_INITIAL_STATE"
    );
    let mut r = BitReader::new(msg_bytes);
    InitialState::unpack(&mut r).unwrap()
}

/// Send a character KeyEvent.
fn send_char_key(writer: &mut FrameWriter<TcpStream>, session_id: u32, ch: char) {
    let key = KeyEvent {
        key: KeyValue::Char {
            codepoint: ch as u32,
        },
        modifiers: KeyModifiers::empty(),
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_INPUT, MSG_KEY_EVENT, session_id);
    let mut w = BitWriter::new();
    key.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&frame).unwrap();
}

/// Send an Enter (Named) KeyEvent.
fn send_enter_key(writer: &mut FrameWriter<TcpStream>, session_id: u32) {
    let key = KeyEvent {
        key: KeyValue::Named {
            key: NamedKey::Enter,
        },
        modifiers: KeyModifiers::empty(),
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_INPUT, MSG_KEY_EVENT, session_id);
    let mut w = BitWriter::new();
    key.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&frame).unwrap();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: VNP handshake succeeds.
///
/// Connect to the listener, send Hello (domain=0, type=0x01), and read a
/// valid HelloAck (domain=0, type=0x02) with negotiated_version=1.
#[test]
fn vnp_handshake_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    let addr = start_test_listener(coordinator);

    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    let ack = do_handshake(&mut writer, &mut reader);
    assert_eq!(ack.negotiated_version, 1, "negotiated version must be 1");
}

/// Test 2: After attaching, the listener sends an InitialState frame.
///
/// Connect + handshake, send AttachSession for a valid session, and verify the
/// response is InitialState (domain=6, type=0x03) with frame_seq=0.
#[test]
fn vnp_attach_receives_initial_state() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let addr = start_test_listener(coordinator);

    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    let _ack = do_handshake(&mut writer, &mut reader);
    do_attach(&mut writer, session_id);

    let initial = read_initial_state(&mut reader);
    assert_eq!(
        initial.frame_seq, 0,
        "InitialState must have frame_seq=0 on first attach"
    );
}

/// Test 3: Sending key input followed by Enter produces a RenderBatch.
///
/// Connect + handshake + attach, type "echo ok" char by char as KeyEvent frames,
/// then send Enter. Read frames until a RenderBatch (domain=6, type=0x01) with
/// non-empty commands appears, or timeout.
#[test]
fn vnp_key_input_followed_by_enter_produces_render_batch() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let addr = start_test_listener(coordinator);

    let stream = TcpStream::connect(&addr).unwrap();
    // Short per-read timeout so the loop can check the deadline frequently.
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(500)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    let _ack = do_handshake(&mut writer, &mut reader);
    do_attach(&mut writer, session_id);

    // Consume the InitialState frame before sending input.
    let _initial = read_initial_state(&mut reader);

    // Type "echo ok" character by character.
    for ch in "echo ok".chars() {
        send_char_key(&mut writer, session_id, ch);
    }
    // Send Enter to execute the command.
    send_enter_key(&mut writer, session_id);

    // The server's main loop drains render_rx only after reading a client frame.
    // Send FrameAck(0) frames periodically to keep the server loop ticking so it
    // can drain the render_rx channel and forward the RenderBatch to us.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let mut last_ping = std::time::Instant::now();
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for a RenderBatch with non-empty commands"
        );

        // Periodically send FrameAck(0) to tick the server's main loop so it
        // drains render_rx and forwards any queued RenderBatch frames to us.
        if last_ping.elapsed() >= std::time::Duration::from_millis(100) {
            let ping = FrameAck {
                frame_seq: 0,
                _unknown: Vec::new(),
            };
            let env = make_envelope(DOMAIN_RENDER, MSG_FRAME_ACK, session_id);
            let mut w = BitWriter::new();
            ping.pack(&mut w).unwrap();
            let payload = w.finish();
            let combined = encode_message(&env, &payload).unwrap();
            let f = Frame {
                flags: FrameFlags::new(),
                payload: combined,
            };
            writer.write_frame(&f).unwrap();
            last_ping = std::time::Instant::now();
        }

        match reader.read_frame() {
            Ok(frame) => {
                let (env, msg_bytes) = decode_envelope(&frame.payload).unwrap();
                if env.domain == DOMAIN_RENDER && env.msg_type == MSG_RENDER_BATCH {
                    let mut r = BitReader::new(msg_bytes);
                    let batch = RenderBatch::unpack(&mut r).unwrap();
                    if !batch.commands.is_empty() {
                        // Success: a render batch with drawing commands was produced.
                        break;
                    }
                }
                // Any other frame — keep reading.
            }
            Err(malt_protocol::framing::FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // No data yet — yield briefly and retry.
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(e) => panic!("unexpected frame error while waiting for RenderBatch: {e}"),
        }
    }
}

/// Test 4: FrameAck is accepted without crashing the listener.
///
/// Connect + handshake + attach, receive InitialState, send a FrameAck for
/// frame_seq=0, wait 100 ms, and assert the connection is still alive by
/// verifying no error occurred.
#[test]
fn vnp_frame_ack_accepted() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let addr = start_test_listener(coordinator);

    std::thread::sleep(std::time::Duration::from_millis(50));

    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    let _ack = do_handshake(&mut writer, &mut reader);
    do_attach(&mut writer, session_id);

    let initial = read_initial_state(&mut reader);
    let initial_seq = initial.frame_seq;

    // Send FrameAck for the initial frame.
    let frame_ack = FrameAck {
        frame_seq: initial_seq,
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_RENDER, MSG_FRAME_ACK, session_id);
    let mut w = BitWriter::new();
    frame_ack.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    let ack_frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&ack_frame).unwrap();

    // Wait a moment; if the listener crashed or disconnected we would see an
    // error on the next read (WouldBlock/TimedOut is fine — it means idle).
    std::thread::sleep(std::time::Duration::from_millis(100));

    // A WouldBlock / TimedOut is expected (no data yet); anything else would
    // indicate the listener dropped the connection.  We only fail on hard errors.
    match reader.read_frame() {
        Ok(_) => {} // A frame arrived unexpectedly — still not a failure.
        Err(malt_protocol::framing::FrameError::Io(ref e))
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
        {
            // Expected: idle connection, no data within the timeout.
        }
        Err(e) => panic!("unexpected frame error after FrameAck: {e}"),
    }
}

#[test]
fn vnp_attach_during_execution_returns_initial_state_promptly() {
    use std::time::{Duration, Instant};

    let (coordinator, session_id) = make_coordinator_with_session();
    let receiver = coordinator
        .lock()
        .unwrap()
        .submit_execution(SessionId(session_id), "sleep 1; echo done".to_string())
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let addr = start_test_listener(coordinator);
    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(BufReader::new(stream));
    let _ = do_handshake(&mut writer, &mut reader);
    let started = Instant::now();
    do_attach(&mut writer, session_id);
    let initial = read_initial_state(&mut reader);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(initial.frame_seq, 0);
    send_char_key(&mut writer, session_id, 'x');
    let ack = FrameAck {
        frame_seq: initial.frame_seq,
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_RENDER, MSG_FRAME_ACK, session_id);
    let mut bits = BitWriter::new();
    ack.pack(&mut bits).unwrap();
    writer
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload: encode_message(&env, &bits.finish()).unwrap(),
        })
        .unwrap();
    assert!(receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_ok());
}

#[test]
fn one_hundred_vnp_attaches_remain_bounded_while_execution_is_busy() {
    use std::time::{Duration, Instant};

    let (coordinator, session_id) = make_coordinator_with_session();
    let receiver = coordinator
        .lock()
        .unwrap()
        .submit_execution(SessionId(session_id), "sleep 3; echo done".to_string())
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let addr = start_test_listener(coordinator);

    for _ in 0..100 {
        let stream = TcpStream::connect(&addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let write_stream = stream.try_clone().unwrap();
        let mut writer = FrameWriter::new(write_stream);
        let mut reader = FrameReader::new(BufReader::new(stream));
        let _ = do_handshake(&mut writer, &mut reader);
        let started = Instant::now();
        do_attach(&mut writer, session_id);
        let initial = read_initial_state(&mut reader);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(initial.frame_seq, 0);
    }
    assert!(receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .is_ok());
}
