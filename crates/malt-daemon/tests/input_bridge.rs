use malt_daemon::input_bridge::vnp_key_to_input_event;
use malt_protocol::common::KeyModifiers;
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};
use malt_term::{InputEvent, SpecialKey};

fn key(value: KeyValue) -> KeyEvent {
    KeyEvent { key: value, modifiers: KeyModifiers::empty(), _unknown: Vec::new() }
}

fn key_with_ctrl(value: KeyValue) -> KeyEvent {
    KeyEvent { key: value, modifiers: KeyModifiers::CTRL, _unknown: Vec::new() }
}

#[test]
fn printable_char_maps_to_char_event() {
    let event = key(KeyValue::Char { codepoint: 'a' as u32 });
    assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Char('a')));
}

#[test]
fn ctrl_char_maps_to_ctrl_event() {
    let event = key_with_ctrl(KeyValue::Char { codepoint: 'c' as u32 });
    assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Ctrl('c')));
}

#[test]
fn enter_maps_to_special_enter() {
    let event = key(KeyValue::Named { key: NamedKey::Enter });
    assert_eq!(
        vnp_key_to_input_event(&event),
        Some(InputEvent::Key(SpecialKey::Enter))
    );
}

#[test]
fn backspace_maps_to_special_backspace() {
    let event = key(KeyValue::Named { key: NamedKey::Backspace });
    assert_eq!(
        vnp_key_to_input_event(&event),
        Some(InputEvent::Key(SpecialKey::Backspace))
    );
}

#[test]
fn arrow_keys_map_to_special_keys() {
    assert_eq!(
        vnp_key_to_input_event(&key(KeyValue::Named { key: NamedKey::Up })),
        Some(InputEvent::Key(SpecialKey::Up))
    );
    assert_eq!(
        vnp_key_to_input_event(&key(KeyValue::Named { key: NamedKey::Down })),
        Some(InputEvent::Key(SpecialKey::Down))
    );
    assert_eq!(
        vnp_key_to_input_event(&key(KeyValue::Named { key: NamedKey::Left })),
        Some(InputEvent::Key(SpecialKey::Left))
    );
    assert_eq!(
        vnp_key_to_input_event(&key(KeyValue::Named { key: NamedKey::Right })),
        Some(InputEvent::Key(SpecialKey::Right))
    );
}

#[test]
fn function_key_returns_none() {
    let event = key(KeyValue::Function { number: 1 });
    assert_eq!(vnp_key_to_input_event(&event), None);
}
