use malt_protocol::envelope::{decode_envelope, encode_envelope, encode_message, Envelope};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

#[test]
fn envelope_roundtrip() {
    let env = Envelope {
        wire_version: 1,
        domain: 1,
        msg_type: 42,
        session_id: 100,
        timestamp: 1_000_000,
        msg_id: Some(99),
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    env.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = Envelope::unpack(&mut r).unwrap();
    assert_eq!(env.wire_version, decoded.wire_version);
    assert_eq!(env.domain, decoded.domain);
    assert_eq!(env.msg_type, decoded.msg_type);
    assert_eq!(env.session_id, decoded.session_id);
    assert_eq!(env.timestamp, decoded.timestamp);
    assert_eq!(env.msg_id, decoded.msg_id);
}

#[test]
fn envelope_without_msg_id() {
    let env = Envelope {
        wire_version: 0,
        domain: 7,
        msg_type: 127,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };

    let mut w = BitWriter::new();
    env.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = Envelope::unpack(&mut r).unwrap();
    assert_eq!(decoded.msg_id, None);
}

#[test]
fn envelope_deterministic_encoding() {
    let env = Envelope {
        wire_version: 1,
        domain: 2,
        msg_type: 5,
        session_id: 42,
        timestamp: 100,
        msg_id: None,
        _unknown: Vec::new(),
    };

    let bytes1 = encode_envelope(&env).unwrap();
    let bytes2 = encode_envelope(&env).unwrap();
    assert_eq!(bytes1, bytes2, "deterministic encoding");
}

#[test]
fn encode_message_appends_payload() {
    let env = Envelope {
        wire_version: 1,
        domain: 1,
        msg_type: 1,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };

    let payload = b"hello";
    let combined = encode_message(&env, payload).unwrap();
    let env_bytes = encode_envelope(&env).unwrap();

    assert_eq!(&combined[..env_bytes.len()], &env_bytes[..]);
    assert_eq!(&combined[env_bytes.len()..], payload);
}

#[test]
fn decode_envelope_splits_payload() {
    let env = Envelope {
        wire_version: 1,
        domain: 1,
        msg_type: 1,
        session_id: 42,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };

    let payload = b"world";
    let combined = encode_message(&env, payload).unwrap();

    let (decoded_env, remaining) = decode_envelope(&combined).unwrap();
    assert_eq!(decoded_env.session_id, 42);
    assert_eq!(remaining, payload);
}

#[test]
fn golden_bytes_first_byte() {
    // wire_version=1 (4 bits: 0001), domain=2 (4 bits: 0010)
    // LSB-first packing: low nibble = wire_version, high nibble = domain
    // byte 0 = 0b0010_0001 = 0x21
    let env = Envelope {
        wire_version: 1,
        domain: 2,
        msg_type: 5,
        session_id: 42,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };

    let bytes = encode_envelope(&env).unwrap();
    assert_eq!(bytes[0], 0x21, "byte 0: wire_version(1) | domain(2) << 4");
}

/// Full golden byte vector for an envelope without msg_id.
///
/// Pins the complete bitpack encoding of the VNP Envelope type.
/// If this test fails after a vexil-runtime or schema change:
///   1. Verify the new encoding is intentional (not a regression)
///   2. Update this vector AND bump the wire_version in Hello/HelloAck
///   3. Add a migration note to docs/baseline-audit-2026-03-31.md
#[test]
fn golden_full_encoding_without_msg_id() {
    let env = Envelope {
        wire_version: 1,
        domain: 2,
        msg_type: 5,
        session_id: 42,
        timestamp: 0x0000_AABB_CCDD_EEFFu64,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let bytes = encode_envelope(&env).unwrap();
    let expected: &[u8] = &[33, 5, 42, 0, 0, 0, 255, 238, 221, 204, 187, 170, 0];
    assert_eq!(
        bytes, expected,
        "Envelope wire encoding changed — see test doc comment for update procedure"
    );
}

/// Full golden byte vector for an envelope with msg_id present.
///
/// Same invariant as golden_full_encoding_without_msg_id.
#[test]
fn golden_full_encoding_with_msg_id() {
    let env = Envelope {
        wire_version: 1,
        domain: 2,
        msg_type: 5,
        session_id: 42,
        timestamp: 0x0000_AABB_CCDD_EEFFu64,
        msg_id: Some(0xDEAD_BEEF),
        _unknown: Vec::new(),
    };
    let bytes = encode_envelope(&env).unwrap();
    let expected: &[u8] = &[
        33, 5, 42, 0, 0, 0, 255, 238, 221, 204, 187, 170, 1, 239, 190, 173, 222,
    ];
    assert_eq!(
        bytes, expected,
        "Envelope wire encoding changed — see test doc comment for update procedure"
    );
}
