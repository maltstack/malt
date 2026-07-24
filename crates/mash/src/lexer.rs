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
/// has not yet been consumed (or has been consumed but body not yet read).
#[derive(Debug)]
pub struct PendingHeredoc {
    pub delimiter: String,
    pub strip_tabs: bool,
    pub quoted: bool,
}

/// Streaming zero-copy lexer for MASH shell input.
pub struct Lexer<'a> {
    input: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    /// Heredocs awaiting delimiter words and body accumulation.
    pub pending_heredocs: Vec<PendingHeredoc>,
    /// Number of `<<`/`<<-` operators whose delimiter word has not yet been consumed.
    awaiting_heredoc_count: usize,
    /// Buffer for tokens that were generated ahead of time (e.g. HereDocBody tokens
    /// resolved at a newline boundary).
    buffered_tokens: Vec<Spanned<Token>>,
    finished: bool,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer over the given input string.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            pending_heredocs: Vec::new(),
            awaiting_heredoc_count: 0,
            buffered_tokens: Vec::new(),
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
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereString),
                            span,
                        })
                    }
                    Some('-') => {
                        self.next_char();
                        self.pending_heredocs.push(PendingHeredoc {
                            delimiter: String::new(),
                            strip_tabs: true,
                            quoted: false,
                        });
                        self.awaiting_heredoc_count += 1;
                        let span = self.make_span(start, start + 3);
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereDocStrip),
                            span,
                        })
                    }
                    _ => {
                        self.pending_heredocs.push(PendingHeredoc {
                            delimiter: String::new(),
                            strip_tabs: false,
                            quoted: false,
                        });
                        self.awaiting_heredoc_count += 1;
                        let span = self.make_span(start, start + 2);
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::HereDoc),
                            span,
                        })
                    }
                }
            }
            Some('>') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::InputOutput),
                    span,
                })
            }
            Some('&') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::DupInput),
                    span,
                })
            }
            _ => {
                let span = self.make_span(start, start + 1);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Input),
                    span,
                })
            }
        }
    }

    /// Lex operators starting with `>`.
    fn lex_greater_than(&mut self, start: usize) -> Result<Spanned<Token>, LexError> {
        match self.peek_char() {
            Some('>') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Append),
                    span,
                })
            }
            Some('|') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Clobber),
                    span,
                })
            }
            Some('&') => {
                self.next_char();
                let span = self.make_span(start, start + 2);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::DupOutput),
                    span,
                })
            }
            _ => {
                let span = self.make_span(start, start + 1);
                Ok(Spanned {
                    node: Token::Redirect(RedirectKind::Output),
                    span,
                })
            }
        }
    }

    /// Read a word starting from position `start` (the first character `first_ch`
    /// at `start` has already been consumed by the caller). Continues until a
    /// word-breaking character is found.
    ///
    /// Handles quoting: single quotes, double quotes, ANSI-C quotes (`$'...'`),
    /// backslash escapes, line continuations, and backtick substitutions. Quote
    /// characters are included in the word span (zero-copy); the expander strips
    /// them later.
    fn read_word(&mut self, start: usize, first_ch: char) -> Result<Span, LexError> {
        // Track the last consumed byte position so we can compute the span end.
        // The first character (at `start`) has already been consumed.
        let mut last_byte_end = start + first_ch.len_utf8();

        // If the first character is a quote/backslash/backtick/dollar, handle it now.
        match first_ch {
            '\'' => {
                // Cannot be $' since this is the first char.
                last_byte_end = self.read_single_quoted(start, false)?;
            }
            '"' => {
                last_byte_end = self.read_double_quoted(start)?;
            }
            '\\' => {
                match self.peek() {
                    Some((_, '\n')) => {
                        self.next_char();
                        // Line continuation at word start — the backslash+newline is
                        // elided. Reset last_byte_end; if nothing follows, we'll
                        // produce a zero-length span which is fine (caller handles it).
                    }
                    Some((_, '\r')) => {
                        self.next_char();
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                        // CRLF line continuation.
                    }
                    Some((_, c)) => {
                        self.next_char();
                        last_byte_end = start + 1 + c.len_utf8();
                    }
                    None => {
                        // Trailing backslash at EOF.
                        last_byte_end = start + 1;
                    }
                }
            }
            '`' => {
                last_byte_end = self.read_backtick(start)?;
            }
            '$' => {
                // Check for $(...), $((...)), ${...} at the start of the word.
                last_byte_end = self.handle_dollar_expansion(start, last_byte_end)?;
            }
            _ => {}
        }

        loop {
            match self.peek() {
                None => break,
                Some((pos, ch)) => {
                    // Word-break characters stop the word — but `(` and `{` after
                    // `$` are expansion starts, not word breaks.
                    if is_word_break(ch) {
                        break;
                    }
                    match ch {
                        '\'' => {
                            // Check for $'...' (ANSI-C quoting): previous char in span was '$'
                            let ansi_c = self.input[start..pos].ends_with('$');
                            self.next_char(); // consume opening '
                            last_byte_end = self.read_single_quoted(pos, ansi_c)?;
                        }
                        '"' => {
                            self.next_char(); // consume opening "
                            last_byte_end = self.read_double_quoted(pos)?;
                        }
                        '\\' => {
                            self.next_char(); // consume the backslash
                            match self.peek() {
                                Some((_, '\n')) => {
                                    // Line continuation: consume backslash + newline, continue word.
                                    // Do NOT extend span to include the \\\n — they are elided.
                                    self.next_char();
                                }
                                Some((_, '\r')) => {
                                    // CRLF line continuation: consume backslash + \r + optional \n.
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
                                    // Trailing backslash at EOF — include it in the word.
                                    last_byte_end = pos + 1;
                                }
                            }
                        }
                        '`' => {
                            self.next_char(); // consume opening `
                            last_byte_end = self.read_backtick(pos)?;
                        }
                        '$' => {
                            self.next_char(); // consume '$'
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
    /// delegate to the appropriate balanced reader. Returns the updated
    /// `last_byte_end`.
    fn handle_dollar_expansion(
        &mut self,
        dollar_pos: usize,
        default_end: usize,
    ) -> Result<usize, LexError> {
        match self.peek_char() {
            Some('(') => {
                self.next_char(); // consume '('
                if self.peek_char() == Some('(') {
                    self.next_char(); // consume second '('
                    Ok(self.read_balanced_arith(dollar_pos)?)
                } else {
                    Ok(self.read_balanced_parens(dollar_pos)?)
                }
            }
            Some('{') => {
                self.next_char(); // consume '{'
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
    /// Returns the byte position just past the closing `'`.
    ///
    /// When `ansi_c` is true (`$'...'`), backslash escapes are recognized but
    /// preserved literally — the span just covers everything including the
    /// closing quote.
    fn read_single_quoted(&mut self, open_pos: usize, ansi_c: bool) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '\'')) => return Ok(pos + 1),
                Some((_, '\\')) if ansi_c => {
                    // In ANSI-C quotes, backslash escapes: consume next char too.
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
    /// Returns the byte position just past the closing `"`.
    ///
    /// Inside double quotes, `\`, `$`, and `` ` `` are special. When `$` is
    /// followed by `(`, `((`, or `{`, we delegate to the balanced-tracking
    /// methods so nested expansions are correctly consumed.
    fn read_double_quoted(&mut self, open_pos: usize) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '"')) => return Ok(pos + 1),
                Some((_, '\\')) => {
                    // Consume the escaped character (if any).
                    // Even if next char is `"`, it does NOT close the quote.
                    if self.peek_char() == Some('\r') {
                        self.next_char();
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                    } else {
                        let _ = self.next_char();
                    }
                }
                Some((pos, '$')) => {
                    match self.peek_char() {
                        Some('(') => {
                            self.next_char(); // consume '('
                            if self.peek_char() == Some('(') {
                                self.next_char(); // consume second '('
                                self.read_balanced_arith(pos)?;
                            } else {
                                self.read_balanced_parens(pos)?;
                            }
                        }
                        Some('{') => {
                            self.next_char(); // consume '{'
                            self.read_balanced_braces(pos)?;
                        }
                        _ => {}
                    }
                }
                Some((pos, '`')) => {
                    self.read_backtick(pos)?;
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

    /// Read balanced parentheses for `$(...)` command substitution.
    /// The `$(` has already been consumed. Returns the byte position just past
    /// the closing `)`.
    ///
    /// Tracks `case...esac` nesting so that a `)` inside a case pattern is not
    /// mistaken for the closing paren of the command substitution.
    fn read_balanced_parens(&mut self, _open_pos: usize) -> Result<usize, LexError> {
        let mut depth: usize = 1;
        let mut at_word_start = true;
        let mut case_depth: usize = 0;
        let mut in_case_pattern = false;
        let mut cur_word = String::new();

        while let Some((pos, c)) = self.next_char() {
            // # at word-start begins a comment — skip to end of line.
            if c == '#' && at_word_start {
                loop {
                    match self.next_char() {
                        Some((_, '\n')) | None => break,
                        Some(_) => {}
                    }
                }
                at_word_start = true;
                cur_word.clear();
                continue;
            }

            match c {
                '(' => {
                    depth += 1;
                    at_word_start = true;
                    cur_word.clear();
                }
                ')' => {
                    // Check keywords ending at ')'.
                    match cur_word.as_str() {
                        "case" => case_depth += 1,
                        "esac" => {
                            case_depth = case_depth.saturating_sub(1);
                            in_case_pattern = false;
                        }
                        "in" if case_depth > 0 => in_case_pattern = true,
                        _ => {}
                    }
                    cur_word.clear();

                    if case_depth > 0 && in_case_pattern && depth == 1 {
                        // Case-pattern delimiter, not a subshell close.
                        in_case_pattern = false;
                        at_word_start = true;
                    } else {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(pos + 1);
                        }
                        at_word_start = false;
                    }
                }
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
                '\'' => {
                    self.read_single_quoted(pos, false)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                '"' => {
                    self.read_double_quoted(pos)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                '\\' => {
                    let _ = self.next_char();
                    at_word_start = false;
                    cur_word.clear();
                }
                ' ' | '\t' | '\n' => {
                    match cur_word.as_str() {
                        "case" => case_depth += 1,
                        "esac" => {
                            case_depth = case_depth.saturating_sub(1);
                            in_case_pattern = false;
                        }
                        "in" if case_depth > 0 => in_case_pattern = true,
                        _ => {}
                    }
                    at_word_start = true;
                    cur_word.clear();
                }
                ';' => {
                    if case_depth > 0 {
                        in_case_pattern = true;
                    }
                    at_word_start = true;
                    cur_word.clear();
                }
                '&' | '|' | '{' | '}' | '<' | '>' => {
                    at_word_start = true;
                    cur_word.clear();
                }
                '`' => {
                    self.read_backtick(pos)?;
                    at_word_start = false;
                    cur_word.clear();
                }
                _ => {
                    if at_word_start {
                        cur_word.clear();
                        cur_word.push(c);
                    } else {
                        cur_word.push(c);
                    }
                    at_word_start = false;
                }
            }
        }
        // Unterminated — let the expander catch it.
        Ok(self.input.len())
    }

    /// Read a `$((...))` arithmetic expansion. The `$((` has been consumed.
    /// Returns the byte position just past the closing `))`.
    fn read_balanced_arith(&mut self, _open_pos: usize) -> Result<usize, LexError> {
        let mut depth: u32 = 1;
        loop {
            let (pos, c) = match self.next_char() {
                Some(v) => v,
                None => return Ok(self.input.len()), // unterminated
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
                        // depth > 0: the second ')' will be processed next iteration.
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
                '\'' => {
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

    /// Read balanced braces for `${...}` parameter expansion. The `${` has been
    /// consumed. Returns the byte position just past the closing `}`.
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
                '\'' => {
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
        Ok(self.input.len()) // unterminated
    }

    /// Read a process substitution `<(...)` or `>(...)`. The `<(` or `>(` has
    /// been consumed. Returns the byte position just past the closing `)`.
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
                '\'' => {
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

    /// Read content inside backtick substitution. The opening `` ` `` has been
    /// consumed. Returns the byte position just past the closing `` ` ``.
    fn read_backtick(&mut self, open_pos: usize) -> Result<usize, LexError> {
        loop {
            match self.next_char() {
                Some((pos, '`')) => return Ok(pos + 1),
                Some((_, '\\')) => {
                    // `\` inside backticks escapes the next character.
                    if self.peek_char() == Some('\r') {
                        self.next_char();
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                    } else {
                        let _ = self.next_char();
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

    /// Extract a heredoc delimiter from a word token's text.
    ///
    /// Returns `(delimiter, quoted)` where `quoted` is true when any quoting
    /// mechanism was used (meaning no expansion should occur in the body).
    fn extract_heredoc_delimiter(word: &str) -> (String, bool) {
        let trimmed = word.trim();
        // Fully single-quoted: 'DELIM' -> (DELIM, true)
        if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
            return (trimmed[1..trimmed.len() - 1].to_string(), true);
        }
        // Fully double-quoted: "DELIM" -> (DELIM, true)
        if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
            return (trimmed[1..trimmed.len() - 1].to_string(), true);
        }
        // Backslash-escaped: \D\E\L -> (DEL, true)
        if trimmed.contains('\\') {
            let stripped: String = trimmed.chars().filter(|&c| c != '\\').collect();
            return (stripped, true);
        }
        // Unquoted
        (trimmed.to_string(), false)
    }

    /// Read the body of a heredoc, consuming characters until a line matching
    /// `delimiter` is found. For `<<-` (strip_tabs), leading tabs are stripped
    /// from each line including the delimiter line.
    ///
    /// Returns the accumulated body string (without the delimiter line).
    fn read_heredoc_body(&mut self, delimiter: &str, strip_tabs: bool) -> Result<String, LexError> {
        let mut body = String::new();
        let mut line = String::new();

        loop {
            match self.next_char() {
                Some((_, '\r')) => {
                    // Handle \r\n as a single newline.
                    if self.peek_char() == Some('\n') {
                        self.next_char();
                    }
                    let check_line = if strip_tabs {
                        line.trim_start_matches('\t')
                    } else {
                        &line
                    };
                    if check_line == delimiter {
                        return Ok(body);
                    }
                    if strip_tabs {
                        body.push_str(line.trim_start_matches('\t'));
                    } else {
                        body.push_str(&line);
                    }
                    body.push('\n');
                    line.clear();
                }
                Some((_, '\n')) => {
                    let check_line = if strip_tabs {
                        line.trim_start_matches('\t')
                    } else {
                        &line
                    };
                    if check_line == delimiter {
                        return Ok(body);
                    }
                    if strip_tabs {
                        body.push_str(line.trim_start_matches('\t'));
                    } else {
                        body.push_str(&line);
                    }
                    body.push('\n');
                    line.clear();
                }
                Some((_, ch)) => {
                    line.push(ch);
                }
                None => {
                    // Check final line (no trailing newline).
                    let check_line = if strip_tabs {
                        line.trim_start_matches('\t')
                    } else {
                        &line
                    };
                    if check_line == delimiter {
                        return Ok(body);
                    }
                    return Err(LexError::UnterminatedHeredoc {
                        delimiter: delimiter.to_string(),
                    });
                }
            }
        }
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
        ' ' | '\t' | '\n' | '\r' | '|' | '&' | ';' | '<' | '>' | '(' | ')'
    )
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token>, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Outer loop replaces tail-recursive `return self.next()` calls for
        // comments and backslash-newline continuations, preventing stack overflow
        // when sourcing files with many comment lines inside deeply-nested parses.
        loop {
            if self.finished {
                return None;
            }

            // Drain buffered tokens first (e.g. HereDocBody tokens queued at newline).
            if let Some(tok) = self.buffered_tokens.pop() {
                return Some(Ok(tok));
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
                    return Some(Ok(Spanned {
                        node: Token::Eof,
                        span,
                    }));
                }
            };

            // Consume the character we just peeked.
            self.next_char();

            let result = match ch {
                // Newline
                '\n' => {
                    let span = self.make_span(pos, pos + 1);
                    Ok(Spanned {
                        node: Token::Newline,
                        span,
                    })
                }

                // CRLF -- treat \r\n as a single newline
                '\r' => {
                    if self.peek_char() == Some('\n') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::Newline,
                            span,
                        })
                    } else {
                        // Bare \r treated as newline
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::Newline,
                            span,
                        })
                    }
                }

                // Comment
                '#' => {
                    self.skip_comment();
                    // Restart the outer loop to get the next real token.
                    continue;
                }

                // Pipe / OrOr
                '|' => {
                    if self.peek_char() == Some('|') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::OrOr,
                            span,
                        })
                    } else {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::Pipe,
                            span,
                        })
                    }
                }

                // Ampersand / AndAnd / &>
                '&' => {
                    if pos > 0
                        && self.input.as_bytes()[pos - 1].is_ascii_digit()
                        && self.peek_char() == Some('<')
                    {
                        Err(LexError::Unexpected {
                            ch: '&',
                            pos: pos as u32,
                        })
                    } else if self.peek_char() == Some('&') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::AndAnd,
                            span,
                        })
                    } else if self.peek_char() == Some('>') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::Redirect(RedirectKind::Both),
                            span,
                        })
                    } else {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::Ampersand,
                            span,
                        })
                    }
                }

                // Semicolon / SemiSemi
                ';' => {
                    if self.peek_char() == Some(';') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::SemiSemi,
                            span,
                        })
                    } else {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::Semicolon,
                            span,
                        })
                    }
                }

                // Parens
                '(' => {
                    if self.peek_char() == Some('(') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::LParenParen,
                            span,
                        })
                    } else {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::LParen,
                            span,
                        })
                    }
                }
                ')' => {
                    if self.peek_char() == Some(')') {
                        self.next_char();
                        let span = self.make_span(pos, pos + 2);
                        Ok(Spanned {
                            node: Token::RParenParen,
                            span,
                        })
                    } else {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::RParen,
                            span,
                        })
                    }
                }

                // Braces
                '{' => {
                    let standalone = match self.peek_char() {
                        None => true,
                        Some(next) => is_word_break(next),
                    };
                    if standalone {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::LBrace,
                            span,
                        })
                    } else {
                        let word_span = match self.read_word(pos, ch) {
                            Ok(span) => span,
                            Err(e) => return Some(Err(e)),
                        };
                        Ok(Spanned {
                            node: Token::Word(word_span),
                            span: word_span,
                        })
                    }
                }
                '}' => {
                    let standalone = match self.peek_char() {
                        None => true,
                        Some(next) => is_word_break(next),
                    };
                    if standalone {
                        let span = self.make_span(pos, pos + 1);
                        Ok(Spanned {
                            node: Token::RBrace,
                            span,
                        })
                    } else {
                        let word_span = match self.read_word(pos, ch) {
                            Ok(span) => span,
                            Err(e) => return Some(Err(e)),
                        };
                        Ok(Spanned {
                            node: Token::Word(word_span),
                            span: word_span,
                        })
                    }
                }

                // Redirects / process substitution
                '<' => {
                    if self.peek_char() == Some('(') {
                        // Process substitution: <(...)
                        self.next_char(); // consume '('
                        let end = match self.read_process_sub(pos) {
                            Ok(end) => end,
                            Err(e) => return Some(Err(e)),
                        };
                        let span = self.make_span(pos, end);
                        Ok(Spanned {
                            node: Token::Word(span),
                            span,
                        })
                    } else {
                        self.lex_less_than(pos)
                    }
                }
                '>' => {
                    if self.peek_char() == Some('(') {
                        // Process substitution: >(...)
                        self.next_char(); // consume '('
                        let end = match self.read_process_sub(pos) {
                            Ok(end) => end,
                            Err(e) => return Some(Err(e)),
                        };
                        let span = self.make_span(pos, end);
                        Ok(Spanned {
                            node: Token::Word(span),
                            span,
                        })
                    } else {
                        self.lex_greater_than(pos)
                    }
                }

                // Default: word (includes brackets, quotes, `$`, backslash, etc.)
                _ => {
                    // Backslash-newline at the start of a token position is a line
                    // continuation — skip both characters and restart tokenization.
                    if ch == '\\' {
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                            continue;
                        }
                        if self.peek_char() == Some('\r') {
                            self.next_char();
                            if self.peek_char() == Some('\n') {
                                self.next_char();
                            }
                            continue;
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
                                let span = self.make_span(pos, pos + 2);
                                return Some(Ok(Spanned {
                                    node: Token::LBracketBracket,
                                    span,
                                }));
                            }
                        }
                        // Otherwise `[` starts a word -- fall through to word reading.
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
                                let span = self.make_span(pos, pos + 2);
                                return Some(Ok(Spanned {
                                    node: Token::RBracketBracket,
                                    span,
                                }));
                            }
                        }
                        // Bare `]` is a word.
                    }

                    // Read the rest of the word.
                    let word_span = match self.read_word(pos, ch) {
                        Ok(span) => span,
                        Err(e) => return Some(Err(e)),
                    };

                    // IoNumber detection: an all-digit word followed by `<` or `>`.
                    let word_text = word_span.text(self.input);
                    if !word_text.is_empty() && word_text.chars().all(|c| c.is_ascii_digit()) {
                        if let Some('<' | '>') = self.peek_char() {
                            if let Ok(num) = word_text.parse::<i32>() {
                                return Some(Ok(Spanned {
                                    node: Token::IoNumber(num, word_span),
                                    span: word_span,
                                }));
                            }
                        }
                    }

                    Ok(Spanned {
                        node: Token::Word(word_span),
                        span: word_span,
                    })
                }
            };

            // Phase 2: If the token is a Word and we are awaiting heredoc delimiters,
            // extract the delimiter from the word text and fill it into the most recent
            // PendingHeredoc that still has an empty delimiter.
            if let Ok(ref spanned) = result {
                if self.awaiting_heredoc_count > 0 {
                    if let Token::Word(span) = &spanned.node {
                        let word_text = span.text(self.input);
                        let (delimiter, quoted) = Self::extract_heredoc_delimiter(word_text);
                        // Find the first pending heredoc with an empty delimiter.
                        for hd in &mut self.pending_heredocs {
                            if hd.delimiter.is_empty() {
                                hd.delimiter = delimiter;
                                hd.quoted = quoted;
                                break;
                            }
                        }
                        self.awaiting_heredoc_count -= 1;
                    }
                }
            }

            // Phase 3: If the token is a Newline and there are pending heredocs with
            // delimiters ready, resolve their bodies now. Buffer the HereDocBody tokens
            // to be drained on subsequent next() calls.
            if let Ok(ref spanned) = result {
                if matches!(spanned.node, Token::Newline) {
                    let ready = self
                        .pending_heredocs
                        .iter()
                        .all(|hd| !hd.delimiter.is_empty());
                    if ready && !self.pending_heredocs.is_empty() {
                        let heredocs: Vec<PendingHeredoc> =
                            self.pending_heredocs.drain(..).collect();
                        let mut bodies = Vec::new();
                        for hd in heredocs {
                            match self.read_heredoc_body(&hd.delimiter, hd.strip_tabs) {
                                Ok(body) => {
                                    let eof_pos = self.input.len();
                                    let span = self.make_span(eof_pos, eof_pos);
                                    bodies.push(Spanned {
                                        node: Token::HereDocBody {
                                            body,
                                            quoted: hd.quoted,
                                        },
                                        span,
                                    });
                                }
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        // Reverse so that pop() yields them in order.
                        bodies.reverse();
                        self.buffered_tokens = bodies;
                    }
                }
            }

            return Some(result);
        } // end outer loop
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
