# Phase 2: `malt-term` — Line Editor

## Goal

Build the line editor — vi/emacs mode state machine, history with search and bang expansion, completion framework with pluggable sources, and syntax highlighting trait. Pure state machine — no terminal I/O.

## Architecture

Generic line editor crate. Takes abstract `InputEvent`s, returns `EditResult`s. No direct terminal handling — the consumer (mash binary) manages raw mode, escape sequence parsing, and rendering. Completion sources are injected via trait. Shell-specific sources (command, variable) implemented by mash, not here.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-term\src/` (6,373 lines). Port the editing logic and state machines. Rewrite sync (reference was async for completion).

---

## Crate Structure

```
orix/malt/crates/malt-term/
  Cargo.toml
  src/
    lib.rs              # Re-exports
    editor.rs           # Editor state machine, LineState, EditResult
    keymap.rs           # EditMode, vi/emacs dispatch, Action enum
    history.rs          # History ring buffer, search, bang expansion, persistence
    completion.rs       # CompletionSource trait, Candidate, CompletionCoordinator
    highlight.rs        # Highlighter trait
    prompt.rs           # PromptRenderer
  tests/
    editor.rs
    history.rs
    completion.rs
```

---

## Dependencies

```toml
[dependencies]
unicode-segmentation = "1"
thiserror = "2"
```

No malt-protocol, no malt-platform. Generic and reusable.

---

## Core Types

### InputEvent

```rust
pub enum InputEvent {
    Char(char),
    Ctrl(char),
    Alt(char),
    Key(SpecialKey),
}

pub enum SpecialKey {
    Up, Down, Left, Right,
    Home, End, Delete, Backspace,
    Tab, BackTab, Enter, Escape,
    PageUp, PageDown,
}
```

### EditResult

```rust
pub enum EditResult {
    Continue,
    Accept(String),
    Interrupt,       // Ctrl-C
    Eof,             // Ctrl-D on empty line
    Suspend,         // Ctrl-Z
}
```

### EditMode

```rust
pub enum EditMode {
    Emacs,
    Vi,
}
```

---

## Editor (editor.rs)

Pure state machine. One instance per prompt interaction.

```rust
pub struct Editor {
    line: LineState,
    mode: EditMode,
    vi_mode: ViMode,            // Insert or Normal (when EditMode::Vi)
    history_nav: HistoryNav,    // Current position in history navigation
    completion_state: Option<CompletionState>,
    kill_ring: KillRing,
    undo_stack: UndoStack,
    prompt: String,
}

pub struct LineState {
    text: String,
    cursor: usize,   // byte offset
}

enum ViMode { Insert, Normal }

impl Editor {
    pub fn new(mode: EditMode) -> Self;
    pub fn feed(&mut self, event: InputEvent, history: &History, completer: Option<&CompletionCoordinator>) -> EditResult;
    pub fn line(&self) -> &str;
    pub fn cursor(&self) -> usize;      // grapheme index
    pub fn cursor_byte(&self) -> usize; // byte offset
    pub fn set_prompt(&mut self, prompt: &str);
    pub fn vi_mode_indicator(&self) -> &str; // "INS" / "NOR" / ""
}
```

### Key dispatch

**Emacs mode:**
- Ctrl-A: beginning of line
- Ctrl-E: end of line
- Ctrl-B / Left: backward char
- Ctrl-F / Right: forward char
- Alt-B: backward word
- Alt-F: forward word
- Ctrl-D: delete char (or EOF if empty)
- Backspace: backward delete
- Ctrl-K: kill to end of line
- Ctrl-U: kill to beginning
- Ctrl-W: kill backward word
- Alt-D: kill forward word
- Ctrl-Y: yank from kill ring
- Ctrl-T: transpose chars
- Ctrl-P / Up: history previous
- Ctrl-N / Down: history next
- Ctrl-R: reverse history search
- Tab: completion
- Enter: accept

**Vi Normal mode:**
- h/l: left/right
- w/b/e: word motions
- 0/$: beginning/end of line
- i/a/A/I: enter insert mode
- x: delete char
- dd: delete line
- dw/db: delete word
- cc/cw: change
- yy/yw: yank
- p/P: paste
- u: undo
- /: search

**Vi Insert mode:**
- Escape: enter normal mode
- All typing inserts characters
- Ctrl-C: interrupt

### LineState operations

Using `unicode-segmentation` for grapheme-aware cursor movement:
- `insert_char(ch)` at cursor
- `delete_char()` at cursor
- `delete_backward()` before cursor
- `move_left()` / `move_right()` by grapheme
- `move_word_left()` / `move_word_right()` by word boundary
- `move_home()` / `move_end()`
- `kill_to_end()` / `kill_to_start()` → returns killed text

### KillRing

Circular buffer of killed text (capacity 8). `yank()` returns most recent, `yank_rotate()` cycles.

### UndoStack

Stack of `(text, cursor)` snapshots. `push()` before mutations, `undo()` restores previous.

---

## History (history.rs)

```rust
pub struct History {
    entries: VecDeque<HistoryEntry>,
    max_entries: usize,
}

pub struct HistoryEntry {
    pub line: String,
    pub timestamp: u64,
}

impl History {
    pub fn new(max_entries: usize) -> Self; // default 10000
    pub fn add(&mut self, line: String);
    pub fn len(&self) -> usize;
    pub fn get(&self, index: usize) -> Option<&HistoryEntry>;

    // Search
    pub fn search_prefix(&self, prefix: &str, from: usize) -> Option<(usize, &str)>;
    pub fn search_backward(&self, substring: &str, from: usize) -> Option<(usize, &str)>;

    // Bang expansion
    pub fn expand_bangs(&self, input: &str) -> Result<String, HistoryError>;
    // !! → last, !n → nth, !str → most recent starting with str, ^old^new → substitute

    // Persistence
    pub fn load(&mut self, path: &std::path::Path) -> Result<(), std::io::Error>;
    pub fn save(&self, path: &std::path::Path) -> Result<(), std::io::Error>;
}
```

File format: one entry per line (simple, portable). Timestamps as comments.

---

## Completion (completion.rs)

```rust
pub struct Candidate {
    pub value: String,
    pub display: String,
    pub kind: CompletionKind,
}

pub enum CompletionKind {
    File, Directory, Command, Variable, History, Custom(String),
}

pub trait CompletionSource: Send {
    fn complete(&self, line: &str, cursor: usize) -> Vec<Candidate>;
}

pub struct CompletionCoordinator {
    sources: Vec<Box<dyn CompletionSource>>,
}

impl CompletionCoordinator {
    pub fn new() -> Self;
    pub fn add_source(&mut self, source: Box<dyn CompletionSource>);
    pub fn complete(&self, line: &str, cursor: usize) -> Vec<Candidate>;
}

/// Helper: find longest common prefix among candidates.
pub fn common_prefix(candidates: &[Candidate]) -> String;
```

The coordinator runs sources sequentially (sync), deduplicates by value, sorts by kind then alphabetically.

**Built-in source: PathCompletionSource** — completes file/directory paths. This is generic (no shell knowledge needed) so it lives in malt-term.

Shell-specific sources (command names, variables, aliases) are implemented by mash and injected via `add_source()`.

---

## Highlight (highlight.rs)

```rust
pub struct Style {
    pub fg: Option<Color>,
    pub bold: bool,
}

pub enum Color {
    Red, Green, Yellow, Blue, Magenta, Cyan, White, Default,
}

pub trait Highlighter: Send {
    fn highlight(&self, line: &str) -> Vec<Span>;
}

pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}
```

Trait only — shell-specific highlighter (keyword coloring, error detection) implemented by mash.

---

## Prompt (prompt.rs)

```rust
pub struct PromptRenderer {
    template: String,
}

impl PromptRenderer {
    pub fn new(template: &str) -> Self;
    pub fn render(&self, cwd: &str, user: &str, hostname: &str, exit_code: i32) -> String;
}
```

Simple template expansion: `\w` = cwd, `\u` = user, `\h` = hostname, `\$` = $ or # (root), `\?` = last exit code. Expandable later.

---

## Testing Strategy

### Editor (10+ tests)
- Insert chars, verify line content
- Backspace deletes backward
- Ctrl-A moves to start, Ctrl-E to end
- Ctrl-K kills to end, Ctrl-Y yanks back
- Vi: Escape enters normal, i returns to insert
- Vi normal: w moves word, x deletes, dd clears line
- Enter returns Accept(line)
- Ctrl-C returns Interrupt
- Ctrl-D on empty returns Eof
- History navigation with Up/Down

### History (8+ tests)
- Add entries, verify count
- Ring buffer eviction at capacity
- search_prefix finds match
- search_backward finds substring
- expand_bangs: !! last command
- expand_bangs: !n nth command
- expand_bangs: !str prefix match
- Load/save file roundtrip

### Completion (5+ tests)
- Mock source returns candidates
- Coordinator merges from multiple sources
- common_prefix calculation
- PathCompletionSource finds files in tempdir
- Empty input returns no candidates
