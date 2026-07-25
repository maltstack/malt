// Input module: crossterm event to VNP input message conversion.

use crossterm::event as ct;
use malt_protocol::common::KeyModifiers as VnpModifiers;
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};

/// Convert a crossterm key event to a VNP KeyEvent.
/// Returns `None` for key codes that have no VNP mapping.
pub fn convert_key_event(event: &ct::KeyEvent) -> Option<KeyEvent> {
    let key = convert_key_code(event.code)?;
    let modifiers = convert_modifiers(event.modifiers);
    Some(KeyEvent {
        key,
        modifiers,
        _unknown: Vec::new(),
    })
}

fn convert_key_code(code: ct::KeyCode) -> Option<KeyValue> {
    match code {
        ct::KeyCode::Char(c) => Some(KeyValue::Char {
            codepoint: c as u32,
        }),
        ct::KeyCode::F(n) => Some(KeyValue::Function { number: n }),
        ct::KeyCode::Enter => Some(KeyValue::Named {
            key: NamedKey::Enter,
        }),
        ct::KeyCode::Esc => Some(KeyValue::Named {
            key: NamedKey::Escape,
        }),
        ct::KeyCode::Tab => Some(KeyValue::Named { key: NamedKey::Tab }),
        ct::KeyCode::Backspace => Some(KeyValue::Named {
            key: NamedKey::Backspace,
        }),
        ct::KeyCode::Insert => Some(KeyValue::Named {
            key: NamedKey::Insert,
        }),
        ct::KeyCode::Delete => Some(KeyValue::Named {
            key: NamedKey::Delete,
        }),
        ct::KeyCode::Home => Some(KeyValue::Named {
            key: NamedKey::Home,
        }),
        ct::KeyCode::End => Some(KeyValue::Named { key: NamedKey::End }),
        ct::KeyCode::PageUp => Some(KeyValue::Named {
            key: NamedKey::PageUp,
        }),
        ct::KeyCode::PageDown => Some(KeyValue::Named {
            key: NamedKey::PageDown,
        }),
        ct::KeyCode::Up => Some(KeyValue::Named { key: NamedKey::Up }),
        ct::KeyCode::Down => Some(KeyValue::Named {
            key: NamedKey::Down,
        }),
        ct::KeyCode::Left => Some(KeyValue::Named {
            key: NamedKey::Left,
        }),
        ct::KeyCode::Right => Some(KeyValue::Named {
            key: NamedKey::Right,
        }),
        _ => None,
    }
}

fn convert_modifiers(ct_mods: ct::KeyModifiers) -> VnpModifiers {
    let mut vnp = VnpModifiers::empty();
    if ct_mods.contains(ct::KeyModifiers::SHIFT) {
        vnp = vnp | VnpModifiers::SHIFT;
    }
    if ct_mods.contains(ct::KeyModifiers::CONTROL) {
        vnp = vnp | VnpModifiers::CTRL;
    }
    if ct_mods.contains(ct::KeyModifiers::ALT) {
        vnp = vnp | VnpModifiers::ALT;
    }
    vnp
}
