use malt_compat::cell::{Cell, Row};

#[test]
fn default_cell_is_space() {
    let cell = Cell::default();
    assert_eq!(cell.ch, ' ');
    assert_eq!(cell.style.fg, (204, 204, 204));
    assert!(!cell.style.bold);
}

#[test]
fn cell_with_char_and_style() {
    let mut cell = Cell::default();
    cell.ch = 'A';
    cell.style.bold = true;
    assert_eq!(cell.ch, 'A');
    assert!(cell.style.bold);
}

#[test]
fn row_creation() {
    let row = Row::new(80);
    assert_eq!(row.cells.len(), 80);
    assert_eq!(row.cells[0].ch, ' ');
}

#[test]
fn row_clear() {
    let mut row = Row::new(10);
    row.cells[0].ch = 'X';
    row.cells[5].ch = 'Y';
    row.clear();
    assert_eq!(row.cells[0].ch, ' ');
    assert_eq!(row.cells[5].ch, ' ');
}
