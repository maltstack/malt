use malt_protocol::common::ResolvedStyle;
use ratatui::style::{Color, Modifier, Style};

/// Convert a MALT ResolvedStyle to a ratatui Style.
pub fn to_ratatui_style(resolved: &ResolvedStyle) -> Style {
    let mut style = Style::default()
        .fg(Color::Rgb(resolved.fg.0, resolved.fg.1, resolved.fg.2))
        .bg(Color::Rgb(resolved.bg.0, resolved.bg.1, resolved.bg.2));

    let mut modifiers = Modifier::empty();
    if resolved.bold {
        modifiers |= Modifier::BOLD;
    }
    if resolved.italic {
        modifiers |= Modifier::ITALIC;
    }
    if resolved.underline {
        modifiers |= Modifier::UNDERLINED;
    }
    if resolved.dim {
        modifiers |= Modifier::DIM;
    }
    if resolved.strikethrough {
        modifiers |= Modifier::CROSSED_OUT;
    }
    if resolved.reverse {
        modifiers |= Modifier::REVERSED;
    }
    if resolved.blink {
        modifiers |= Modifier::SLOW_BLINK;
    }

    style = style.add_modifier(modifiers);
    style
}
