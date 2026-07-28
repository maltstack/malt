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
    make_envelope, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION, MSG_ATTACH_SESSION,
    MSG_CREATE_SESSION, MSG_FRAME_ACK, MSG_HELLO, MSG_HELLO_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT,
    MSG_RENDER_BATCH, MSG_SESSION_LIST,
};
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, InputAuthority, IsolationPolicy, IsolationTier,
    KeyModifiers, SessionId, UnicodeLevel,
};
use malt_protocol::envelope::{decode_envelope, encode_message};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck};
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};
use malt_protocol::render::{FrameAck, InitialState, RenderBatch};
use malt_protocol::session::{AttachSession, CreateSession, SessionList};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_daemon::vnp_listener::accept_vnp_connections;
use malt_gateway::auth::{AuthScope, TokenStore};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_coordinator_with_session() -> (Arc<Mutex<Coordinator>>, u32) {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
        .expect("create debounce store");
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    let sid = coord
        .create_session(None, IsolationTier::Bare, None)
        .unwrap();
    let session_id = sid.0;
    (Arc::new(Mutex::new(coord)), session_id)
}

/// Start a listener and seed this thread's credential so `make_hello`
/// produces an acceptable Hello.
fn start_test_listener_seeded(coordinator: Arc<Mutex<Coordinator>>) -> String {
    let (addr, token) = start_test_listener_with_auth(coordinator);
    set_test_credential(&token);
    addr
}

/// Start a listener and return its address plus a credential it will accept.
fn start_test_listener_with_auth(coordinator: Arc<Mutex<Coordinator>>) -> (String, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let counter = Arc::new(AtomicU64::new(1));
    let tokens = Arc::new(TokenStore::new());
    let token = tokens
        .generate_token(AuthScope::Admin)
        .expect("generate test token");
    std::thread::spawn(move || {
        accept_vnp_connections(listener, coordinator, counter, tokens);
    });
    (addr, token)
}

/// Build a Hello message with test capabilities and a credential.
fn make_hello() -> Hello {
    make_hello_with(Some(test_credential()))
}

/// The credential the shared test listener accepts. Tests that start their
/// own listener use the token it returns instead.
fn test_credential() -> String {
    TEST_TOKEN.with(|t| t.borrow().clone())
}

thread_local! {
    static TEST_TOKEN: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn set_test_credential(token: &str) {
    TEST_TOKEN.with(|t| *t.borrow_mut() = token.to_string());
}

fn make_hello_with(credential: Option<String>) -> Hello {
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
        credential,
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

/// Create a session through the VNP pre-attach phase and return the one
/// status-bearing SessionInfo response. The next client frame can then attach
/// to its returned id on the same authenticated socket.
fn do_create_session(
    writer: &mut FrameWriter<TcpStream>,
    reader: &mut FrameReader<BufReader<TcpStream>>,
    isolation: IsolationTier,
    policy: IsolationPolicy,
) -> malt_protocol::common::SessionInfo {
    let create = CreateSession {
        name: Some("vnp-created".to_string()),
        isolation,
        group: None,
        policy,
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_SESSION, MSG_CREATE_SESSION, 0);
    let mut w = BitWriter::new();
    create.pack(&mut w).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: encode_message(&env, &w.finish()).unwrap(),
    };
    writer.write_frame(&frame).unwrap();

    let frame = reader.read_frame().unwrap();
    let (env, bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(env.domain, DOMAIN_SESSION);
    assert_eq!(env.msg_type, MSG_SESSION_LIST);
    let mut r = BitReader::new(bytes);
    let response = SessionList::unpack(&mut r).unwrap();
    assert_eq!(response.sessions.len(), 1);
    response.sessions.into_iter().next().unwrap()
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
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
        .expect("create debounce store");
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    let addr = start_test_listener_seeded(coordinator);

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

#[test]
fn vnp_create_session_applies_policy_and_returns_stored_status() {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
        .expect("create debounce store");
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    let addr = start_test_listener_seeded(coordinator);

    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut writer = FrameWriter::new(stream.try_clone().unwrap());
    let mut reader = FrameReader::new(BufReader::new(stream));
    let ack = do_handshake(&mut writer, &mut reader);
    assert!(ack.sessions.is_empty());

    let created = do_create_session(
        &mut writer,
        &mut reader,
        IsolationTier::Bare,
        IsolationPolicy::Required,
    );
    assert_eq!(created.isolation.effective, IsolationTier::Bare);
    assert_eq!(created.isolation.requested, IsolationTier::Bare);

    do_attach(&mut writer, created.session_id.0);
    let _ = read_initial_state(&mut reader);
}

/// Test 2: After attaching, the listener sends an InitialState frame.
///
/// Connect + handshake, send AttachSession for a valid session, and verify the
/// response is InitialState (domain=6, type=0x03) with frame_seq=0.
#[test]
fn vnp_attach_receives_initial_state() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let addr = start_test_listener_seeded(coordinator);

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
    let addr = start_test_listener_seeded(coordinator);

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
    let addr = start_test_listener_seeded(coordinator);

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
    let addr = start_test_listener_seeded(coordinator);
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
    let addr = start_test_listener_seeded(coordinator);

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

// --- US1: authentication at the socket (audit A-01/A-08) -----------------

#[test]
fn an_unauthenticated_client_is_refused_and_learns_no_session_names() {
    use std::io::Read;

    // A session with a distinctive name, so a leak is unmistakable.
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()))
        .expect("create debounce store");
    let mut coord = Coordinator::new(PoolConfig::default(), store);
    coord
        .create_session(
            Some("leak-canary-session".to_string()),
            IsolationTier::Bare,
            None,
        )
        .unwrap();
    let coordinator = Arc::new(Mutex::new(coord));

    let (addr, _token) = start_test_listener_with_auth(coordinator);
    let mut stream = TcpStream::connect(&addr).unwrap();

    // Send a Hello with no credential.
    let hello = make_hello_with(None);
    let mut w = BitWriter::new();
    hello.pack(&mut w).unwrap();
    let envelope = make_envelope(0, MSG_HELLO, 0);
    let combined = encode_message(&envelope, &w.finish()).unwrap();
    FrameWriter::new(&mut stream)
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload: combined,
        })
        .unwrap();

    // Read whatever the daemon says before it closes.
    let mut received = Vec::new();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let _ = stream.read_to_end(&mut received);

    // The assertion that matters is byte-level: refusing *after* sending the
    // inventory would satisfy "the connection was refused" while still
    // leaking. Only inspecting the wire catches that.
    let as_text = String::from_utf8_lossy(&received);
    assert!(
        !as_text.contains("leak-canary-session"),
        "an unauthenticated client received a session name: {as_text:?}"
    );
}

#[test]
fn an_authenticated_client_still_completes_the_handshake() {
    let (coordinator, _sid) = make_coordinator_with_session();
    let (addr, token) = start_test_listener_with_auth(coordinator);
    set_test_credential(&token);

    let stream = TcpStream::connect(&addr).unwrap();
    let write_stream = stream.try_clone().unwrap();
    let mut reader = FrameReader::new(BufReader::new(stream));
    let mut writer = FrameWriter::new(write_stream);

    let ack = do_handshake(&mut writer, &mut reader);
    assert_eq!(ack.negotiated_version, 1);
}

#[test]
fn a_connection_that_never_identifies_is_closed() {
    use std::io::Read;

    let (coordinator, _sid) = make_coordinator_with_session();
    let (addr, _token) = start_test_listener_with_auth(coordinator);

    // Connect and send nothing at all.
    let mut stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();

    let started = std::time::Instant::now();
    let mut buf = [0u8; 64];
    // Returns Ok(0) on clean close, or an error; either means the daemon let
    // go. What must not happen is blocking until the test's own timeout.
    let _ = stream.read(&mut buf);
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(25),
        "a silent connection was held for {elapsed:?}; the daemon must bound \
         identification rather than keeping the thread and socket forever"
    );
}

#[test]
fn stalled_connections_do_not_block_a_legitimate_client() {
    let (coordinator, _sid) = make_coordinator_with_session();
    let (addr, token) = start_test_listener_with_auth(coordinator);
    set_test_credential(&token);

    // Hold a batch of connections open without identifying on any of them.
    let mut stalled = Vec::new();
    for _ in 0..16 {
        if let Ok(s) = TcpStream::connect(&addr) {
            stalled.push(s);
        }
    }

    // A real client must still get through promptly.
    let started = std::time::Instant::now();
    let stream = TcpStream::connect(&addr).unwrap();
    let write_stream = stream.try_clone().unwrap();
    let mut reader = FrameReader::new(BufReader::new(stream));
    let mut writer = FrameWriter::new(write_stream);
    let ack = do_handshake(&mut writer, &mut reader);
    let elapsed = started.elapsed();

    assert_eq!(ack.negotiated_version, 1);
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "a legitimate handshake took {elapsed:?} while stalled connections were open"
    );
    drop(stalled);
}

/// Attach over the real wire with a chosen authority.
fn do_attach_with(writer: &mut FrameWriter<TcpStream>, session_id: u32, authority: InputAuthority) {
    let attach = AttachSession {
        session_id: SessionId(session_id),
        authority,
        _unknown: Vec::new(),
    };
    let env = make_envelope(DOMAIN_SESSION, MSG_ATTACH_SESSION, session_id);
    let mut w = BitWriter::new();
    attach.pack(&mut w).unwrap();
    let payload = w.finish();
    let combined = encode_message(&env, &payload).unwrap();
    writer
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload: combined,
        })
        .unwrap();
}

/// The authority a client requests when attaching is honoured.
///
/// This drives the real listener over real TCP. It is the evidence that
/// matters, because `AttachSession.authority` used to be decoded here and
/// discarded: a test that called `AuthorityTracker` directly passed the whole
/// time the request was being thrown away.
#[test]
fn attaching_over_the_wire_applies_the_requested_authority() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let (addr, token) = start_test_listener_with_auth(coordinator.clone());
    set_test_credential(&token);

    let stream = TcpStream::connect(&addr).unwrap();
    let mut writer = FrameWriter::new(stream.try_clone().unwrap());
    let mut reader = FrameReader::new(BufReader::new(stream));
    let _ = do_handshake(&mut writer, &mut reader);
    do_attach_with(&mut writer, session_id, InputAuthority::Exclusive);
    let _ = read_initial_state(&mut reader);

    let holder = coordinator
        .lock()
        .unwrap()
        .input_authority_holder(&SessionId(session_id))
        .unwrap();
    assert!(
        holder.is_some(),
        "a client attaching Exclusive over the wire must hold input authority"
    );
}

/// An observer attaching over the wire does not take the keyboard.
///
/// The inverse of the test above, and the one that fails if the requested
/// authority is ignored and every attacher is treated as a typist.
#[test]
fn attaching_as_an_observer_over_the_wire_does_not_take_authority() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let (addr, token) = start_test_listener_with_auth(coordinator.clone());
    set_test_credential(&token);

    let stream = TcpStream::connect(&addr).unwrap();
    let mut writer = FrameWriter::new(stream.try_clone().unwrap());
    let mut reader = FrameReader::new(BufReader::new(stream));
    let _ = do_handshake(&mut writer, &mut reader);
    do_attach_with(&mut writer, session_id, InputAuthority::Observe);
    let _ = read_initial_state(&mut reader);

    let holder = coordinator
        .lock()
        .unwrap()
        .input_authority_holder(&SessionId(session_id))
        .unwrap();
    assert_eq!(
        holder, None,
        "attaching to observe must not seize input authority"
    );
}

/// A disconnected holder releases the keyboard without a clean detach.
///
/// The socket is dropped rather than a DetachSession being sent, which is
/// what an abrupt client death looks like to the daemon.
#[test]
fn dropping_the_connection_releases_authority() {
    let (coordinator, session_id) = make_coordinator_with_session();
    let (addr, token) = start_test_listener_with_auth(coordinator.clone());
    set_test_credential(&token);

    {
        let stream = TcpStream::connect(&addr).unwrap();
        let mut writer = FrameWriter::new(stream.try_clone().unwrap());
        let mut reader = FrameReader::new(BufReader::new(stream));
        let _ = do_handshake(&mut writer, &mut reader);
        do_attach_with(&mut writer, session_id, InputAuthority::Exclusive);
        let _ = read_initial_state(&mut reader);
        assert!(coordinator
            .lock()
            .unwrap()
            .input_authority_holder(&SessionId(session_id))
            .unwrap()
            .is_some());
        // Socket dropped here: no DetachSession, no goodbye.
    }

    // The listener notices on its next read and unregisters. Poll rather than
    // sleep a fixed amount -- the point is that it happens without a timeout
    // or grace period, not that it happens within any particular millisecond.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let holder = coordinator
            .lock()
            .unwrap()
            .input_authority_holder(&SessionId(session_id))
            .unwrap();
        if holder.is_none() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "an abruptly departed holder must release authority, or the \
             session is stranded until restart"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
