use malt_protocol::common::ResolvedStyle;
use malt_tui::style::to_ratatui_style;
use ratatui::style::{Color, Modifier};

fn make_style(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> ResolvedStyle {
    ResolvedStyle {
        fg,
        bg,
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        _unknown: Vec::new(),
    }
}

#[test]
fn converts_colors() {
    let resolved = make_style((255, 128, 0), (0, 0, 0));
    let style = to_ratatui_style(&resolved);
    assert_eq!(style.fg, Some(Color::Rgb(255, 128, 0)));
    assert_eq!(style.bg, Some(Color::Rgb(0, 0, 0)));
}

#[test]
fn converts_bold() {
    let mut resolved = make_style((255, 255, 255), (0, 0, 0));
    resolved.bold = true;
    let style = to_ratatui_style(&resolved);
    assert!(style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn converts_multiple_modifiers() {
    let mut resolved = make_style((255, 255, 255), (0, 0, 0));
    resolved.bold = true;
    resolved.italic = true;
    let style = to_ratatui_style(&resolved);
    assert!(style.add_modifier.contains(Modifier::BOLD));
    assert!(style.add_modifier.contains(Modifier::ITALIC));
}

#[test]
fn default_style_no_modifiers() {
    let resolved = make_style((204, 204, 204), (0, 0, 0));
    let style = to_ratatui_style(&resolved);
    assert_eq!(style.add_modifier, Modifier::empty());
}
