use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

/// Map a winit key event to a string suitable for sending to the daemon.
///
/// Returns `None` for keys that have no textual representation (e.g. Shift alone).
pub fn map_key(event: &KeyEvent) -> Option<String> {
    match &event.logical_key {
        Key::Character(c) => Some(c.to_string()),
        Key::Named(named) => map_named(*named),
        _ => None,
    }
}

fn map_named(key: NamedKey) -> Option<String> {
    let s = match key {
        NamedKey::Enter => "\r",
        NamedKey::Backspace => "\x08",
        NamedKey::Tab => "\t",
        NamedKey::Escape => "\x1b",
        NamedKey::ArrowUp => "\x1b[A",
        NamedKey::ArrowDown => "\x1b[B",
        NamedKey::ArrowRight => "\x1b[C",
        NamedKey::ArrowLeft => "\x1b[D",
        NamedKey::Home => "\x1b[H",
        NamedKey::End => "\x1b[F",
        NamedKey::PageUp => "\x1b[5~",
        NamedKey::PageDown => "\x1b[6~",
        NamedKey::Insert => "\x1b[2~",
        NamedKey::Delete => "\x1b[3~",
        _ => return None,
    };
    Some(s.to_string())
}
