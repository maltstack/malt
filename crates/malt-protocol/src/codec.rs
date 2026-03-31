//! VNP domain ID and message type constants, plus the `make_envelope` helper.
//!
//! Domain IDs are 4-bit values matching the architecture spec table.
//! Message type IDs are 7-bit values matching `@type(N)` annotations in .vexil schemas.

pub use crate::envelope::Envelope;

// ── Domain IDs ────────────────────────────────────────────────────────────────
pub const DOMAIN_HANDSHAKE: u8 = 0;
pub const DOMAIN_SHELL:     u8 = 1;
pub const DOMAIN_INPUT:     u8 = 2;
pub const DOMAIN_MUX:       u8 = 3;
pub const DOMAIN_SESSION:   u8 = 4;
pub const DOMAIN_TASK:      u8 = 5;
pub const DOMAIN_RENDER:    u8 = 6;
pub const DOMAIN_SYSTEM:    u8 = 7;

// ── Handshake message types (domain = 0) ──────────────────────────────────────
pub const MSG_HELLO:     u8 = 0x01;
pub const MSG_HELLO_ACK: u8 = 0x02;

// ── Input message types (domain = 2) ──────────────────────────────────────────
pub const MSG_KEY_EVENT:    u8 = 0x01;
pub const MSG_MOUSE_EVENT:  u8 = 0x02;
pub const MSG_SIGNAL_INPUT: u8 = 0x03;
pub const MSG_RESIZE:       u8 = 0x04;

// ── Session message types (domain = 4) ────────────────────────────────────────
pub const MSG_CREATE_SESSION:  u8 = 0x01;
pub const MSG_ATTACH_SESSION:  u8 = 0x02;
pub const MSG_DETACH_SESSION:  u8 = 0x03;
pub const MSG_LIST_SESSIONS:   u8 = 0x04;
pub const MSG_SESSION_LIST:    u8 = 0x05;

// ── Render message types (domain = 6) ─────────────────────────────────────────
pub const MSG_RENDER_BATCH:             u8 = 0x01;
pub const MSG_FRAME_ACK:                u8 = 0x02;
pub const MSG_INITIAL_STATE:            u8 = 0x03;
pub const MSG_SYNC_REQUEST:             u8 = 0x04;
pub const MSG_SLOW_CLIENT_DISCONNECT:   u8 = 0x05;

// ── Envelope constructor ───────────────────────────────────────────────────────

/// Build a VNP Envelope with the given domain, message type, and target session.
///
/// `wire_version` is always 0. `timestamp` is left at 0 (acceptable for
/// single-host operation). `msg_id` is `None`.
pub fn make_envelope(domain: u8, msg_type: u8, session_id: u32) -> Envelope {
    Envelope {
        wire_version: 0,
        domain,
        msg_type,
        session_id,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    }
}
