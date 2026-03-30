# Phase 3A: Message Bus + Session-Sharded Executor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the daemon core — message bus with priority-based eviction, session-sharded executor with per-session tokio runtimes, coordinator thread, and client connection handling.

**Architecture:** Per-session synchronous message bus with bounded ring buffers and priority eviction. Coordinator thread accepts connections, performs VNP handshake, and routes messages to session threads via async channels. Each session runs its own `current_thread` tokio runtime on a dedicated thread.

**Tech Stack:** Rust, tokio (current_thread), malt-protocol (VNP types), malt-platform (sockets, PTY, signals), malt-session (SessionRuntime), malt-layout (resolution), thiserror, tracing

---

## File Structure

```
crates/malt-daemon/
  Cargo.toml
  src/
    lib.rs                    — crate root, module declarations, re-exports
    error.rs                  — DaemonError enum
    bus/
      mod.rs                  — Bus struct, BusMessage, SubscriberId, publish/subscribe API
      ring_buffer.rs          — RingBuffer<T> with priority-aware eviction
      flow_control.rs         — FlowController: per-producer cap + global saturation guard
    executor/
      mod.rs                  — re-exports
      coordinator.rs          — Coordinator: accept connections, route to sessions, lifecycle
      session_thread.rs       — SessionExecutor: per-session runtime, tick loop, bus dispatch
      pools.rs                — PoolConfig: shared thread pool sizing
    connection/
      mod.rs                  — ClientConnection: framed VNP I/O over transport
      handshake.rs            — perform_handshake: Hello/HelloAck exchange
      authority.rs            — AuthorityTracker: per-session input authority FIFO
  tests/
    ring_buffer.rs            — ring buffer unit tests
    bus.rs                    — bus publish/subscribe/eviction tests
    flow_control.rs           — flow control tests
    authority.rs              — input authority tests
    handshake.rs              — handshake protocol tests
    session_thread.rs         — session executor tests
    coordinator.rs            — coordinator integration tests
```

---

### Task 1: Crate Scaffolding

**Files:**
- Create: `crates/malt-daemon/Cargo.toml`
- Create: `crates/malt-daemon/src/lib.rs`
- Create: `crates/malt-daemon/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "malt-daemon"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Daemon core for MALT — message bus, session-sharded executor, coordinator"

[dependencies]
malt-protocol = { path = "../malt-protocol" }
malt-platform = { path = "../malt-platform", features = ["tokio"] }
malt-session = { path = "../malt-session" }
malt-layout = { path = "../malt-layout" }
malt-config = { path = "../malt-config" }
thiserror = "2"
tracing = "0.1"
tokio = { version = "1", features = ["rt", "net", "sync", "macros", "time", "io-util"] }
vexil-runtime = { git = "https://github.com/orix-systems/vexil", branch = "main" }
```

- [ ] **Step 2: Create error.rs**

```rust
use malt_session::session::SessionError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    #[error("session not found: {0:?}")]
    SessionNotFound(malt_protocol::common::SessionId),

    #[error("session error: {0}")]
    Session(#[from] SessionError),

    #[error("transport error: {0}")]
    Transport(#[from] malt_platform::sockets::TransportError),

    #[error("handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("bus full for subscriber {0}")]
    BusFull(u64),

    #[error("frame error: {0}")]
    Frame(#[from] malt_protocol::framing::FrameError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 3: Create lib.rs**

```rust
pub mod bus;
pub mod connection;
pub mod error;
pub mod executor;

pub use error::DaemonError;
```

- [ ] **Step 4: Create stub modules for bus, connection, executor**

Create `crates/malt-daemon/src/bus/mod.rs`:
```rust
pub mod flow_control;
pub mod ring_buffer;
```

Create `crates/malt-daemon/src/executor/mod.rs`:
```rust
pub mod coordinator;
pub mod pools;
pub mod session_thread;
```

Create `crates/malt-daemon/src/connection/mod.rs`:
```rust
pub mod authority;
pub mod handshake;
```

Create `crates/malt-daemon/src/bus/ring_buffer.rs`:
```rust
// Ring buffer with priority-aware eviction — Task 2
```

Create `crates/malt-daemon/src/bus/flow_control.rs`:
```rust
// Per-producer cap + global saturation guard — Task 3
```

Create `crates/malt-daemon/src/executor/coordinator.rs`:
```rust
// Coordinator: accept connections, route to sessions — Task 7
```

Create `crates/malt-daemon/src/executor/session_thread.rs`:
```rust
// Per-session executor with own tokio runtime — Task 6
```

Create `crates/malt-daemon/src/executor/pools.rs`:
```rust
// Shared thread pool configuration — Task 6
```

Create `crates/malt-daemon/src/connection/authority.rs`:
```rust
// Input authority FIFO tracker — Task 5
```

Create `crates/malt-daemon/src/connection/handshake.rs`:
```rust
// VNP Hello/HelloAck exchange — Task 8
```

- [ ] **Step 5: Add malt-daemon to workspace**

In root `Cargo.toml`, change the members line to:
```toml
members = ["crates/malt-protocol", "crates/malt-platform", "crates/malt-config", "crates/mash", "crates/malt-layout", "crates/malt-session", "crates/malt-term", "crates/malt-tools", "crates/malt-elevate", "crates/malt-daemon"]
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check -p malt-daemon`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add crates/malt-daemon/ Cargo.toml
git commit -m "feat(malt-daemon): crate scaffolding with module stubs"
```

---

### Task 2: Ring Buffer with Priority Eviction

**Files:**
- Create: `crates/malt-daemon/src/bus/ring_buffer.rs`
- Create: `crates/malt-daemon/tests/ring_buffer.rs`

- [ ] **Step 1: Write failing tests for basic ring buffer operations**

Create `crates/malt-daemon/tests/ring_buffer.rs`:
```rust
use malt_daemon::bus::ring_buffer::RingBuffer;
use malt_protocol::priority::Priority;

#[test]
fn push_and_drain() {
    let mut rb = RingBuffer::new(4);
    rb.push(10u32, Priority::Normal);
    rb.push(20, Priority::Normal);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![10, 20]);
}

#[test]
fn drain_empties_buffer() {
    let mut rb = RingBuffer::new(4);
    rb.push(1u32, Priority::Normal);
    let _: Vec<u32> = rb.drain().collect();
    assert!(rb.is_empty());
    assert_eq!(rb.len(), 0);
}

#[test]
fn capacity_respected() {
    let mut rb = RingBuffer::new(2);
    rb.push(1u32, Priority::Normal);
    rb.push(2, Priority::Normal);
    // Buffer full — next push must evict
    rb.push(3, Priority::Normal);
    assert_eq!(rb.len(), 2);
    let items: Vec<u32> = rb.drain().collect();
    // Oldest Normal evicted
    assert_eq!(items, vec![2, 3]);
}

#[test]
fn fifo_ordering_preserved() {
    let mut rb = RingBuffer::new(8);
    for i in 0..5u32 {
        rb.push(i, Priority::Normal);
    }
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![0, 1, 2, 3, 4]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test ring_buffer`
Expected: FAIL — `ring_buffer` module not implemented

- [ ] **Step 3: Implement RingBuffer**

Write `crates/malt-daemon/src/bus/ring_buffer.rs`:
```rust
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
                _ => return, // Drop: buffer full of Reliable entries
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
        // Find the oldest entry of the lowest priority
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test ring_buffer`
Expected: all 4 tests PASS

- [ ] **Step 5: Write tests for priority eviction behavior**

Append to `crates/malt-daemon/tests/ring_buffer.rs`:
```rust
#[test]
fn evicts_low_before_normal() {
    let mut rb = RingBuffer::new(3);
    rb.push(1u32, Priority::Normal);
    rb.push(2, Priority::Low);
    rb.push(3, Priority::Normal);
    // Full — push Normal, should evict oldest Low (value 2)
    rb.push(4, Priority::Normal);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![1, 3, 4]);
}

#[test]
fn evicts_normal_before_high() {
    let mut rb = RingBuffer::new(3);
    rb.push(1u32, Priority::High);
    rb.push(2, Priority::Normal);
    rb.push(3, Priority::High);
    // Full — push High, should evict oldest Normal (value 2)
    rb.push(4, Priority::High);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![1, 3, 4]);
}

#[test]
fn reliable_never_evicted() {
    let mut rb = RingBuffer::new(2);
    rb.push(1u32, Priority::Reliable);
    rb.push(2, Priority::Reliable);
    // Full of Reliable — push Normal, Normal gets dropped
    rb.push(3, Priority::Normal);
    // Reliable entries intact, Normal was dropped
    assert_eq!(rb.len(), 2);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![1, 2]);
}

#[test]
fn reliable_grows_beyond_capacity_if_needed() {
    let mut rb = RingBuffer::new(2);
    rb.push(1u32, Priority::Reliable);
    rb.push(2, Priority::Reliable);
    // Full of Reliable — push another Reliable, must not drop
    rb.push(3, Priority::Reliable);
    assert_eq!(rb.len(), 3);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![1, 2, 3]);
}

#[test]
fn high_evicts_oldest_high_when_no_lower() {
    let mut rb = RingBuffer::new(3);
    rb.push(1u32, Priority::High);
    rb.push(2, Priority::High);
    rb.push(3, Priority::Reliable);
    // Full — push High, should evict oldest High (value 1)
    rb.push(4, Priority::High);
    let items: Vec<u32> = rb.drain().collect();
    assert_eq!(items, vec![2, 3, 4]);
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test ring_buffer`
Expected: all 9 tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/malt-daemon/src/bus/ring_buffer.rs crates/malt-daemon/tests/ring_buffer.rs
git commit -m "feat(malt-daemon): ring buffer with priority-aware eviction"
```

---

### Task 3: Flow Control

**Files:**
- Create: `crates/malt-daemon/src/bus/flow_control.rs`
- Create: `crates/malt-daemon/tests/flow_control.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/flow_control.rs`:
```rust
use malt_daemon::bus::flow_control::FlowController;

#[test]
fn new_producer_allowed() {
    let fc = FlowController::new(100);
    assert!(fc.try_publish(1));
}

#[test]
fn producer_under_cap_allowed() {
    let mut fc = FlowController::new(100);
    for _ in 0..24 {
        assert!(fc.try_publish(1));
    }
    // 24 out of 100 = 24%, under 25% cap
    assert!(fc.try_publish(1));
}

#[test]
fn producer_at_cap_rejected() {
    let mut fc = FlowController::new(100);
    for _ in 0..25 {
        assert!(fc.try_publish(1));
    }
    // 25 out of 100 = 25%, at cap
    assert!(!fc.try_publish(1));
}

#[test]
fn drain_resets_counts() {
    let mut fc = FlowController::new(100);
    for _ in 0..25 {
        fc.try_publish(1);
    }
    fc.drain(25);
    // After drain, producer count should be reset proportionally
    assert!(fc.try_publish(1));
}

#[test]
fn global_saturation_blocks_all() {
    let mut fc = FlowController::new(100);
    // Two producers each at 30 — total 60 = 60% global cap
    for _ in 0..30 {
        fc.try_publish(1);
    }
    for _ in 0..30 {
        fc.try_publish(2);
    }
    // Total is 60, which hits 60% global saturation
    assert!(!fc.try_publish(3));
}

#[test]
fn reliable_bypasses_flow_control() {
    let mut fc = FlowController::new(100);
    for _ in 0..25 {
        fc.try_publish(1);
    }
    // Producer 1 at cap, but Reliable bypasses
    assert!(fc.try_publish_reliable(1));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test flow_control`
Expected: FAIL — module not implemented

- [ ] **Step 3: Implement FlowController**

Write `crates/malt-daemon/src/bus/flow_control.rs`:
```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test flow_control`
Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-daemon/src/bus/flow_control.rs crates/malt-daemon/tests/flow_control.rs
git commit -m "feat(malt-daemon): flow control — per-producer 25% cap, 60% global saturation"
```

---

### Task 4: Message Bus

**Files:**
- Create: `crates/malt-daemon/src/bus/mod.rs`
- Create: `crates/malt-daemon/tests/bus.rs`

- [ ] **Step 1: Write failing tests for bus publish/subscribe**

Create `crates/malt-daemon/tests/bus.rs`:
```rust
use malt_daemon::bus::{Bus, BusConfig, BusMessage};
use malt_protocol::priority::Priority;

fn make_msg(domain: u8, msg_type: u8, priority: Priority, data: Vec<u8>) -> BusMessage {
    BusMessage {
        domain,
        msg_type,
        priority,
        producer_id: 0,
        payload: data,
    }
}

#[test]
fn subscribe_and_receive() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(16);
    let msg = make_msg(1, 1, Priority::Normal, vec![42]);
    bus.publish(msg);
    let msgs = bus.drain(id);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].payload, vec![42]);
}

#[test]
fn multiple_subscribers_receive_same_message() {
    let mut bus = Bus::new(BusConfig::default());
    let id1 = bus.subscribe(16);
    let id2 = bus.subscribe(16);
    bus.publish(make_msg(1, 1, Priority::Normal, vec![1]));
    assert_eq!(bus.drain(id1).len(), 1);
    assert_eq!(bus.drain(id2).len(), 1);
}

#[test]
fn unsubscribe_stops_delivery() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(16);
    bus.unsubscribe(id);
    bus.publish(make_msg(1, 1, Priority::Normal, vec![1]));
    let msgs = bus.drain(id);
    assert!(msgs.is_empty());
}

#[test]
fn critical_delivered_to_critical_inbox() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(4);
    bus.publish(make_msg(2, 4, Priority::Critical, vec![99]));
    let critical = bus.drain_critical(id);
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].payload, vec![99]);
    // Not in regular drain
    let regular = bus.drain(id);
    assert!(regular.is_empty());
}

#[test]
fn publish_never_blocks() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(2);
    // Publish more than capacity — should not panic or block
    for i in 0..100u8 {
        bus.publish(make_msg(1, 1, Priority::Normal, vec![i]));
    }
    let msgs = bus.drain(id);
    // Should have exactly 2 (buffer size), with the most recent surviving
    assert_eq!(msgs.len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test bus`
Expected: FAIL — Bus not implemented

- [ ] **Step 3: Implement Bus**

Write `crates/malt-daemon/src/bus/mod.rs`:
```rust
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
            let sub = self.subscribers.get_mut(&id).unwrap();
            match msg.priority {
                Priority::Critical => {
                    sub.critical.push(msg.clone());
                }
                Priority::Reliable => {
                    sub.flow.try_publish_reliable(msg.producer_id);
                    sub.ring.push(msg.clone(), Priority::Reliable);
                }
                _ => {
                    if sub.flow.try_publish(msg.producer_id) {
                        sub.ring.push(msg.clone(), msg.priority);
                    }
                    // If flow control rejects, message is dropped — by design
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test bus`
Expected: all 5 tests PASS

- [ ] **Step 5: Write tests for flow control integration**

Append to `crates/malt-daemon/tests/bus.rs`:
```rust
#[test]
fn reliable_never_dropped_even_when_full() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(2);
    // Fill with Reliable
    for i in 0..5u8 {
        bus.publish(make_msg(1, 1, Priority::Reliable, vec![i]));
    }
    let msgs = bus.drain(id);
    // All 5 Reliable messages must survive
    assert_eq!(msgs.len(), 5);
}

#[test]
fn mixed_priorities_evict_correctly() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(4);
    bus.publish(make_msg(1, 1, Priority::Low, vec![1]));
    bus.publish(make_msg(1, 1, Priority::Normal, vec![2]));
    bus.publish(make_msg(1, 1, Priority::High, vec![3]));
    bus.publish(make_msg(1, 1, Priority::Reliable, vec![4]));
    // Full at 4 — push Normal, should evict Low
    bus.publish(make_msg(1, 1, Priority::Normal, vec![5]));
    let msgs = bus.drain(id);
    let payloads: Vec<u8> = msgs.iter().map(|m| m.payload[0]).collect();
    assert_eq!(payloads, vec![2, 3, 4, 5]);
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test bus`
Expected: all 7 tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/malt-daemon/src/bus/mod.rs crates/malt-daemon/tests/bus.rs
git commit -m "feat(malt-daemon): message bus — publish/subscribe with priority eviction"
```

---

### Task 5: Input Authority Tracker

**Files:**
- Create: `crates/malt-daemon/src/connection/authority.rs`
- Create: `crates/malt-daemon/tests/authority.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/authority.rs`:
```rust
use malt_daemon::connection::authority::AuthorityTracker;
use malt_protocol::common::InputAuthority;

#[test]
fn first_attach_gets_authority() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    assert_eq!(tracker.holder(), Some(1));
}

#[test]
fn observe_does_not_claim() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Observe);
    assert_eq!(tracker.holder(), Some(1));
}

#[test]
fn latest_exclusive_takes_authority() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Exclusive);
    assert_eq!(tracker.holder(), Some(2));
}

#[test]
fn detach_holder_falls_back_to_fifo() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Exclusive);
    // Client 2 holds — detach it
    tracker.detach(2);
    // Falls back to client 1 (FIFO)
    assert_eq!(tracker.holder(), Some(1));
}

#[test]
fn detach_observer_does_not_change_holder() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Observe);
    tracker.detach(2);
    assert_eq!(tracker.holder(), Some(1));
}

#[test]
fn detach_last_client_returns_none() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.detach(1);
    assert_eq!(tracker.holder(), None);
}

#[test]
fn claim_transfers_authority() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Observe);
    tracker.claim(2, InputAuthority::Exclusive);
    assert_eq!(tracker.holder(), Some(2));
}

#[test]
fn attached_clients_list() {
    let mut tracker = AuthorityTracker::new();
    tracker.attach(1, InputAuthority::Exclusive);
    tracker.attach(2, InputAuthority::Observe);
    tracker.attach(3, InputAuthority::Exclusive);
    let clients = tracker.attached_clients();
    assert_eq!(clients.len(), 3);
    assert!(clients.contains(&1));
    assert!(clients.contains(&2));
    assert!(clients.contains(&3));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test authority`
Expected: FAIL — module not implemented

- [ ] **Step 3: Implement AuthorityTracker**

Write `crates/malt-daemon/src/connection/authority.rs`:
```rust
use malt_protocol::common::InputAuthority;
use std::collections::VecDeque;

#[derive(Debug)]
struct AttachedClient {
    client_id: u64,
    authority: InputAuthority,
}

/// Tracks input authority for a session's attached clients.
///
/// Rules:
/// - Most recent Exclusive/Shared attach gets authority
/// - Observe attach never claims authority
/// - On holder detach, authority falls to next eligible client (FIFO)
/// - `claim()` transfers authority explicitly
#[derive(Debug)]
pub struct AuthorityTracker {
    clients: VecDeque<AttachedClient>,
    holder: Option<u64>,
}

impl AuthorityTracker {
    pub fn new() -> Self {
        Self {
            clients: VecDeque::new(),
            holder: None,
        }
    }

    /// Attach a client. If authority is Exclusive or Shared, claim input.
    pub fn attach(&mut self, client_id: u64, authority: InputAuthority) {
        self.clients.push_back(AttachedClient {
            client_id,
            authority,
        });
        match authority {
            InputAuthority::Exclusive | InputAuthority::Shared => {
                self.holder = Some(client_id);
            }
            InputAuthority::Observe => {
                // First client gets authority even if Observe
                if self.holder.is_none() && self.clients.len() == 1 {
                    // No — observe explicitly does not claim
                }
            }
            _ => {}
        }
    }

    /// Detach a client. If they held authority, fall back to FIFO.
    pub fn detach(&mut self, client_id: u64) {
        self.clients.retain(|c| c.client_id != client_id);
        if self.holder == Some(client_id) {
            self.holder = self.find_next_eligible();
        }
    }

    /// Explicitly claim authority for a client.
    pub fn claim(&mut self, client_id: u64, authority: InputAuthority) {
        if let Some(c) = self.clients.iter_mut().find(|c| c.client_id == client_id) {
            c.authority = authority;
        }
        match authority {
            InputAuthority::Exclusive | InputAuthority::Shared => {
                self.holder = Some(client_id);
            }
            _ => {}
        }
    }

    /// Returns the client that currently holds input authority.
    pub fn holder(&self) -> Option<u64> {
        self.holder
    }

    /// Returns all attached client IDs.
    pub fn attached_clients(&self) -> Vec<u64> {
        self.clients.iter().map(|c| c.client_id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Find the next eligible client for authority (FIFO order).
    /// Prefers Exclusive/Shared over Observe.
    fn find_next_eligible(&self) -> Option<u64> {
        // First try Exclusive/Shared in FIFO order
        for c in &self.clients {
            match c.authority {
                InputAuthority::Exclusive | InputAuthority::Shared => {
                    return Some(c.client_id);
                }
                _ => {}
            }
        }
        None
    }
}

impl Default for AuthorityTracker {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test authority`
Expected: all 8 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-daemon/src/connection/authority.rs crates/malt-daemon/tests/authority.rs
git commit -m "feat(malt-daemon): input authority tracker — FIFO fallback, claim, observe"
```

---

### Task 6: Session Executor and Pool Config

**Files:**
- Create: `crates/malt-daemon/src/executor/pools.rs`
- Create: `crates/malt-daemon/src/executor/session_thread.rs`
- Create: `crates/malt-daemon/tests/session_thread.rs`

- [ ] **Step 1: Implement PoolConfig**

Write `crates/malt-daemon/src/executor/pools.rs`:
```rust
/// Configuration for shared thread pools.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Thread pool for WASM plugin execution.
    pub wasm_threads: usize,
    /// Thread pool for PTY read I/O.
    pub pty_io_threads: usize,
    /// Thread pool for disk I/O (persistence, scrollback).
    pub disk_io_threads: usize,
    /// Bounded channel capacity from coordinator to session.
    pub session_channel_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            wasm_threads: cpus.max(4) / 2,
            pty_io_threads: cpus.max(4),
            disk_io_threads: 4,
            session_channel_size: 256,
        }
    }
}
```

- [ ] **Step 2: Write failing tests for SessionExecutor**

Create `crates/malt-daemon/tests/session_thread.rs`:
```rust
use malt_daemon::bus::{Bus, BusConfig, BusMessage};
use malt_daemon::executor::session_thread::{SessionExecutor, SessionCommand};
use malt_protocol::common::{IsolationTier, PaneId, SessionId};
use malt_protocol::priority::Priority;

#[test]
fn session_executor_starts_and_stops() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );
    // Send shutdown
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_processes_input() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );

    // Send a bus message through the command channel
    let msg = BusMessage {
        domain: 2,   // Input
        msg_type: 1, // KeyEvent
        priority: Priority::Critical,
        producer_id: 0,
        payload: vec![1, 2, 3],
    };
    cmd_tx.send(SessionCommand::Deliver(msg)).unwrap();
    // Give it a moment to process
    std::thread::sleep(std::time::Duration::from_millis(50));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test session_thread`
Expected: FAIL — SessionExecutor not implemented

- [ ] **Step 4: Implement SessionExecutor**

Write `crates/malt-daemon/src/executor/session_thread.rs`:
```rust
use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{IsolationTier, LayoutNode, PaneId, SessionId};
use malt_session::session::SessionRuntime;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use tracing::{info, warn};

/// Commands sent from the coordinator to a session executor.
#[derive(Debug)]
pub enum SessionCommand {
    /// Deliver a message to the session's bus.
    Deliver(BusMessage),
    /// Attach a client to this session.
    AttachClient {
        client_id: u64,
        authority: malt_protocol::common::InputAuthority,
    },
    /// Detach a client from this session.
    DetachClient { client_id: u64 },
    /// Resize the terminal.
    Resize { cols: u16, rows: u16 },
    /// Graceful shutdown.
    Shutdown,
}

/// Per-session executor running on a dedicated thread with its own tokio runtime.
pub struct SessionExecutor {
    session: SessionRuntime,
    bus: Bus,
    authority: AuthorityTracker,
    terminal_size: Rect,
    layout_config: LayoutConfig,
}

impl SessionExecutor {
    /// Spawn a new session executor on a dedicated thread.
    /// Returns the command sender and thread handle.
    pub fn spawn(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
    ) -> (mpsc::Sender<SessionCommand>, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel();

        let handle = thread::Builder::new()
            .name(format!("session-{}", session_id.0))
            .spawn(move || {
                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                };
                executor.run(rx);
            })
            .expect("failed to spawn session thread");

        (tx, handle)
    }

    fn run(&mut self, rx: mpsc::Receiver<SessionCommand>) {
        info!(session = ?self.session.id(), "session executor started");

        loop {
            match rx.recv() {
                Ok(SessionCommand::Shutdown) => {
                    info!(session = ?self.session.id(), "session executor shutting down");
                    break;
                }
                Ok(SessionCommand::Deliver(msg)) => {
                    self.bus.publish(msg);
                }
                Ok(SessionCommand::AttachClient {
                    client_id,
                    authority,
                }) => {
                    self.authority.attach(client_id, authority);
                    let _ = self.session.attach(client_id, authority);
                }
                Ok(SessionCommand::DetachClient { client_id }) => {
                    self.authority.detach(client_id);
                    let _ = self.session.detach(client_id);
                }
                Ok(SessionCommand::Resize { cols, rows }) => {
                    self.terminal_size = Rect::new(0, 0, cols, rows);
                    self.recompute_layout();
                }
                Err(_) => {
                    // Channel closed — coordinator dropped the sender
                    warn!(session = ?self.session.id(), "command channel closed");
                    break;
                }
            }
        }
    }

    fn recompute_layout(&mut self) {
        let _resolved = compute_resolved_panes(
            self.session.layout(),
            self.terminal_size,
            *self.session.focused_pane(),
            &self.layout_config,
        );
        // Layout results will be published to bus when renderer is integrated (Phase 3C)
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test session_thread`
Expected: all 2 tests PASS

- [ ] **Step 6: Write test for attach/detach lifecycle**

Append to `crates/malt-daemon/tests/session_thread.rs`:
```rust
use malt_protocol::common::InputAuthority;

#[test]
fn session_executor_attach_detach() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );

    cmd_tx
        .send(SessionCommand::AttachClient {
            client_id: 100,
            authority: InputAuthority::Exclusive,
        })
        .unwrap();

    cmd_tx
        .send(SessionCommand::DetachClient { client_id: 100 })
        .unwrap();

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_resize() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );

    cmd_tx
        .send(SessionCommand::Resize { cols: 120, rows: 40 })
        .unwrap();

    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test session_thread`
Expected: all 4 tests PASS

- [ ] **Step 8: Commit**

```bash
git add crates/malt-daemon/src/executor/pools.rs crates/malt-daemon/src/executor/session_thread.rs crates/malt-daemon/tests/session_thread.rs
git commit -m "feat(malt-daemon): session executor — per-session thread with bus and authority"
```

---

### Task 7: Coordinator

**Files:**
- Create: `crates/malt-daemon/src/executor/coordinator.rs`
- Create: `crates/malt-daemon/tests/coordinator.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/coordinator.rs`:
```rust
use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_protocol::common::{IsolationTier, SessionId};

#[test]
fn create_session() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id = coord.create_session(None, IsolationTier::Bare, None);
    assert_eq!(id, SessionId(1));
}

#[test]
fn create_multiple_sessions() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id1 = coord.create_session(None, IsolationTier::Bare, None);
    let id2 = coord.create_session(None, IsolationTier::Bare, None);
    assert_eq!(id1, SessionId(1));
    assert_eq!(id2, SessionId(2));
    assert_eq!(coord.session_count(), 2);
}

#[test]
fn destroy_session() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id = coord.create_session(None, IsolationTier::Bare, None);
    coord.destroy_session(id);
    assert_eq!(coord.session_count(), 0);
}

#[test]
fn list_sessions() {
    let mut coord = Coordinator::new(PoolConfig::default());
    coord.create_session(Some("alpha".to_string()), IsolationTier::Bare, None);
    coord.create_session(Some("beta".to_string()), IsolationTier::Restricted, None);
    let sessions = coord.list_sessions();
    assert_eq!(sessions.len(), 2);
}

#[test]
fn session_ids_never_recycled() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id1 = coord.create_session(None, IsolationTier::Bare, None);
    coord.destroy_session(id1);
    let id2 = coord.create_session(None, IsolationTier::Bare, None);
    assert_ne!(id1, id2);
    assert_eq!(id2, SessionId(2));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test coordinator`
Expected: FAIL — Coordinator not implemented

- [ ] **Step 3: Implement Coordinator**

Write `crates/malt-daemon/src/executor/coordinator.rs`:
```rust
use crate::bus::BusMessage;
use crate::executor::pools::PoolConfig;
use crate::executor::session_thread::{SessionCommand, SessionExecutor};
use crate::DaemonError;
use malt_protocol::common::{
    GroupId, IsolationTier, PaneId, SessionId, SessionInfo, SessionState,
};
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread::JoinHandle;
use tracing::{info, warn};

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    cmd_tx: mpsc::Sender<SessionCommand>,
    thread: Option<JoinHandle<()>>,
}

/// Coordinator manages session lifecycle and routes messages to session threads.
///
/// Monotonically increasing session IDs — never recycled within daemon lifetime.
pub struct Coordinator {
    sessions: HashMap<u32, SessionHandle>,
    next_session_id: u32,
    next_pane_id: u32,
    pool_config: PoolConfig,
}

impl Coordinator {
    pub fn new(pool_config: PoolConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            next_session_id: 1,
            next_pane_id: 1,
            pool_config,
        }
    }

    /// Create a new session. Returns the assigned SessionId.
    pub fn create_session(
        &mut self,
        name: Option<String>,
        isolation: IsolationTier,
        group: Option<GroupId>,
    ) -> SessionId {
        let session_id = SessionId(self.next_session_id);
        self.next_session_id += 1;

        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;

        let (cmd_tx, thread) = SessionExecutor::spawn(session_id, pane_id, isolation);

        info!(?session_id, ?name, ?isolation, "session created");

        self.sessions.insert(
            session_id.0,
            SessionHandle {
                id: session_id,
                name,
                isolation,
                cmd_tx,
                thread: Some(thread),
            },
        );

        session_id
    }

    /// Destroy a session. Sends shutdown and joins the thread.
    pub fn destroy_session(&mut self, id: SessionId) {
        if let Some(mut handle) = self.sessions.remove(&id.0) {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown);
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
            info!(?id, "session destroyed");
        }
    }

    /// Route a message to a specific session.
    pub fn route_to_session(
        &self,
        session_id: SessionId,
        msg: BusMessage,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        let _ = handle.cmd_tx.send(SessionCommand::Deliver(msg));
        Ok(())
    }

    /// Route a command to a specific session.
    pub fn send_command(
        &self,
        session_id: SessionId,
        cmd: SessionCommand,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        let _ = handle.cmd_tx.send(cmd);
        Ok(())
    }

    /// List all active sessions.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .values()
            .map(|h| SessionInfo {
                session_id: h.id,
                name: h.name.clone(),
                pane_count: 1, // TODO: track actual pane count when supervisor is added
                isolation: h.isolation,
                state: SessionState::Active,
                _unknown: Vec::new(),
            })
            .collect()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn has_session(&self, id: SessionId) -> bool {
        self.sessions.contains_key(&id.0)
    }

    /// Shutdown all sessions gracefully.
    pub fn shutdown_all(&mut self) {
        let ids: Vec<u32> = self.sessions.keys().copied().collect();
        for id in ids {
            self.destroy_session(SessionId(id));
        }
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test coordinator`
Expected: all 5 tests PASS

- [ ] **Step 5: Write tests for routing and error handling**

Append to `crates/malt-daemon/tests/coordinator.rs`:
```rust
use malt_daemon::bus::BusMessage;
use malt_daemon::executor::session_thread::SessionCommand;
use malt_protocol::common::InputAuthority;
use malt_protocol::priority::Priority;

#[test]
fn route_to_nonexistent_session_errors() {
    let coord = Coordinator::new(PoolConfig::default());
    let msg = BusMessage {
        domain: 1,
        msg_type: 1,
        priority: Priority::Normal,
        producer_id: 0,
        payload: vec![],
    };
    let err = coord.route_to_session(SessionId(999), msg).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("session not found"), "got: {msg}");
}

#[test]
fn send_attach_command() {
    let mut coord = Coordinator::new(PoolConfig::default());
    let id = coord.create_session(None, IsolationTier::Bare, None);
    coord
        .send_command(
            id,
            SessionCommand::AttachClient {
                client_id: 42,
                authority: InputAuthority::Exclusive,
            },
        )
        .unwrap();
    // Cleanup
    coord.destroy_session(id);
}

#[test]
fn shutdown_all_cleans_up() {
    let mut coord = Coordinator::new(PoolConfig::default());
    coord.create_session(None, IsolationTier::Bare, None);
    coord.create_session(None, IsolationTier::Bare, None);
    coord.shutdown_all();
    assert_eq!(coord.session_count(), 0);
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test coordinator`
Expected: all 8 tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/malt-daemon/src/executor/coordinator.rs crates/malt-daemon/src/executor/pools.rs crates/malt-daemon/tests/coordinator.rs
git commit -m "feat(malt-daemon): coordinator — session lifecycle, message routing, shutdown"
```

---

### Task 8: Connection Handshake

**Files:**
- Create: `crates/malt-daemon/src/connection/handshake.rs`
- Create: `crates/malt-daemon/src/connection/mod.rs`
- Create: `crates/malt-daemon/tests/handshake.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/handshake.rs`:
```rust
use malt_daemon::connection::handshake::{perform_server_handshake, HandshakeResult};
use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck};
use malt_protocol::envelope::{encode_message, decode_envelope, Envelope};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};
use std::io::Cursor;

const WIRE_VERSION: u32 = 1;

fn encode_hello(hello: &Hello) -> Vec<u8> {
    let envelope = Envelope {
        wire_version: 0,
        domain: 0, // Handshake
        msg_type: 0x01,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let mut w = BitWriter::new();
    hello.pack(&mut w).unwrap();
    let msg_bytes = w.finish();
    let combined = encode_message(&envelope, &msg_bytes);
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    buf
}

fn decode_hello_ack(data: &[u8]) -> HelloAck {
    let mut reader = FrameReader::new(Cursor::new(data));
    let frame = reader.read_frame().unwrap();
    let (envelope, msg_bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(envelope.domain, 0); // Handshake
    assert_eq!(envelope.msg_type, 0x02); // HelloAck
    let mut r = BitReader::new(msg_bytes);
    HelloAck::unpack(&mut r).unwrap()
}

#[test]
fn successful_handshake() {
    let hello = Hello {
        version: WIRE_VERSION,
        client_type: "test".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::TrueColor,
            unicode: UnicodeLevel::Full,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: true,
            max_fps: 60,
            _unknown: Vec::new(),
        },
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &[]).unwrap();

    assert_eq!(result.client_type, "test");
    assert!(matches!(result.capabilities.color_depth, ColorDepth::TrueColor));

    // Verify HelloAck was written
    let ack = decode_hello_ack(&output);
    assert_eq!(ack.negotiated_version, WIRE_VERSION);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test handshake`
Expected: FAIL — module not implemented

- [ ] **Step 3: Implement handshake**

Write `crates/malt-daemon/src/connection/handshake.rs`:
```rust
use crate::DaemonError;
use malt_protocol::common::{ClientCapabilities, SessionInfo};
use malt_protocol::envelope::{decode_envelope, encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck, VersionSkew};
use std::io::{Read, Write};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

const SUPPORTED_WIRE_VERSION: u32 = 1;

/// Result of a successful handshake.
#[derive(Debug)]
pub struct HandshakeResult {
    pub client_type: String,
    pub capabilities: ClientCapabilities,
    pub negotiated_version: u32,
}

/// Perform the server side of the VNP handshake.
///
/// 1. Read Hello from client
/// 2. Validate wire version
/// 3. Send HelloAck with negotiated version and session list
pub fn perform_server_handshake<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    sessions: &[SessionInfo],
) -> Result<HandshakeResult, DaemonError> {
    // Read Hello frame
    let mut frame_reader = FrameReader::new(reader);
    let frame = frame_reader.read_frame()?;
    let (envelope, msg_bytes) = decode_envelope(&frame.payload)
        .map_err(|e| DaemonError::HandshakeFailed(format!("envelope decode: {e}")))?;

    if envelope.domain != 0 || envelope.msg_type != 0x01 {
        return Err(DaemonError::HandshakeFailed(format!(
            "expected Hello (domain=0, type=1), got domain={}, type={}",
            envelope.domain, envelope.msg_type
        )));
    }

    let mut bit_reader = BitReader::new(msg_bytes);
    let hello = Hello::unpack(&mut bit_reader)
        .map_err(|e| DaemonError::HandshakeFailed(format!("Hello decode: {e}")))?;

    // Version negotiation: use the lower of client and server version
    let negotiated = hello.version.min(SUPPORTED_WIRE_VERSION);
    if negotiated == 0 {
        // No compatible version — send VersionSkew and disconnect
        let skew = VersionSkew {
            expected_min: 1,
            expected_max: SUPPORTED_WIRE_VERSION,
            client_version: hello.version,
            reason: "no compatible wire version".to_string(),
            _unknown: Vec::new(),
        };
        let skew_envelope = Envelope {
            wire_version: 0,
            domain: 0,
            msg_type: 0x03,
            session_id: 0,
            timestamp: 0,
            msg_id: None,
            _unknown: Vec::new(),
        };
        let mut w = BitWriter::new();
        skew.pack(&mut w)
            .map_err(|e| DaemonError::HandshakeFailed(format!("VersionSkew encode: {e}")))?;
        let skew_bytes = w.finish();
        let combined = encode_message(&skew_envelope, &skew_bytes);
        let frame = Frame {
            flags: FrameFlags::new(),
            payload: combined,
        };
        FrameWriter::new(writer).write_frame(&frame)?;
        return Err(DaemonError::HandshakeFailed(
            "version skew — no compatible version".to_string(),
        ));
    }

    // Send HelloAck
    let ack = HelloAck {
        negotiated_version: negotiated,
        sessions: sessions.to_vec(),
        start_time_offset: 0,
        _unknown: Vec::new(),
    };
    let ack_envelope = Envelope {
        wire_version: 0,
        domain: 0,
        msg_type: 0x02,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let mut w = BitWriter::new();
    ack.pack(&mut w)
        .map_err(|e| DaemonError::HandshakeFailed(format!("HelloAck encode: {e}")))?;
    let ack_bytes = w.finish();
    let combined = encode_message(&ack_envelope, &ack_bytes);
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    FrameWriter::new(writer).write_frame(&frame)?;

    Ok(HandshakeResult {
        client_type: hello.client_type,
        capabilities: hello.capabilities,
        negotiated_version: negotiated,
    })
}
```

- [ ] **Step 4: Update connection/mod.rs**

Write `crates/malt-daemon/src/connection/mod.rs`:
```rust
pub mod authority;
pub mod handshake;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test handshake`
Expected: all 1 test PASS

- [ ] **Step 6: Write test for version skew**

Append to `crates/malt-daemon/tests/handshake.rs`:
```rust
#[test]
fn version_skew_rejects_incompatible() {
    let hello = Hello {
        version: 0, // Incompatible version
        client_type: "old".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::None,
            unicode: UnicodeLevel::None,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: false,
            max_fps: 30,
            _unknown: Vec::new(),
        },
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    let result = perform_server_handshake(&mut input, &mut output, &[]);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("version skew"), "got: {err}");
}

#[test]
fn handshake_includes_session_list() {
    let sessions = vec![SessionInfo {
        session_id: malt_protocol::common::SessionId(1),
        name: Some("main".to_string()),
        pane_count: 2,
        isolation: malt_protocol::common::IsolationTier::Bare,
        state: malt_protocol::common::SessionState::Active,
        _unknown: Vec::new(),
    }];

    let hello = Hello {
        version: WIRE_VERSION,
        client_type: "test".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::TrueColor,
            unicode: UnicodeLevel::Full,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: true,
            max_fps: 60,
            _unknown: Vec::new(),
        },
        _unknown: Vec::new(),
    };

    let hello_bytes = encode_hello(&hello);
    let mut input = Cursor::new(hello_bytes);
    let mut output = Vec::new();

    perform_server_handshake(&mut input, &mut output, &sessions).unwrap();

    let ack = decode_hello_ack(&output);
    assert_eq!(ack.sessions.len(), 1);
    assert_eq!(ack.sessions[0].name, Some("main".to_string()));
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p malt-daemon --test handshake`
Expected: all 3 tests PASS

- [ ] **Step 8: Commit**

```bash
git add crates/malt-daemon/src/connection/ crates/malt-daemon/tests/handshake.rs
git commit -m "feat(malt-daemon): VNP handshake — Hello/HelloAck, version negotiation"
```

---

### Task 9: Executor Module Re-exports and Integration Wiring

**Files:**
- Modify: `crates/malt-daemon/src/executor/mod.rs`
- Modify: `crates/malt-daemon/src/lib.rs`

- [ ] **Step 1: Update executor/mod.rs with clean re-exports**

Write `crates/malt-daemon/src/executor/mod.rs`:
```rust
pub mod coordinator;
pub mod pools;
pub mod session_thread;

pub use coordinator::Coordinator;
pub use pools::PoolConfig;
pub use session_thread::{SessionCommand, SessionExecutor};
```

- [ ] **Step 2: Update lib.rs with clean re-exports**

Write `crates/malt-daemon/src/lib.rs`:
```rust
pub mod bus;
pub mod connection;
pub mod error;
pub mod executor;

pub use error::DaemonError;
pub use executor::{Coordinator, PoolConfig, SessionCommand, SessionExecutor};
```

- [ ] **Step 3: Verify all tests still pass**

Run: `cargo test -p malt-daemon`
Expected: all tests PASS (ring_buffer: 9, flow_control: 6, bus: 7, authority: 8, session_thread: 4, coordinator: 8, handshake: 3 = 45 total)

- [ ] **Step 4: Commit**

```bash
git add crates/malt-daemon/src/lib.rs crates/malt-daemon/src/executor/mod.rs
git commit -m "feat(malt-daemon): clean re-exports for public API"
```

---

### Task 10: Final Verification

- [ ] **Step 1: Run full workspace check**

Run: `cargo check --workspace`
Expected: compiles with no errors

- [ ] **Step 2: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all workspace tests PASS (635 existing + 45 new = 680 total)

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p malt-daemon -- -D warnings`
Expected: no warnings

- [ ] **Step 4: Fix any clippy issues if found**

Address any clippy warnings in the malt-daemon crate.

- [ ] **Step 5: Commit fixes if any**

```bash
git add -u
git commit -m "fix(malt-daemon): clippy fixes"
```
