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
