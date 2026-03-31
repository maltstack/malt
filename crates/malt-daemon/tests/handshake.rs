use malt_daemon::connection::handshake::perform_server_handshake;
use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck};
use malt_protocol::envelope::{encode_message, decode_envelope, Envelope};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};
use std::io::Cursor;

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

#[test]
fn successful_handshake() {
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
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &[]).unwrap();

    assert_eq!(result.client_type, "test");
    assert!(matches!(result.capabilities.color_depth, ColorDepth::TrueColor));

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.negotiated_version, WIRE_VERSION);
}

#[test]
fn version_skew_rejects_incompatible() {
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
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &[]);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("version skew"), "got: {err}");
}

#[test]
fn handshake_includes_session_list() {
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
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    perform_server_handshake(&mut input, &mut output, &sessions).unwrap();

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.sessions.len(), 1);
    assert_eq!(ack.sessions[0].name, Some("main".to_string()));
}
