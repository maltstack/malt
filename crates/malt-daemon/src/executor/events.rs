//! Command lifecycle events: the bounded catch-up log and the bounded,
//! non-blocking per-subscriber sink.
//!
//! These three types are only correct together. A retained log without a lag
//! policy is exactly the failure this feature exists to prevent: a subscriber
//! that stops reading must never grow daemon memory without bound, and must
//! never be left believing it received a complete stream.
//!
//! Deliberately *not* built on [`crate::bus::Bus`]: the bus's `Reliable`
//! priority grows its ring beyond capacity rather than evict, and both
//! `CommandStarted` and `CommandFinished` are specified `Reliable`. Routing
//! them through it would give an untrusted consumer unbounded growth. See
//! `specs/004-command-lifecycle-events/research.md` R2.

use std::collections::VecDeque;

/// Maximum lifecycle events retained per session for reconnecting
/// subscribers. Bounded on purpose: this window exists to make reconnection
/// correct, not to be a durable audit log — persistent command history covers
/// durability.
pub const MAX_RETAINED_EVENTS: usize = 1024;

/// Per-subscriber queue depth for ordinary events. A subscriber that exceeds
/// it is lagged, told so, and dropped — never accommodated by growing the
/// channel.
///
/// The underlying channel is allocated one slot larger, and that extra slot
/// is **reserved for the terminal gap notification**. Without the
/// reservation a lagged subscriber could not be told it lagged — the gap's
/// own `try_send` would fail on the very buffer that just overflowed, and
/// the client would be dropped silently while believing it had a complete
/// stream. That is precisely the failure this feature exists to prevent, so
/// the reservation is load-bearing, not a nicety.
pub const SUBSCRIBER_BUFFER: usize = 256;

/// Why a subscriber lost events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GapReason {
    /// Resumed from a position older than the retained window.
    RetentionExceeded,
    /// This subscriber could not keep up with the event rate.
    SubscriberLagged,
}

/// What happened. `CommandStarted`/`CommandFinished` mirror the schema
/// messages in `schemas/shell.vexil` field-for-field so the deferred VNP path
/// carries the same information under the same names.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LifecycleEventKind {
    CommandStarted {
        command_id: u32,
        cmd: String,
        started_at: u64,
    },
    CommandFinished {
        command_id: u32,
        exit_code: i32,
        finished_at: u64,
        duration_us: u64,
    },
    /// This connection missed events. Describes the *stream*, not the shell —
    /// it is a per-subscriber fact, synthesized at the moment loss is
    /// detected, and never stored in the [`EventLog`].
    Gap {
        missed_from: u64,
        missed_through: u64,
        reason: GapReason,
    },
}

/// One event plus its position in the session's stream.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleEvent {
    /// Monotonic per-session position, starting at 1. The resume token.
    pub sequence: u64,
    pub kind: LifecycleEventKind,
}

/// The bounded catch-up window for one session, and the authority on
/// sequence numbers.
#[derive(Debug)]
pub struct EventLog {
    entries: VecDeque<LifecycleEvent>,
    next_sequence: u64,
    max_retained: usize,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self::with_capacity(MAX_RETAINED_EVENTS)
    }

    /// Construct with an explicit retention bound. Exists so tests can force
    /// a retention overrun without generating a thousand real commands.
    pub fn with_capacity(max_retained: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_sequence: 1,
            max_retained: max_retained.max(1),
        }
    }

    /// Assign the next sequence, retain the event, and return it for fan-out.
    pub fn publish(&mut self, kind: LifecycleEventKind) -> LifecycleEvent {
        let event = LifecycleEvent {
            sequence: self.next_sequence,
            kind,
        };
        self.next_sequence += 1;
        if self.entries.len() >= self.max_retained {
            self.entries.pop_front();
        }
        self.entries.push_back(event.clone());
        event
    }

    /// Events after `sequence`, plus whether `sequence` predates what is
    /// still retained.
    ///
    /// The bool is the load-bearing part: `true` means the caller owes the
    /// subscriber a [`LifecycleEventKind::Gap`] *before* the replayed events,
    /// so it learns about the hole before receiving data that would otherwise
    /// look contiguous.
    pub fn replay_after(&self, sequence: u64) -> (Vec<LifecycleEvent>, bool) {
        let replay: Vec<LifecycleEvent> = self
            .entries
            .iter()
            .filter(|e| e.sequence > sequence)
            .cloned()
            .collect();
        let oldest_retained = self.entries.front().map(|e| e.sequence);
        // A gap exists when events after the client's position were evicted:
        // the oldest we still hold is more than one past where it left off.
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
}

/// Outcome of attempting delivery to one subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    /// The subscriber's buffer is full — it is lagged and must be told, then
    /// dropped. The channel is never grown.
    Lagged,
    /// The receiver is gone (client disconnected, cleanly or not).
    Closed,
}

/// One client's live attachment to a session's event stream.
#[derive(Debug)]
pub struct SubscriberSink {
    pub id: u64,
    tx: tokio::sync::mpsc::Sender<LifecycleEvent>,
    last_sent: u64,
}

impl SubscriberSink {
    /// Create a sink and its receiver, sized so one slot is always free for
    /// the terminal gap notification.
    pub fn new(id: u64) -> (Self, tokio::sync::mpsc::Receiver<LifecycleEvent>) {
        Self::with_buffer(id, SUBSCRIBER_BUFFER)
    }

    /// As [`SubscriberSink::new`] with an explicit ordinary-event depth.
    /// Exists so tests can overflow a sink without queueing 256 events.
    pub fn with_buffer(
        id: u64,
        buffer: usize,
    ) -> (Self, tokio::sync::mpsc::Receiver<LifecycleEvent>) {
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
    /// actually saw — reporting loss that never happened is as wrong as
    /// hiding loss that did.
    pub fn set_position(&mut self, sequence: u64) {
        self.last_sent = self.last_sent.max(sequence);
    }

    /// Non-blocking delivery. `try_send` only — never `send().await`, never
    /// `blocking_send`. The control actor calls this, so anything that could
    /// wait here would let a client stall command execution.
    pub fn try_deliver(&mut self, event: &LifecycleEvent) -> DeliveryOutcome {
        if self.tx.is_closed() {
            return DeliveryOutcome::Closed;
        }
        // Stop one slot short: that slot belongs to the gap notification, so
        // a lagged subscriber can always be told why its stream ended.
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
    ///
    /// This is the reserved slot's purpose, so for a subscriber that lagged
    /// it fits by construction. It can still fail if the receiver is already
    /// gone — fine, that client is not listening anyway.
    pub fn try_notify_gap(&mut self, through: u64, reason: GapReason) {
        let missed_from = self.last_sent.saturating_add(1);
        if through < missed_from {
            return;
        }
        let gap = LifecycleEvent {
            sequence: through,
            kind: LifecycleEventKind::Gap {
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

    fn started(id: u32) -> LifecycleEventKind {
        LifecycleEventKind::CommandStarted {
            command_id: id,
            cmd: format!("echo {id}"),
            started_at: 1_000 + u64::from(id),
        }
    }

    #[test]
    fn sequence_starts_at_one_and_increases_monotonically() {
        let mut log = EventLog::new();
        let a = log.publish(started(1));
        let b = log.publish(started(2));
        let c = log.publish(started(3));
        assert_eq!(a.sequence, 1);
        assert_eq!(b.sequence, 2);
        assert_eq!(c.sequence, 3);
        assert_eq!(log.latest_sequence(), 3);
    }

    #[test]
    fn log_evicts_oldest_at_capacity() {
        let mut log = EventLog::with_capacity(3);
        for i in 1..=5 {
            log.publish(started(i));
        }
        assert_eq!(log.len(), 3);
        // Sequences keep counting even though early entries were evicted.
        assert_eq!(log.oldest_sequence(), Some(3));
        assert_eq!(log.latest_sequence(), 5);
    }

    #[test]
    fn replay_after_returns_only_later_events() {
        let mut log = EventLog::new();
        for i in 1..=4 {
            log.publish(started(i));
        }
        let (replay, lost) = log.replay_after(2);
        assert!(!lost, "nothing was evicted, so there is no gap");
        let seqs: Vec<u64> = replay.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![3, 4]);
    }

    #[test]
    fn replay_after_reports_loss_when_the_position_predates_retention() {
        let mut log = EventLog::with_capacity(2);
        for i in 1..=5 {
            log.publish(started(i));
        }
        // Retains 4 and 5; a client last saw 1, so 2..=3 are gone.
        let (replay, lost) = log.replay_after(1);
        assert!(lost, "events after the client's position were evicted");
        let seqs: Vec<u64> = replay.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![4, 5]);
    }

    #[test]
    fn replay_after_the_latest_position_is_empty_and_lossless() {
        let mut log = EventLog::new();
        log.publish(started(1));
        let (replay, lost) = log.replay_after(1);
        assert!(replay.is_empty());
        assert!(!lost, "being fully caught up is not a gap");
    }

    #[test]
    fn replay_on_an_empty_log_is_empty_not_an_error() {
        let log = EventLog::new();
        let (replay, lost) = log.replay_after(0);
        assert!(replay.is_empty());
        assert!(!lost);
    }

    #[test]
    fn sink_delivers_until_full_then_reports_lagged_without_growing() {
        let (mut sink, rx) = SubscriberSink::with_buffer(1, 2);
        let mut log = EventLog::new();

        assert_eq!(
            sink.try_deliver(&log.publish(started(1))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(started(2))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(sink.last_sent(), 2);

        // Third exceeds the buffer. It must be reported, not queued.
        assert_eq!(
            sink.try_deliver(&log.publish(started(3))),
            DeliveryOutcome::Lagged
        );
        assert_eq!(
            sink.last_sent(),
            2,
            "last_sent must advance only on success — it is the basis of the gap range"
        );
        assert_eq!(
            rx.len(),
            2,
            "the channel must not have grown past its ordinary-event bound"
        );
    }

    #[test]
    fn sink_reports_closed_when_the_receiver_is_dropped() {
        let (mut sink, rx) = SubscriberSink::with_buffer(1, 4);
        drop(rx);
        let mut log = EventLog::new();
        assert_eq!(
            sink.try_deliver(&log.publish(started(1))),
            DeliveryOutcome::Closed
        );
    }

    #[test]
    fn gap_notification_names_the_range_the_subscriber_missed() {
        let (mut sink, mut rx) = SubscriberSink::with_buffer(1, 4);
        let mut log = EventLog::new();
        sink.try_deliver(&log.publish(started(1)));
        let _ = rx.try_recv();

        sink.try_notify_gap(7, GapReason::SubscriberLagged);

        let event = rx.try_recv().expect("gap should be queued");
        match event.kind {
            LifecycleEventKind::Gap {
                missed_from,
                missed_through,
                reason,
            } => {
                assert_eq!(missed_from, 2, "resumes just past the last delivered event");
                assert_eq!(missed_through, 7);
                assert_eq!(reason, GapReason::SubscriberLagged);
            }
            other => panic!("expected a Gap, got {other:?}"),
        }
    }

    #[test]
    fn a_completely_full_sink_can_still_be_told_it_lagged() {
        // Regression: the gap used to share the ordinary buffer, so a
        // subscriber that overflowed could not be told it had -- it was
        // dropped silently while believing its stream was complete, which is
        // the exact failure this feature exists to prevent.
        let (mut sink, mut rx) = SubscriberSink::with_buffer(1, 2);
        let mut log = EventLog::new();
        assert_eq!(
            sink.try_deliver(&log.publish(started(1))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(started(2))),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            sink.try_deliver(&log.publish(started(3))),
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
                .any(|k| matches!(k, LifecycleEventKind::Gap { .. })),
            "a full sink must still receive its terminal gap, got {kinds:?}"
        );
    }

    #[test]
    fn a_resuming_sinks_gap_starts_after_what_it_already_holds() {
        // Regression: a resuming subscriber's sink starts with last_sent == 0,
        // so a gap computed from it claimed the client had missed events it
        // had already seen.
        let (mut sink, mut rx) = SubscriberSink::with_buffer(1, 4);
        sink.set_position(10);

        sink.try_notify_gap(20, GapReason::RetentionExceeded);

        let event = rx.try_recv().expect("gap should be queued");
        match event.kind {
            LifecycleEventKind::Gap { missed_from, .. } => assert_eq!(
                missed_from, 11,
                "the gap must begin after the position the client already holds"
            ),
            other => panic!("expected a Gap, got {other:?}"),
        }
    }

    #[test]
    fn gap_notification_is_skipped_when_nothing_was_actually_missed() {
        let (mut sink, mut rx) = SubscriberSink::with_buffer(1, 4);
        let mut log = EventLog::new();
        sink.try_deliver(&log.publish(started(1)));
        let _ = rx.try_recv();

        // Caught up through 1; a gap "through 1" would be a lie.
        sink.try_notify_gap(1, GapReason::SubscriberLagged);
        assert!(rx.try_recv().is_err(), "must not invent an empty gap");
    }
}
