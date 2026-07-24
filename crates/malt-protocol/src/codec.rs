//! VNP domain ID and message type constants, plus the `make_envelope` helper.
//!
//! Domain IDs are 4-bit values matching the architecture spec table.
//! Message type IDs are 7-bit values matching `@type(N)` annotations in .vexil
//! schemas. Every constant below is checked against its schema file in
//! `crates/malt-protocol/tests/codec.rs` — do not add a constant here that
//! doesn't correspond to a real `message` declaration in `schemas/*.vexil`.

pub use crate::envelope::Envelope;

// ── Domain IDs ────────────────────────────────────────────────────────────────
pub const DOMAIN_HANDSHAKE: u8 = 0;
pub const DOMAIN_SHELL: u8 = 1;
pub const DOMAIN_INPUT: u8 = 2;
pub const DOMAIN_MUX: u8 = 3;
pub const DOMAIN_SESSION: u8 = 4;
pub const DOMAIN_TASK: u8 = 5;
pub const DOMAIN_RENDER: u8 = 6;
pub const DOMAIN_SYSTEM: u8 = 7;

// ── Handshake message types (domain = 0, schemas/handshake.vexil) ─────────────
pub const MSG_HELLO: u8 = 0x01;
pub const MSG_HELLO_ACK: u8 = 0x02;
pub const MSG_VERSION_SKEW: u8 = 0x03;

// ── Shell message types (domain = 1, schemas/shell.vexil) ─────────────────────
pub const MSG_COMMAND_STARTED: u8 = 0x01;
pub const MSG_COMMAND_FINISHED: u8 = 0x02;
pub const MSG_PROMPT_READY: u8 = 0x03;
pub const MSG_OUTPUT_CHUNK: u8 = 0x04;

// ── Input message types (domain = 2, schemas/input.vexil) ─────────────────────
pub const MSG_KEY_EVENT: u8 = 0x01;
pub const MSG_MOUSE_EVENT: u8 = 0x02;
pub const MSG_SIGNAL_INPUT: u8 = 0x03;
pub const MSG_RESIZE: u8 = 0x04;

// ── Mux message types (domain = 3, schemas/mux.vexil) ─────────────────────────
pub const MSG_PANE_CREATED: u8 = 0x01;
pub const MSG_PANE_DESTROYED: u8 = 0x02;
pub const MSG_LAYOUT_CHANGED: u8 = 0x03;
pub const MSG_SPLIT_PANE: u8 = 0x04;
pub const MSG_CLOSE_PANE: u8 = 0x05;
pub const MSG_FLOAT_PANE: u8 = 0x06;
pub const MSG_SWAP_PANES: u8 = 0x07;
pub const MSG_FOCUS_DIRECTION: u8 = 0x08;
pub const MSG_RESIZE_SPLIT: u8 = 0x09;
pub const MSG_SAVE_LAYOUT: u8 = 0x0A;
pub const MSG_LOAD_LAYOUT: u8 = 0x0B;

// ── Session message types (domain = 4, schemas/session.vexil) ─────────────────
pub const MSG_CREATE_SESSION: u8 = 0x01;
pub const MSG_ATTACH_SESSION: u8 = 0x02;
pub const MSG_DETACH_SESSION: u8 = 0x03;
pub const MSG_LIST_SESSIONS: u8 = 0x04;
pub const MSG_SESSION_LIST: u8 = 0x05;
pub const MSG_INPUT_CLAIM: u8 = 0x06;
pub const MSG_INPUT_AUTHORITY_CHANGED: u8 = 0x07;

// ── Task message types (domain = 5, schemas/task.vexil) ───────────────────────
pub const MSG_TASK_CREATE: u8 = 0x01;
pub const MSG_TASK_STATUS: u8 = 0x02;
pub const MSG_TASK_COMPLETE: u8 = 0x03;

// ── Render message types (domain = 6, schemas/render.vexil) ───────────────────
pub const MSG_RENDER_BATCH: u8 = 0x01;
pub const MSG_FRAME_ACK: u8 = 0x02;
pub const MSG_INITIAL_STATE: u8 = 0x03;
pub const MSG_SYNC_REQUEST: u8 = 0x04;
pub const MSG_SLOW_CLIENT_DISCONNECT: u8 = 0x05;
pub const MSG_SCROLLBACK_REQUEST: u8 = 0x06;
pub const MSG_SCROLLBACK_RESPONSE: u8 = 0x07;

// ── System message types (domain = 7, schemas/system.vexil) ───────────────────
pub const MSG_STRUCTURED_OUTPUT: u8 = 0x01;
pub const MSG_PLUGIN_EVENT: u8 = 0x02;
pub const MSG_DIAGNOSTIC: u8 = 0x03;
pub const MSG_HEARTBEAT: u8 = 0x04;
pub const MSG_ERROR: u8 = 0x05;

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
