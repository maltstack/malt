//! Core line editor state machine.

use std::collections::VecDeque;

use unicode_segmentation::UnicodeSegmentation;

use crate::keymap;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// An input event from the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputEvent {
    /// A printable character.
    Char(char),
    /// Ctrl + character (lowercased letter, e.g. Ctrl-A = Ctrl('a')).
    Ctrl(char),
    /// Alt/Meta + character.
    Alt(char),
    /// A special (non-character) key.
    Key(SpecialKey),
}

/// Special (non-character) keys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpecialKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Backspace,
    Tab,
    BackTab,
    Enter,
    Escape,
}

/// The editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditMode {
    Emacs,
    Vi,
}

/// Result of processing one input event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditResult {
    /// More input needed.
    Continue,
    /// User pressed Enter — the finished line.
    Accept(String),
    /// User pressed Ctrl-C.
    Interrupt,
    /// User pressed Ctrl-D on an empty line.
    Eof,
    /// User pressed Ctrl-Z.
    Suspend,
}

// ---------------------------------------------------------------------------
// LineState — grapheme-aware text buffer
// ---------------------------------------------------------------------------

/// Text buffer with a byte-offset cursor, all operations grapheme-aware.
#[derive(Debug, Clone)]
pub struct LineState {
    text: String,
    /// Byte offset into `text`. Always on a grapheme boundary.
    pub(crate) cursor: usize,
}

impl LineState {
    /// Create an empty line state.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte offset of the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Cursor position as a grapheme index (0-based).
    pub fn cursor_grapheme_index(&self) -> usize {
        self.text[..self.cursor].graphemes(true).count()
    }

    /// Replace the entire line, placing the cursor at the end.
    pub fn set_text(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.len();
    }

    /// Insert a character at the cursor and advance.
    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Delete the grapheme cluster at the cursor (forward delete).
    pub fn delete_char(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        if let Some(g) = rest.graphemes(true).next() {
            let len = g.len();
            self.text.drain(self.cursor..self.cursor + len);
        }
    }

    /// Delete the grapheme cluster before the cursor (backspace).
    pub fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        if let Some(g) = before.graphemes(true).next_back() {
            let len = g.len();
            self.cursor -= len;
            self.text.drain(self.cursor..self.cursor + len);
        }
    }

    /// Move cursor one grapheme to the left.
    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        if let Some(g) = before.graphemes(true).next_back() {
            self.cursor -= g.len();
        }
    }

    /// Move cursor one grapheme to the right.
    pub fn move_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        if let Some(g) = rest.graphemes(true).next() {
            self.cursor += g.len();
        }
    }

    /// Move cursor to the beginning of the previous word.
    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        // Skip trailing whitespace, then skip word chars.
        let trimmed = before.trim_end();
        if trimmed.is_empty() {
            self.cursor = 0;
            return;
        }
        // Find the start of the word: last whitespace boundary in trimmed.
        if let Some(pos) = trimmed.rfind(|c: char| c.is_whitespace()) {
            self.cursor = pos + trimmed[pos..].chars().next().map_or(0, |c| c.len_utf8());
        } else {
            self.cursor = 0;
        }
    }

    /// Move cursor to the end of the next word.
    pub fn move_word_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        // Skip leading whitespace, then skip word chars.
        let mut offset = 0;
        let mut chars = rest.char_indices();
        // Skip whitespace
        for (i, c) in chars.by_ref() {
            if !c.is_whitespace() {
                offset = i;
                break;
            }
            offset = i + c.len_utf8();
        }
        // If we consumed the whole rest while skipping whitespace
        if offset >= rest.len() {
            self.cursor = self.text.len();
            return;
        }
        // Now skip non-whitespace
        for (i, c) in rest[offset..].char_indices() {
            if c.is_whitespace() {
                self.cursor += offset + i;
                return;
            }
        }
        self.cursor = self.text.len();
    }

    /// Move cursor to the beginning of the line.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to the end of the line.
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Kill from cursor to end of line. Returns the killed text.
    pub fn kill_to_end(&mut self) -> String {
        let killed: String = self.text[self.cursor..].to_string();
        self.text.truncate(self.cursor);
        killed
    }

    /// Kill from start of line to cursor. Returns the killed text.
    pub fn kill_to_start(&mut self) -> String {
        let killed: String = self.text[..self.cursor].to_string();
        self.text = self.text[self.cursor..].to_string();
        self.cursor = 0;
        killed
    }

    /// Kill the word before the cursor. Returns the killed text.
    pub fn kill_word_backward(&mut self) -> String {
        if self.cursor == 0 {
            return String::new();
        }
        let old_cursor = self.cursor;
        self.move_word_left();
        let killed = self.text[self.cursor..old_cursor].to_string();
        self.text.drain(self.cursor..old_cursor);
        killed
    }

    /// Kill the word after the cursor. Returns the killed text.
    pub fn kill_word_forward(&mut self) -> String {
        if self.cursor >= self.text.len() {
            return String::new();
        }
        let old_cursor = self.cursor;
        self.move_word_right();
        let end = self.cursor;
        self.cursor = old_cursor;
        let killed = self.text[old_cursor..end].to_string();
        self.text.drain(old_cursor..end);
        killed
    }

    /// Transpose the two characters before and at the cursor.
    pub fn transpose_chars(&mut self) {
        // Need at least 2 graphemes and cursor > 0.
        let graphemes: Vec<String> = self.text.graphemes(true).map(|g| g.to_string()).collect();
        if graphemes.len() < 2 {
            return;
        }
        let gi = self.cursor_grapheme_index();
        // If at end of line, transpose the last two.
        let (a, b) = if gi == 0 {
            return;
        } else if gi >= graphemes.len() {
            (gi - 2, gi - 1)
        } else {
            (gi - 1, gi)
        };

        // Rebuild text with a and b swapped.
        let mut new_text = String::with_capacity(self.text.len());
        for (i, g) in graphemes.iter().enumerate() {
            if i == a {
                new_text.push_str(&graphemes[b]);
            } else if i == b {
                new_text.push_str(&graphemes[a]);
            } else {
                new_text.push_str(g);
            }
        }

        // Place cursor after the transposed pair.
        let new_cursor: usize = graphemes[..=b].iter().map(|g| g.len()).sum();
        self.text = new_text;
        self.cursor = new_cursor;
    }

    /// Insert a string at the cursor position.
    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Clear the entire line. Returns the cleared text.
    pub fn clear(&mut self) -> String {
        let old = std::mem::take(&mut self.text);
        self.cursor = 0;
        old
    }
}

impl Default for LineState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// KillRing
// ---------------------------------------------------------------------------

/// Circular buffer of killed text.
#[derive(Debug, Clone)]
pub struct KillRing {
    ring: VecDeque<String>,
    capacity: usize,
}

impl KillRing {
    /// Create a kill ring with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push killed text onto the ring.
    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.ring.push_front(text);
        if self.ring.len() > self.capacity {
            self.ring.pop_back();
        }
    }

    /// Yank the most recently killed text.
    pub fn yank(&self) -> Option<&str> {
        self.ring.front().map(|s| s.as_str())
    }
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new(8)
    }
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// Stack of `(text, cursor)` snapshots for undo.
#[derive(Debug, Clone)]
pub struct UndoStack {
    states: Vec<(String, usize)>,
    capacity: usize,
}

impl UndoStack {
    /// Create an undo stack with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            states: Vec::new(),
            capacity,
        }
    }

    /// Save a snapshot before a mutation.
    pub fn push(&mut self, text: &str, cursor: usize) {
        self.states.push((text.to_string(), cursor));
        if self.states.len() > self.capacity {
            self.states.remove(0);
        }
    }

    /// Undo: pop and return the previous state.
    pub fn undo(&mut self) -> Option<(String, usize)> {
        self.states.pop()
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(100)
    }
}

// ---------------------------------------------------------------------------
// Vi mode
// ---------------------------------------------------------------------------

/// Vi sub-mode (only relevant when `EditMode::Vi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViMode {
    Insert,
    Normal,
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

/// Pure state-machine line editor.
///
/// Feed [`InputEvent`]s via [`Editor::feed`] and inspect the returned
/// [`EditResult`] to know when a line has been accepted, interrupted, etc.
pub struct Editor {
    line: LineState,
    mode: EditMode,
    vi_mode: ViMode,
    kill_ring: KillRing,
    undo_stack: UndoStack,
    prompt: String,
}

impl Editor {
    /// Create a new editor in the given mode.
    pub fn new(mode: EditMode) -> Self {
        Self {
            line: LineState::new(),
            mode,
            vi_mode: ViMode::Insert,
            kill_ring: KillRing::default(),
            undo_stack: UndoStack::default(),
            prompt: String::new(),
        }
    }

    /// Process one input event and return the result.
    pub fn feed(&mut self, event: InputEvent) -> EditResult {
        match self.mode {
            EditMode::Emacs => keymap::handle_emacs(self, event),
            EditMode::Vi => match self.vi_mode {
                ViMode::Insert => keymap::handle_vi_insert(self, event),
                ViMode::Normal => keymap::handle_vi_normal(self, event),
            },
        }
    }

    /// The current line text.
    pub fn line(&self) -> &str {
        self.line.text()
    }

    /// Cursor position as a grapheme index.
    pub fn cursor(&self) -> usize {
        self.line.cursor_grapheme_index()
    }

    /// Set the prompt string.
    pub fn set_prompt(&mut self, prompt: &str) {
        self.prompt = prompt.to_string();
    }

    /// Vi mode indicator: `"INS"`, `"NOR"`, or `""` (emacs).
    pub fn vi_mode_indicator(&self) -> &str {
        match self.mode {
            EditMode::Emacs => "",
            EditMode::Vi => match self.vi_mode {
                ViMode::Insert => "INS",
                ViMode::Normal => "NOR",
            },
        }
    }

    // -- internal helpers used by keymap module --

    pub(crate) fn line_mut(&mut self) -> &mut LineState {
        &mut self.line
    }

    pub(crate) fn kill_ring_mut(&mut self) -> &mut KillRing {
        &mut self.kill_ring
    }

    pub(crate) fn undo_stack_mut(&mut self) -> &mut UndoStack {
        &mut self.undo_stack
    }

    pub(crate) fn kill_ring_ref(&self) -> &KillRing {
        &self.kill_ring
    }

    pub(crate) fn enter_vi_insert(&mut self) {
        self.vi_mode = ViMode::Insert;
    }

    pub(crate) fn enter_vi_normal(&mut self) {
        self.vi_mode = ViMode::Normal;
    }

    /// Clear the current line buffer and return to Insert mode (for Vi) or
    /// the start state (for Emacs). Call this after Accept, Interrupt, or Eof
    /// so the editor is ready for the next line without constructing a new instance.
    pub fn reset(&mut self) {
        self.line.clear();
        self.vi_mode = ViMode::Insert;
        self.undo_stack = UndoStack::default();
    }
}
