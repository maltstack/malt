use malt_protocol::priority::Priority;
use std::collections::VecDeque;

#[derive(Debug)]
struct Entry<T> {
    value: T,
    priority: Priority,
}

/// Bounded ring buffer with priority-aware eviction.
///
/// When full, evicts the oldest entry of the lowest priority level present.
/// Eviction order: Low → Normal → High. Reliable entries are never evicted.
/// Critical entries bypass the ring buffer entirely (handled by Bus).
#[derive(Debug)]
pub struct RingBuffer<T> {
    buf: VecDeque<Entry<T>>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: T, priority: Priority) {
        if self.buf.len() >= self.capacity {
            self.evict_one(priority);
        }
        // After eviction attempt, only insert if there's room.
        // If eviction failed (all Reliable), drop the incoming message
        // unless it's Reliable itself — Reliable never dropped.
        if self.buf.len() >= self.capacity {
            match priority {
                Priority::Reliable => {
                    // Reliable never dropped — grow beyond capacity as last resort
                    self.buf.push_back(Entry { value, priority });
                }
                _ => {} // Drop: buffer full of Reliable entries
            }
        } else {
            self.buf.push_back(Entry { value, priority });
        }
    }

    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.buf.drain(..).map(|e| e.value)
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Evict the oldest entry of the lowest evictable priority.
    /// Eviction order: Low → Normal → High.
    /// Reliable is never evicted. Critical is never in the ring buffer.
    fn evict_one(&mut self, _incoming: Priority) {
        let evict_idx = self.find_eviction_candidate();
        if let Some(idx) = evict_idx {
            self.buf.remove(idx);
        }
    }

    fn find_eviction_candidate(&self) -> Option<usize> {
        // Priority ordering for eviction: Low first, then Normal, then High
        // Reliable is never evicted
        let priority_order = [Priority::Low, Priority::Normal, Priority::High];
        for target_priority in &priority_order {
            for (i, entry) in self.buf.iter().enumerate() {
                if std::mem::discriminant(&entry.priority)
                    == std::mem::discriminant(target_priority)
                {
                    return Some(i);
                }
            }
        }
        None
    }
}
