//! Envelope encode/decode helpers.
//!
//! Re-exports the Vexil-generated `Envelope` type and adds convenience
//! functions for the common encode/decode pattern.

pub use crate::envelope_generated::Envelope;

use vexil_runtime::{BitReader, BitWriter, DecodeError, Pack, Unpack};

/// Encode an envelope + message payload into bytes suitable for framing.
///
/// Returns an error if envelope encoding fails (should not happen with valid fields).
pub fn encode_message(envelope: &Envelope, payload: &[u8]) -> Result<Vec<u8>, vexil_runtime::EncodeError> {
    let mut w = BitWriter::new();
    envelope.pack(&mut w)?;
    let mut bytes = w.finish();
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Encode just the envelope, returning the encoded bytes.
pub fn encode_envelope(envelope: &Envelope) -> Result<Vec<u8>, vexil_runtime::EncodeError> {
    let mut w = BitWriter::new();
    envelope.pack(&mut w)?;
    Ok(w.finish())
}

/// Decode an envelope from a frame payload buffer.
///
/// Since `BitReader` doesn't expose a position accessor, this encodes a
/// default envelope to determine the envelope byte size, then splits the
/// data accordingly. This works because the Vexil bitpack encoding has a
/// fixed size for a given field layout (the optional msg_id is the only
/// variable part).
///
/// Returns the envelope and the remaining bytes (message body).
pub fn decode_envelope(data: &[u8]) -> Result<(Envelope, &[u8]), DecodeError> {
    let mut r = BitReader::new(data);
    let envelope = Envelope::unpack(&mut r)?;

    // Re-encode to determine consumed byte count
    let envelope_bytes = encode_envelope(&envelope)
        .map_err(|_| DecodeError::UnexpectedEof)?;
    let consumed = envelope_bytes.len();

    Ok((envelope, &data[consumed..]))
}
