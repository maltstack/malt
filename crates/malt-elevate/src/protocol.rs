//! Wire protocol helpers for the elevate IPC channel.
//!
//! Uses VNP framing (`malt_protocol::framing`) for transport. The elevate
//! protocol is a restricted subset of VNP — only the 6 message types from
//! `elevate.vexil` are valid on this channel.
//!
//! Phase 2: this module provides the message type tags and serialization
//! helpers. Full encode/decode via vexilc-generated Pack/Unpack traits
//! will replace the hand-written serialization when the codegen pipeline
//! is integrated.

/// Message type tags for the elevate protocol.
///
/// Each message on the elevate IPC channel is framed with a 1-byte type tag
/// as the first byte of the payload, followed by the serialized message body.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTag {
    /// `ElevateHello` — daemon to helper, initiates connection.
    Hello = 0,
    /// `ElevateHelloAck` — helper to daemon, authentication result.
    HelloAck = 1,
    /// `ElevateRequestEnvelope` — daemon to helper, operation request.
    Request = 2,
    /// `ElevateResponse` — helper to daemon, operation result.
    Response = 3,
    /// `ElevateShutdown` — either direction, graceful shutdown.
    Shutdown = 4,
}

impl MessageTag {
    /// Parse a byte into a `MessageTag`, returning `None` for unknown tags.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Hello),
            1 => Some(Self::HelloAck),
            2 => Some(Self::Request),
            3 => Some(Self::Response),
            4 => Some(Self::Shutdown),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_round_trip() {
        for tag in [
            MessageTag::Hello,
            MessageTag::HelloAck,
            MessageTag::Request,
            MessageTag::Response,
            MessageTag::Shutdown,
        ] {
            let byte = tag as u8;
            assert_eq!(MessageTag::from_byte(byte), Some(tag));
        }
    }

    #[test]
    fn unknown_tag_returns_none() {
        assert_eq!(MessageTag::from_byte(5), None);
        assert_eq!(MessageTag::from_byte(255), None);
    }
}
