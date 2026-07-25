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
                self.grid.move_cursor_to(self.grid.cursor_row(), target);
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

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
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
                        2
                            // True color: R;G;B
                            if params.len() > i + 3 => {
                                let r = params[i + 1] as u8;
                                let g = params[i + 2] as u8;
                                let b = params[i + 3] as u8;
                                i += 3;
                                let mut s = grid.current_style().clone();
                                s.fg = (r, g, b);
                                grid.set_current_style(s);
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
                        2 if params.len() > i + 3 => {
                            let r = params[i + 1] as u8;
                            let g = params[i + 2] as u8;
                            let b = params[i + 3] as u8;
                            i += 3;
                            let mut s = grid.current_style().clone();
                            s.bg = (r, g, b);
                            grid.set_current_style(s);
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

const MAX_PARAMS: usize = 32;
const MAX_SUBPARAM_TOTAL: usize = 64;
const MAX_INTERMEDIATES: usize = 4;

/// Fixed-capacity CSI/DCS parameter storage for event-mode parsing.
#[derive(Clone, PartialEq)]
pub struct CsiParams {
    values: [u16; MAX_SUBPARAM_TOTAL],
    lengths: [u8; MAX_PARAMS],
    param_count: u8,
    value_count: u8,
}

impl CsiParams {
    pub fn new() -> Self {
        Self {
            values: [0; MAX_SUBPARAM_TOTAL],
            lengths: [0; MAX_PARAMS],
            param_count: 0,
            value_count: 0,
        }
    }

    pub fn from_vte(params: &vte::Params) -> Self {
        let mut result = Self::new();
        for subparams in params.iter() {
            if result.param_count as usize >= MAX_PARAMS {
                break;
            }
            let len = subparams
                .len()
                .min(MAX_SUBPARAM_TOTAL - result.value_count as usize);
            let start = result.value_count as usize;
            result.values[start..start + len].copy_from_slice(&subparams[..len]);
            result.lengths[result.param_count as usize] = len as u8;
            result.param_count += 1;
            result.value_count += len as u8;
        }
        result
    }

    pub fn get(&self, index: usize) -> Option<&[u16]> {
        if index >= self.param_count as usize {
            return None;
        }
        let start: usize = self.lengths[..index].iter().map(|&l| l as usize).sum();
        let len = self.lengths[index] as usize;
        Some(&self.values[start..start + len])
    }

    pub fn iter(&self) -> CsiParamsIter<'_> {
        CsiParamsIter {
            params: self,
            index: 0,
            offset: 0,
        }
    }
}

impl Default for CsiParams {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CsiParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut list = f.debug_list();
        for param in self.iter() {
            list.entry(&param);
        }
        list.finish()
    }
}

pub struct CsiParamsIter<'a> {
    params: &'a CsiParams,
    index: usize,
    offset: usize,
}

impl<'a> Iterator for CsiParamsIter<'a> {
    type Item = &'a [u16];

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.params.param_count as usize {
            return None;
        }
        let len = self.params.lengths[self.index] as usize;
        let slice = &self.params.values[self.offset..self.offset + len];
        self.index += 1;
        self.offset += len;
        Some(slice)
    }
}

#[derive(Clone, PartialEq)]
pub struct Intermediates {
    bytes: [u8; MAX_INTERMEDIATES],
    len: u8,
}

impl Intermediates {
    pub fn from_slice(src: &[u8]) -> Self {
        let mut bytes = [0u8; MAX_INTERMEDIATES];
        let len = src.len().min(MAX_INTERMEDIATES);
        bytes[..len].copy_from_slice(&src[..len]);
        Self {
            bytes,
            len: len as u8,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

impl std::fmt::Debug for Intermediates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

/// Event-mode VT parser output used by compatibility adapters and tests.
///
/// This is a second, deliberately separate VT parsing path from
/// `GridPerformer` (used by `CompatTranslator`, which mutates a shared
/// `TerminalGrid` in place). `GridPerformer` is right for the in-process
/// compat pane path used today. This one exists for Phase H's not-yet-built
/// out-of-process compat worker (`docs/design/architecture.md`), which needs
/// parsed VT events to cross a process boundary — hence the fixed-capacity,
/// allocation-free `CsiParams`/`Intermediates` storage instead of `vte::Params`
/// borrows tied to the parser's lifetime. Not wired into anything yet; keep
/// it and its tests (`event_parser_tests` below) until Phase H lands or is
/// dropped from the roadmap.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum VtEvent {
    Print(char),
    Execute(u8),
    Csi {
        params: CsiParams,
        intermediates: Intermediates,
        ignore: bool,
        action: char,
    },
    Osc {
        params: Vec<Vec<u8>>,
        bell_terminated: bool,
    },
    Dcs {
        params: CsiParams,
        intermediates: Intermediates,
        ignore: bool,
        byte: u8,
    },
    Esc {
        intermediates: Intermediates,
        ignore: bool,
        byte: u8,
    },
    Unhook,
}

pub struct VtParser {
    parser: vte::Parser,
    collector: EventCollector,
}

impl VtParser {
    pub fn new() -> Self {
        Self {
            parser: vte::Parser::new(),
            collector: EventCollector { events: Vec::new() },
        }
    }

    pub fn advance(&mut self, bytes: &[u8]) -> &[VtEvent] {
        self.collector.events.clear();
        self.parser.advance(&mut self.collector, bytes);
        &self.collector.events
    }
}

impl Default for VtParser {
    fn default() -> Self {
        Self::new()
    }
}

struct EventCollector {
    events: Vec<VtEvent>,
}

impl vte::Perform for EventCollector {
    fn print(&mut self, c: char) {
        self.events.push(VtEvent::Print(c));
    }

    fn execute(&mut self, byte: u8) {
        self.events.push(VtEvent::Execute(byte));
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.events.push(VtEvent::Csi {
            params: CsiParams::from_vte(params),
            intermediates: Intermediates::from_slice(intermediates),
            ignore,
            action,
        });
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.events.push(VtEvent::Osc {
            params: params.iter().map(|p| p.to_vec()).collect(),
            bell_terminated,
        });
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.events.push(VtEvent::Esc {
            intermediates: Intermediates::from_slice(intermediates),
            ignore,
            byte,
        });
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, byte: char) {
        self.events.push(VtEvent::Dcs {
            params: CsiParams::from_vte(params),
            intermediates: Intermediates::from_slice(intermediates),
            ignore,
            byte: byte as u8,
        });
    }

    fn put(&mut self, byte: u8) {
        self.events.push(VtEvent::Dcs {
            params: CsiParams::new(),
            intermediates: Intermediates::from_slice(&[]),
            ignore: false,
            byte,
        });
    }

    fn unhook(&mut self) {
        self.events.push(VtEvent::Unhook);
    }
}

#[cfg(test)]
mod event_parser_tests {
    use super::*;

    #[test]
    fn vt_parser_parses_printable() {
        let mut parser = VtParser::new();
        let events = parser.advance(b"hello");
        let chars: Vec<char> = events
            .iter()
            .filter_map(|e| match e {
                VtEvent::Print(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(chars, vec!['h', 'e', 'l', 'l', 'o']);
    }

    #[test]
    fn vt_parser_parses_csi_sgr() {
        let mut parser = VtParser::new();
        let events = parser.advance(b"\x1b[31m");
        assert!(matches!(
            &events[0],
            VtEvent::Csi {
                params,
                action: 'm',
                ..
            } if params.get(0) == Some(&[31u16][..])
        ));
    }

    #[test]
    fn vt_parser_parses_osc() {
        let mut parser = VtParser::new();
        let events = parser.advance(b"\x1b]133;A\x07");
        assert!(matches!(
            &events[0],
            VtEvent::Osc {
                params,
                bell_terminated: true,
            } if params == &[b"133".to_vec(), b"A".to_vec()]
        ));
    }

    #[test]
    fn vt_parser_split_input_matches_whole() {
        let input = b"\x1b[31mhello\x1b[0m\x1b]133;A\x07";
        let mut whole = VtParser::new();
        let whole_events: Vec<VtEvent> = whole.advance(input).to_vec();

        let mut split = VtParser::new();
        let mut split_events = Vec::new();
        for b in input {
            split_events.extend_from_slice(split.advance(&[*b]));
        }
        assert_eq!(whole_events, split_events);
    }
}
