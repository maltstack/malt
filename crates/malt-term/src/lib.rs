//! `malt-term` — Line editor for MALT.
//!
//! Pure state machine: takes abstract [`InputEvent`]s, returns [`EditResult`]s.
//! No terminal I/O — the consumer manages raw mode, escape sequence parsing,
//! and rendering.

mod editor;
mod keymap;

pub mod completion;
pub mod highlight;
pub mod history;
pub mod prompt;

pub use editor::{
    EditMode, EditResult, Editor, InputEvent, KillRing, LineState, SpecialKey, UndoStack,
};
