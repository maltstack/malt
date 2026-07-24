// Translator — converts VT byte stream into structured FrameElements

use malt_protocol::frame_element::FrameElement;

use crate::grid::TerminalGrid;
use crate::parser::GridPerformer;

/// Translates VT terminal byte streams into structured FrameElements.
///
/// Wraps a `TerminalGrid` and `vte::Parser`, feeding raw bytes through the VT
/// parser to update grid state while retaining the raw data for passthrough.
pub struct CompatTranslator {
    grid: TerminalGrid,
    parser: vte::Parser,
    last_data: Vec<u8>,
}

impl CompatTranslator {
    /// Create a new translator with the given terminal dimensions.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            grid: TerminalGrid::new(cols, rows),
            parser: vte::Parser::new(),
            last_data: Vec::new(),
        }
    }

    /// Feed raw bytes through the VT parser, updating the grid and storing
    /// the most recent chunk for delta passthrough via `frame_element()`.
    ///
    /// An empty slice is a no-op and does not mark the translator as dirty.
    pub fn feed(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.last_data = data.to_vec();
        let mut performer = GridPerformer::new(&mut self.grid);
        self.parser.advance(&mut performer, data);
    }

    /// Return a `FrameElement::VtPassthrough` containing the most recent fed chunk.
    pub fn frame_element(&self) -> FrameElement {
        FrameElement::VtPassthrough {
            data: self.last_data.clone(),
        }
    }

    /// Resize the underlying terminal grid.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid.resize(cols, rows);
    }

    /// Whether the grid has been modified since the last `clear_dirty()` call.
    pub fn is_dirty(&self) -> bool {
        self.grid.is_dirty()
    }

    /// Clear the dirty flag on the underlying grid.
    pub fn clear_dirty(&mut self) {
        self.grid.clear_dirty();
    }

    /// Borrow the underlying terminal grid.
    pub fn grid(&self) -> &TerminalGrid {
        &self.grid
    }
}
