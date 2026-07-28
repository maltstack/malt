//! POSIX shell lexer -- zero-copy tokenization.
//!
//! Returns `Vec<Spanned<Token>>` via `tokenize()`. Words are zero-copy spans
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
//! - Here-document body collection during tokenization (not streaming)

use crate::ast::{LexError, RedirectKind, Span, Spanned, Token};

/// A pending heredoc where we've seen `<<`/`<<-` and the delimiter word,
/// but haven't collected the body yet (waiting for newline).
#[derive(Debug, Clone)]
struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
}

/// Heredoc where we've seen `<<`/`<<-` but not yet the delimiter word.
#[derive(Debug, Clone)]
struct AwaitingDelimiter {
    strip_tabs: bool,
}

/// Lexer state for tokenizing shell input.
struct LexerState<'a> {
    input: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    tokens: Vec<Spanned<Token>>,
    awaiting_delimiter: Vec<AwaitingDelimiter>,
    pending_heredocs: Vec<PendingHeredoc>,
}

impl<'a> LexerState<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            tokens: Vec::new(),
            awaiting_delimiter: Vec::new(),
            pending_heredocs: Vec::new(),
        }
    }

    /// Build a `Span` from byte offsets.
    fn make_span(&self, start: usize, end: usize) -> Span {
        Span::new(start as u32, end as u32)
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

    /// Run the tokenizer, producing all tokens.
    fn run(mut self) -> Result<Vec<Spanned<Token>>, LexError> {
        loop {
            self.skip_whitespace();

            match self.peek() {
                None => break,
                Some((pos, ch)) => {
                    match ch {
                        '\n' => {
                            self.next_char();
                            self.tokens.push(Spanned {
                                node: Token::Newline,
                                span: self.make_span(pos, pos + 1),
                            });
                            // After newline, resolve any pending heredocs
                            self.resolve_pending_heredocs()?;
                        }
                        '\r' => {
                            self.next_char();
                            let end = if self.peek_char() == Some('\n') {
                                self.next_char();
                                pos + 2
                            } else {
                                pos + 1
                            };
                            self.tokens.push(Spanned {
                                node: Token::Newline,
                                span: self.make_span(pos, end),
                            });
                            self.resolve_pending_heredocs()?;
                        }
                        '#' => {
                            self.skip_comment();
                            // Comment ends at newline or EOF - continue loop
                        }
                        _ => {
                            self.lex_token()?;
                        }
                    }
                }
            }
        }

        // Any unresolved heredocs at end-of-input are errors.
        if let Some(pending) = self.pending_heredocs.first() {
            return Err(LexError::UnterminatedHeredoc {
                delimiter: pending.delimiter.clone(),
            });
        }

        // Add EOF token
        let eof_pos = self.input.len();
        self.tokens.push(Spanned {
            node: Token::Eof,
            span: self.make_span(eof_pos, eof_pos),
        });

        Ok(self.tokens)
    }

    /// Lex a single token and add it to self.tokens.
    fn lex_token(&mut self) -> Result<(), LexError> {
        let Some((pos, ch)) = self.peek() else {
            return Ok(());
        };

        let token = match ch {
            // Pipe / OrOr
            '|' => {
                self.next_char();
                if self.peek_char() == Some('|') {
                    self.next_char();
                    Spanned {
                        node: Token::OrOr,
                        span: self.make_span(pos, pos + 2),
                    }
                } else {
                    Spanned {
                        node: Token::Pipe,
                        span: self.make_span(pos, pos + 1),
                    }
                }
            }

            // Ampersand / AndAnd / &>
            '&' => {
                self.next_char();
                if pos > 0
                    && self.input.as_bytes()[pos - 1].is_ascii_digit()
                    && self.peek_char() == Some('<')
                {
                    return Err(LexError::Unexpected {
                        ch: '&',
                        pos: pos as u32,
                    });
                } else if self.peek_char() == Some('&') {
                    self.next_char();
                    Spanned {
                        node: Token::AndAnd,
                        span: self.make_span(pos, pos + 2),
                    }
                } else if self.peek_char() == Some('>') {
                    self.next_char();
                    Spanned {
                        node: Token::Redirect(RedirectKind::Both),
                        span: self.make_span(pos, pos + 2),
                    }
                } else {
                    Spanned {
                        node: Token::Ampersand,
                        span: self.make_span(pos, pos + 1),
                    }
                }
            }

            // Semicolon / SemiSemi
            ';' => {
                self.next_char();
                if self.peek_char() == Some(';') {
                    self.next_char();
                    Spanned {
                        node: Token::SemiSemi,
                        span: self.make_span(pos, pos + 2),
                    }
                } else {
                    Spanned {
                        node: Token::Semicolon,
                        span: self.make_span(pos, pos + 1),
                    }
                }
            }

            // Parens
            '(' => {
                self.next_char();
                if self.peek_char() == Some('(') {
                    self.next_char();
                    Spanned {
                        node: Token::LParenParen,
                        span: self.make_span(pos, pos + 2),
                    }
                } else {
                    Spanned {
                        node: Token::LParen,
                        span: self.make_span(pos, pos + 1),
                    }
                }
            }
            ')' => {
                self.next_char();
                if self.peek_char() == Some(')') {
                    self.next_char();
                    Spanned {
                        node: Token::RParenParen,
                        span: self.make_span(pos, pos + 2),
                    }
                } else {
                    Spanned {
                        node: Token::RParen,
                        span: self.make_span(pos, pos + 1),
                    }
                }
            }

            // Braces (check if standalone or part of word)
            '{' => {
                let standalone = match self.peek_char() {
                    None => true,
                    Some(next) => is_word_break(next),
                };
                if standalone {
                    self.next_char();
                    Spanned {
                        node: Token::LBrace,
                        span: self.make_span(pos, pos + 1),
                    }
                } else {
                    let word_span = self.read_word(pos, ch)?;
                    Spanned {
                        node: Token::Word(word_span),
                        span: word_span,
                    }
                }
            }
            '}' => {
                let standalone = match self.peek_char() {
                    None => true,
                    Some(next) => is_word_break(next),
                };
                if standalone {
                    self.next_char();
                    Spanned {
                        node: Token::RBrace,
                        span: self.make_span(pos, pos + 1),
                    }
                } else {
                    let word_span = self.read_word(pos, ch)?;
                    Spanned {
                        node: Token::Word(word_span),
                        span: word_span,
                    }
                }
            }

            // Redirects / process substitution / heredocs
            '<' => self.lex_less_than(pos)?,
            '>' => self.lex_greater_than(pos)?,

            // Default: word (includes brackets, quotes, `$`, backslash, etc.)
            _ => {
                // Backslash-newline at the start of a token position is a line
                // continuation — skip both characters and restart tokenization.
                if ch == '\\' {
                    if self.peek_char() == Some('\n') {
                        self.next_char();
                        self.next_char();
                        return self.lex_token(); // Recurse to get next token
                    }
                    if self.peek_char() == Some('\r') {
                        self.next_char();
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                        return self.lex_token(); // Recurse to get next token
                    }
                }

                // `[` at word start: check for `[[`
                if ch == '[' {
                    if self.peek_char() == Some('[') {
                        let standalone = match self.input[pos + 2..].chars().next() {
                            None => true,
                            Some(next) => is_word_break(next),
                        };
                        if standalone {
                            self.next_char();
                            self.next_char();
                            return Ok(self.tokens.push(Spanned {
                                node: Token::LBracketBracket,
                                span: self.make_span(pos, pos + 2),
                            }));
                        }
                    }
                }

                // `]` at word start: check for `]]`
                if ch == ']' {
                    if self.peek_char() == Some(']') {
                        let standalone = match self.input[pos + 2..].chars().next() {
                            None => true,
                            Some(next) => is_word_break(next),
                        };
                        if standalone {
                            self.next_char();
                            self.next_char();
                            return Ok(self.tokens.push(Spanned {
                                node: Token::RBracketBracket,
                                span: self.make_span(pos, pos + 2),
                            }));
                        }
                    }
                }

                // Read the rest of the word.
                let word_span = self.read_word(pos, ch)?;

                // IoNumber detection: an all-digit word followed by `<` or `>`.
                let word_text = word_span.text(self.input);
                if !word_text.is_empty() && word_text.chars().all(|c| c.is_ascii_digit()) {
                    if let Some('<' | '>') = self.peek_char() {
                        if let Ok(num) = word_text.parse::<i32>() {
                            self.tokens.push(Spanned {
                                node: Token::IoNumber(num, word_span),
                                span: word_span,
                            });
                            // Don't return - let caller continue to lex the redirect
                            return Ok(());
                        }
                    }
                }

                Spanned {
                    node: Token::Word(word_span),
                    span: word_span,
                }
            }
        };

        self.tokens.push(token);

        // Handle heredoc delimiter extraction for words
        if let Some(Token::Word(span)) = self.tokens.last().map(|t| &t.node) {
            if !self.awaiting_delimiter.is_empty() {
                let word_text = span.text(self.input);
                let (delimiter, quoted) = extract_heredoc_delimiter(word_text);

                // Find first awaiting delimiter and fill it in
                if let Some(awaiting) = self.awaiting_delimiter.pop() {
                    self.pending_heredocs.push(PendingHeredoc {
                        delimiter,
                        strip_tabs: awaiting.strip_tabs,
                        quoted,
                    });
                }
            }
        }

        Ok(())
    }

    /// Lex operators starting with `<`.
    fn lex_less_than(&mut self, start: usize) -> Result<Spanned<Token>, LexError> {
        self.next_char(); // consume '<'

        match self.peek_char() {
            Some('<') => {
                self.next_char();
                match self.peek_char() {
                    Some('<') => {
                        self.next_char();
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereString),
                            span: self.make_span(start, start + 3),
                        })
                    }
                    Some('-') => {
                        self.next_char();
                        self.awaiting_delimiter.push(AwaitingDelimiter { strip_tabs: true });
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereDocStrip),
                            span: self.make_span(start, start + 3),
                        })
                    }
                    _ => {
                        self.awaiting_delimiter.push(AwaitingDelimiter { strip_tabs: false });
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereDoc),
                            span: self.make_span(start, start + 2),
                        })
                    }
                }
            }
            Some('>') => {
                self.next_char();
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::InputOutput),
                    span: self.make_span(start, start + 2),
                })
            }
            Some('&') => {
                self.next_char();
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::DupInput),
                    span: self.make_span(start, start + 2),
                })
            }
            Some('(') => {
                // Process substitution: <(...)
                self.next_char();
                let end = self.read_process_sub(start)?;
                Ok(Spanned {
                    node: Token::Word(self.make_span(start, end)),
                    span: self.make_span(start, end),
                })
            }
            _ => {
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Input),
                    span: self.make_span(start, start + 1),
                })
            }
        }
    }

    /// Lex operators starting with `>`.
    fn lex_greater_than(&mut self, start: usize) -> Result<Spanned<Token>, LexError> {
        self.next_char(); // consume '>'

        match self.peek_char() {
            Some('>') => {
                self.next_char();
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Append),
                    span: self.make_span(start, start + 2),
                })
            }
            Some('|') => {
                self.next_char();
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Clobber),
                    span: self.make_span(start, start + 2),
                })
            }
            Some('&') => {
                self.next_char();
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::DupOutput),
                    span: self.make_span(start, start + 2),
                })
            }
            Some('(') => {
                // Process substitution: >(...)
                self.next_char();
                let end = self.read_process_sub(start)?;
                Ok(Spanned {
                    node: Token::Word(self.make_span(start, end)),
                    span: self.make_span(start, end),
                })
            }
            _ => {
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Output),
                    span: self.make_span(start, start + 1),
                })
            }
        }
    }

    /// Read a word starting at `start` with initial character `first_ch`.
    /// Returns the span covering the full word.
    fn read_word(&mut self, start: usize, first_ch: char) -> Result<Span, LexError> {
        let mut last_byte_end = start + first_ch.len_utf8();

        // Handle special starting characters
        match first_ch {
            '"' => {
                last_byte_end = self.read_double_quoted(start)?;
            }
            '\'' => {
                last_byte_end = self.read_single_quoted(start, false)?;
            }
            '`' => {
                last_byte_end = self.read_backtick(start)?;
            }
            '$' => {
                last_byte_end = self.handle_dollar_expansion(start, last_byte_end)?;
            }
            _ => {}
        }

        loop {
            match self.peek() {
                None => break,
                Some((pos, ch)) => {
                    if is_word_break(ch) {
                        break;
                    }
                    match ch {
                        '\'' => {
                            let ansi_c = self.input[start..pos].ends_with('$');
                            self.next_char();
                            last_byte_end = self.read_single_quoted(pos, ansi_c)?;
                        }
                        '"' => {
                            self.next_char();
                            last_byte_end = self.read_double_quoted(pos)?;
                        }
                        '\\' => {
                            self.next_char();
                            match self.peek() {
                                Some((_, '\n')) => {
                                    self.next_char(); // Line continuation
                                }
                                Some((_, '\r')) => {
                                    self.next_char();
                                    if self.peek_char() == Some('\n') {
                                        self.next_char();
                                    }
                                }
                                Some((_, c)) => {
                                    self.next_char();
                                    last_byte_end = pos + 1 + c.len_utf8();
                                }
                                None => {
                                    last_byte_end = pos + 1;
                                }
                            }
                        }
                        '`' => {
                            self.next_char();
                            last_byte_end = self.read_backtick(pos)?;
                        }
                        '$' => {
                            self.next_char();
                            last_byte_end = self.handle_dollar_expansion(pos, pos + 1)?;
                        }
                        _ => {
                            self.next_char();
                            last_byte_end = pos + ch.len_utf8();
                        }
                    }
                }
            }
        }
        Ok(self.make_span(start, last_byte_end))
    }

    /// After consuming `$` at `dollar_pos`, check for `$(`, `$((`, or `${` and
    /// delegate to the appropriate balanced reader.
    fn handle_dollar_expansion(
        &mut self,
        dollar_pos: usize,
        default_end: usize,
    ) -> Result<usize, LexError> {
        match self.peek_char() {
            Some('(') => {
                self.next_char();
                if self.peek_char() == Some('(') {
                    self.next_char();
                    Ok(self.read_balanced_arith(dollar_pos)?)
                } else {
                    Ok(self.read_balanced_parens(dollar_pos)?)
                }
            }
            Some('{') => {
                self.next_char();
                Ok(self.read_balanced_braces(dollar_pos)?)
            }
            Some(c) if "?!$#@*-".contains(c) || c.is_ascii_digit() => {
                self.next_char();
                Ok(dollar_pos + 1 + c.len_utf8())
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = dollar_pos + 1 + c.len_utf8();
                self.next_char();
                while let Some(next) = self.peek_char() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        self.next_char();
                        end += next.len_utf8();
                    } else {
                        break;
                    }
                }
                Ok(end)
            }
            _ => Ok(default_end),
        }
    }

    /// Read content inside single quotes. The opening `'` has been consumed.
    fn read_single_quoted(&mut self, open_pos: usize, ansi_c: bool) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '\'')) => return Ok(pos + 1),
                Some((_, '\\')) if ansi_c => {
                    if self.next_char().is_none() {
                        return Err(LexError::UnterminatedString {
                            pos: open_pos as u32,
                        });
                    }
                }
                Some(_) => {}
                None => {
                    return Err(LexError::UnterminatedString {
                        pos: open_pos as u32,
                    })
                }
            }
        }
    }

    /// Read content inside double quotes. The opening `"` has been consumed.
    fn read_double_quoted(&mut self, open_pos: usize) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '"')) => return Ok(pos + 1),
                Some((_, '\\')) => {
                    if let Some((_, '\n')) = self.peek() {
                        self.next_char(); // Line continuation inside quotes
                    } else {
                        self.next_char(); // Escaped char
                    }
                }
                Some((_, '`')) => {
                    self.read_backtick(open_pos)?;
                }
                Some((_, '$')) => {
                    match self.peek_char() {
                        Some('(') => {
                            self.next_char();
                            if self.peek_char() == Some('(') {
                                self.next_char();
                                self.read_balanced_arith(open_pos)?;
                            } else {
                                self.read_balanced_parens(open_pos)?;
                            }
                        }
                        Some('{') => {
                            self.next_char();
                            self.read_balanced_braces(open_pos)?;
                        }
                        _ => {}
                    }
                }
                Some(_) => {}
                None => {
                    return Err(LexError::UnterminatedString {
                        pos: open_pos as u32,
                    })
                }
            }
        }
    }

    /// Read a `$()` command substitution. The `$(` has been consumed.
    fn read_balanced_parens(&mut self, _open_pos: usize) -> Result<usize, LexError> {
        let mut depth: usize = 1;
        let mut case_depth: usize = 0;
        let mut in_case_pattern: bool = false;
        let mut at_word_start: bool = true;
        let mut cur_word: String = String::new();

        while let Some((pos, c)) = self.next_char() {
            match c {
                '$' => {
                    at_word_start = false;
                    cur_word.clear();
                    match self.peek_char() {
                        Some('(') => {
                            self.next_char();
                            if self.peek_char() == Some('(') {
                                self.next_char();
                                self.read_balanced_arith(pos)?;
                            } else {
                                depth += 1;
                                self.read_balanced_parens(pos)?;
                            }
                        }
                        Some('{') => {
                            self.next_char();
                            self.read_balanced_braces(pos)?;
                        }
                        _ => {}
                    }
                }
                '(' => {
                    at_word_start = false;
                    cur_word.clear();
                    depth += 1;
                }
                ')' => {
                    depth -= 1;
                    at_word_start = false;
                    cur_word.clear();
                    if depth == 0 {
                        return Ok(pos + 1);
                    }
                }
                '"' => {
                    self.read_double_quoted(pos)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                '"' => {
                    self.read_single_quoted(pos, false)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                '`' => {
                    self.read_backtick(pos)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                _ => {}
            }
        }
        // Unterminated - let the expander catch it
        Ok(self.input.len())
    }

    /// Read a `$((...))` arithmetic expansion. The `$((` has been consumed.
    fn read_balanced_arith(&mut self, _open_pos: usize) -> Result<usize, LexError> {
        let mut depth: u32 = 1;
        loop {
            let (pos, c) = match self.next_char() {
                Some(v) => v,
                None => return Ok(self.input.len()),
            };
            match c {
                '$' => match self.peek_char() {
                    Some('(') => {
                        self.next_char();
                        if self.peek_char() == Some('(') {
                            self.next_char();
                            self.read_balanced_arith(pos)?;
                        } else {
                            self.read_balanced_parens(pos)?;
                        }
                    }
                    Some('{') => {
                        self.next_char();
                        self.read_balanced_braces(pos)?;
                    }
                    _ => {}
                },
                '(' => {
                    depth += 1;
                }
                ')' => {
                    if self.peek_char() == Some(')') {
                        depth -= 1;
                        if depth == 0 {
                            let (close_pos, _) = self.next_char().unwrap_or((pos, ')'));
                            return Ok(close_pos + 1);
                        }
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
                '"' => {
                    self.read_single_quoted(pos, false)?;
                }
                '"' => {
                    self.read_double_quoted(pos)?;
                }
                '\\' => {
                    let _ = self.next_char();
                }
                _ => {}
            }
        }
    }

    /// Read balanced braces for `${...}` parameter expansion. The `${` has been consumed.
    fn read_balanced_braces(&mut self, _open_pos: usize) -> Result<usize, LexError> {
        let mut depth: usize = 1;
        while let Some((pos, c)) = self.next_char() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(pos + 1);
                    }
                }
                '"' => {
                    self.read_single_quoted(pos, false)?;
                }
                '"' => {
                    self.read_double_quoted(pos)?;
                }
                '\\' => {
                    let _ = self.next_char();
                }
                _ => {}
            }
        }
        Ok(self.input.len())
    }

    /// Read a process substitution `<(...)` or `>(...)`. The `<(` or `>(` has been consumed.
    fn read_process_sub(&mut self, open_pos: usize) -> Result<usize, LexError> {
        let mut depth: usize = 1;
        while let Some((pos, c)) = self.next_char() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(pos + 1);
                    }
                }
                '"' => {
                    self.read_single_quoted(pos, false)?;
                }
                '"' => {
                    self.read_double_quoted(pos)?;
                }
                '\\' => {
                    let _ = self.next_char();
                }
                '$' => match self.peek_char() {
                    Some('(') => {
                        self.next_char();
                        if self.peek_char() == Some('(') {
                            self.next_char();
                            self.read_balanced_arith(pos)?;
                        } else {
                            self.read_balanced_parens(pos)?;
                        }
                    }
                    Some('{') => {
                        self.next_char();
                        self.read_balanced_braces(pos)?;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Err(LexError::UnterminatedProcessSub {
            pos: open_pos as u32,
        })
    }

    /// Read content inside backtick substitution. The opening `` ` `` has been consumed.
    fn read_backtick(&mut self, open_pos: usize) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '`')) => return Ok(pos + 1),
                Some((_, '\\')) => {
                    let _ = self.next_char();
                }
                Some((_, '$')) => {
                    match self.peek_char() {
                        Some('(') => {
                            self.next_char();
                            if self.peek_char() == Some('(') {
                                self.next_char();
                                self.read_balanced_arith(open_pos)?;
                            } else {
                                self.read_balanced_parens(open_pos)?;
                            }
                        }
                        Some('{') => {
                            self.next_char();
                            self.read_balanced_braces(open_pos)?;
                        }
                        _ => {}
                    }
                }
                Some(_) => {}
                None => return Ok(self.input.len()),
            }
        }
    }

    /// After a newline, resolve any pending heredocs by reading their bodies.
    fn resolve_pending_heredocs(&mut self) -> Result<(), LexError> {
        if self.pending_heredocs.is_empty() {
            return Ok(());
        }

        // Collect pending heredocs first to avoid borrow checker issues
        let pending: Vec<_> = self.pending_heredocs.drain(..).collect();
        for hd in pending {
            let body = self.read_heredoc_body(&hd.delimiter, hd.strip_tabs)?;
            let eof_pos = self.input.len();
            self.tokens.push(Spanned {
                node: Token::HereDocBody {
                    body,
                    quoted: hd.quoted,
                },
                span: self.make_span(eof_pos, eof_pos),
            });
        }

        Ok(())
    }

    /// Read a heredoc body up to the delimiter.
    fn read_heredoc_body(
        &mut self,
        delimiter: &str,
        strip_tabs: bool,
    ) -> Result<String, LexError> {
        let mut body = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(LexError::UnterminatedHeredoc {
                        delimiter: delimiter.to_string(),
                    });
                }
                Some((start, _)) => {
                    let line = self.read_heredoc_line(strip_tabs)?;
                    let check_line = if strip_tabs {
                        line.trim_start_matches('\t').to_string()
                    } else {
                        line.clone()
                    };

                    if check_line == delimiter {
                        return Ok(body);
                    }

                    // Not the delimiter - include this line in the body
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(&line);

                    // Check if we reached EOF without finding delimiter
                    if self.peek().is_none() {
                        return Err(LexError::UnterminatedHeredoc {
                            delimiter: delimiter.to_string(),
                        });
                    }
                }
            }
        }
    }

    /// Read one line of heredoc content (including the newline if present).
    fn read_heredoc_line(&mut self, strip_tabs: bool) -> Result<String, LexError> {
        let mut line = String::new();

        loop {
            match self.next_char() {
                Some((_, '\n')) => {
                    line.push('\n');
                    return Ok(line);
                }
                Some((_, '\r')) => {
                    if self.peek_char() == Some('\n') {
                        self.next_char();
                    }
                    line.push('\n');
                    return Ok(line);
                }
                Some((_, c)) => {
                    line.push(c);
                }
                None => {
                    return Ok(line);
                }
            }
        }
    }
}

/// Extract the heredoc delimiter from a word, handling quoting rules.
/// Returns (delimiter, quoted) where quoted indicates if the delimiter was quoted.
fn extract_heredoc_delimiter(word: &str) -> (String, bool) {
    if word.is_empty() {
        return (String::new(), false);
    }

    // Check if the word is quoted (single or double quotes around whole word)
    let quoted = (word.starts_with('\'') && word.ends_with('\''))
        || (word.starts_with('"') && word.ends_with('"'));

    if quoted {
        // Remove surrounding quotes
        let inner = &word[1..word.len() - 1];
        // In quoted delimiters, expansion is suppressed
        (inner.to_string(), true)
    } else {
        // Unquoted delimiter - may contain backslash escapes
        // For now, return as-is (expander handles escapes)
        (word.to_string(), false)
    }
}

/// Characters that break a word.
fn is_word_break(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '\n' | '\r' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '#'
    )
}

// ============================================================================
// Public API
// ============================================================================

/// Tokenize shell input into a vector of tokens with span information.
///
/// This is the primary entry point for lexing. It tokenizes the entire input
/// in one pass, including reading heredoc bodies immediately when encountered.
///
/// # Example
/// ```
/// use mash::lexer::tokenize;
///
/// let tokens = tokenize("echo hello world").unwrap();
/// ```
pub fn tokenize(input: &str) -> Result<Vec<Spanned<Token>>, LexError> {
    let state = LexerState::new(input);
    state.run()
}

/// For backward compatibility: streaming interface is removed.
/// Use `tokenize()` instead for pre-tokenized Vec<Token>.
pub struct Lexer<'a> {
    _phantom: std::marker::PhantomData<&'a str>,
}

impl<'a> Lexer<'a> {
    /// This function is deprecated. Use `tokenize(input)` instead.
    #[deprecated(since = "0.2.0", note = "Use tokenize(input) instead")]
    pub fn new(_input: &'a str) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
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
        assert_eq!(nodes.len(), 4);
        assert!(matches!(nodes[0], Token::Word(_)));
        assert!(matches!(nodes[1], Token::Word(_)));
        assert!(matches!(nodes[2], Token::Word(_)));
        assert_eq!(*nodes[3], Token::Eof);

        assert_eq!(toks[0].span.text(input), "echo");
        assert_eq!(toks[1].span.text(input), "hello");
        assert_eq!(toks[2].span.text(input), "world");
    }

    #[test]
    fn single_quotes() {
        let input = "echo 'hello world'";
        let toks = tokenize(input).unwrap();
        assert_eq!(toks[0].span.text(input), "echo");
        assert_eq!(toks[1].span.text(input), "'hello world'");
    }

    #[test]
    fn double_quotes() {
        let input = r#"echo "hello world""#;
        let toks = tokenize(input).unwrap();
        assert_eq!(toks[0].span.text(input), "echo");
        assert_eq!(toks[1].span.text(input), r#""hello world""#);
    }

    #[test]
    fn command_substitution() {
        let input = "echo $(date)";
        let toks = tokenize(input).unwrap();
        assert_eq!(toks[0].span.text(input), "echo");
        assert_eq!(toks[1].span.text(input), "$(date)");
    }

    #[test]
    fn here_document() {
        let input = "cat <<EOF\nHello\nEOF";
        let toks = tokenize(input).unwrap();
        assert!(matches!(toks[0].node, Token::Word(_)));
        assert!(matches!(toks[1].node, Token::Redirect(RedirectKind::HereDoc)));
        assert!(matches!(toks[2].node, Token::Word(_)));
        assert!(matches!(toks[3].node, Token::HereDocBody { .. }));
    }
}
