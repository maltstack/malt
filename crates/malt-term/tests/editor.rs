use malt_term::{EditMode, EditResult, Editor, InputEvent, SpecialKey};

#[test]
fn insert_chars() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('h'));
    ed.feed(InputEvent::Char('i'));
    assert_eq!(ed.line(), "hi");
}

#[test]
fn accept_on_enter() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('h'));
    ed.feed(InputEvent::Char('i'));
    let result = ed.feed(InputEvent::Key(SpecialKey::Enter));
    assert!(matches!(result, EditResult::Accept(s) if s == "hi"));
}

#[test]
fn ctrl_c_interrupts() {
    let mut ed = Editor::new(EditMode::Emacs);
    let result = ed.feed(InputEvent::Ctrl('c'));
    assert!(matches!(result, EditResult::Interrupt));
}

#[test]
fn ctrl_d_eof_on_empty() {
    let mut ed = Editor::new(EditMode::Emacs);
    let result = ed.feed(InputEvent::Ctrl('d'));
    assert!(matches!(result, EditResult::Eof));
}

#[test]
fn ctrl_d_deletes_on_nonempty() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Ctrl('a')); // move to start
    ed.feed(InputEvent::Ctrl('d')); // delete 'a'
    assert_eq!(ed.line(), "b");
}

#[test]
fn backspace_deletes_backward() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Key(SpecialKey::Backspace));
    assert_eq!(ed.line(), "a");
}

#[test]
fn ctrl_a_moves_home() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Ctrl('a'));
    assert_eq!(ed.cursor(), 0);
}

#[test]
fn ctrl_e_moves_end() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Ctrl('a'));
    ed.feed(InputEvent::Ctrl('e'));
    assert_eq!(ed.cursor(), 2);
}

#[test]
fn ctrl_k_kills_to_end() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('h'));
    ed.feed(InputEvent::Char('e'));
    ed.feed(InputEvent::Char('l'));
    ed.feed(InputEvent::Char('l'));
    ed.feed(InputEvent::Char('o'));
    ed.feed(InputEvent::Ctrl('a')); // move home
    ed.feed(InputEvent::Ctrl('f')); // move right to 'e'
    ed.feed(InputEvent::Ctrl('k')); // kill "ello"
    assert_eq!(ed.line(), "h");
}

#[test]
fn ctrl_y_yanks() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('h'));
    ed.feed(InputEvent::Char('i'));
    ed.feed(InputEvent::Ctrl('u')); // kill "hi"
    assert_eq!(ed.line(), "");
    ed.feed(InputEvent::Ctrl('y')); // yank "hi"
    assert_eq!(ed.line(), "hi");
}

#[test]
fn vi_insert_to_normal() {
    let mut ed = Editor::new(EditMode::Vi);
    // Starts in insert mode
    ed.feed(InputEvent::Char('h'));
    ed.feed(InputEvent::Char('i'));
    assert_eq!(ed.line(), "hi");
    ed.feed(InputEvent::Key(SpecialKey::Escape)); // enter normal
    assert_eq!(ed.vi_mode_indicator(), "NOR");
}

#[test]
fn vi_normal_motions() {
    let mut ed = Editor::new(EditMode::Vi);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Char('c'));
    ed.feed(InputEvent::Key(SpecialKey::Escape)); // normal mode
    // Cursor moves back one on Escape (vi convention), now on 'c' (index 2)
    // h twice: index 0 -> on 'a'
    ed.feed(InputEvent::Char('h')); // move left -> on 'b'
    ed.feed(InputEvent::Char('h')); // move left -> on 'a'
    ed.feed(InputEvent::Char('x')); // delete char at cursor ('a')
    assert_eq!(ed.line(), "bc");
}

#[test]
fn vi_i_returns_to_insert() {
    let mut ed = Editor::new(EditMode::Vi);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Key(SpecialKey::Escape));
    // Cursor moved back to 'a' (byte 0)
    ed.feed(InputEvent::Char('i')); // back to insert at current position
    ed.feed(InputEvent::Char('x'));
    assert_eq!(ed.line(), "xa"); // inserted before 'a'
}

#[test]
fn unicode_grapheme_cursor() {
    let mut ed = Editor::new(EditMode::Emacs);
    // Insert emoji (multi-byte)
    ed.feed(InputEvent::Char('😀'));
    ed.feed(InputEvent::Char('x'));
    assert_eq!(ed.line(), "😀x");
    ed.feed(InputEvent::Key(SpecialKey::Backspace));
    assert_eq!(ed.line(), "😀");
}

#[test]
fn ctrl_w_kills_word_backward() {
    let mut ed = Editor::new(EditMode::Emacs);
    for c in "hello world".chars() {
        ed.feed(InputEvent::Char(c));
    }
    ed.feed(InputEvent::Ctrl('w'));
    assert_eq!(ed.line(), "hello ");
}

#[test]
fn vi_dd_clears_line() {
    let mut ed = Editor::new(EditMode::Vi);
    for c in "hello".chars() {
        ed.feed(InputEvent::Char(c));
    }
    ed.feed(InputEvent::Key(SpecialKey::Escape));
    ed.feed(InputEvent::Char('d'));
    ed.feed(InputEvent::Char('d'));
    assert_eq!(ed.line(), "");
}

#[test]
fn vi_d_capital_kills_to_end() {
    let mut ed = Editor::new(EditMode::Vi);
    for c in "hello".chars() {
        ed.feed(InputEvent::Char(c));
    }
    ed.feed(InputEvent::Key(SpecialKey::Escape));
    // cursor on 'o' (last char); move to start
    ed.feed(InputEvent::Char('0'));
    ed.feed(InputEvent::Char('l')); // on 'e'
    ed.feed(InputEvent::Char('D')); // kill "ello"
    assert_eq!(ed.line(), "h");
}

#[test]
fn transpose_chars() {
    let mut ed = Editor::new(EditMode::Emacs);
    ed.feed(InputEvent::Char('a'));
    ed.feed(InputEvent::Char('b'));
    ed.feed(InputEvent::Ctrl('t'));
    assert_eq!(ed.line(), "ba");
}

#[test]
fn suspend_signal() {
    let mut ed = Editor::new(EditMode::Emacs);
    let result = ed.feed(InputEvent::Ctrl('z'));
    assert!(matches!(result, EditResult::Suspend));
}
