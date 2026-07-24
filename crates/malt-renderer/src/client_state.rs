// Per-client renderer state — capabilities, sequence numbers, last-sent frame.

use malt_protocol::common::ClientCapabilities;

const LAGGING_THRESHOLD: u64 = 30;
const SHED_TIMEOUT_MS: u64 = 10_000;

/// Whether a client is keeping up with frame production.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckStatus {
    /// Client is acknowledging frames promptly.
    Ok,
    /// Client has more than [`LAGGING_THRESHOLD`] unacknowledged frames.
    Lagging,
}

/// Current wall-clock time in milliseconds since the Unix epoch, or 0 if the
/// system clock is unavailable.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Tracks per-client frame sequencing and ack state for flow control.
pub struct ClientState {
    id: u64,
    capabilities: ClientCapabilities,
    frame_seq: u64,
    last_acked_seq: u64,
    last_ack_time_ms: u64,
}

impl ClientState {
    /// Create a new client state with the given id and capabilities.
    ///
    /// `last_ack_time_ms` starts at the current time (not 0) so a
    /// freshly-registered client isn't immediately eligible for shedding
    /// before it has had any chance to send a first ack.
    pub fn new(id: u64, capabilities: ClientCapabilities) -> Self {
        Self {
            id,
            capabilities,
            frame_seq: 0,
            last_acked_seq: 0,
            last_ack_time_ms: now_ms(),
        }
    }

    /// Returns the client identifier.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the client's declared capabilities.
    pub fn capabilities(&self) -> &ClientCapabilities {
        &self.capabilities
    }

    /// Returns the current frame sequence number.
    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }

    /// Increments the frame sequence and returns the new value.
    pub fn advance_seq(&mut self) -> u64 {
        self.frame_seq += 1;
        self.frame_seq
    }

    /// Returns the number of frames sent but not yet acknowledged.
    pub fn unacked_count(&self) -> u64 {
        self.frame_seq - self.last_acked_seq
    }

    /// Records an ack from the client. `last_acked_seq` only advances if
    /// `seq` is greater than the previously recorded ack, but any ack
    /// (including a duplicate) is evidence the client is alive, so
    /// `last_ack_time_ms` always resets.
    pub fn ack(&mut self, seq: u64) {
        if seq > self.last_acked_seq {
            self.last_acked_seq = seq;
        }
        self.last_ack_time_ms = now_ms();
    }

    /// Checks whether the client is keeping up or lagging behind.
    pub fn check_status(&self) -> AckStatus {
        if self.unacked_count() >= LAGGING_THRESHOLD {
            AckStatus::Lagging
        } else {
            AckStatus::Ok
        }
    }

    /// Returns `true` if the client should be shed (disconnected) due to
    /// prolonged unresponsiveness: has unacked frames and the elapsed time
    /// since last ack meets or exceeds the shed timeout.
    pub fn should_shed(&self, elapsed_since_last_ack_ms: u64) -> bool {
        self.unacked_count() > 0 && elapsed_since_last_ack_ms >= SHED_TIMEOUT_MS
    }

    /// Whether the client should be shed right now, given the current
    /// wall-clock time. Convenience wrapper around `should_shed` for
    /// non-test callers; tests should keep using `should_shed` directly
    /// with an injected elapsed time for determinism.
    pub fn should_shed_at(&self, now_ms: u64) -> bool {
        self.should_shed(now_ms.saturating_sub(self.last_ack_time_ms))
    }
}
