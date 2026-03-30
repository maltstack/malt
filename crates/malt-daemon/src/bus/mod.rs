pub mod flow_control;
pub mod ring_buffer;

use crate::bus::flow_control::FlowController;
use crate::bus::ring_buffer::RingBuffer;
use malt_protocol::priority::Priority;
use std::collections::HashMap;

/// Identifies a bus subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(u64);

/// A message on the bus.
#[derive(Debug, Clone)]
pub struct BusMessage {
    pub domain: u8,
    pub msg_type: u8,
    pub priority: Priority,
    pub producer_id: u64,
    pub payload: Vec<u8>,
}

/// Bus configuration.
#[derive(Debug, Clone)]
pub struct BusConfig {
    pub default_buffer_size: usize,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            default_buffer_size: 2048,
        }
    }
}

struct Subscriber {
    ring: RingBuffer<BusMessage>,
    critical: Vec<BusMessage>,
    flow: FlowController,
}

/// Per-session synchronous message bus.
///
/// Each subscriber has a bounded ring buffer. Critical messages are delivered
/// to a separate unbounded inbox. The bus never blocks producers — overflow
/// causes priority-based eviction.
pub struct Bus {
    subscribers: HashMap<u64, Subscriber>,
    next_id: u64,
    #[allow(dead_code)]
    config: BusConfig,
}

impl Bus {
    pub fn new(config: BusConfig) -> Self {
        Self {
            subscribers: HashMap::new(),
            next_id: 0,
            config,
        }
    }

    /// Subscribe to the bus with the given buffer size.
    /// Returns a subscriber ID for draining messages.
    pub fn subscribe(&mut self, buffer_size: usize) -> SubscriberId {
        let id = self.next_id;
        self.next_id += 1;
        self.subscribers.insert(
            id,
            Subscriber {
                ring: RingBuffer::new(buffer_size),
                critical: Vec::new(),
                flow: FlowController::new(buffer_size),
            },
        );
        SubscriberId(id)
    }

    /// Remove a subscriber.
    pub fn unsubscribe(&mut self, id: SubscriberId) {
        self.subscribers.remove(&id.0);
    }

    /// Publish a message to all subscribers. Never blocks.
    pub fn publish(&mut self, msg: BusMessage) {
        let ids: Vec<u64> = self.subscribers.keys().copied().collect();
        for id in ids {
            let Some(sub) = self.subscribers.get_mut(&id) else {
                continue;
            };
            match msg.priority {
                Priority::Critical => {
                    sub.critical.push(msg.clone());
                }
                Priority::Reliable => {
                    // Reliable messages are never dropped. Flow control tracks them
                    // for accounting but always accepts. The ring buffer grows beyond
                    // capacity rather than evict Reliable entries.
                    sub.flow.try_publish_reliable(msg.producer_id);
                    sub.ring.push(msg.clone(), Priority::Reliable);
                }
                _ => {
                    // Non-Reliable messages are gated by flow control to prevent
                    // one producer from monopolizing the buffer. If rejected, the
                    // message is silently dropped. If accepted, priority eviction
                    // in the ring buffer handles overflow (Low→Normal→High).
                    if sub.flow.try_publish(msg.producer_id) {
                        sub.ring.push(msg.clone(), msg.priority);
                    }
                }
            }
        }
    }

    /// Drain all non-critical messages for a subscriber.
    pub fn drain(&mut self, id: SubscriberId) -> Vec<BusMessage> {
        let Some(sub) = self.subscribers.get_mut(&id.0) else {
            return Vec::new();
        };
        let count = sub.ring.len();
        let msgs: Vec<BusMessage> = sub.ring.drain().collect();
        sub.flow.drain(count);
        msgs
    }

    /// Drain critical messages for a subscriber.
    pub fn drain_critical(&mut self, id: SubscriberId) -> Vec<BusMessage> {
        let Some(sub) = self.subscribers.get_mut(&id.0) else {
            return Vec::new();
        };
        std::mem::take(&mut sub.critical)
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}
