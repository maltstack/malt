use malt_protocol::common::ResolvedStyle;

const DEFAULT_FG: (u8, u8, u8) = (204, 204, 204);
const DEFAULT_BG: (u8, u8, u8) = (0, 0, 0);

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: ResolvedStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self { ch: ' ', style: default_style() }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub dirty: bool,
}

impl Row {
    pub fn new(cols: usize) -> Self {
        Self { cells: vec![Cell::default(); cols], dirty: false }
    }
    pub fn clear(&mut self) {
        for cell in &mut self.cells { *cell = Cell::default(); }
        self.dirty = true;
    }
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        self.dirty = true;
    }
}

pub fn default_style() -> ResolvedStyle {
    ResolvedStyle {
        fg: DEFAULT_FG, bg: DEFAULT_BG,
        bold: false, italic: false, underline: false,
        dim: false, strikethrough: false, reverse: false, blink: false,
        token_name: None,
        _unknown: Vec::new(),
    }
}
