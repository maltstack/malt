// Parser — VT escape sequence parser (wraps vte crate)

use crate::cell::default_style;
use crate::grid::TerminalGrid;
/// Basic ANSI colors (SGR 30-37).
const BASIC_COLORS: [(u8, u8, u8); 8] = [
    (0, 0, 0),       // black
    (170, 0, 0),     // red
    (0, 170, 0),     // green
    (170, 85, 0),    // yellow
    (0, 0, 170),     // blue
    (170, 0, 170),   // magenta
    (0, 170, 170),   // cyan
    (170, 170, 170), // white
];

/// Bright ANSI colors (SGR 90-97).
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

/// Convert a 256-color index to an RGB triple.
fn color_from_256(idx: u16) -> (u8, u8, u8) {
    match idx {
        0..=7 => BASIC_COLORS[idx as usize],
        8..=15 => BRIGHT_COLORS[(idx - 8) as usize],
        16..=231 => {
            // 6x6x6 color cube
            let idx = idx - 16;
            let b = (idx % 6) as u8;
            let g = ((idx / 6) % 6) as u8;
            let r = (idx / 36) as u8;
            let to_val = |c: u8| if c == 0 { 0 } else { 55 + 40 * c };
            (to_val(r), to_val(g), to_val(b))
        }
        232..=255 => {
            // Grayscale ramp
            let v = (8 + 10 * (idx - 232)) as u8;
            (v, v, v)
        }
        _ => (0, 0, 0),
    }
}

/// A `vte::Perform` implementation that drives a `TerminalGrid`.
pub struct GridPerformer<'a> {
    grid: &'a mut TerminalGrid,
}

impl<'a> GridPerformer<'a> {
    /// Create a new performer wrapping the given grid.
    pub fn new(grid: &'a mut TerminalGrid) -> Self {
        Self { grid }
    }
}

impl vte::Perform for GridPerformer<'_> {
    fn print(&mut self, c: char) {
        self.grid.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                // Backspace — move cursor left by 1
                self.grid.move_cursor_relative(0, -1);
            }
            0x09 => {
                // Tab — advance cursor to next tab stop (every 8 columns)
                let col = self.grid.cursor_col();
                let next_tab = ((col / 8) + 1) * 8;
                let target = next_tab.min(self.grid.cols().saturating_sub(1));
                self.grid
                    .move_cursor_to(self.grid.cursor_row(), target);
            }
            0x0A..=0x0C => {
                // LF, VT, FF — linefeed
                self.grid.linefeed();
            }
            0x0D => {
                // CR — carriage return
                self.grid.carriage_return();
            }
            _ => {
                // Ignore other C0 controls
            }
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // If there are intermediates (like '?' in DECSET), we don't handle those — ignore silently.
        if !intermediates.is_empty() {
            return;
        }

        let raw: Vec<u16> = params.iter().map(|p| p[0]).collect();

        match action {
            // CUU — Cursor Up
            'A' => {
                let n = param_or(&raw, 0, 1);
                self.grid.move_cursor_relative(-(n as i32), 0);
            }
            // CUB — Cursor Down
            'B' => {
                let n = param_or(&raw, 0, 1);
                self.grid.move_cursor_relative(n as i32, 0);
            }
            // CUF — Cursor Forward
            'C' => {
                let n = param_or(&raw, 0, 1);
                self.grid.move_cursor_relative(0, n as i32);
            }
            // CUB — Cursor Back
            'D' => {
                let n = param_or(&raw, 0, 1);
                self.grid.move_cursor_relative(0, -(n as i32));
            }
            // CUP — Cursor Position  /  HVP
            'H' | 'f' => {
                let row = param_or(&raw, 0, 1).saturating_sub(1);
                let col = param_or(&raw, 1, 1).saturating_sub(1);
                self.grid.move_cursor_to(row as usize, col as usize);
            }
            // ED — Erase in Display
            'J' => {
                let mode = param_or(&raw, 0, 0);
                self.grid.erase_in_display(mode);
            }
            // EL — Erase in Line
            'K' => {
                let mode = param_or(&raw, 0, 0);
                self.grid.erase_in_line(mode);
            }
            // SU — Scroll Up
            'S' => {
                let n = param_or(&raw, 0, 1) as usize;
                self.grid.scroll_up(n);
            }
            // SD — Scroll Down
            'T' => {
                let n = param_or(&raw, 0, 1) as usize;
                self.grid.scroll_down(n);
            }
            // SGR — Select Graphic Rendition
            'm' => {
                if raw.is_empty() {
                    self.grid.set_current_style(default_style());
                } else {
                    apply_sgr(self.grid, &raw);
                }
            }
            // DECSTBM — Set Top and Bottom Margins
            'r' => {
                let top = param_or(&raw, 0, 1).saturating_sub(1) as usize;
                let bottom = param_or(&raw, 1, self.grid.rows() as u16).saturating_sub(1) as usize;
                self.grid.set_scroll_region(top, bottom);
            }
            _ => {
                // Unknown CSI — ignore silently
            }
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        // Not handling ESC sequences for now — ignore silently.
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // Not handling OSC sequences for now — ignore silently.
    }

    fn hook(
        &mut self,
        _params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {
        // DCS hook — ignore.
    }

    fn put(&mut self, _byte: u8) {
        // DCS put — ignore.
    }

    fn unhook(&mut self) {
        // DCS unhook — ignore.
    }
}

/// Get a parameter value from the raw list, or a default if missing/zero.
fn param_or(raw: &[u16], idx: usize, default: u16) -> u16 {
    match raw.get(idx) {
        Some(&v) if v != 0 => v,
        _ => default,
    }
}

/// Apply SGR (Select Graphic Rendition) parameters to the grid's current style.
fn apply_sgr(grid: &mut TerminalGrid, params: &[u16]) {
    let mut i = 0;
    while i < params.len() {
        let code = params[i];
        match code {
            0 => grid.set_current_style(default_style()),
            1 => {
                let mut s = grid.current_style().clone();
                s.bold = true;
                grid.set_current_style(s);
            }
            2 => {
                let mut s = grid.current_style().clone();
                s.dim = true;
                grid.set_current_style(s);
            }
            3 => {
                let mut s = grid.current_style().clone();
                s.italic = true;
                grid.set_current_style(s);
            }
            4 => {
                let mut s = grid.current_style().clone();
                s.underline = true;
                grid.set_current_style(s);
            }
            5 | 6 => {
                let mut s = grid.current_style().clone();
                s.blink = true;
                grid.set_current_style(s);
            }
            7 => {
                let mut s = grid.current_style().clone();
                s.reverse = true;
                grid.set_current_style(s);
            }
            9 => {
                let mut s = grid.current_style().clone();
                s.strikethrough = true;
                grid.set_current_style(s);
            }
            22 => {
                let mut s = grid.current_style().clone();
                s.bold = false;
                s.dim = false;
                grid.set_current_style(s);
            }
            23 => {
                let mut s = grid.current_style().clone();
                s.italic = false;
                grid.set_current_style(s);
            }
            24 => {
                let mut s = grid.current_style().clone();
                s.underline = false;
                grid.set_current_style(s);
            }
            25 => {
                let mut s = grid.current_style().clone();
                s.blink = false;
                grid.set_current_style(s);
            }
            27 => {
                let mut s = grid.current_style().clone();
                s.reverse = false;
                grid.set_current_style(s);
            }
            29 => {
                let mut s = grid.current_style().clone();
                s.strikethrough = false;
                grid.set_current_style(s);
            }
            // Basic foreground colors
            30..=37 => {
                let mut s = grid.current_style().clone();
                s.fg = BASIC_COLORS[(code - 30) as usize];
                grid.set_current_style(s);
            }
            // Extended foreground: 38;5;N or 38;2;R;G;B
            38 => {
                i += 1;
                if let Some(&kind) = params.get(i) {
                    match kind {
                        5 => {
                            // 256-color
                            i += 1;
                            if let Some(&n) = params.get(i) {
                                let mut s = grid.current_style().clone();
                                s.fg = color_from_256(n);
                                grid.set_current_style(s);
                            }
                        }
                        2 => {
                            // True color: R;G;B
                            if params.len() > i + 3 {
                                let r = params[i + 1] as u8;
                                let g = params[i + 2] as u8;
                                let b = params[i + 3] as u8;
                                i += 3;
                                let mut s = grid.current_style().clone();
                                s.fg = (r, g, b);
                                grid.set_current_style(s);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Default foreground
            39 => {
                let mut s = grid.current_style().clone();
                s.fg = default_style().fg;
                grid.set_current_style(s);
            }
            // Basic background colors
            40..=47 => {
                let mut s = grid.current_style().clone();
                s.bg = BASIC_COLORS[(code - 40) as usize];
                grid.set_current_style(s);
            }
            // Extended background: 48;5;N or 48;2;R;G;B
            48 => {
                i += 1;
                if let Some(&kind) = params.get(i) {
                    match kind {
                        5 => {
                            i += 1;
                            if let Some(&n) = params.get(i) {
                                let mut s = grid.current_style().clone();
                                s.bg = color_from_256(n);
                                grid.set_current_style(s);
                            }
                        }
                        2 => {
                            if params.len() > i + 3 {
                                let r = params[i + 1] as u8;
                                let g = params[i + 2] as u8;
                                let b = params[i + 3] as u8;
                                i += 3;
                                let mut s = grid.current_style().clone();
                                s.bg = (r, g, b);
                                grid.set_current_style(s);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Default background
            49 => {
                let mut s = grid.current_style().clone();
                s.bg = default_style().bg;
                grid.set_current_style(s);
            }
            // Bright foreground colors
            90..=97 => {
                let mut s = grid.current_style().clone();
                s.fg = BRIGHT_COLORS[(code - 90) as usize];
                grid.set_current_style(s);
            }
            // Bright background colors
            100..=107 => {
                let mut s = grid.current_style().clone();
                s.bg = BRIGHT_COLORS[(code - 100) as usize];
                grid.set_current_style(s);
            }
            _ => {
                // Unknown SGR code — ignore
            }
        }
        i += 1;
    }
}
