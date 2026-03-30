use std::collections::HashMap;

/// Flow controller for bus Reliable-priority messages.
///
/// Two-level saturation guard:
/// - Per-producer: no single producer may exceed 25% of buffer capacity
/// - Global: total occupancy cannot exceed 60% of buffer capacity
///
/// Reliable messages bypass flow control (they are never dropped).
/// Critical messages bypass the ring buffer entirely (not tracked here).
#[derive(Debug)]
pub struct FlowController {
    capacity: usize,
    producer_counts: HashMap<u64, usize>,
    total: usize,
}

impl FlowController {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            producer_counts: HashMap::new(),
            total: 0,
        }
    }

    /// Try to publish a message from the given producer.
    /// Returns `true` if allowed, `false` if flow control rejects it.
    pub fn try_publish(&mut self, producer_id: u64) -> bool {
        let per_producer_cap = self.capacity / 4; // 25%
        let global_cap = self.capacity * 3 / 5; // 60%

        // Check global saturation
        if self.total >= global_cap {
            return false;
        }

        // Check per-producer cap
        let count = self.producer_counts.entry(producer_id).or_insert(0);
        if *count >= per_producer_cap {
            return false;
        }

        *count += 1;
        self.total += 1;
        true
    }

    /// Reliable messages always succeed (bypass flow control).
    /// Still tracked for accounting but never rejected.
    pub fn try_publish_reliable(&mut self, producer_id: u64) -> bool {
        let count = self.producer_counts.entry(producer_id).or_insert(0);
        *count += 1;
        self.total += 1;
        true
    }

    /// Called when messages are drained from the buffer.
    /// Reduces the total count. Producer-level counts are reset proportionally.
    pub fn drain(&mut self, count: usize) {
        if count >= self.total {
            self.producer_counts.clear();
            self.total = 0;
        } else {
            // Proportional reduction: scale all producer counts down
            let ratio = (self.total - count) as f64 / self.total.max(1) as f64;
            self.total -= count;
            for v in self.producer_counts.values_mut() {
                *v = (*v as f64 * ratio).ceil() as usize;
            }
            self.producer_counts.retain(|_, v| *v > 0);
        }
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn producer_count(&self, producer_id: u64) -> usize {
        self.producer_counts.get(&producer_id).copied().unwrap_or(0)
    }
}
