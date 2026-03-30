use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_renderer::client_state::{AckStatus, ClientState};

fn full_caps() -> ClientCapabilities {
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}

#[test]
fn new_client_starts_at_seq_zero() {
    let cs = ClientState::new(1, full_caps());
    assert_eq!(cs.frame_seq(), 0);
    assert_eq!(cs.unacked_count(), 0);
}

#[test]
fn advance_seq_increments() {
    let mut cs = ClientState::new(1, full_caps());
    let seq = cs.advance_seq();
    assert_eq!(seq, 1);
    assert_eq!(cs.frame_seq(), 1);
    assert_eq!(cs.unacked_count(), 1);
}

#[test]
fn ack_reduces_unacked() {
    let mut cs = ClientState::new(1, full_caps());
    cs.advance_seq();
    cs.advance_seq();
    cs.ack(2);
    assert_eq!(cs.unacked_count(), 0);
}

#[test]
fn ack_partial() {
    let mut cs = ClientState::new(1, full_caps());
    cs.advance_seq();
    cs.advance_seq();
    cs.advance_seq();
    cs.ack(2);
    assert_eq!(cs.unacked_count(), 1);
}

#[test]
fn lagging_at_30_unacked() {
    let mut cs = ClientState::new(1, full_caps());
    for _ in 0..30 {
        cs.advance_seq();
    }
    assert_eq!(cs.check_status(), AckStatus::Lagging);
}

#[test]
fn not_lagging_below_30() {
    let mut cs = ClientState::new(1, full_caps());
    for _ in 0..29 {
        cs.advance_seq();
    }
    assert_eq!(cs.check_status(), AckStatus::Ok);
}

#[test]
fn ack_clears_lagging() {
    let mut cs = ClientState::new(1, full_caps());
    for _ in 0..35 {
        cs.advance_seq();
    }
    assert_eq!(cs.check_status(), AckStatus::Lagging);
    cs.ack(35);
    assert_eq!(cs.check_status(), AckStatus::Ok);
}

#[test]
fn shed_after_timeout() {
    let mut cs = ClientState::new(1, full_caps());
    cs.advance_seq();
    assert!(!cs.should_shed(9_999));
    assert!(cs.should_shed(10_000));
}
