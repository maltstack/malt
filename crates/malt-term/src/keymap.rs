//! Key dispatch for Emacs and Vi modes.

use crate::editor::{EditResult, Editor, InputEvent, SpecialKey};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Save undo snapshot, then run a closure that mutates the editor.
fn with_undo<F>(ed: &mut Editor, f: F) -> EditResult
where
    F: FnOnce(&mut Editor) -> EditResult,
{
    let text = ed.line().to_string();
    let cursor = ed.line_mut().cursor();
    ed.undo_stack_mut().push(&text, cursor);
    f(ed)
}

// ---------------------------------------------------------------------------
// Emacs mode
// ---------------------------------------------------------------------------

pub(crate) fn handle_emacs(ed: &mut Editor, event: InputEvent) -> EditResult {
    match event {
        // -- accept / signals --
        InputEvent::Key(SpecialKey::Enter) => EditResult::Accept(ed.line().to_string()),

        InputEvent::Ctrl('c') => EditResult::Interrupt,

        InputEvent::Ctrl('d') => {
            if ed.line().is_empty() {
                EditResult::Eof
            } else {
                with_undo(ed, |ed| {
                    ed.line_mut().delete_char();
                    EditResult::Continue
                })
            }
        }

        InputEvent::Ctrl('z') => EditResult::Suspend,

        // -- movement --
        InputEvent::Ctrl('a') | InputEvent::Key(SpecialKey::Home) => {
            ed.line_mut().move_home();
            EditResult::Continue
        }

        InputEvent::Ctrl('e') | InputEvent::Key(SpecialKey::End) => {
            ed.line_mut().move_end();
            EditResult::Continue
        }

        InputEvent::Ctrl('b') | InputEvent::Key(SpecialKey::Left) => {
            ed.line_mut().move_left();
            EditResult::Continue
        }

        InputEvent::Ctrl('f') | InputEvent::Key(SpecialKey::Right) => {
            ed.line_mut().move_right();
            EditResult::Continue
        }

        InputEvent::Alt('b') => {
            ed.line_mut().move_word_left();
            EditResult::Continue
        }

        InputEvent::Alt('f') => {
            ed.line_mut().move_word_right();
            EditResult::Continue
        }

        // -- kill / yank --
        InputEvent::Ctrl('k') => with_undo(ed, |ed| {
            let killed = ed.line_mut().kill_to_end();
            ed.kill_ring_mut().push(killed);
            EditResult::Continue
        }),

        InputEvent::Ctrl('u') => with_undo(ed, |ed| {
            let killed = ed.line_mut().kill_to_start();
            ed.kill_ring_mut().push(killed);
            EditResult::Continue
        }),

        InputEvent::Ctrl('w') => with_undo(ed, |ed| {
            let killed = ed.line_mut().kill_word_backward();
            ed.kill_ring_mut().push(killed);
            EditResult::Continue
        }),

        InputEvent::Alt('d') => with_undo(ed, |ed| {
            let killed = ed.line_mut().kill_word_forward();
            ed.kill_ring_mut().push(killed);
            EditResult::Continue
        }),

        InputEvent::Ctrl('y') => {
            if let Some(text) = ed.kill_ring_ref().yank() {
                let text = text.to_string();
                with_undo(ed, |ed| {
                    ed.line_mut().insert_str(&text);
                    EditResult::Continue
                })
            } else {
                EditResult::Continue
            }
        }

        // -- editing --
        InputEvent::Ctrl('t') => with_undo(ed, |ed| {
            ed.line_mut().transpose_chars();
            EditResult::Continue
        }),

        InputEvent::Key(SpecialKey::Backspace) => {
            if ed.line().is_empty() {
                EditResult::Continue
            } else {
                with_undo(ed, |ed| {
                    ed.line_mut().delete_backward();
                    EditResult::Continue
                })
            }
        }

        InputEvent::Key(SpecialKey::Delete) => {
            if ed.line().is_empty() {
                EditResult::Continue
            } else {
                with_undo(ed, |ed| {
                    ed.line_mut().delete_char();
                    EditResult::Continue
                })
            }
        }

        // -- clear screen (consumer handles actual clear) --
        InputEvent::Ctrl('l') => EditResult::Continue,

        // -- history stubs --
        InputEvent::Key(SpecialKey::Up) | InputEvent::Ctrl('p') => EditResult::Continue,
        InputEvent::Key(SpecialKey::Down) | InputEvent::Ctrl('n') => EditResult::Continue,

        // -- completion stub --
        InputEvent::Key(SpecialKey::Tab) => EditResult::Continue,

        // -- character insertion --
        InputEvent::Char(c) => with_undo(ed, |ed| {
            ed.line_mut().insert_char(c);
            EditResult::Continue
        }),

        // -- unhandled --
        _ => EditResult::Continue,
    }
}

// ---------------------------------------------------------------------------
// Vi Insert mode
// ---------------------------------------------------------------------------

pub(crate) fn handle_vi_insert(ed: &mut Editor, event: InputEvent) -> EditResult {
    match event {
        InputEvent::Key(SpecialKey::Escape) => {
            // Move cursor back one position when entering normal mode (vi convention).
            if ed.line_mut().cursor() > 0 {
                ed.line_mut().move_left();
            }
            ed.enter_vi_normal();
            EditResult::Continue
        }

        InputEvent::Key(SpecialKey::Enter) => EditResult::Accept(ed.line().to_string()),

        InputEvent::Ctrl('c') => EditResult::Interrupt,

        InputEvent::Ctrl('d') => {
            if ed.line().is_empty() {
                EditResult::Eof
            } else {
                with_undo(ed, |ed| {
                    ed.line_mut().delete_char();
                    EditResult::Continue
                })
            }
        }

        InputEvent::Key(SpecialKey::Backspace) => {
            if ed.line().is_empty() {
                EditResult::Continue
            } else {
                with_undo(ed, |ed| {
                    ed.line_mut().delete_backward();
                    EditResult::Continue
                })
            }
        }

        InputEvent::Char(c) => with_undo(ed, |ed| {
            ed.line_mut().insert_char(c);
            EditResult::Continue
        }),

        // Movement keys still work in insert mode.
        InputEvent::Key(SpecialKey::Left) => {
            ed.line_mut().move_left();
            EditResult::Continue
        }
        InputEvent::Key(SpecialKey::Right) => {
            ed.line_mut().move_right();
            EditResult::Continue
        }
        InputEvent::Key(SpecialKey::Home) => {
            ed.line_mut().move_home();
            EditResult::Continue
        }
        InputEvent::Key(SpecialKey::End) => {
            ed.line_mut().move_end();
            EditResult::Continue
        }
        InputEvent::Key(SpecialKey::Up) => EditResult::Continue,
        InputEvent::Key(SpecialKey::Down) => EditResult::Continue,

        _ => EditResult::Continue,
    }
}

// ---------------------------------------------------------------------------
// Vi Normal mode
// ---------------------------------------------------------------------------

pub(crate) fn handle_vi_normal(ed: &mut Editor, event: InputEvent) -> EditResult {
    // If we previously saw a 'd', check for the second 'd'.
    if ed.vi_dd_pending() {
        ed.set_vi_dd_pending(false);
        if event == InputEvent::Char('d') {
            return with_undo(ed, |ed| {
                let killed = ed.line_mut().clear();
                ed.kill_ring_mut().push(killed);
                EditResult::Continue
            });
        }
        // Not 'dd' — treat the new event normally (the single 'd' is lost).
    }

    match event {
        // -- mode switching --
        InputEvent::Char('i') => {
            ed.enter_vi_insert();
            EditResult::Continue
        }

        InputEvent::Char('a') => {
            ed.line_mut().move_right();
            ed.enter_vi_insert();
            EditResult::Continue
        }

        InputEvent::Char('A') => {
            ed.line_mut().move_end();
            ed.enter_vi_insert();
            EditResult::Continue
        }

        InputEvent::Char('I') => {
            ed.line_mut().move_home();
            ed.enter_vi_insert();
            EditResult::Continue
        }

        // -- movement --
        InputEvent::Char('h') | InputEvent::Key(SpecialKey::Left) => {
            ed.line_mut().move_left();
            EditResult::Continue
        }

        InputEvent::Char('l') | InputEvent::Key(SpecialKey::Right) => {
            ed.line_mut().move_right();
            EditResult::Continue
        }

        InputEvent::Char('w') => {
            ed.line_mut().move_word_right();
            EditResult::Continue
        }

        InputEvent::Char('b') => {
            ed.line_mut().move_word_left();
            EditResult::Continue
        }

        InputEvent::Char('0') => {
            ed.line_mut().move_home();
            EditResult::Continue
        }

        InputEvent::Char('$') => {
            ed.line_mut().move_end();
            EditResult::Continue
        }

        // -- editing --
        InputEvent::Char('x') => with_undo(ed, |ed| {
            ed.line_mut().delete_char();
            EditResult::Continue
        }),

        InputEvent::Char('X') => with_undo(ed, |ed| {
            ed.line_mut().delete_backward();
            EditResult::Continue
        }),

        InputEvent::Char('d') => {
            ed.set_vi_dd_pending(true);
            EditResult::Continue
        }

        InputEvent::Char('D') => with_undo(ed, |ed| {
            let killed = ed.line_mut().kill_to_end();
            ed.kill_ring_mut().push(killed);
            EditResult::Continue
        }),

        InputEvent::Char('u') => {
            if let Some((text, cursor)) = ed.undo_stack_mut().undo() {
                ed.line_mut().set_text(&text);
                // Restore cursor byte offset.
                let line = ed.line_mut();
                if cursor <= line.text().len() {
                    line.cursor = cursor;
                }
            }
            EditResult::Continue
        }

        InputEvent::Char('p') => {
            if let Some(text) = ed.kill_ring_ref().yank() {
                let text = text.to_string();
                with_undo(ed, |ed| {
                    ed.line_mut().move_right();
                    ed.line_mut().insert_str(&text);
                    EditResult::Continue
                })
            } else {
                EditResult::Continue
            }
        }

        InputEvent::Char('P') => {
            if let Some(text) = ed.kill_ring_ref().yank() {
                let text = text.to_string();
                with_undo(ed, |ed| {
                    ed.line_mut().insert_str(&text);
                    EditResult::Continue
                })
            } else {
                EditResult::Continue
            }
        }

        // -- enter on normal mode also accepts --
        InputEvent::Key(SpecialKey::Enter) => EditResult::Accept(ed.line().to_string()),

        InputEvent::Ctrl('c') => EditResult::Interrupt,

        // -- stubs --
        InputEvent::Key(SpecialKey::Escape) => EditResult::Continue, // no-op in normal
        InputEvent::Char(':') => EditResult::Continue,               // ex mode stub
        InputEvent::Char('/') => EditResult::Continue,               // search stub

        _ => EditResult::Continue,
    }
}
