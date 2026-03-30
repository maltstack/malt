use malt_protocol::common::{
    InputAuthority, IsolationTier, PaneId, SessionId, SessionState,
};
use malt_session::session::SessionRuntime;

fn make_session() -> SessionRuntime {
    SessionRuntime::new(SessionId(1), PaneId(1), IsolationTier::Bare)
}

#[test]
fn new_session_is_active() {
    let s = make_session();
    assert_eq!(s.state(), SessionState::Active);
    assert_eq!(s.client_count(), 0);
}

#[test]
fn attach_adds_client() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();

    assert_eq!(s.client_count(), 1);
    assert_eq!(s.attached_clients(), &[100]);
    assert_eq!(s.input_holder(), Some(100));
}

#[test]
fn detach_last_client_goes_dormant() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.detach(100).unwrap();

    assert_eq!(s.state(), SessionState::Dormant);
    assert_eq!(s.client_count(), 0);
    assert_eq!(s.input_holder(), None);
}

#[test]
fn attach_to_dormant_goes_active() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.detach(100).unwrap();
    assert_eq!(s.state(), SessionState::Dormant);

    s.attach(200, InputAuthority::Shared).unwrap();
    assert_eq!(s.state(), SessionState::Active);
    assert_eq!(s.input_holder(), Some(200));
}

#[test]
fn destroy_from_active() {
    let mut s = make_session();
    s.destroy().unwrap();
    assert_eq!(s.state(), SessionState::Destroyed);
}

#[test]
fn destroy_from_dormant() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.detach(100).unwrap();
    assert_eq!(s.state(), SessionState::Dormant);

    s.destroy().unwrap();
    assert_eq!(s.state(), SessionState::Destroyed);
}

#[test]
fn destroy_from_destroyed_is_error() {
    let mut s = make_session();
    s.destroy().unwrap();

    let err = s.destroy().unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("invalid transition"), "got: {msg}");
}

#[test]
fn checkpoint_only_from_active() {
    let mut s = make_session();
    s.checkpoint().unwrap();
    assert_eq!(s.state(), SessionState::Checkpoint);

    // Restore and try from dormant — should fail
    s.restore().unwrap();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.detach(100).unwrap();
    assert_eq!(s.state(), SessionState::Dormant);

    let err = s.checkpoint().unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("invalid transition"), "got: {msg}");
}

#[test]
fn input_transfers_on_detach() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.attach(200, InputAuthority::Shared).unwrap();

    assert_eq!(s.input_holder(), Some(100));

    s.detach(100).unwrap();
    assert_eq!(s.input_holder(), Some(200));
    assert_eq!(s.state(), SessionState::Active);
}

#[test]
fn claim_input_transfers_to_new_client() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();
    s.attach(200, InputAuthority::Shared).unwrap();

    s.claim_input(200, InputAuthority::Exclusive).unwrap();
    assert_eq!(s.input_holder(), Some(200));
    assert_eq!(s.input_authority(), InputAuthority::Exclusive);
}

#[test]
fn claim_input_requires_attached() {
    let mut s = make_session();
    let err = s.claim_input(999, InputAuthority::Exclusive).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("not attached"), "got: {msg}");
}

#[test]
fn restore_from_checkpoint() {
    let mut s = make_session();
    s.checkpoint().unwrap();
    assert_eq!(s.state(), SessionState::Checkpoint);

    s.restore().unwrap();
    assert_eq!(s.state(), SessionState::Active);
}

#[test]
fn restore_from_active_is_error() {
    let mut s = make_session();
    let err = s.restore().unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("invalid transition"), "got: {msg}");
}

#[test]
fn add_and_remove_pane() {
    let mut s = make_session();
    s.add_pane(PaneId(2));
    assert_eq!(s.panes().len(), 2);

    s.remove_pane(PaneId(2)).unwrap();
    assert_eq!(s.panes().len(), 1);
}

#[test]
fn set_focused_requires_membership() {
    let mut s = make_session();
    let err = s.set_focused(PaneId(999)).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("not found"), "got: {msg}");
}

#[test]
fn to_info_reflects_state() {
    let s = make_session();
    let info = s.to_info();
    assert_eq!(info.session_id, SessionId(1));
    assert_eq!(info.pane_count, 1);
    assert_eq!(info.state, SessionState::Active);
}

#[test]
fn attach_to_destroyed_is_error() {
    let mut s = make_session();
    s.destroy().unwrap();

    let err = s.attach(100, InputAuthority::Exclusive).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("destroyed"), "got: {msg}");
}

#[test]
fn duplicate_attach_is_error() {
    let mut s = make_session();
    s.attach(100, InputAuthority::Exclusive).unwrap();

    let err = s.attach(100, InputAuthority::Shared).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("already attached"), "got: {msg}");
}
