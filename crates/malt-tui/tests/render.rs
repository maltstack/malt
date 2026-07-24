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
        token_name: None,
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
fn write_raw_renders_plain_text() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    let cmds = vec![RenderCommand::WriteRaw {
        data: b"hello".to_vec(),
        x: 2,
        y: 3,
        width: 20,
        height: 5,
    }];
    renderer.apply(&cmds, &mut buf);

    let cell = buf.cell(Position::new(2, 3)).unwrap();
    assert_eq!(
        cell.symbol(),
        "h",
        "WriteRaw must actually render VT bytes to the buffer, not be \
         silently dropped"
    );
    let cell = buf.cell(Position::new(6, 3)).unwrap();
    assert_eq!(cell.symbol(), "o");
}

#[test]
fn write_raw_renders_sgr_styled_text() {
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    // ESC[1m = bold, ESC[31m = red foreground (256/16-color, but the VT
    // parser should still register bold via SGR regardless of the exact
    // color mapping malt-compat applies for the basic palette).
    let cmds = vec![RenderCommand::WriteRaw {
        data: b"\x1b[1mBOLD".to_vec(),
        x: 0,
        y: 0,
        width: 20,
        height: 5,
    }];
    renderer.apply(&cmds, &mut buf);

    let cell = buf.cell(Position::new(0, 0)).unwrap();
    assert_eq!(cell.symbol(), "B");
    assert!(
        cell.modifier.contains(ratatui::style::Modifier::BOLD),
        "SGR bold (ESC[1m) must survive through CompatTranslator -> \
         TerminalGrid -> ratatui Style, not just plain characters"
    );
}

#[test]
fn write_raw_accumulates_across_multiple_feeds_without_resetting() {
    // Two WriteRaw commands in the same frame batch (or across frames) --
    // the second must not wipe out content the first one placed elsewhere
    // on the grid, since CompatTranslator's grid is persistent state, not
    // reset per feed() call (that was the P0 staircase bug's fix).
    let mut renderer = TuiRenderer::new();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    renderer.apply(
        &[RenderCommand::WriteRaw {
            data: b"line one\r\n".to_vec(),
            x: 0,
            y: 0,
            width: 40,
            height: 10,
        }],
        &mut buf,
    );
    renderer.apply(
        &[RenderCommand::WriteRaw {
            data: b"line two".to_vec(),
            x: 0,
            y: 0,
            width: 40,
            height: 10,
        }],
        &mut buf,
    );

    let first_line_cell = buf.cell(Position::new(0, 0)).unwrap();
    assert_eq!(
        first_line_cell.symbol(),
        "l",
        "first line's content must still be present after a second WriteRaw \
         command feeds more bytes into the same persistent grid"
    );
    let second_line_cell = buf.cell(Position::new(0, 1)).unwrap();
    assert_eq!(second_line_cell.symbol(), "l");
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
