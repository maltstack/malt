//! Translates VNP KeyEvent messages into malt-term InputEvent values.
//!
//! This is the only place in malt-daemon that depends on the malt-term
//! input representation. The session executor uses this to feed the
//! line editor.

use malt_protocol::common::KeyModifiers;
use malt_protocol::input::{KeyEvent, KeyValue, NamedKey};
use malt_term::{InputEvent, SpecialKey};

/// Translate a VNP [`KeyEvent`] into a malt-term [`InputEvent`].
///
/// Returns `None` for key values that have no line-editor mapping
/// (function keys, unknown keys, etc.).
pub fn vnp_key_to_input_event(key: &KeyEvent) -> Option<InputEvent> {
    match &key.key {
        KeyValue::Char { codepoint } => {
            let ch = char::from_u32(*codepoint)?;
            // When both ALT and CTRL are set, ALT takes precedence.
            // malt-term has no combined modifier variant.
            if key.modifiers.contains(KeyModifiers::ALT) {
                Some(InputEvent::Alt(ch))
            } else if key.modifiers.contains(KeyModifiers::CTRL) {
                // SAFETY: char::to_lowercase() always yields at least one char.
                Some(InputEvent::Ctrl(ch.to_lowercase().next().unwrap_or(ch)))
            } else {
                Some(InputEvent::Char(ch))
            }
        }
        KeyValue::Named { key: named } => {
            let special = match named {
                NamedKey::Enter     => SpecialKey::Enter,
                NamedKey::Escape    => SpecialKey::Escape,
                NamedKey::Tab       => {
                    // BackTab is Shift+Tab; must be decided before returning Key(Tab).
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        return Some(InputEvent::Key(SpecialKey::BackTab));
                    }
                    SpecialKey::Tab
                }
                NamedKey::Backspace => SpecialKey::Backspace,
                NamedKey::Delete    => SpecialKey::Delete,
                NamedKey::Home      => SpecialKey::Home,
                NamedKey::End       => SpecialKey::End,
                NamedKey::Up        => SpecialKey::Up,
                NamedKey::Down      => SpecialKey::Down,
                NamedKey::Left      => SpecialKey::Left,
                NamedKey::Right     => SpecialKey::Right,
                NamedKey::Insert    => return None, // no Insert in malt-term SpecialKey
                NamedKey::PageUp | NamedKey::PageDown => return None,
                NamedKey::Unknown(_) => return None,
                _ => return None, // future NamedKey variants
            };
            Some(InputEvent::Key(special))
        }
        KeyValue::Function { .. } => None,
        KeyValue::Unknown { .. } => None,
        _ => None, // future KeyValue variants
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_key_returns_none() {
        let event = KeyEvent {
            key: KeyValue::Unknown { discriminant: 0xdeadbeef, data: Vec::new() },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), None);
    }

    #[test]
    fn invalid_codepoint_returns_none() {
        let event = KeyEvent {
            key: KeyValue::Char { codepoint: 0xffffffff },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), None);
    }

    #[test]
    fn shift_tab_maps_to_backtab() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::Tab },
            modifiers: KeyModifiers::SHIFT,
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Key(SpecialKey::BackTab)));
    }

    #[test]
    fn insert_key_returns_none() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::Insert },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), None);
    }

    #[test]
    fn pageup_returns_none() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::PageUp },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), None);
    }

    #[test]
    fn pagedown_returns_none() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::PageDown },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), None);
    }

    #[test]
    fn alt_char_maps_to_alt_event() {
        let event = KeyEvent {
            key: KeyValue::Char { codepoint: 'x' as u32 },
            modifiers: KeyModifiers::ALT,
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Alt('x')));
    }

    #[test]
    fn ctrl_lowercase_mapping() {
        // Ctrl-C (uppercase 'C') should map to Ctrl('c')
        let event = KeyEvent {
            key: KeyValue::Char { codepoint: 'C' as u32 },
            modifiers: KeyModifiers::CTRL,
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Ctrl('c')));
    }

    #[test]
    fn escape_key_maps_correctly() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::Escape },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Key(SpecialKey::Escape)));
    }

    #[test]
    fn home_key_maps_correctly() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::Home },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Key(SpecialKey::Home)));
    }

    #[test]
    fn end_key_maps_correctly() {
        let event = KeyEvent {
            key: KeyValue::Named { key: NamedKey::End },
            modifiers: KeyModifiers::empty(),
            _unknown: Vec::new(),
        };
        assert_eq!(vnp_key_to_input_event(&event), Some(InputEvent::Key(SpecialKey::End)));
    }
}
