// TUI renderer: applies RenderCommands to a ratatui Buffer.

use malt_compat::CompatTranslator;
use malt_protocol::render::RenderCommand;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

/// Applies a stream of `RenderCommand`s to a ratatui `Buffer`.
///
/// Tracks clip state across commands. `Flush` and `SetCursor` are not
/// handled here — they are the responsibility of the application-level
/// display loop. `WriteRaw` (VT passthrough for compat/shell panes) *is*
/// handled here, via an owned `CompatTranslator` — reusing the one crate
/// that's allowed to touch VT escape codes (Hard Invariant #1) rather than
/// hand-rolling VT parsing in this client.
pub struct TuiRenderer {
    clip: Option<Rect>,
    /// Client-side VT grid for `WriteRaw` content. Single instance,
    /// matching today's single-pane-per-session model; resized to match
    /// whatever `(width, height)` the next `WriteRaw` command carries.
    compat: CompatTranslator,
}

impl TuiRenderer {
    pub fn new() -> Self {
        Self {
            clip: None,
            compat: CompatTranslator::new(1, 1),
        }
    }

    /// Apply a list of RenderCommands to a ratatui Buffer.
    pub fn apply(&mut self, commands: &[RenderCommand], buf: &mut Buffer) {
        for cmd in commands {
            self.apply_one(cmd, buf);
        }
    }

    fn apply_one(&mut self, cmd: &RenderCommand, buf: &mut Buffer) {
        match cmd {
            RenderCommand::DrawText { x, y, text, style } => {
                let rat_style = crate::style::to_ratatui_style(style);
                if let Some(clip) = &self.clip {
                    // Only draw the portion that falls within the clip rect.
                    if *y < clip.y || *y >= clip.y + clip.height {
                        return;
                    }
                    let start = if *x < clip.x {
                        (clip.x - *x) as usize
                    } else {
                        0
                    };
                    let end_col = clip.x + clip.width;
                    let text_end = (*x as usize) + text.len();
                    let end = if text_end as u16 > end_col {
                        (end_col - *x) as usize
                    } else {
                        text.len()
                    };
                    if start < end {
                        let clipped = &text[start..end];
                        let draw_x = (*x).max(clip.x);
                        buf.set_string(draw_x, *y, clipped, rat_style);
                    }
                } else {
                    buf.set_string(*x, *y, text, rat_style);
                }
            }

            RenderCommand::DrawRect {
                x,
                y,
                width,
                height,
                style,
            } => {
                let rat_style = crate::style::to_ratatui_style(style);
                let rect = self.clipped_rect(*x, *y, *width, *height);
                for row in rect.y..rect.y + rect.height {
                    for col in rect.x..rect.x + rect.width {
                        if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
                            cell.set_char(' ');
                            cell.set_style(rat_style);
                        }
                    }
                }
            }

            RenderCommand::DrawBorder {
                x,
                y,
                width,
                height,
                style,
            } => {
                let rat_style = crate::style::to_ratatui_style(style);
                let rect = Rect::new(*x, *y, *width, *height);
                // Draw border characters around the perimeter.
                if rect.width < 2 || rect.height < 2 {
                    return;
                }
                let right = rect.x + rect.width - 1;
                let bottom = rect.y + rect.height - 1;

                // Corners
                self.set_cell(buf, rect.x, rect.y, '┌', rat_style);
                self.set_cell(buf, right, rect.y, '┐', rat_style);
                self.set_cell(buf, rect.x, bottom, '└', rat_style);
                self.set_cell(buf, right, bottom, '╘', rat_style);

                // Horizontal edges
                for col in (rect.x + 1)..right {
                    self.set_cell(buf, col, rect.y, '─', rat_style);
                    self.set_cell(buf, col, bottom, '─', rat_style);
                }

                // Vertical edges
                for row in (rect.y + 1)..bottom {
                    self.set_cell(buf, rect.x, row, '│', rat_style);
                    self.set_cell(buf, right, row, '│', rat_style);
                }
            }

            RenderCommand::Clear {} => {
                buf.reset();
            }

            RenderCommand::SetClip {
                x,
                y,
                width,
                height,
            } => {
                self.clip = Some(Rect::new(*x, *y, *width, *height));
            }

            RenderCommand::ClearClip {} => {
                self.clip = None;
            }

            RenderCommand::WriteRaw {
                data,
                x,
                y,
                width,
                height,
            } => {
                if self.compat.grid().cols() != *width as usize
                    || self.compat.grid().rows() != *height as usize
                {
                    self.compat.resize(*width, *height);
                }
                self.compat.feed(data);

                let grid = self.compat.grid();
                for (row_idx, row) in grid.rows_data().iter().enumerate() {
                    let Some(draw_y) = y.checked_add(row_idx as u16) else {
                        break;
                    };
                    for (col_idx, cell) in row.cells.iter().enumerate() {
                        let Some(draw_x) = x.checked_add(col_idx as u16) else {
                            break;
                        };
                        let rat_style = crate::style::to_ratatui_style(&cell.style);
                        self.set_cell(buf, draw_x, draw_y, cell.ch, rat_style);
                    }
                }
            }

            // Flush and SetCursor are handled at the application level,
            // not by the buffer renderer. Unknown future commands are
            // skipped for forward compatibility.
            _ => {}
        }
    }

    /// Intersect a rect with the current clip region.
    fn clipped_rect(&self, x: u16, y: u16, w: u16, h: u16) -> Rect {
        let rect = Rect::new(x, y, w, h);
        match &self.clip {
            Some(clip) => rect.intersection(*clip),
            None => rect,
        }
    }

    /// Set a single cell, respecting clip bounds and buffer area.
    fn set_cell(&self, buf: &mut Buffer, x: u16, y: u16, ch: char, style: ratatui::style::Style) {
        if let Some(clip) = &self.clip {
            if x < clip.x || x >= clip.x + clip.width || y < clip.y || y >= clip.y + clip.height {
                return;
            }
        }
        if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
            cell.set_char(ch);
            cell.set_style(style);
        }
    }
}

impl Default for TuiRenderer {
    fn default() -> Self {
        Self::new()
    }
}
