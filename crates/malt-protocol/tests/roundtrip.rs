//! Roundtrip encode/decode tests for representative message types.

use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

#[test]
fn shell_command_started_roundtrip() {
    let msg = malt_protocol::shell::CommandStarted {
        command_id: 42,
        cmd: "cargo build --release".to_string(),
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::shell::CommandStarted::unpack(&mut r).unwrap();
    assert_eq!(msg.command_id, decoded.command_id);
    assert_eq!(msg.cmd, decoded.cmd);
}

#[test]
fn common_pane_id_roundtrip() {
    let id = malt_protocol::common::PaneId(12345);

    let mut w = BitWriter::new();
    id.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::common::PaneId::unpack(&mut r).unwrap();
    assert_eq!(id.0, decoded.0);
}

#[test]
fn common_isolation_tier_roundtrip() {
    let tier = malt_protocol::common::IsolationTier::Contained;

    let mut w = BitWriter::new();
    tier.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::common::IsolationTier::unpack(&mut r).unwrap();
    assert_eq!(tier, decoded);
}

#[test]
fn input_key_event_roundtrip() {
    let msg = malt_protocol::input::KeyEvent {
        key: malt_protocol::input::KeyValue::Char { codepoint: 0x41 },
        modifiers: malt_protocol::common::KeyModifiers::empty(),
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::input::KeyEvent::unpack(&mut r).unwrap();
    assert_eq!(msg.key, decoded.key);
    assert_eq!(msg.modifiers, decoded.modifiers);
}

#[test]
fn session_create_roundtrip() {
    let msg = malt_protocol::session::CreateSession {
        name: Some("dev".to_string()),
        isolation: malt_protocol::common::IsolationTier::Restricted,
        group: None,
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::session::CreateSession::unpack(&mut r).unwrap();
    assert_eq!(msg.name, decoded.name);
    assert_eq!(msg.isolation, decoded.isolation);
    assert_eq!(msg.group, decoded.group);
}

#[test]
fn shell_output_chunk_with_tag_roundtrip() {
    let msg = malt_protocol::shell::OutputChunk {
        data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        command_tag: Some("cargo test".to_string()),
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::shell::OutputChunk::unpack(&mut r).unwrap();
    assert_eq!(msg.data, decoded.data);
    assert_eq!(msg.command_tag, decoded.command_tag);
}
