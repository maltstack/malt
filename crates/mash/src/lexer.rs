//! POSIX shell lexer -- streaming zero-copy tokenization.
//!
//! Returns `Spanned<Token>` items via `Iterator`. Words are zero-copy spans
//! into the original input. Operators and redirects carry spans covering their
//! exact characters.
//!
//! This module implements:
//! - Whitespace / comment skipping
//! - Operator dispatch (`|`, `||`, `&&`, `;`, `;;`, `&`, `(`, `((`, `)`, `))`,
//!   `{`, `}`, `[[`, `]]`)
//! - Redirect operators (`<`, `>`, `>>`, `>|`, `<>`, `<&`, `>&`, `&>`, `<<<`,
//!   `<<`, `<<-`)
//! - Word reading (basic boundary detection -- quoting and expansions are later tasks)
//! - IoNumber detection (all-digit word followed by `<` or `>`)
//! - CRLF handling (in-stream, no pre-normalization)

use crate::ast::{LexError, RedirectKind, Span, Spanned, Token};

/// A pending heredoc whose `<<` / `<<-` has been emitted but whose delimiter
/// has not yet been consumed.
#[derive(Debug)]
pub struct PendingHeredoc {
    pub strip_tabs: bool,
}

/// Streaming zero-copy lexer for MASH shell input.
pub struct Lexer<'a> {
    input: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    /// Heredocs awaiting delimiter words and body accumulation.
    pub pending_heredocs: Vec<PendingHeredoc>,
    finished: bool,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer over the given input string.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            pending_heredocs: Vec::new(),
            finished: false,
        }
    }

    /// Peek at the next character without consuming it.
    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().map(|&(_, c)| c)
    }

    /// Peek at the byte position and character without consuming.
    fn peek(&mut self) -> Option<(usize, char)> {
        self.chars.peek().copied()
    }

    /// Consume and return the next (position, char) pair.
    fn next_char(&mut self) -> Option<(usize, char)> {
        self.chars.next()
    }

    /// Build a `Span` from byte offsets.
    fn make_span(&self, start: usize, end: usize) -> Span {
        Span::new(start as u32, end as u32)
    }

    /// Lex operators starting with `<`.
    fn lex_less_than(&mut self, start: usize) -> Result<Spanned<Token>, LexError> {
        match self.peek_char() {
            Some('<') => {
                self.next_char();
                match self.peek_char() {
                    Some('<') => {
                        self.next_char();
                        let span = self.make_span(start, start + 3);
                        Ok(Spanned { node: Token::Redirect(RedirectKind::HereString), span })
                    }
                    Some('-') => {
                        self.next_char();
                        self.pending_heredocs.push(PendingHeredoc { strip_tabs: true });
                        let span = self.make_span(start, start + 3);
                        Ok(Spanned { node: Token::Redirect(RedirectKind::HereDocStrip), span })
                    }
                    _ => {
                        self.pending_heredocs.push(PendingHeredoc { strip_tabs: false });
                        let span = self.make_span(start, start + 2);
                        Ok(Spanned { node: Token::Redirect(RedirectKind::HereDoc), span })
                    }
                }
            }
            Some('>') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned { node: Token::Redirect(RedirectKind::InputOutput), span })
            }
            Some('&') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned { node: Token::Redirect(RedirectKind::DupInput), span })
            }
            _ => {
                let span = self.make_span(start, start + 1);
                Ok(Spanned { node: Token::Redirect(RedirectKind::Input), span })
            }
        }
    }

    /// Lex operators starting with `>`.
    fn lex_greater_than(&mut self, start: usize) -> Result<Spanned<Token>, LexError> {
        match self.peek_char() {
            Some('>') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned { node: Token::Redirect(RedirectKind::Append), span })
            }
            Some('|') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned { node: Token::Redirect(RedirectKind::Clobber), span })
            }
            Some('&') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned { node: Token::Redirect(RedirectKind::DupOutput), span })
            }
            _ => {
                let span = self.make_span(start, start + 1);
                Ok(Spanned { node: Token::Redirect(RedirectKind::Output), span })
            }
        }
    }

    /// Read a word starting from position `start` (the first character at `start`
    /// has already been consumed by the caller). Continues until a word-breaking
    /// character is found.
    ///
    /// For this initial implementation, quoting and expansion characters are
    /// included as part of the word -- they do not break it. Later tasks will add
    /// proper quoting / expansion tracking.
    fn read_word(&mut self, start: usize) -> Span {
        // The first character has already been consumed. Keep going until we hit
        // a word-breaking character.
        loop {
            match self.peek() {
                None => break,
                Some((_, ch)) => {
                    if is_word_break(ch) {
                        break;
                    }
                    self.next_char();
                }
            }
        }
        // Compute end position: either the position of the next char we stopped
        // at, or end of input.
        let end = match self.peek() {
            Some((pos, _)) => pos,
            None => self.input.len(),
        };
        self.make_span(start, end)
    }

    /// Try to skip whitespace (space, tab). Returns true if any was skipped.
    fn skip_whitespace(&mut self) -> bool {
        let mut skipped = false;
        while let Some(&(_, ch)) = self.chars.peek() {
            if ch == ' ' || ch == '\t' {
                self.next_char();
                skipped = true;
            } else {
                break;
            }
        }
        skipped
    }

    /// Skip a comment (`#` to end of line). The `#` has already been consumed.
    /// Does NOT consume the newline -- the caller handles that.
    fn skip_comment(&mut self) {
        while let Some(&(_, ch)) = self.chars.peek() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            self.next_char();
        }
    }
}

/// Characters that break a word. Does not include `#` because `#` only starts
/// a comment at the beginning of a token, not inside a word.
fn is_word_break(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '|'
            | '&'
            | ';'
            | '<'
            | '>'
            | '('
            | ')'
            | '{'
            | '}'
            | '#'
    )
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token>, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        // Skip whitespace.
        self.skip_whitespace();

        // Get the next character (or emit Eof).
        let (pos, ch) = match self.peek() {
            Some(pair) => pair,
            None => {
                self.finished = true;
                let eof_pos = self.input.len();
                let span = self.make_span(eof_pos, eof_pos);
                return Some(Ok(Spanned { node: Token::Eof, span }));
            }
        };

        // Consume the character we just peeked.
        self.next_char();

        let result = match ch {
            // Newline
            '\n' => {
                let span = self.make_span(pos, pos + 1);
                Ok(Spanned { node: Token::Newline, span })
            }

            // CRLF -- treat \r\n as a single newline
            '\r' => {
                if self.peek_char() == Some('\n') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::Newline, span })
                } else {
                    // Bare \r treated as newline
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::Newline, span })
                }
            }

            // Comment
            '#' => {
                self.skip_comment();
                // After skipping the comment, we need to return the next token.
                // Recurse (the iterator will handle the next call).
                return self.next();
            }

            // Pipe / OrOr
            '|' => {
                if self.peek_char() == Some('|') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::OrOr, span })
                } else {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::Pipe, span })
                }
            }

            // Ampersand / AndAnd / &>
            '&' => {
                if self.peek_char() == Some('&') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::AndAnd, span })
                } else if self.peek_char() == Some('>') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::Redirect(RedirectKind::Both), span })
                } else {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::Ampersand, span })
                }
            }

            // Semicolon / SemiSemi
            ';' => {
                if self.peek_char() == Some(';') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::SemiSemi, span })
                } else {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::Semicolon, span })
                }
            }

            // Parens
            '(' => {
                if self.peek_char() == Some('(') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::LParenParen, span })
                } else {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::LParen, span })
                }
            }
            ')' => {
                if self.peek_char() == Some(')') {
                    self.next_char();
                    let span = self.make_span(pos, pos + 2);
                    Ok(Spanned { node: Token::RParenParen, span })
                } else {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned { node: Token::RParen, span })
                }
            }

            // Braces
            '{' => {
                let span = self.make_span(pos, pos + 1);
                Ok(Spanned { node: Token::LBrace, span })
            }
            '}' => {
                let span = self.make_span(pos, pos + 1);
                Ok(Spanned { node: Token::RBrace, span })
            }

            // Redirects
            '<' => self.lex_less_than(pos),
            '>' => self.lex_greater_than(pos),

            // Default: word (includes brackets, quotes, `$`, backslash, etc.)
            _ => {
                // `[` at word start: check for `[[`
                if ch == '[' {
                    if self.peek_char() == Some('[') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        return Some(Ok(Spanned { node: Token::LBracketBracket, span }));
                    }
                    // Otherwise `[` starts a word -- fall through to word reading.
                }

                // `]` at word start: check for `]]`
                if ch == ']' {
                    if self.peek_char() == Some(']') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        return Some(Ok(Spanned { node: Token::RBracketBracket, span }));
                    }
                    // Bare `]` is a word.
                }

                // Read the rest of the word.
                let word_span = self.read_word(pos);

                // IoNumber detection: an all-digit word followed by `<` or `>`.
                let word_text = word_span.text(self.input);
                if !word_text.is_empty()
                    && word_text.chars().all(|c| c.is_ascii_digit())
                {
                    if let Some('<' | '>') = self.peek_char() {
                        if let Ok(num) = word_text.parse::<i32>() {
                            return Some(Ok(Spanned {
                                node: Token::IoNumber(num, word_span),
                                span: word_span,
                            }));
                        }
                    }
                }

                Ok(Spanned { node: Token::Word(word_span), span: word_span })
            }
        };

        Some(result)
    }
}

/// Convenience function to collect all tokens from an input string.
pub fn tokenize(input: &str) -> Result<Vec<Spanned<Token>>, LexError> {
    let lexer = Lexer::new(input);
    lexer.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: collect token nodes (without spans) from input.
    fn tokens(input: &str) -> Vec<Token> {
        tokenize(input)
            .expect("lexer should not error")
            .into_iter()
            .map(|s| s.node)
            .collect()
    }

    #[test]
    fn empty_input() {
        assert_eq!(tokens(""), vec![Token::Eof]);
    }

    #[test]
    fn simple_words() {
        let input = "echo hello world";
        let toks = tokenize(input).unwrap();
        let nodes: Vec<_> = toks.iter().map(|s| &s.node).collect();
        // Three words + Eof
        assert_eq!(nodes.len(), 4);
        assert!(matches!(nodes[0], Token::Word(_)));
        assert!(matches!(nodes[1], Token::Word(_)));
        assert!(matches!(nodes[2], Token::Word(_)));
        assert_eq!(*nodes[3], Token::Eof);

        // Verify span text
        assert_eq!(toks[0].span.text(input), "echo");
        assert_eq!(toks[1].span.text(input), "hello");
        assert_eq!(toks[2].span.text(input), "world");
    }
}
