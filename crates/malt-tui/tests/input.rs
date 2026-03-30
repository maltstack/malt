use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use malt_protocol::common::KeyModifiers as VnpModifiers;
use malt_protocol::input::{KeyValue, NamedKey};
use malt_tui::input::{convert_key_event, convert_resize};

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn char_key_mapped() {
    let ct_event = make_key(KeyCode::Char('a'), KeyModifiers::empty());
    let vnp = convert_key_event(&ct_event).unwrap();
    match vnp.key {
        KeyValue::Char { codepoint } => assert_eq!(codepoint, 'a' as u32),
        other => panic!("expected Char, got {other:?}"),
    }
    assert_eq!(vnp.modifiers, VnpModifiers::empty());
}

#[test]
fn named_key_mapped() {
    let ct_event = make_key(KeyCode::Enter, KeyModifiers::empty());
    let vnp = convert_key_event(&ct_event).unwrap();
    match vnp.key {
        KeyValue::Named { key } => assert!(matches!(key, NamedKey::Enter)),
        other => panic!("expected Named, got {other:?}"),
    }
}

#[test]
fn modifier_keys_mapped() {
    let ct_event = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    let vnp = convert_key_event(&ct_event).unwrap();
    match vnp.key {
        KeyValue::Char { codepoint } => assert_eq!(codepoint, 'c' as u32),
        other => panic!("expected Char, got {other:?}"),
    }
    assert!(vnp.modifiers.contains(VnpModifiers::CTRL));
    assert!(!vnp.modifiers.contains(VnpModifiers::SHIFT));
    assert!(!vnp.modifiers.contains(VnpModifiers::ALT));
}

#[test]
fn resize_mapped() {
    let resize = convert_resize(120, 40);
    assert_eq!(resize.cols, 120);
    assert_eq!(resize.rows, 40);
}
