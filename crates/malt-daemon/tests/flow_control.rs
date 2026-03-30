use malt_daemon::bus::flow_control::FlowController;

#[test]
fn new_producer_allowed() {
    let mut fc = FlowController::new(100);
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
    // Three producers each at 20 — total 60 = 60% global cap
    for _ in 0..20 {
        fc.try_publish(1);
    }
    for _ in 0..20 {
        fc.try_publish(2);
    }
    for _ in 0..20 {
        fc.try_publish(3);
    }
    // Total is 60, which hits 60% global saturation
    assert!(!fc.try_publish(4));
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
