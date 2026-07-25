use malt_daemon::connection::handshake::perform_server_handshake;
use malt_gateway::auth::{AuthScope, TokenStore};
use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_protocol::envelope::{decode_envelope, encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{AuthRejected, Hello, HelloAck};
use std::io::Cursor;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

const WIRE_VERSION: u32 = 1;

fn encode_hello(hello: &Hello) -> Vec<u8> {
    let envelope = Envelope {
        wire_version: 0,
        domain: 0,
        msg_type: 0x01,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let mut w = BitWriter::new();
    hello.pack(&mut w).unwrap();
    let msg_bytes = w.finish();
    let combined = encode_message(&envelope, &msg_bytes).unwrap();
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    buf
}

fn decode_hello_ack(data: &[u8]) -> HelloAck {
    let mut reader = FrameReader::new(Cursor::new(data));
    let frame = reader.read_frame().unwrap();
    let (envelope, msg_bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(envelope.domain, 0);
    assert_eq!(envelope.msg_type, 0x02);
    let mut r = BitReader::new(msg_bytes);
    HelloAck::unpack(&mut r).unwrap()
}

/// A store with one valid Admin credential, for tests whose subject is not
/// authentication itself.
fn store_with_token() -> (TokenStore, String) {
    let store = TokenStore::new();
    let token = store.generate_token(AuthScope::Admin);
    (store, token)
}

#[test]
fn successful_handshake() {
    let (store, token) = store_with_token();
    let hello = Hello {
        version: WIRE_VERSION,
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
        credential: Some(token.clone()),
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, Vec::new).unwrap();

    assert_eq!(result.client_type, "test");
    assert!(matches!(
        result.capabilities.color_depth,
        ColorDepth::TrueColor
    ));

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.negotiated_version, WIRE_VERSION);
}

#[test]
fn version_skew_rejects_incompatible() {
    let (store, token) = store_with_token();
    let hello = Hello {
        version: 0,
        client_type: "old".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::None,
            unicode: UnicodeLevel::None,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: false,
            max_fps: 30,
            _unknown: Vec::new(),
        },
        credential: Some(token.clone()),
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, Vec::new);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("version skew"), "got: {err}");
}

#[test]
fn handshake_includes_session_list() {
    let (store, token) = store_with_token();
    let sessions = vec![malt_protocol::common::SessionInfo {
        session_id: malt_protocol::common::SessionId(1),
        name: Some("main".to_string()),
        pane_count: 2,
        isolation: malt_protocol::common::IsolationTier::Bare,
        state: malt_protocol::common::SessionState::Active,
        _unknown: Vec::new(),
    }];

    let hello = Hello {
        version: WIRE_VERSION,
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
        credential: Some(token.clone()),
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    perform_server_handshake(&mut input, &mut output, &store, || sessions.clone()).unwrap();

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.sessions.len(), 1);
    assert_eq!(ack.sessions[0].name, Some("main".to_string()));
}

// --- Authentication (spec 005 US1, audit A-01) ---------------------------

fn hello_with(credential: Option<String>) -> Hello {
    Hello {
        version: WIRE_VERSION,
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

fn a_session_named(name: &str) -> malt_protocol::common::SessionInfo {
    malt_protocol::common::SessionInfo {
        session_id: malt_protocol::common::SessionId(1),
        name: Some(name.to_string()),
        pane_count: 1,
        isolation: malt_protocol::common::IsolationTier::Bare,
        state: malt_protocol::common::SessionState::Active,
        _unknown: Vec::new(),
    }
}

#[test]
fn a_missing_credential_is_refused() {
    let (store, _token) = store_with_token();
    let mut input = Cursor::new(encode_hello(&hello_with(None)));
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, Vec::new);
    assert!(result.is_err(), "an unauthenticated client must be refused");
}

#[test]
fn an_invalid_credential_is_refused() {
    let (store, _token) = store_with_token();
    let mut input = Cursor::new(encode_hello(&hello_with(Some(
        "malt_not_a_real_token".to_string(),
    ))));
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, Vec::new);
    assert!(result.is_err());

    // The refusal must be an AuthRejected frame, not a HelloAck.
    let mut reader = FrameReader::new(Cursor::new(output.as_slice()));
    let frame = reader.read_frame().unwrap();
    let (envelope, msg_bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(envelope.msg_type, 0x04, "expected AuthRejected");
    let mut r = BitReader::new(msg_bytes);
    let rejected = AuthRejected::unpack(&mut r).unwrap();
    assert!(!rejected.reason.is_empty());
}

#[test]
fn a_refused_client_learns_nothing_about_sessions() {
    // The heart of FR-002, and the reason the inventory is a closure rather
    // than a parameter: it used to be computed by the caller and handed in,
    // so it existed before anything was checked. Here the closure panics if
    // called, proving the daemon never even asks what sessions exist for a
    // caller it has not authenticated -- and the byte-level check proves no
    // session name reached the wire.
    let (store, _token) = store_with_token();
    let mut input = Cursor::new(encode_hello(&hello_with(None)));
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, || {
        panic!("the session inventory must not be consulted for an unauthenticated client")
    });
    assert!(result.is_err());
    assert!(
        !String::from_utf8_lossy(&output).contains("super-secret-session"),
        "no session name may appear in what an unauthenticated client receives"
    );
}

#[test]
fn an_authenticated_client_receives_the_inventory() {
    // The other half: authentication must not break the legitimate path.
    let (store, token) = store_with_token();
    let mut input = Cursor::new(encode_hello(&hello_with(Some(token))));
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, || {
        vec![a_session_named("super-secret-session")]
    })
    .unwrap();
    assert_eq!(result.scope, AuthScope::Admin);

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.sessions.len(), 1);
    assert_eq!(
        ack.sessions[0].name,
        Some("super-secret-session".to_string())
    );
}

#[test]
fn the_resolved_scope_follows_the_credential() {
    let store = TokenStore::new();
    let read_token = store.generate_token(AuthScope::Read);
    let mut input = Cursor::new(encode_hello(&hello_with(Some(read_token))));
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &store, Vec::new).unwrap();
    assert_eq!(
        result.scope,
        AuthScope::Read,
        "a connection must carry the scope its credential grants, not a default"
    );
}
