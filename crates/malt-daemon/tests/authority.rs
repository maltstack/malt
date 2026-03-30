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
