use crate::DaemonError;
use malt_gateway::auth::{AuthScope, TokenStore};
use malt_protocol::common::{ClientCapabilities, SessionInfo};
use malt_protocol::envelope::{decode_envelope, encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{AuthRejected, Hello, HelloAck, VersionSkew};
use std::io::{Read, Write};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

const SUPPORTED_WIRE_VERSION: u32 = 1;

/// Result of a successful handshake.
#[derive(Debug)]
pub struct HandshakeResult {
    pub client_type: String,
    pub capabilities: ClientCapabilities,
    pub negotiated_version: u32,
    /// Scope resolved from the presented credential. Every session-affecting
    /// operation on this connection is checked against it.
    pub scope: AuthScope,
}

/// Perform the server side of the VNP handshake.
///
/// 1. Read Hello from the client
/// 2. Validate the wire version
/// 3. **Authenticate the credential**
/// 4. Only then obtain the session inventory and send HelloAck
///
/// `sessions` is a closure rather than a slice on purpose. The inventory used
/// to be computed by the caller and handed in, which meant it existed before
/// anything had been checked and went out inside `HelloAck` unconditionally.
/// Taking a `FnOnce` makes the ordering structural: there is no way to obtain
/// the inventory except by calling it, and it is only called after the
/// credential validates. An unauthenticated caller cannot learn what sessions
/// exist, or whether any do.
pub fn perform_server_handshake<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    token_store: &TokenStore,
    sessions: F,
) -> Result<HandshakeResult, DaemonError>
where
    R: Read,
    W: Write,
    F: FnOnce() -> Vec<SessionInfo>,
{
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
        let combined = encode_message(&skew_envelope, &skew_bytes)
            .map_err(|e| DaemonError::HandshakeFailed(format!("encode: {e}")))?;
        let frame = Frame {
            flags: FrameFlags::new(),
            payload: combined,
        };
        FrameWriter::new(writer).write_frame(&frame)?;
        return Err(DaemonError::HandshakeFailed(
            "version skew — no compatible version".to_string(),
        ));
    }

    // Authenticate before anything about this daemon's state is disclosed.
    let scope = match hello
        .credential
        .as_deref()
        .and_then(|c| token_store.validate(c))
    {
        Some(scope) => scope,
        None => {
            let rejected = AuthRejected {
                // Deliberately uninformative: distinguishing "absent" from
                // "wrong" tells an unauthenticated caller how far it got.
                reason: "authentication required".to_string(),
                _unknown: Vec::new(),
            };
            let envelope = Envelope {
                wire_version: 0,
                domain: 0,
                msg_type: 0x04,
                session_id: 0,
                timestamp: 0,
                msg_id: None,
                _unknown: Vec::new(),
            };
            let mut w = BitWriter::new();
            rejected
                .pack(&mut w)
                .map_err(|e| DaemonError::HandshakeFailed(format!("AuthRejected encode: {e}")))?;
            let combined = encode_message(&envelope, &w.finish())
                .map_err(|e| DaemonError::HandshakeFailed(format!("encode: {e}")))?;
            FrameWriter::new(writer).write_frame(&Frame {
                flags: FrameFlags::new(),
                payload: combined,
            })?;
            return Err(DaemonError::HandshakeFailed(
                "authentication failed".to_string(),
            ));
        }
    };

    // Only now is it safe to look at what sessions exist.
    let ack = HelloAck {
        negotiated_version: negotiated,
        sessions: sessions(),
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
    let combined = encode_message(&ack_envelope, &ack_bytes)
        .map_err(|e| DaemonError::HandshakeFailed(format!("encode: {e}")))?;
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    FrameWriter::new(writer).write_frame(&frame)?;

    Ok(HandshakeResult {
        client_type: hello.client_type,
        capabilities: hello.capabilities,
        negotiated_version: negotiated,
        scope,
    })
}
