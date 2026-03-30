use crate::DaemonError;
use malt_protocol::common::{ClientCapabilities, SessionInfo};
use malt_protocol::envelope::{decode_envelope, encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck, VersionSkew};
use std::io::{Read, Write};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

const SUPPORTED_WIRE_VERSION: u32 = 1;

/// Result of a successful handshake.
#[derive(Debug)]
pub struct HandshakeResult {
    pub client_type: String,
    pub capabilities: ClientCapabilities,
    pub negotiated_version: u32,
}

/// Perform the server side of the VNP handshake.
///
/// 1. Read Hello from client
/// 2. Validate wire version
/// 3. Send HelloAck with negotiated version and session list
pub fn perform_server_handshake<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    sessions: &[SessionInfo],
) -> Result<HandshakeResult, DaemonError> {
    // Read Hello frame
    let mut frame_reader = FrameReader::new(reader);
    let frame = frame_reader.read_frame()?;
    let (envelope, msg_bytes) = decode_envelope(&frame.payload)
        .map_err(|e| DaemonError::HandshakeFailed(format!("envelope decode: {e}")))?;

    if envelope.domain != 0 || envelope.msg_type != 0x01 {
        return Err(DaemonError::HandshakeFailed(format!(
            "expected Hello (domain=0, type=1), got domain={}, type={}",
            envelope.domain, envelope.msg_type
        )));
    }

    let mut bit_reader = BitReader::new(msg_bytes);
    let hello = Hello::unpack(&mut bit_reader)
        .map_err(|e| DaemonError::HandshakeFailed(format!("Hello decode: {e}")))?;

    // Version negotiation: use the lower of client and server version
    let negotiated = hello.version.min(SUPPORTED_WIRE_VERSION);
    if negotiated == 0 {
        // No compatible version — send VersionSkew and disconnect
        let skew = VersionSkew {
            expected_min: 1,
            expected_max: SUPPORTED_WIRE_VERSION,
            client_version: hello.version,
            reason: "no compatible wire version".to_string(),
            _unknown: Vec::new(),
        };
        let skew_envelope = Envelope {
            wire_version: 0,
            domain: 0,
            msg_type: 0x03,
            session_id: 0,
            timestamp: 0,
            msg_id: None,
            _unknown: Vec::new(),
        };
        let mut w = BitWriter::new();
        skew.pack(&mut w)
            .map_err(|e| DaemonError::HandshakeFailed(format!("VersionSkew encode: {e}")))?;
        let skew_bytes = w.finish();
        let combined = encode_message(&skew_envelope, &skew_bytes);
        let frame = Frame {
            flags: FrameFlags::new(),
            payload: combined,
        };
        FrameWriter::new(writer).write_frame(&frame)?;
        return Err(DaemonError::HandshakeFailed(
            "version skew — no compatible version".to_string(),
        ));
    }

    // Send HelloAck
    let ack = HelloAck {
        negotiated_version: negotiated,
        sessions: sessions.to_vec(),
        start_time_offset: 0,
        _unknown: Vec::new(),
    };
    let ack_envelope = Envelope {
        wire_version: 0,
        domain: 0,
        msg_type: 0x02,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let mut w = BitWriter::new();
    ack.pack(&mut w)
        .map_err(|e| DaemonError::HandshakeFailed(format!("HelloAck encode: {e}")))?;
    let ack_bytes = w.finish();
    let combined = encode_message(&ack_envelope, &ack_bytes);
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    FrameWriter::new(writer).write_frame(&frame)?;

    Ok(HandshakeResult {
        client_type: hello.client_type,
        capabilities: hello.capabilities,
        negotiated_version: negotiated,
    })
}
