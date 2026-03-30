# Phase 3B: Compat Translator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the VT terminal emulator that parses escape sequences, maintains a virtual terminal grid, and emits VtPassthrough FrameElements — the ONLY place VT codes exist in MALT.

**Architecture:** `vte` crate parses raw bytes into actions via the `Perform` trait. Our `Perform` implementation drives a `TerminalGrid` (2D cell buffer with cursor, scroll region, SGR state). The `CompatTranslator` wraps both and exposes a clean feed→emit API.

**Tech Stack:** Rust, vte 0.15 (VT parser), malt-protocol (FrameElement, ResolvedStyle), thiserror, tracing

---

## File Structure

```
crates/malt-compat/
  Cargo.toml
  src/
    lib.rs              — crate root, module declarations, re-exports
    error.rs            — CompatError enum
    cell.rs             — Cell struct (char + style), Row type alias
    grid.rs             — TerminalGrid: 2D buffer, cursor, scroll region, SGR state
    parser.rs           — GridPerformer: vte::Perform impl driving the grid
    translator.rs       — CompatTranslator: public API wrapping parser+grid
  tests/
    cell.rs             — cell/row operations
    grid.rs             — grid cursor, scrolling, erase
    parser.rs           — VT sequences → grid state
    translator.rs       — end-to-end bytes → FrameElement
```

---

### Task 1: Crate Scaffolding

**Files:**
- Create: `crates/malt-compat/Cargo.toml`
- Create: `crates/malt-compat/src/lib.rs`
- Create: `crates/malt-compat/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "malt-compat"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "VT terminal emulator for MALT — the only place VT codes exist"

[dependencies]
malt-protocol = { path = "../malt-protocol" }
vte = "0.15"
thiserror = "2"
tracing = "0.1"
```

- [ ] **Step 2: Create error.rs**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CompatError {
    #[error("invalid grid dimensions: {cols}x{rows}")]
    InvalidDimensions { cols: u16, rows: u16 },
}
```

- [ ] **Step 3: Create lib.rs**

```rust
pub mod cell;
pub mod error;
pub mod grid;
pub mod parser;
pub mod translator;

pub use error::CompatError;
pub use translator::CompatTranslator;
```

- [ ] **Step 4: Create stub modules**

Create `crates/malt-compat/src/cell.rs`:
```rust
// Cell and Row — Task 2
```

Create `crates/malt-compat/src/grid.rs`:
```rust
// TerminalGrid — Task 3
```

Create `crates/malt-compat/src/parser.rs`:
```rust
// GridPerformer — Task 4
```

Create `crates/malt-compat/src/translator.rs`:
```rust
// CompatTranslator — Task 5
```

- [ ] **Step 5: Add malt-compat to workspace**

In root `Cargo.toml`, add `"crates/malt-compat"` to workspace members.

- [ ] **Step 6: Verify it compiles**

Run: `cargo check -p malt-compat`
Expected: compiles

- [ ] **Step 7: Commit**

```bash
git add crates/malt-compat/ Cargo.toml
git commit -m "feat(malt-compat): crate scaffolding with module stubs"
```

---

### Task 2: Cell and Row

**Files:**
- Create: `crates/malt-compat/src/cell.rs`
- Create: `crates/malt-compat/tests/cell.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-compat/tests/cell.rs`:
```rust
use malt_compat::cell::{Cell, Row};

#[test]
fn default_cell_is_space() {
    let cell = Cell::default();
    assert_eq!(cell.ch, ' ');
    assert_eq!(cell.style.fg, (204, 204, 204));
    assert_eq!(cell.style.bg, (0, 0, 0));
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-compat --test cell`
Expected: FAIL

- [ ] **Step 3: Implement Cell and Row**

Write `crates/malt-compat/src/cell.rs`:
```rust
use malt_protocol::common::ResolvedStyle;

/// Default foreground color (light gray).
const DEFAULT_FG: (u8, u8, u8) = (204, 204, 204);
/// Default background color (black).
const DEFAULT_BG: (u8, u8, u8) = (0, 0, 0);

/// A single cell in the terminal grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: ResolvedStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: default_style(),
        }
    }
}

/// A row of cells.
#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub dirty: bool,
}

impl Row {
    pub fn new(cols: usize) -> Self {
        Self {
            cells: vec![Cell::default(); cols],
            dirty: false,
        }
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
        self.dirty = true;
    }

    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        self.dirty = true;
    }
}

/// Returns the default terminal style.
pub fn default_style() -> ResolvedStyle {
    ResolvedStyle {
        fg: DEFAULT_FG,
        bg: DEFAULT_BG,
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p malt-compat --test cell`
Expected: all 4 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-compat/src/cell.rs crates/malt-compat/tests/cell.rs
git commit -m "feat(malt-compat): cell and row types for terminal grid"
```

---

### Task 3: Terminal Grid

**Files:**
- Create: `crates/malt-compat/src/grid.rs`
- Create: `crates/malt-compat/tests/grid.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-compat/tests/grid.rs`:
```rust
use malt_compat::grid::TerminalGrid;

#[test]
fn new_grid_has_correct_dimensions() {
    let grid = TerminalGrid::new(80, 24);
    assert_eq!(grid.cols(), 80);
    assert_eq!(grid.rows(), 24);
    assert_eq!(grid.cursor_col(), 0);
    assert_eq!(grid.cursor_row(), 0);
}

#[test]
fn put_char_advances_cursor() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.put_char('A');
    assert_eq!(grid.cell(0, 0).ch, 'A');
    assert_eq!(grid.cursor_col(), 1);
}

#[test]
fn cursor_wraps_at_line_end() {
    let mut grid = TerminalGrid::new(5, 3);
    for ch in "hello!".chars() {
        grid.put_char(ch);
    }
    // "hello" fills row 0, "!" wraps to row 1 col 0
    assert_eq!(grid.cell(0, 4).ch, 'o');
    assert_eq!(grid.cursor_row(), 1);
    assert_eq!(grid.cursor_col(), 1);
    assert_eq!(grid.cell(1, 0).ch, '!');
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
    let mut grid = TerminalGrid::new(10, 3);
    grid.put_char('A'); // row 0
    grid.move_cursor_to(1, 0);
    grid.put_char('B'); // row 1
    grid.move_cursor_to(2, 0);
    grid.put_char('C'); // row 2
    grid.scroll_up(1);
    // Row 0 should now have 'B', row 1 'C', row 2 cleared
    assert_eq!(grid.cell(0, 0).ch, 'B');
    assert_eq!(grid.cell(1, 0).ch, 'C');
    assert_eq!(grid.cell(2, 0).ch, ' ');
}

#[test]
fn erase_line_to_end() {
    let mut grid = TerminalGrid::new(10, 3);
    for ch in "0123456789".chars() {
        grid.put_char(ch);
    }
    grid.move_cursor_to(0, 5);
    grid.erase_in_line(0); // erase from cursor to end
    assert_eq!(grid.cell(0, 4).ch, '4');
    assert_eq!(grid.cell(0, 5).ch, ' ');
    assert_eq!(grid.cell(0, 9).ch, ' ');
}

#[test]
fn resize_grid() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.put_char('X');
    grid.resize(40, 12);
    assert_eq!(grid.cols(), 40);
    assert_eq!(grid.rows(), 12);
    assert_eq!(grid.cell(0, 0).ch, 'X');
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-compat --test grid`
Expected: FAIL

- [ ] **Step 3: Implement TerminalGrid**

Write `crates/malt-compat/src/grid.rs`:
```rust
use crate::cell::{default_style, Cell, Row};
use malt_protocol::common::ResolvedStyle;

/// Virtual terminal grid — 2D cell buffer with cursor and scroll region.
#[derive(Debug)]
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
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols as usize;
        let num_rows = rows as usize;
        Self {
            rows_data: (0..num_rows).map(|_| Row::new(cols)).collect(),
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

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.num_rows
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.rows_data[row].cells[col]
    }

    pub fn current_style(&self) -> &ResolvedStyle {
        &self.current_style
    }

    pub fn set_current_style(&mut self, style: ResolvedStyle) {
        self.current_style = style;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
        for row in &mut self.rows_data {
            row.dirty = false;
        }
    }

    /// Write a character at cursor position and advance.
    pub fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row > self.scroll_bottom {
                self.scroll_up(1);
                self.cursor_row = self.scroll_bottom;
            }
        }
        if self.cursor_row < self.num_rows && self.cursor_col < self.cols {
            self.rows_data[self.cursor_row].cells[self.cursor_col] = Cell {
                ch,
                style: self.current_style.clone(),
            };
            self.rows_data[self.cursor_row].dirty = true;
            self.dirty = true;
            self.cursor_col += 1;
        }
    }

    /// Move cursor to absolute position (0-indexed), clamped to bounds.
    pub fn move_cursor_to(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.num_rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    /// Move cursor relative.
    pub fn move_cursor_relative(&mut self, row_delta: i32, col_delta: i32) {
        let new_row = (self.cursor_row as i32 + row_delta)
            .max(0)
            .min(self.num_rows as i32 - 1) as usize;
        let new_col = (self.cursor_col as i32 + col_delta)
            .max(0)
            .min(self.cols as i32 - 1) as usize;
        self.cursor_row = new_row;
        self.cursor_col = new_col;
    }

    /// Scroll the scroll region up by `count` lines.
    pub fn scroll_up(&mut self, count: usize) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.num_rows {
                self.rows_data.remove(self.scroll_top);
                self.rows_data
                    .insert(self.scroll_bottom, Row::new(self.cols));
            }
        }
        self.dirty = true;
    }

    /// Scroll the scroll region down by `count` lines.
    pub fn scroll_down(&mut self, count: usize) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.num_rows {
                self.rows_data.remove(self.scroll_bottom);
                self.rows_data.insert(self.scroll_top, Row::new(self.cols));
            }
        }
        self.dirty = true;
    }

    /// Erase in line: 0=cursor-to-end, 1=start-to-cursor, 2=whole line.
    pub fn erase_in_line(&mut self, mode: u16) {
        let row = self.cursor_row;
        if row >= self.num_rows {
            return;
        }
        match mode {
            0 => {
                for col in self.cursor_col..self.cols {
                    self.rows_data[row].cells[col] = Cell::default();
                }
            }
            1 => {
                for col in 0..=self.cursor_col.min(self.cols - 1) {
                    self.rows_data[row].cells[col] = Cell::default();
                }
            }
            2 => {
                self.rows_data[row].clear();
            }
            _ => {}
        }
        self.rows_data[row].dirty = true;
        self.dirty = true;
    }

    /// Erase in display: 0=cursor-to-end, 1=start-to-cursor, 2=whole screen.
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_in_line(0);
                for row in (self.cursor_row + 1)..self.num_rows {
                    self.rows_data[row].clear();
                }
            }
            1 => {
                for row in 0..self.cursor_row {
                    self.rows_data[row].clear();
                }
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

    /// Set scroll region (0-indexed, inclusive).
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        if top < bottom && bottom < self.num_rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
    }

    /// Carriage return — move cursor to column 0.
    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    /// Linefeed — move cursor down, scroll if at bottom of scroll region.
    pub fn linefeed(&mut self) {
        if self.cursor_row >= self.scroll_bottom {
            self.scroll_up(1);
        } else {
            self.cursor_row += 1;
        }
    }

    /// Resize the grid, preserving content where possible.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_cols = cols as usize;
        let new_rows = rows as usize;
        // Resize existing rows
        for row in &mut self.rows_data {
            row.resize(new_cols);
        }
        // Add or remove rows
        while self.rows_data.len() < new_rows {
            self.rows_data.push(Row::new(new_cols));
        }
        self.rows_data.truncate(new_rows);
        self.cols = new_cols;
        self.num_rows = new_rows;
        self.scroll_top = 0;
        self.scroll_bottom = new_rows.saturating_sub(1);
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.dirty = true;
    }

    /// Get all rows (for rendering).
    pub fn rows_data(&self) -> &[Row] {
        &self.rows_data
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p malt-compat --test grid`
Expected: all 8 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-compat/src/grid.rs crates/malt-compat/tests/grid.rs
git commit -m "feat(malt-compat): terminal grid — cursor, scrolling, erase, resize"
```

---

### Task 4: VT Parser (vte::Perform impl)

**Files:**
- Create: `crates/malt-compat/src/parser.rs`
- Create: `crates/malt-compat/tests/parser.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-compat/tests/parser.rs`:
```rust
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
    assert_eq!(grid.cell(0, 4).ch, 'o');
    assert_eq!(grid.cursor_col(), 5);
}

#[test]
fn sgr_sets_foreground() {
    let mut grid = TerminalGrid::new(80, 24);
    // ESC[31m = red foreground, then 'X'
    feed(&mut grid, b"\x1b[31mX");
    let style = &grid.cell(0, 0).style;
    // SGR 31 = red (basic color), expect non-default fg
    assert_ne!(style.fg, (204, 204, 204));
}

#[test]
fn sgr_sets_bold() {
    let mut grid = TerminalGrid::new(80, 24);
    // ESC[1m = bold, then 'B'
    feed(&mut grid, b"\x1b[1mB");
    assert!(grid.cell(0, 0).style.bold);
}

#[test]
fn cursor_up_sequence() {
    let mut grid = TerminalGrid::new(80, 24);
    grid.move_cursor_to(5, 0);
    // ESC[2A = cursor up 2
    feed(&mut grid, b"\x1b[2A");
    assert_eq!(grid.cursor_row(), 3);
}

#[test]
fn cursor_position_sequence() {
    let mut grid = TerminalGrid::new(80, 24);
    // ESC[10;20H = move to row 10, col 20 (1-indexed)
    feed(&mut grid, b"\x1b[10;20H");
    assert_eq!(grid.cursor_row(), 9);  // 0-indexed
    assert_eq!(grid.cursor_col(), 19); // 0-indexed
}

#[test]
fn unknown_sequence_ignored() {
    let mut grid = TerminalGrid::new(80, 24);
    // Some unknown private sequence — should not crash
    feed(&mut grid, b"\x1b[?9999h");
    feed(&mut grid, b"OK");
    assert_eq!(grid.cell(0, 0).ch, 'O');
    assert_eq!(grid.cell(0, 1).ch, 'K');
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-compat --test parser`
Expected: FAIL

- [ ] **Step 3: Implement GridPerformer**

Write `crates/malt-compat/src/parser.rs`:
```rust
use crate::cell::default_style;
use crate::grid::TerminalGrid;
use malt_protocol::common::ResolvedStyle;
use vte::{Params, Perform};

/// Basic ANSI colors (SGR 30-37 / 40-47).
const BASIC_COLORS: [(u8, u8, u8); 8] = [
    (0, 0, 0),       // black
    (170, 0, 0),     // red
    (0, 170, 0),     // green
    (170, 85, 0),    // yellow/brown
    (0, 0, 170),     // blue
    (170, 0, 170),   // magenta
    (0, 170, 170),   // cyan
    (170, 170, 170), // white
];

/// Bright ANSI colors (SGR 90-97 / 100-107).
const BRIGHT_COLORS: [(u8, u8, u8); 8] = [
    (85, 85, 85),    // bright black
    (255, 85, 85),   // bright red
    (85, 255, 85),   // bright green
    (255, 255, 85),  // bright yellow
    (85, 85, 255),   // bright blue
    (255, 85, 255),  // bright magenta
    (85, 255, 255),  // bright cyan
    (255, 255, 255), // bright white
];

/// Adapter between `vte::Perform` and `TerminalGrid`.
pub struct GridPerformer<'a> {
    grid: &'a mut TerminalGrid,
}

impl<'a> GridPerformer<'a> {
    pub fn new(grid: &'a mut TerminalGrid) -> Self {
        Self { grid }
    }
}

impl<'a> Perform for GridPerformer<'a> {
    fn print(&mut self, c: char) {
        self.grid.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                // Backspace
                self.grid.move_cursor_relative(0, -1);
            }
            0x09 => {
                // Tab — advance to next 8-col tab stop
                let col = self.grid.cursor_col();
                let next_tab = (col / 8 + 1) * 8;
                let target = next_tab.min(self.grid.cols().saturating_sub(1));
                self.grid
                    .move_cursor_to(self.grid.cursor_row(), target);
            }
            0x0A | 0x0B | 0x0C => {
                // LF, VT, FF — linefeed
                self.grid.linefeed();
            }
            0x0D => {
                // CR
                self.grid.carriage_return();
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let params: Vec<u16> = params.iter().map(|p| p[0]).collect();
        match action {
            'A' => {
                // CUU — cursor up
                let n = params.first().copied().unwrap_or(1).max(1) as i32;
                self.grid.move_cursor_relative(-n, 0);
            }
            'B' => {
                // CUD — cursor down
                let n = params.first().copied().unwrap_or(1).max(1) as i32;
                self.grid.move_cursor_relative(n, 0);
            }
            'C' => {
                // CUF — cursor forward
                let n = params.first().copied().unwrap_or(1).max(1) as i32;
                self.grid.move_cursor_relative(0, n);
            }
            'D' => {
                // CUB — cursor backward
                let n = params.first().copied().unwrap_or(1).max(1) as i32;
                self.grid.move_cursor_relative(0, -n);
            }
            'H' | 'f' => {
                // CUP — cursor position (1-indexed in VT)
                let row = params.first().copied().unwrap_or(1).max(1) as usize - 1;
                let col = params.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                self.grid.move_cursor_to(row, col);
            }
            'J' => {
                // ED — erase in display
                let mode = params.first().copied().unwrap_or(0);
                self.grid.erase_in_display(mode);
            }
            'K' => {
                // EL — erase in line
                let mode = params.first().copied().unwrap_or(0);
                self.grid.erase_in_line(mode);
            }
            'S' => {
                // SU — scroll up
                let n = params.first().copied().unwrap_or(1).max(1) as usize;
                self.grid.scroll_up(n);
            }
            'T' => {
                // SD — scroll down
                let n = params.first().copied().unwrap_or(1).max(1) as usize;
                self.grid.scroll_down(n);
            }
            'r' => {
                // DECSTBM — set scroll region (1-indexed)
                let top = params.first().copied().unwrap_or(1).max(1) as usize - 1;
                let bottom = params
                    .get(1)
                    .copied()
                    .unwrap_or(self.grid.rows() as u16)
                    .max(1) as usize
                    - 1;
                self.grid.set_scroll_region(top, bottom);
            }
            'm' => {
                // SGR — select graphic rendition
                if params.is_empty() {
                    self.grid.set_current_style(default_style());
                    return;
                }
                self.apply_sgr(&params);
            }
            _ => {
                // Unknown CSI — ignore
            }
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        // ESC sequences — ignore for now
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC sequences — ignore for now
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
}

impl<'a> GridPerformer<'a> {
    fn apply_sgr(&mut self, params: &[u16]) {
        let mut style = self.grid.current_style().clone();
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => style = default_style(),
                1 => style.bold = true,
                2 => style.dim = true,
                3 => style.italic = true,
                4 => style.underline = true,
                5 | 6 => style.blink = true,
                7 => style.reverse = true,
                9 => style.strikethrough = true,
                22 => {
                    style.bold = false;
                    style.dim = false;
                }
                23 => style.italic = false,
                24 => style.underline = false,
                25 => style.blink = false,
                27 => style.reverse = false,
                29 => style.strikethrough = false,
                30..=37 => style.fg = BASIC_COLORS[(params[i] - 30) as usize],
                38 => {
                    // Extended foreground: 38;5;N or 38;2;R;G;B
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        let idx = params[i + 2] as usize;
                        style.fg = color_from_256(idx);
                        i += 2;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        style.fg = (params[i + 2] as u8, params[i + 3] as u8, params[i + 4] as u8);
                        i += 4;
                    }
                }
                39 => style.fg = (204, 204, 204), // default fg
                40..=47 => style.bg = BASIC_COLORS[(params[i] - 40) as usize],
                48 => {
                    // Extended background: 48;5;N or 48;2;R;G;B
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        let idx = params[i + 2] as usize;
                        style.bg = color_from_256(idx);
                        i += 2;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        style.bg = (params[i + 2] as u8, params[i + 3] as u8, params[i + 4] as u8);
                        i += 4;
                    }
                }
                49 => style.bg = (0, 0, 0), // default bg
                90..=97 => style.fg = BRIGHT_COLORS[(params[i] - 90) as usize],
                100..=107 => style.bg = BRIGHT_COLORS[(params[i] - 100) as usize],
                _ => {} // Unknown SGR — ignore
            }
            i += 1;
        }
        self.grid.set_current_style(style);
    }
}

/// Convert 256-color index to RGB.
fn color_from_256(idx: usize) -> (u8, u8, u8) {
    match idx {
        0..=7 => BASIC_COLORS[idx],
        8..=15 => BRIGHT_COLORS[idx - 8],
        16..=231 => {
            // 6x6x6 color cube
            let idx = idx - 16;
            let r = (idx / 36) as u8;
            let g = ((idx / 6) % 6) as u8;
            let b = (idx % 6) as u8;
            let to_rgb = |v: u8| -> u8 { if v == 0 { 0 } else { 55 + 40 * v } };
            (to_rgb(r), to_rgb(g), to_rgb(b))
        }
        232..=255 => {
            // Grayscale ramp
            let v = (8 + 10 * (idx - 232)) as u8;
            (v, v, v)
        }
        _ => (204, 204, 204),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p malt-compat --test parser`
Expected: all 6 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-compat/src/parser.rs crates/malt-compat/tests/parser.rs
git commit -m "feat(malt-compat): VT parser — vte::Perform impl driving terminal grid"
```

---

### Task 5: Compat Translator

**Files:**
- Create: `crates/malt-compat/src/translator.rs`
- Create: `crates/malt-compat/tests/translator.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-compat/tests/translator.rs`:
```rust
use malt_compat::CompatTranslator;
use malt_protocol::frame_element::FrameElement;

#[test]
fn feed_produces_vt_passthrough() {
    let mut translator = CompatTranslator::new(80, 24);
    translator.feed(b"Hello, world!");
    let elem = translator.frame_element();
    match elem {
        FrameElement::VtPassthrough { data, .. } => {
            assert_eq!(data, b"Hello, world!");
        }
        other => panic!("expected VtPassthrough, got {other:?}"),
    }
}

#[test]
fn dirty_tracking() {
    let mut translator = CompatTranslator::new(80, 24);
    assert!(!translator.is_dirty());
    translator.feed(b"test");
    assert!(translator.is_dirty());
    translator.clear_dirty();
    assert!(!translator.is_dirty());
}

#[test]
fn empty_feed_not_dirty() {
    let mut translator = CompatTranslator::new(80, 24);
    translator.feed(b"");
    assert!(!translator.is_dirty());
}

#[test]
fn resize_updates_dimensions() {
    let mut translator = CompatTranslator::new(80, 24);
    translator.resize(40, 12);
    translator.feed(b"X");
    // Should not crash — grid resized properly
    let elem = translator.frame_element();
    match elem {
        FrameElement::VtPassthrough { .. } => {}
        other => panic!("expected VtPassthrough, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-compat --test translator`
Expected: FAIL

- [ ] **Step 3: Implement CompatTranslator**

Write `crates/malt-compat/src/translator.rs`:
```rust
use crate::grid::TerminalGrid;
use crate::parser::GridPerformer;
use malt_protocol::frame_element::FrameElement;
use vte::Parser;

/// Compat Translator: feeds raw bytes through VT parser into terminal grid,
/// emits VtPassthrough FrameElements.
///
/// This is the ONLY component in MALT that handles VT escape codes.
pub struct CompatTranslator {
    grid: TerminalGrid,
    parser: Parser,
    last_data: Vec<u8>,
}

impl CompatTranslator {
    /// Create a new translator with the given terminal dimensions.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            grid: TerminalGrid::new(cols, rows),
            parser: Parser::new(),
            last_data: Vec::new(),
        }
    }

    /// Feed raw bytes from a PTY into the parser.
    /// Updates the internal grid state.
    pub fn feed(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.last_data = data.to_vec();
        let mut performer = GridPerformer::new(&mut self.grid);
        self.parser.advance(&mut performer, data);
    }

    /// Emit the current state as a VtPassthrough FrameElement.
    /// Contains the raw bytes for clients that support VT rendering.
    pub fn frame_element(&self) -> FrameElement {
        FrameElement::VtPassthrough {
            data: self.last_data.clone(),
        }
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid.resize(cols, rows);
    }

    /// Check if the grid has changed since the last clear.
    pub fn is_dirty(&self) -> bool {
        self.grid.is_dirty()
    }

    /// Clear dirty flags.
    pub fn clear_dirty(&mut self) {
        self.grid.clear_dirty();
    }

    /// Access the underlying grid (for testing or advanced use).
    pub fn grid(&self) -> &TerminalGrid {
        &self.grid
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p malt-compat --test translator`
Expected: all 4 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-compat/src/translator.rs crates/malt-compat/tests/translator.rs
git commit -m "feat(malt-compat): compat translator — feed bytes, emit VtPassthrough"
```

---

### Task 6: Re-exports and Final Verification

**Files:**
- Modify: `crates/malt-compat/src/lib.rs`

- [ ] **Step 1: Update lib.rs with clean re-exports**

```rust
pub mod cell;
pub mod error;
pub mod grid;
pub mod parser;
pub mod translator;

pub use error::CompatError;
pub use translator::CompatTranslator;
```

- [ ] **Step 2: Run all malt-compat tests**

Run: `cargo test -p malt-compat`
Expected: all tests PASS (4 cell + 8 grid + 6 parser + 4 translator = 22 total)

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p malt-compat -- -W clippy::all -A unused-imports`
Expected: no warnings from malt-compat code

- [ ] **Step 4: Fix any clippy issues**

- [ ] **Step 5: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add crates/malt-compat/
git commit -m "feat(malt-compat): clean re-exports for public API"
```
