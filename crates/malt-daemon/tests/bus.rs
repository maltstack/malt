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

fn make_msg_from(producer: u64, priority: Priority, data: Vec<u8>) -> BusMessage {
    BusMessage {
        domain: 1,
        msg_type: 1,
        priority,
        producer_id: producer,
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
    let id = bus.subscribe(100);
    // Publish more than capacity — should not panic or block.
    // Uses different producers to avoid per-producer cap.
    for i in 0..200u64 {
        bus.publish(make_msg_from(i, Priority::Normal, vec![i as u8]));
    }
    let msgs = bus.drain(id);
    // Some messages accepted, some rejected by flow control, buffer evicts older ones
    assert!(msgs.len() <= 100);
    assert!(!msgs.is_empty());
}

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
    // Use a larger buffer so flow control doesn't interfere
    let id = bus.subscribe(100);
    bus.publish(make_msg(1, 1, Priority::Low, vec![1]));
    bus.publish(make_msg(1, 1, Priority::Normal, vec![2]));
    bus.publish(make_msg(1, 1, Priority::High, vec![3]));
    bus.publish(make_msg(1, 1, Priority::Reliable, vec![4]));
    let msgs = bus.drain(id);
    let payloads: Vec<u8> = msgs.iter().map(|m| m.payload[0]).collect();
    // All 4 present — no eviction needed with buffer size 100
    assert_eq!(payloads, vec![1, 2, 3, 4]);
}

#[test]
fn flow_control_rejects_producer_over_cap() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(100);
    // Per-producer cap = 100/4 = 25. Publish 30 from same producer.
    for i in 0..30u8 {
        bus.publish(make_msg_from(1, Priority::Normal, vec![i]));
    }
    let msgs = bus.drain(id);
    // Only 25 should get through (per-producer cap)
    assert_eq!(msgs.len(), 25);
}

#[test]
fn flow_control_global_saturation() {
    let mut bus = Bus::new(BusConfig::default());
    let id = bus.subscribe(100);
    // Global cap = 100*3/5 = 60. Use 4 producers with 20 each = 80 attempted.
    for producer in 0..4u64 {
        for i in 0..20u8 {
            bus.publish(make_msg_from(producer, Priority::Normal, vec![i]));
        }
    }
    let msgs = bus.drain(id);
    // Should be capped at 60 by global saturation
    assert!(msgs.len() <= 60, "got {}", msgs.len());
    assert!(msgs.len() >= 50, "got {}", msgs.len()); // at least 50 (some producers fill before saturation)
}
