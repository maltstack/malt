//! Tiling layout engine — N-ary splits, tabbed, float, gaps, WM strategy support.

pub mod resolve;
pub mod ops;
pub mod focus;
pub mod strategy;
pub mod strategies;

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub inner_gap: u16,
    pub outer_gap: u16,
    pub min_cols: u16,
    pub min_rows: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { inner_gap: 0, outer_gap: 0, min_cols: 8, min_rows: 4 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self { Self { x, y, w, h } }
    pub fn center_x(&self) -> u16 { self.x + self.w / 2 }
    pub fn center_y(&self) -> u16 { self.y + self.h / 2 }
    pub fn inset(&self, gap: u16) -> Self {
        Self {
            x: self.x + gap,
            y: self.y + gap,
            w: self.w.saturating_sub(gap * 2),
            h: self.h.saturating_sub(gap * 2),
        }
    }
}
