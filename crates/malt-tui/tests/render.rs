use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;
use malt_tui::render::TuiRenderer;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

fn default_style() -> ResolvedStyle {
    ResolvedStyle {
        fg: (204, 204, 204),
        bg: (0, 0, 0),
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
fn draw_text_writes_to_buffer() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    let cmds = vec![RenderCommand::DrawText {
        x: 0,
        y: 0,
        text: "hello".to_string(),
        style: default_style(),
    }];
    renderer.apply(&cmds, &mut buf);
    let cell = buf.cell(Position::new(0, 0)).unwrap();
    assert_eq!(cell.symbol(), "h");
    let cell = buf.cell(Position::new(4, 0)).unwrap();
    assert_eq!(cell.symbol(), "o");
}

#[test]
fn draw_rect_fills_area() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
    let cmds = vec![RenderCommand::DrawRect {
        x: 1,
        y: 1,
        width: 3,
        height: 2,
        style: default_style(),
    }];
    renderer.apply(&cmds, &mut buf);
    // Cell inside rect should be space with applied style
    let cell = buf.cell(Position::new(2, 1)).unwrap();
    assert_eq!(cell.symbol(), " ");
    // Cell outside rect should be untouched (also space in empty buffer,
    // but we can verify the style difference)
    let inside = buf.cell(Position::new(1, 1)).unwrap();
    let outside = buf.cell(Position::new(0, 0)).unwrap();
    assert_ne!(inside.fg, outside.fg);
}

#[test]
fn clear_resets_buffer() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
    renderer.apply(
        &[
            RenderCommand::DrawText {
                x: 0,
                y: 0,
                text: "test".to_string(),
                style: default_style(),
            },
            RenderCommand::Clear {},
        ],
        &mut buf,
    );
    let cell = buf.cell(Position::new(0, 0)).unwrap();
    assert_eq!(cell.symbol(), " ");
}

#[test]
fn multiple_commands_applied_in_order() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    renderer.apply(
        &[
            RenderCommand::DrawText {
                x: 0,
                y: 0,
                text: "first".to_string(),
                style: default_style(),
            },
            RenderCommand::DrawText {
                x: 0,
                y: 1,
                text: "second".to_string(),
                style: default_style(),
            },
        ],
        &mut buf,
    );
    let cell0 = buf.cell(Position::new(0, 0)).unwrap();
    assert_eq!(cell0.symbol(), "f");
    let cell1 = buf.cell(Position::new(0, 1)).unwrap();
    assert_eq!(cell1.symbol(), "s");
}
