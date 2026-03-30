use malt_compat::cell::default_style;
use malt_compat::grid::TerminalGrid;
use malt_compat::parser::GridPerformer;
use vte::Parser;

fn feed(grid: &mut TerminalGrid, data: &[u8]) {
    let mut parser = Parser::new();
    let mut performer = GridPerformer::new(grid);
    parser.advance(&mut performer, data);
}

#[test]
fn plain_text_writes_chars() {
    let mut grid = TerminalGrid::new(80, 24);
    feed(&mut grid, b"Hello");
    assert_eq!(grid.cell(0, 0).ch, 'H');
    assert_eq!(grid.cell(0, 1).ch, 'e');
    assert_eq!(grid.cell(0, 2).ch, 'l');
    assert_eq!(grid.cell(0, 3).ch, 'l');
    assert_eq!(grid.cell(0, 4).ch, 'o');
    assert_eq!(grid.cursor_col(), 5);
}

#[test]
fn sgr_sets_foreground() {
    let mut grid = TerminalGrid::new(80, 24);
    feed(&mut grid, b"\x1b[31mX");
    let cell = grid.cell(0, 0);
    assert_ne!(cell.style.fg, default_style().fg, "foreground should differ from default after SGR 31");
}

#[test]
fn sgr_sets_bold() {
    let mut grid = TerminalGrid::new(80, 24);
    feed(&mut grid, b"\x1b[1mB");
    let cell = grid.cell(0, 0);
    assert!(cell.style.bold, "cell should be bold after SGR 1");
}

#[test]
fn cursor_up_sequence() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.move_cursor_to(5, 0);
    feed(&mut grid, b"\x1b[2A");
    assert_eq!(grid.cursor_row(), 3);
}

#[test]
fn cursor_position_sequence() {
    let mut grid = TerminalGrid::new(80, 24);
    feed(&mut grid, b"\x1b[10;20H");
    assert_eq!(grid.cursor_row(), 9, "CUP row is 1-indexed, so 10 -> row 9");
    assert_eq!(grid.cursor_col(), 19, "CUP col is 1-indexed, so 20 -> col 19");
}

#[test]
fn unknown_sequence_ignored() {
    let mut grid = TerminalGrid::new(80, 24);
    feed(&mut grid, b"\x1b[?9999h");
    feed(&mut grid, b"OK");
    assert_eq!(grid.cell(0, 0).ch, 'O');
    assert_eq!(grid.cell(0, 1).ch, 'K');
}
