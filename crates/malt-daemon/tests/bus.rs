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
