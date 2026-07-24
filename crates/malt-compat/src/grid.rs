// Grid — virtual terminal grid (rows x cols)

use malt_protocol::common::ResolvedStyle;

use crate::cell::{default_style, Cell, Row};

/// A virtual terminal grid: a 2D array of cells with cursor, scrolling, and erase operations.
#[derive(Debug, Clone)]
pub struct TerminalGrid {
    rows_data: Vec<Row>,
    cols: usize,
    num_rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_top: usize,
    scroll_bottom: usize,
    current_style: ResolvedStyle,
    dirty: bool,
}

impl TerminalGrid {
    /// Create a new grid with the given dimensions. Cursor starts at (0, 0).
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols as usize;
        let num_rows = rows as usize;
        let rows_data = (0..num_rows).map(|_| Row::new(cols)).collect();
        Self {
            rows_data,
            cols,
            num_rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_bottom: num_rows.saturating_sub(1),
            current_style: default_style(),
            dirty: false,
        }
    }

    /// Number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.num_rows
    }

    /// Current cursor row (0-indexed).
    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    /// Current cursor column (0-indexed).
    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    /// Reference to a cell at the given position.
    ///
    /// # Panics
    /// Panics if row or col is out of bounds.
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.rows_data[row].cells[col]
    }

    /// Current style used for new characters.
    pub fn current_style(&self) -> &ResolvedStyle {
        &self.current_style
    }

    /// Set the style used for new characters.
    pub fn set_current_style(&mut self, style: ResolvedStyle) {
        self.current_style = style;
    }

    /// Whether any row has been modified since the last `clear_dirty()` call.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag on the grid and all rows.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
        for row in &mut self.rows_data {
            row.dirty = false;
        }
    }

    /// Write a character at the cursor position, advance the cursor.
    /// Wraps to the next line at end of row. Scrolls up if at the bottom.
    pub fn put_char(&mut self, ch: char) {
        // If cursor is past the last column, wrap to next line
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            if self.cursor_row >= self.scroll_bottom {
                self.scroll_up(1);
            } else {
                self.cursor_row += 1;
            }
        }

        self.rows_data[self.cursor_row].cells[self.cursor_col] = Cell {
            ch,
            style: self.current_style.clone(),
        };
        self.rows_data[self.cursor_row].dirty = true;
        self.dirty = true;
        self.cursor_col += 1;
    }

    /// Move cursor to an absolute position, clamped to grid bounds.
    pub fn move_cursor_to(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.num_rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    /// Move cursor by a relative delta, clamped to grid bounds.
    pub fn move_cursor_relative(&mut self, row_delta: i32, col_delta: i32) {
        let new_row = (self.cursor_row as i32).saturating_add(row_delta);
        let new_col = (self.cursor_col as i32).saturating_add(col_delta);
        self.move_cursor_to(new_row.max(0) as usize, new_col.max(0) as usize);
    }

    /// Scroll the scroll region up by `count` lines.
    /// Top lines are removed, blank lines are inserted at the bottom.
    pub fn scroll_up(&mut self, count: usize) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom {
                self.rows_data.remove(self.scroll_top);
                self.rows_data
                    .insert(self.scroll_bottom, Row::new(self.cols));
            }
        }
        self.dirty = true;
    }

    /// Scroll the scroll region down by `count` lines.
    /// Bottom lines are removed, blank lines are inserted at the top.
    pub fn scroll_down(&mut self, count: usize) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom {
                self.rows_data.remove(self.scroll_bottom);
                self.rows_data.insert(self.scroll_top, Row::new(self.cols));
            }
        }
        self.dirty = true;
    }

    /// Erase characters in the current line.
    /// - mode 0: from cursor to end of line
    /// - mode 1: from start of line to cursor (inclusive)
    /// - mode 2: entire line
    pub fn erase_in_line(&mut self, mode: u16) {
        let row = &mut self.rows_data[self.cursor_row];
        match mode {
            0 => {
                for col in self.cursor_col..self.cols {
                    row.cells[col] = Cell::default();
                }
            }
            1 => {
                for col in 0..=self.cursor_col.min(self.cols.saturating_sub(1)) {
                    row.cells[col] = Cell::default();
                }
            }
            2 => {
                row.clear();
            }
            _ => {}
        }
        row.dirty = true;
        self.dirty = true;
    }

    /// Erase characters in the display.
    /// - mode 0: from cursor to end of display
    /// - mode 1: from start of display to cursor
    /// - mode 2: entire display
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                // Current line from cursor to end
                self.erase_in_line(0);
                // All lines below
                for r in (self.cursor_row + 1)..self.num_rows {
                    self.rows_data[r].clear();
                }
            }
            1 => {
                // All lines above
                for r in 0..self.cursor_row {
                    self.rows_data[r].clear();
                }
                // Current line from start to cursor
                self.erase_in_line(1);
            }
            2 => {
                for row in &mut self.rows_data {
                    row.clear();
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Set the scroll region (0-indexed, inclusive top and bottom).
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.min(self.num_rows.saturating_sub(1));
        let bottom = bottom.min(self.num_rows.saturating_sub(1));
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
    }

    /// Move cursor to column 0.
    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    /// Move cursor down one row, scrolling if at the bottom of the scroll region.
    pub fn linefeed(&mut self) {
        if self.cursor_row >= self.scroll_bottom {
            self.scroll_up(1);
        } else {
            self.cursor_row += 1;
        }
    }

    /// Resize the grid, preserving as much content as possible.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_cols = cols as usize;
        let new_rows = rows as usize;

        // Resize existing rows' column count
        for row in &mut self.rows_data {
            row.resize(new_cols);
        }

        // Add or remove rows
        if new_rows > self.num_rows {
            for _ in 0..(new_rows - self.num_rows) {
                self.rows_data.push(Row::new(new_cols));
            }
        } else if new_rows < self.num_rows {
            self.rows_data.truncate(new_rows);
        }

        self.cols = new_cols;
        self.num_rows = new_rows;
        self.scroll_top = 0;
        self.scroll_bottom = new_rows.saturating_sub(1);

        // Clamp cursor
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.dirty = true;
    }

    /// Borrow the underlying row data.
    pub fn rows_data(&self) -> &[Row] {
        &self.rows_data
    }
}
