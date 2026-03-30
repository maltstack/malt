use malt_compat::grid::TerminalGrid;

#[test]
fn new_grid_has_correct_dimensions() {
    let grid = TerminalGrid::new(80, 24);
    assert_eq!(grid.cols(), 80);
    assert_eq!(grid.rows(), 24);
    assert_eq!(grid.cursor_row(), 0);
    assert_eq!(grid.cursor_col(), 0);
}

#[test]
fn put_char_advances_cursor() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.put_char('A');
    assert_eq!(grid.cell(0, 0).ch, 'A');
    assert_eq!(grid.cursor_col(), 1);
    assert_eq!(grid.cursor_row(), 0);
}

#[test]
fn cursor_wraps_at_line_end() {
    let mut grid = TerminalGrid::new(5, 3);
    for ch in "hello!".chars() {
        grid.put_char(ch);
    }
    // "hello" fills row 0 (cols 0-4), '!' wraps to row 1 col 0
    assert_eq!(grid.cell(0, 0).ch, 'h');
    assert_eq!(grid.cell(0, 4).ch, 'o');
    assert_eq!(grid.cell(1, 0).ch, '!');
    assert_eq!(grid.cursor_row(), 1);
    assert_eq!(grid.cursor_col(), 1);
}

#[test]
fn cursor_move_absolute() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.move_cursor_to(5, 10);
    assert_eq!(grid.cursor_row(), 5);
    assert_eq!(grid.cursor_col(), 10);
}

#[test]
fn cursor_move_clamped_to_bounds() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.move_cursor_to(100, 200);
    assert_eq!(grid.cursor_row(), 23);
    assert_eq!(grid.cursor_col(), 79);
}

#[test]
fn scroll_up() {
    let mut grid = TerminalGrid::new(5, 3);
    // Write 'A' in row 0, 'B' in row 1, 'C' in row 2
    grid.move_cursor_to(0, 0);
    grid.put_char('A');
    grid.move_cursor_to(1, 0);
    grid.put_char('B');
    grid.move_cursor_to(2, 0);
    grid.put_char('C');

    grid.scroll_up(1);

    // Row 0 should now have what was row 1 ('B')
    assert_eq!(grid.cell(0, 0).ch, 'B');
    // Row 1 should now have what was row 2 ('C')
    assert_eq!(grid.cell(1, 0).ch, 'C');
    // Row 2 (bottom) should be blank
    assert_eq!(grid.cell(2, 0).ch, ' ');
}

#[test]
fn erase_line_to_end() {
    let mut grid = TerminalGrid::new(10, 1);
    for ch in "0123456789".chars() {
        grid.put_char(ch);
    }
    grid.move_cursor_to(0, 5);
    grid.erase_in_line(0); // erase from cursor to end

    // Chars 0-4 preserved
    assert_eq!(grid.cell(0, 0).ch, '0');
    assert_eq!(grid.cell(0, 4).ch, '4');
    // Chars 5-9 erased
    assert_eq!(grid.cell(0, 5).ch, ' ');
    assert_eq!(grid.cell(0, 9).ch, ' ');
}

#[test]
fn resize_grid() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.move_cursor_to(0, 0);
    grid.put_char('X');
    grid.move_cursor_to(11, 39);
    grid.put_char('Y');

    grid.resize(40, 12);
    assert_eq!(grid.cols(), 40);
    assert_eq!(grid.rows(), 12);
    // Content preserved
    assert_eq!(grid.cell(0, 0).ch, 'X');
    assert_eq!(grid.cell(11, 39).ch, 'Y');
}
