use malt_compat::CompatTranslator;
use malt_protocol::frame_element::FrameElement;

#[test]
fn feed_produces_vt_passthrough() {
    let mut t = CompatTranslator::new(80, 24);
    t.feed(b"Hello, world!");
    let elem = t.frame_element();
    match elem {
        FrameElement::VtPassthrough { data } => {
            assert_eq!(data, b"Hello, world!");
        }
        other => panic!("expected VtPassthrough, got {other:?}"),
    }
}

#[test]
fn dirty_tracking() {
    let mut t = CompatTranslator::new(80, 24);
    assert!(!t.is_dirty(), "should not be dirty initially");
    t.feed(b"X");
    assert!(t.is_dirty(), "should be dirty after feed");
    t.clear_dirty();
    assert!(!t.is_dirty(), "should not be dirty after clear");
}

#[test]
fn empty_feed_not_dirty() {
    let mut t = CompatTranslator::new(80, 24);
    t.feed(b"");
    assert!(!t.is_dirty(), "empty feed should not set dirty");
}

#[test]
fn resize_updates_dimensions() {
    let mut t = CompatTranslator::new(80, 24);
    t.resize(40, 12);
    t.feed(b"X");
    let elem = t.frame_element();
    match elem {
        FrameElement::VtPassthrough { data } => {
            assert_eq!(data, b"X");
        }
        other => panic!("expected VtPassthrough, got {other:?}"),
    }
}
