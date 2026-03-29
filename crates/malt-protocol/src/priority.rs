//! Bus priority mapping for VNP message types.
//!
//! Maps (domain, msg_type) from the envelope to a bus priority class.
//! This is a workaround for vexil-lang Gap 2 (no custom annotations).
//! When vexil-lang ships custom annotation support, this hand-written
//! table will be replaced by codegen-emitted constants.

/// Bus priority class for VNP messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Priority {
    /// Resize, Signal — inline delivery, never dropped.
    Critical,
    /// CommandStarted, StructuredOutput — never evicted.
    Reliable,
    /// RenderBatch — oldest overwritten by newer.
    High,
    /// OutputChunk, FrameAck — oldest dropped when full.
    Normal,
    /// Heartbeat, PluginEvent, Diagnostic — oldest dropped when full.
    Low,
}

/// Domain IDs from the VNP schema design spec.
mod domain {
    pub const HANDSHAKE: u8 = 0;
    pub const SHELL: u8 = 1;
    pub const INPUT: u8 = 2;
    pub const MUX: u8 = 3;
    pub const SESSION: u8 = 4;
    pub const TASK: u8 = 5;
    pub const RENDER: u8 = 6;
    pub const SYSTEM: u8 = 7;
}

/// Look up the bus priority for a message by its envelope domain and type.
///
/// Returns `None` for unknown domain/type combinations.
pub const fn priority_of(domain_id: u8, msg_type: u8) -> Option<Priority> {
    match (domain_id, msg_type) {
        // Handshake: all Reliable
        (domain::HANDSHAKE, 0x01..=0x03) => Some(Priority::Reliable),

        // Shell: CommandStarted, CommandFinished, PromptReady = Reliable; OutputChunk = Normal
        (domain::SHELL, 0x01..=0x03) => Some(Priority::Reliable),
        (domain::SHELL, 0x04) => Some(Priority::Normal),

        // Input: all Critical
        (domain::INPUT, 0x01..=0x04) => Some(Priority::Critical),

        // Mux: all Reliable
        (domain::MUX, 0x01..=0x0B) => Some(Priority::Reliable),

        // Session: all Reliable
        (domain::SESSION, 0x01..=0x07) => Some(Priority::Reliable),

        // Task: all Reliable
        (domain::TASK, 0x01..=0x03) => Some(Priority::Reliable),

        // Render: RenderBatch = High, FrameAck = Normal,
        //         InitialState/SyncRequest/SlowClientDisconnect = Reliable,
        //         ScrollbackRequest/Response = Normal
        (domain::RENDER, 0x01) => Some(Priority::High),
        (domain::RENDER, 0x02) => Some(Priority::Normal),
        (domain::RENDER, 0x03..=0x05) => Some(Priority::Reliable),
        (domain::RENDER, 0x06..=0x07) => Some(Priority::Normal),

        // System: StructuredOutput = Reliable, PluginEvent = Low, Diagnostic = Low,
        //         Heartbeat = Low, Error = Reliable
        (domain::SYSTEM, 0x01) => Some(Priority::Reliable),
        (domain::SYSTEM, 0x02..=0x04) => Some(Priority::Low),
        (domain::SYSTEM, 0x05) => Some(Priority::Reliable),

        _ => None,
    }
}
