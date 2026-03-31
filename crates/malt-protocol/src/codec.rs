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
pub const MSG_ERROR:     u8 = 0x03;

// ── Shell message types (domain = 1) ──────────────────────────────────────────
pub const MSG_COMMAND_OUTPUT: u8 = 0x01;
pub const MSG_SHELL_READY:    u8 = 0x02;
pub const MSG_PROMPT:         u8 = 0x03;

// ── Input message types (domain = 2) ──────────────────────────────────────────
pub const MSG_KEY_EVENT:   u8 = 0x01;
pub const MSG_MOUSE_EVENT: u8 = 0x02;
pub const MSG_PASTE:       u8 = 0x03;
pub const MSG_RESIZE:      u8 = 0x04;

// ── Mux message types (domain = 3) ────────────────────────────────────────────
pub const MSG_SPLIT_PANE:  u8 = 0x01;
pub const MSG_CLOSE_PANE:  u8 = 0x02;
pub const MSG_RESIZE_PANE: u8 = 0x03;
pub const MSG_FOCUS_PANE:  u8 = 0x04;

// ── Session message types (domain = 4) ────────────────────────────────────────
pub const MSG_CREATE_SESSION:  u8 = 0x01;
pub const MSG_ATTACH_SESSION:  u8 = 0x02;
pub const MSG_DETACH_SESSION:  u8 = 0x03;
pub const MSG_DESTROY_SESSION: u8 = 0x04;

// ── Task message types (domain = 5) ───────────────────────────────────────────
pub const MSG_TASK_START:    u8 = 0x01;
pub const MSG_TASK_PROGRESS: u8 = 0x02;
pub const MSG_TASK_COMPLETE: u8 = 0x03;
pub const MSG_TASK_FAIL:     u8 = 0x04;

// ── Render message types (domain = 6) ─────────────────────────────────────────
pub const MSG_RENDER_BATCH:   u8 = 0x01;
pub const MSG_FRAME_ACK:      u8 = 0x02;
pub const MSG_INITIAL_STATE:  u8 = 0x03;

// ── System message types (domain = 7) ─────────────────────────────────────────
pub const MSG_PING:     u8 = 0x01;
pub const MSG_PONG:     u8 = 0x02;
pub const MSG_SHUTDOWN: u8 = 0x03;

// ── Envelope constructor ───────────────────────────────────────────────────────

/// Build a VNP Envelope with the given domain, message type, and target session.
///
/// `wire_version` is always 0. `timestamp` is the current wall-clock time in
/// milliseconds since the Unix epoch, or 0 if the system clock is unavailable.
/// `msg_id` is `None`.
pub fn make_envelope(domain: u8, msg_type: u8, session_id: u32) -> Envelope {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Envelope {
        wire_version: 0,
        domain,
        msg_type,
        session_id,
        timestamp,
        msg_id: None,
        _unknown: Vec::new(),
    }
}
