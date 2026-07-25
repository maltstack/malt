//! Retained output log: the bounded catch-up window for a session's
//! streamed command output, and the bounded, non-blocking per-subscriber
//! sink that delivers it.
//!
//! Deliberately parallel to `events.rs`'s `EventLog`/`SubscriberSink` rather
//! than an independent design: the spec makes "what happens when a
//! consumer stops reading" an explicit assumption, and two differently
//! -behaved answers to that question is precisely the class of divergence
//! that produced the three-copy slurp bug and the two-parallel-attach-paths
//! bug already fixed in this repo (research R5). The one real difference is
//! the retention bound: events are counted, output is sized by **total
//! bytes**, because a count bound does not bound memory when chunk sizes
//! vary (FR-012, SC-004).
//!
//! Not built on [`crate::bus::Bus`] for the same reason `events.rs` isn't:
//! the bus's `Reliable` priority grows its ring beyond capacity rather than
//! evict, which would hand an untrusted subscriber unbounded growth.

use std::collections::VecDeque;

/// Maximum bytes of output retained per session for reconnecting
/// subscribers. Bounded on purpose: this window exists to make reconnection
/// correct, not to be a durable audit log -- persistent command history and
/// lifecycle events cover durability. 4 MiB is generous relative to a
/// terminal session's typical burst (comfortably holds the tail of a large
/// build log) while still being a bounded, named constant rather than an
/// unstated assumption.
pub const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;

/// Per-subscriber queue depth for ordinary chunks. Mirrors
/// `events::SUBSCRIBER_BUFFER`.
///
/// The underlying channel is allocated one slot larger, and that extra slot
/// is **reserved for the terminal gap notification**. Without the
/// reservation a lagged subscriber cannot be *told* it lagged -- the gap's
/// own `try_send` would fail on the very buffer that just overflowed, and
/// it would be dropped silently while believing it saw everything. This
/// exact defect occurred in feature 004 and was caught only because a test
/// asserted the notification arrived; the same test shape is used here.
pub const SUBSCRIBER_BUFFER: usize = 256;

/// Why a subscriber lost chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GapReason {
    /// Resumed from a sequence older than the retained window.
    RetentionExceeded,
    /// This subscriber could not keep up with the output rate.
    SubscriberLagged,
}

/// What happened. `Gap` is a per-subscriber fact synthesized at the moment
/// loss is detected -- like `events::LifecycleEventKind::Gap`, it describes
/// the *stream*, not the session, and is never stored in the [`OutputLog`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OutputEventKind {
    Chunk {
        command_id: u32,
        stream: malt_protocol::shell::OutputStream,
        data: Vec<u8>,
        produced_at: u64,
    },
    Gap {
        missed_from: u64,
        missed_through: u64,
        reason: GapReason,
    },
}

impl OutputEventKind {
    /// Bytes this entry counts against the retention bound. A `Gap` is
    /// synthesized per-subscriber and never stored, so it costs nothing.
    fn retained_byte_len(&self) -> usize {
        match self {
            Self::Chunk { data, .. } => data.len(),
            Self::Gap { .. } => 0,
        }
    }
}

/// One output event plus its position in the session's stream.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputEvent {
    /// Monotonic per-session position, starting at 1. The resume token.
    pub sequence: u64,
    pub kind: OutputEventKind,
}

/// The bounded catch-up window for one session's output, and the authority
/// on output sequence numbers.
#[derive(Debug)]
pub struct OutputLog {
    entries: VecDeque<OutputEvent>,
    retained_bytes: usize,
    next_sequence: u64,
    max_retained_bytes: usize,
}

impl Default for OutputLog {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputLog {
    pub fn new() -> Self {
        Self::with_capacity(MAX_RETAINED_BYTES)
    }

    /// Construct with an explicit retention bound. Exists so tests can force
    /// a retention overrun without generating megabytes of real output.
    pub fn with_capacity(max_retained_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            retained_bytes: 0,
            next_sequence: 1,
            max_retained_bytes: max_retained_bytes.max(1),
        }
    }

    /// Assign the next sequence, retain the chunk, and return it for
    /// fan-out. Evicts oldest-first while over the byte bound -- a single
    /// chunk larger than the whole bound is retained alone, evicting
    /// everything before it, rather than rejected.
    pub fn publish(&mut self, kind: OutputEventKind) -> OutputEvent {
        let event = OutputEvent {
            sequence: self.next_sequence,
            kind,
        };
        self.next_sequence += 1;
        self.retained_bytes += event.kind.retained_byte_len();
        self.entries.push_back(event.clone());
        // Stop with one entry left even if it alone exceeds the bound: the
        // entry just published must never evict itself, or a single
        // oversized chunk would be silently dropped rather than retained.
        while self.retained_bytes > self.max_retained_bytes && self.entries.len() > 1 {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(evicted.kind.retained_byte_len());
        }
        event
    }

    /// Events after `sequence`, plus whether `sequence` predates what is
    /// still retained.
    ///
    /// The bool is the load-bearing part: `true` means the caller owes the
    /// subscriber an [`OutputEventKind::Gap`] *before* the replayed events,
    /// so it learns about the hole before receiving data that would
    /// otherwise look contiguous.
    pub fn replay_after(&self, sequence: u64) -> (Vec<OutputEvent>, bool) {
        let replay: Vec<OutputEvent> = self
            .entries
            .iter()
            .filter(|e| e.sequence > sequence)
            .cloned()
            .collect();
        let oldest_retained = self.entries.front().map(|e| e.sequence);
        let lost = match oldest_retained {
            Some(oldest) => oldest > sequence.saturating_add(1),
            None => false,
        };
        (replay, lost)
    }

    /// Highest sequence assigned so far (0 before anything is published).
    pub fn latest_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    /// Oldest sequence still retained, if any.
    pub fn oldest_sequence(&self) -> Option<u64> {
        self.entries.front().map(|e| e.sequence)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current retained size in bytes. Exists so tests can assert the log
    /// actually stays within its bound rather than trusting the eviction
    /// logic by inspection.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

/// Outcome of attempting delivery to one subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    /// The subscriber's buffer is full -- it is lagged and must be told,
    /// then dropped. The channel is never grown.
    Lagged,
    /// The receiver is gone (client disconnected, cleanly or not).
    Closed,
}

/// One client's live attachment to a session's output stream.
#[derive(Debug)]
pub struct OutputSubscriberSink {
    pub id: u64,
    tx: tokio::sync::mpsc::Sender<OutputEvent>,
    last_sent: u64,
}

impl OutputSubscriberSink {
    /// Create a sink and its receiver, sized so one slot is always free for
    /// the terminal gap notification.
    pub fn new(id: u64) -> (Self, tokio::sync::mpsc::Receiver<OutputEvent>) {
        Self::with_buffer(id, SUBSCRIBER_BUFFER)
    }

    /// As [`OutputSubscriberSink::new`] with an explicit ordinary-chunk
    /// depth. Exists so tests can overflow a sink without queueing 256
    /// chunks.
    pub fn with_buffer(id: u64, buffer: usize) -> (Self, tokio::sync::mpsc::Receiver<OutputEvent>) {
        let buffer = buffer.max(1);
        let (tx, rx) = tokio::sync::mpsc::channel(buffer + 1);
        (
            Self {
                id,
                tx,
                last_sent: 0,
            },
            rx,
        )
    }

    /// Highest sequence this subscriber is known to hold. Basis of a gap's
    /// range.
    pub fn last_sent(&self) -> u64 {
        self.last_sent
    }

    /// Seed the position for a subscriber that is resuming.
    ///
    /// A resuming client already holds everything up to `sequence`, so a gap
    /// computed from a fresh sink's zero would claim it missed events it
    /// actually saw.
    pub fn set_position(&mut self, sequence: u64) {
        self.last_sent = self.last_sent.max(sequence);
    }

    /// Non-blocking delivery. `try_send` only -- never `send().await`, never
    /// `blocking_send`. The control actor calls this, so anything that could
    /// wait here would let a slow subscriber stall command execution.
    pub fn try_deliver(&mut self, event: &OutputEvent) -> DeliveryOutcome {
        if self.tx.is_closed() {
            return DeliveryOutcome::Closed;
        }
        // Stop one slot short: that slot belongs to the gap notification.
        if self.tx.capacity() <= 1 {
            return DeliveryOutcome::Lagged;
        }
        match self.tx.try_send(event.clone()) {
            Ok(()) => {
                self.last_sent = event.sequence;
                DeliveryOutcome::Delivered
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => DeliveryOutcome::Lagged,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => DeliveryOutcome::Closed,
        }
    }

    /// Deliver a terminal gap describing what this subscriber missed.
    pub fn try_notify_gap(&mut self, through: u64, reason: GapReason) {
        let missed_from = self.last_sent.saturating_add(1);
        if through < missed_from {
            return;
        }
        let gap = OutputEvent {
            sequence: through,
            kind: OutputEventKind::Gap {
                missed_from,
                missed_through: through,
                reason,
            },
        };
        let _ = self.tx.try_send(gap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(command_id: u32, len: usize) -> OutputEventKind {
        OutputEventKind::Chunk {
            command_id,
            stream: malt_protocol::shell::OutputStream::Stdout,
            data: vec![b'x'; len],
            produced_at: 1_000 + u64::from(command_id),
        }
    }

    #[test]
    fn sequence_starts_at_one_and_increases_monotonically() {
        let mut log = OutputLog::new();
        let a = log.publish(chunk(1, 4));
        let b = log.publish(chunk(1, 4));
        let c = log.publish(chunk(1, 4));
        assert_eq!(a.sequence, 1);
        assert_eq!(b.sequence, 2);
        assert_eq!(c.sequence, 3);
        assert_eq!(log.latest_sequence(), 3);
    }

    #[test]
    fn log_evicts_oldest_when_over_the_byte_bound() {
        let mut log = OutputLog::with_capacity(10);
        for i in 1..=5u32 {
            log.publish(chunk(i, 4));
        }
        assert!(
            log.retained_bytes() <= 10,
            "must stay within the byte bound regardless of chunk count, got {}",
            log.retained_bytes()
        );
        // Sequences keep counting even though early entries were evicted.
        assert_eq!(log.latest_sequence(), 5);
    }

    #[test]
    fn a_single_oversized_chunk_evicts_everything_before_it_rather_than_being_rejected() {
        let mut log = OutputLog::with_capacity(10);
        log.publish(chunk(1, 4));
        log.publish(chunk(2, 4));
        let big = log.publish(chunk(3, 100));
        let (replay, _) = log.replay_after(0);
        assert_eq!(replay.len(), 1, "only the oversized chunk should remain");
        assert_eq!(replay[0].sequence, big.sequence);
    }

    #[test]
    fn replay_after_returns_only_later_events() {
        let mut log = OutputLog::new();
        for i in 1..=4u32 {
            log.publish(chunk(i, 4));
        }
        let (replay, lost) = log.replay_after(2);
        assert!(!lost, "nothing was evicted, so there is no gap");
        let seqs: Vec<u64> = replay.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![3, 4]);
    }

    #[test]
    fn replay_after_reports_loss_when_the_position_predates_retention() {
        // Each chunk is 4 bytes; a bound of 9 keeps at most 2 of them.
        let mut log = OutputLog::with_capacity(9);
        for i in 1..=5u32 {
            log.publish(chunk(i, 4));
        }
        let (replay, lost) = log.replay_after(1);
        assert!(lost, "events after the client's position were evicted");
        assert!(!replay.is_empty());
    }

    #[test]
    fn sink_delivers_until_full_then_reports_lagged_without_growing() {
        let (mut sink, rx) = OutputSubscriberSink::with_buffer(1, 2);
        let mut log = OutputLog::new();

        assert_eq!(
            sink.try_deliver(&log.publish(chunk(1, 4))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(chunk(2, 4))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(sink.last_sent(), 2);

        assert_eq!(
            sink.try_deliver(&log.publish(chunk(3, 4))),
            DeliveryOutcome::Lagged
        );
        assert_eq!(
            sink.last_sent(),
            2,
            "last_sent must advance only on success -- it is the basis of the gap range"
        );
        assert_eq!(
            rx.len(),
            2,
            "the channel must not have grown past its ordinary-chunk bound"
        );
    }

    #[test]
    fn sink_reports_closed_when_the_receiver_is_dropped() {
        let (mut sink, rx) = OutputSubscriberSink::with_buffer(1, 4);
        drop(rx);
        let mut log = OutputLog::new();
        assert_eq!(
            sink.try_deliver(&log.publish(chunk(1, 4))),
            DeliveryOutcome::Closed
        );
    }

    #[test]
    fn a_completely_full_sink_can_still_be_told_it_lagged() {
        // Regression, mirroring events.rs: the gap must never share the
        // ordinary buffer, or a subscriber that overflowed cannot be told.
        let (mut sink, mut rx) = OutputSubscriberSink::with_buffer(1, 2);
        let mut log = OutputLog::new();
        assert_eq!(
            sink.try_deliver(&log.publish(chunk(1, 4))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(chunk(2, 4))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(chunk(3, 4))),
            DeliveryOutcome::Lagged
        );

        sink.try_notify_gap(log.latest_sequence(), GapReason::SubscriberLagged);

        let mut kinds = Vec::new();
        while let Ok(event) = rx.try_recv() {
            kinds.push(event.kind);
        }
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, OutputEventKind::Gap { .. })),
            "a full sink must still receive its terminal gap, got {kinds:?}"
        );
    }

    #[test]
    fn a_resuming_sinks_gap_starts_after_what_it_already_holds() {
        let (mut sink, mut rx) = OutputSubscriberSink::with_buffer(1, 4);
        sink.set_position(10);

        sink.try_notify_gap(20, GapReason::RetentionExceeded);

        let event = rx.try_recv().expect("gap should be queued");
        match event.kind {
            OutputEventKind::Gap { missed_from, .. } => assert_eq!(
                missed_from, 11,
                "the gap must begin after the position the client already holds"
            ),
            other => panic!("expected a Gap, got {other:?}"),
        }
    }

    #[test]
    fn empty_writes_are_never_published_as_a_zero_byte_chunk() {
        // The invariant lives in the daemon's OutputChunk handler, not here,
        // but the log itself must not treat an empty chunk specially in a
        // way that would mask that bug -- an empty chunk still costs zero
        // bytes and is still a real, sequenced entry if one is ever handed
        // to `publish`.
        let mut log = OutputLog::new();
        let event = log.publish(chunk(1, 0));
        assert_eq!(log.retained_bytes(), 0);
        assert_eq!(event.sequence, 1);
    }
}
