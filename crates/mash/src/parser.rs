//! POSIX shell parser — recursive descent, span-annotated AST.
//!
//! Implements the core POSIX grammar: command lists, and-or lists, pipelines,
//! simple commands with redirects and environment assignments. Compound
//! commands (if/while/for/case/etc.) are recognized at dispatch but stubbed
//! with `SyntaxError` — they will be implemented in a follow-up task.

use crate::ast::*;
use crate::lexer::Lexer;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a shell input string into a list of commands.
pub fn parse(input: &str) -> Result<Vec<Spanned<Command>>, ParseError> {
    let mut parser = Parser::new(input)?;
    parser.parse_command_list()
}

// ---------------------------------------------------------------------------
// Parser internals
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    current: Option<Spanned<Token>>,
}

/// Sentinel token returned when peeking past EOF.
fn eof_sentinel() -> Spanned<Token> {
    Spanned {
        node: Token::Eof,
        span: Span::new(0, 0),
    }
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Result<Self, ParseError> {
        let mut lexer = Lexer::new(input);
        let current = Self::read_next(&mut lexer)?;
        Ok(Self {
            input,
            lexer,
            current,
        })
    }

    /// Read the next token from the lexer, converting lex errors.
    fn read_next(lexer: &mut Lexer<'_>) -> Result<Option<Spanned<Token>>, ParseError> {
        match lexer.next() {
            Some(Ok(tok)) => Ok(Some(tok)),
            Some(Err(e)) => Err(ParseError::Lex(e)),
            None => Ok(None),
        }
    }

    // -- Utility methods ---------------------------------------------------

    /// Look at current token without consuming.
    fn peek(&self) -> &Spanned<Token> {
        static EOF: std::sync::LazyLock<Spanned<Token>> = std::sync::LazyLock::new(eof_sentinel);
        self.current.as_ref().unwrap_or(&EOF)
    }

    /// Consume current token and read the next one from the lexer.
    fn advance(&mut self) -> Result<Spanned<Token>, ParseError> {
        let tok = self
            .current
            .take()
            .unwrap_or_else(eof_sentinel);
        self.current = Self::read_next(&mut self.lexer)?;
        Ok(tok)
    }

    /// Consume a Word token or return an error.
    fn expect_word(&mut self) -> Result<Spanned<Token>, ParseError> {
        match &self.peek().node {
            Token::Word(_) => self.advance(),
            Token::Eof => Err(ParseError::UnexpectedEof),
            _ => {
                let t = self.peek();
                Err(ParseError::Unexpected {
                    token: t.node.clone(),
                    span: t.span,
                })
            }
        }
    }

    /// Consume a Word matching `kw`, or return a SyntaxError.
    #[allow(dead_code)]
    fn expect_keyword(&mut self, kw: &str) -> Result<Span, ParseError> {
        match &self.peek().node {
            Token::Word(span) if span.text(self.input) == kw => {
                let sp = self.advance()?.span;
                Ok(sp)
            }
            Token::Eof => Err(ParseError::SyntaxError {
                pos: self.peek().span.start,
                message: format!("expected '{kw}', got EOF"),
            }),
            _ => Err(ParseError::SyntaxError {
                pos: self.peek().span.start,
                message: format!("expected '{kw}', got {:?}", self.peek().node),
            }),
        }
    }

    /// Skip consecutive Newline tokens.
    fn skip_newlines(&mut self) -> Result<(), ParseError> {
        while matches!(self.peek().node, Token::Newline) {
            self.advance()?;
        }
        Ok(())
    }

    /// Skip Semicolon and Newline tokens.
    fn skip_terminators(&mut self) -> Result<(), ParseError> {
        while matches!(self.peek().node, Token::Semicolon | Token::Newline) {
            self.advance()?;
        }
        Ok(())
    }

    /// Check if current word matches `kw` without consuming.
    fn peek_keyword(&self, kw: &str) -> bool {
        matches!(&self.peek().node, Token::Word(span) if span.text(self.input) == kw)
    }

    /// Check current token against a predicate without consuming.
    #[allow(dead_code)]
    fn peek_is(&self, f: impl Fn(&Token) -> bool) -> bool {
        f(&self.peek().node)
    }

    /// Get text from input for a span.
    #[allow(dead_code)]
    fn word_text(&self, span: Span) -> &str {
        span.text(self.input)
    }

    /// Get the word span from a Word token (panics if not a Word — caller must check).
    fn word_span(tok: &Token) -> Option<Span> {
        match tok {
            Token::Word(span) => Some(*span),
            _ => None,
        }
    }

    // -- Top-level: command list -------------------------------------------

    /// Parse a complete command list (the top-level production).
    fn parse_command_list(&mut self) -> Result<Vec<Spanned<Command>>, ParseError> {
        self.parse_command_list_until(|t| matches!(t, Token::Eof))
    }

    /// Parse a command list that terminates when `is_end` returns true.
    fn parse_command_list_until(
        &mut self,
        is_end: impl Fn(&Token) -> bool,
    ) -> Result<Vec<Spanned<Command>>, ParseError> {
        let mut commands = Vec::new();
        self.skip_newlines()?;

        while !is_end(&self.peek().node) && !matches!(self.peek().node, Token::Eof) {
            // Skip stray terminators between commands.
            if matches!(self.peek().node, Token::Semicolon | Token::Newline) {
                self.skip_terminators()?;
                continue;
            }

            let before_span = self.peek().span;
            let cmd = self.parse_and_or()?;

            match &self.peek().node {
                Token::Semicolon | Token::Newline => {
                    self.advance()?;
                    self.skip_newlines()?;
                    if !matches!(cmd.node, Command::Empty) {
                        commands.push(cmd);
                    }
                }
                Token::Ampersand => {
                    let amp = self.advance()?;
                    self.skip_newlines()?;
                    if !matches!(cmd.node, Command::Empty) {
                        let span = cmd.span.merge(amp.span);
                        commands.push(Spanned {
                            node: Command::Background(Box::new(cmd)),
                            span,
                        });
                    }
                }
                _ => {
                    // Guard against infinite loops.
                    if self.peek().span.start == before_span.start
                        && self.peek().span.end == before_span.end
                    {
                        let t = self.peek();
                        return Err(ParseError::Unexpected {
                            token: t.node.clone(),
                            span: t.span,
                        });
                    }
                    if !matches!(cmd.node, Command::Empty) {
                        commands.push(cmd);
                    }
                }
            }
        }

        Ok(commands)
    }

    // -- And-or list -------------------------------------------------------

    /// Parse an and-or list: `pipeline (('&&' | '||') pipeline)*`
    fn parse_and_or(&mut self) -> Result<Spanned<Command>, ParseError> {
        let first = self.parse_pipeline()?;

        let mut pairs: Vec<(Spanned<Command>, ListOp)> = Vec::new();
        let mut current = first;

        loop {
            let op = match &self.peek().node {
                Token::AndAnd => ListOp::AndIf,
                Token::OrOr => ListOp::OrIf,
                _ => break,
            };
            self.advance()?;
            self.skip_newlines()?;
            pairs.push((current, op));
            current = self.parse_pipeline()?;
        }

        if pairs.is_empty() {
            Ok(current)
        } else {
            let span = pairs[0].0.span.merge(current.span);
            Ok(Spanned {
                node: Command::List {
                    pairs,
                    last: Box::new(current),
                },
                span,
            })
        }
    }

    // -- Pipeline ----------------------------------------------------------

    /// Parse a pipeline: `['!'] command ('|' command)*`
    fn parse_pipeline(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start_span = self.peek().span;

        // Check for `!` (pipeline negation).
        let negated = if self.peek_keyword("!") {
            self.advance()?;
            self.skip_newlines()?;
            true
        } else {
            false
        };

        let first = self.parse_command()?;

        if matches!(self.peek().node, Token::Pipe) {
            let mut cmds = vec![first];
            while matches!(self.peek().node, Token::Pipe) {
                self.advance()?;
                self.skip_newlines()?;
                cmds.push(self.parse_command()?);
            }
            let end_span = cmds.last().map(|c| c.span).unwrap_or(start_span);
            let span = start_span.merge(end_span);
            Ok(Spanned {
                node: Command::Pipeline {
                    commands: cmds,
                    negated,
                },
                span,
            })
        } else if negated {
            let span = start_span.merge(first.span);
            Ok(Spanned {
                node: Command::Pipeline {
                    commands: vec![first],
                    negated: true,
                },
                span,
            })
        } else {
            Ok(first)
        }
    }

    // -- Command dispatch --------------------------------------------------

    /// Parse a single command (compound, function def, or simple).
    fn parse_command(&mut self) -> Result<Spanned<Command>, ParseError> {
        #[allow(unused_variables)]
        let cmd = match &self.peek().node {
            Token::Word(span) => {
                let w = span.text(self.input).to_owned();
                match w.as_str() {
                    "if" | "while" | "until" | "for" | "case" | "select" | "function"
                    | "coproc" | "time" => {
                        return Err(ParseError::SyntaxError {
                            pos: self.peek().span.start,
                            message: format!(
                                "compound command '{w}' not yet implemented"
                            ),
                        });
                    }
                    _ => {
                        // Check for `name()` function definition: Word LParen RParen.
                        // We can't easily lookahead in a streaming lexer, so we
                        // fall through to simple command (function defs will be
                        // handled in Task 7).
                        return self.parse_simple_command();
                    }
                }
            }
            Token::LBrace => {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "brace group not yet implemented".to_string(),
                });
            }
            Token::LParen => {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "subshell not yet implemented".to_string(),
                });
            }
            Token::LParenParen => {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "arithmetic expression not yet implemented".to_string(),
                });
            }
            Token::LBracketBracket => {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "conditional expression not yet implemented".to_string(),
                });
            }
            // Everything else is a simple command.
            _ => return self.parse_simple_command(),
        };

        // Collect trailing I/O redirections on compound commands.
        // (Currently unreachable due to stubs, but ready for Task 7.)
        #[allow(unreachable_code)]
        {
            let trailing = self.collect_trailing_redirects()?;
            if trailing.is_empty() {
                Ok(cmd)
            } else {
                let span = cmd.span.merge(trailing.last().map(|r| r.span).unwrap_or(cmd.span));
                Ok(Spanned {
                    node: Command::Redirected {
                        cmd: Box::new(cmd),
                        redirects: trailing,
                    },
                    span,
                })
            }
        }
    }

    // -- Simple command ----------------------------------------------------

    fn parse_simple_command(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start_span = self.peek().span;
        let mut env_assigns: Vec<(Span, Span)> = Vec::new();
        let mut words: Vec<Span> = Vec::new();
        let mut redirects: Vec<Spanned<Redirect>> = Vec::new();
        let mut end_span = start_span;

        loop {
            match &self.peek().node {
                Token::Word(word_span) => {
                    let text = word_span.text(self.input);
                    // Check for VAR=value assignment (only before command name).
                    if words.is_empty() && is_assignment(text) {
                        let ws = *word_span;
                        let tok = self.advance()?;
                        end_span = tok.span;
                        let (k, v) = split_assignment(text, ws);
                        env_assigns.push((k, v));
                    } else {
                        let ws = *word_span;
                        let tok = self.advance()?;
                        end_span = tok.span;
                        words.push(ws);
                    }
                }
                Token::IoNumber(fd, _fd_span) => {
                    let fd_val = *fd;
                    let fd_tok = self.advance()?;
                    end_span = fd_tok.span;
                    // IoNumber should be followed by a redirect operator.
                    if let Token::Redirect(kind) = &self.peek().node {
                        let kind = *kind;
                        self.advance()?;
                        // For heredoc redirects, the target is the delimiter word.
                        let target_tok = self.expect_word()?;
                        end_span = target_tok.span;
                        let target_span = Self::word_span(&target_tok.node)
                            .unwrap_or(target_tok.span);
                        let redir_span = fd_tok.span.merge(target_tok.span);
                        redirects.push(Spanned {
                            node: Redirect {
                                kind,
                                target: target_span,
                                fd: Some(fd_val),
                                quoted: false,
                            },
                            span: redir_span,
                        });
                    } else {
                        // IoNumber not followed by redirect — treat as word fallback.
                        // The lexer emits IoNumber only before redirects, so this
                        // shouldn't normally happen, but be defensive.
                        let fallback_span = fd_tok.span;
                        words.push(fallback_span);
                    }
                }
                Token::Redirect(kind) => {
                    let kind = *kind;
                    let redir_tok = self.advance()?;
                    let target_tok = self.expect_word()?;
                    end_span = target_tok.span;
                    let target_span = Self::word_span(&target_tok.node)
                        .unwrap_or(target_tok.span);
                    let redir_span = redir_tok.span.merge(target_tok.span);
                    redirects.push(Spanned {
                        node: Redirect {
                            kind,
                            target: target_span,
                            fd: None,
                            quoted: false,
                        },
                        span: redir_span,
                    });
                    // For heredoc redirects, the body follows after a newline.
                    // Skip the newline so the loop continues to pick up HereDocBody.
                    if matches!(kind, RedirectKind::HereDoc | RedirectKind::HereDocStrip) {
                        if matches!(self.peek().node, Token::Newline) {
                            self.advance()?;
                        }
                    }
                }
                Token::HereDocBody { body: _, quoted } => {
                    let quoted_val = *quoted;
                    let body_tok = self.advance()?;
                    end_span = body_tok.span;
                    // Determine the heredoc kind from the preceding redirect.
                    let kind = if let Some(prev) = redirects.last() {
                        if matches!(
                            prev.node.kind,
                            RedirectKind::HereDoc | RedirectKind::HereDocStrip
                        ) {
                            prev.node.kind
                        } else {
                            RedirectKind::HereDoc
                        }
                    } else {
                        RedirectKind::HereDoc
                    };
                    // Remove the preceding delimiter redirect — only the body matters.
                    if let Some(prev) = redirects.last() {
                        if matches!(
                            prev.node.kind,
                            RedirectKind::HereDoc | RedirectKind::HereDocStrip
                        ) {
                            redirects.pop();
                        }
                    }
                    // For heredoc body, the target span covers the body text.
                    // We store the body string via a synthetic span. Since
                    // HereDocBody owns its string and the AST Redirect uses Span,
                    // we use the body_tok span as the target (the executor will
                    // need to reconstruct the body from the token stream or we
                    // store it differently). For now, use body_tok.span.
                    redirects.push(Spanned {
                        node: Redirect {
                            kind,
                            target: body_tok.span,
                            fd: None,
                            quoted: quoted_val,
                        },
                        span: body_tok.span,
                    });
                }
                _ => break,
            }
        }

        let full_span = start_span.merge(end_span);

        // Classification.
        if words.is_empty() && !env_assigns.is_empty() && redirects.is_empty() {
            return Ok(Spanned {
                node: Command::EnvAssign { assigns: env_assigns },
                span: full_span,
            });
        }

        if words.is_empty() {
            if redirects.is_empty() && env_assigns.is_empty() {
                return Ok(Spanned {
                    node: Command::Empty,
                    span: full_span,
                });
            }
            // Null command with redirects (and/or env assigns only).
            // Use Simple with a zero-length span as the name.
            return Ok(Spanned {
                node: Command::Simple {
                    name: Span::new(start_span.start, start_span.start),
                    args: vec![],
                    redirects,
                    env_assigns,
                },
                span: full_span,
            });
        }

        let name = words.remove(0);
        Ok(Spanned {
            node: Command::Simple {
                name,
                args: words,
                redirects,
                env_assigns,
            },
            span: full_span,
        })
    }

    // -- Trailing redirects ------------------------------------------------

    /// Collect trailing I/O redirections after compound commands.
    fn collect_trailing_redirects(&mut self) -> Result<Vec<Spanned<Redirect>>, ParseError> {
        let mut redirects = Vec::new();
        loop {
            match &self.peek().node {
                Token::IoNumber(fd, _) => {
                    let fd_val = *fd;
                    let fd_tok = self.advance()?;
                    if let Token::Redirect(kind) = &self.peek().node {
                        let kind = *kind;
                        self.advance()?;
                        let target_tok = self.expect_word()?;
                        let target_span = Self::word_span(&target_tok.node)
                            .unwrap_or(target_tok.span);
                        let redir_span = fd_tok.span.merge(target_tok.span);
                        redirects.push(Spanned {
                            node: Redirect {
                                kind,
                                target: target_span,
                                fd: Some(fd_val),
                                quoted: false,
                            },
                            span: redir_span,
                        });
                    } else {
                        // IoNumber not followed by redirect — stop collecting.
                        break;
                    }
                }
                Token::Redirect(kind) => {
                    let kind = *kind;
                    let redir_tok = self.advance()?;
                    let target_tok = self.expect_word()?;
                    let target_span = Self::word_span(&target_tok.node)
                        .unwrap_or(target_tok.span);
                    let redir_span = redir_tok.span.merge(target_tok.span);
                    redirects.push(Spanned {
                        node: Redirect {
                            kind,
                            target: target_span,
                            fd: None,
                            quoted: false,
                        },
                        span: redir_span,
                    });
                }
                _ => break,
            }
        }
        Ok(redirects)
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Check if a word is a variable assignment (`VAR=value`).
fn is_assignment(word: &str) -> bool {
    if let Some(eq_pos) = word.find('=') {
        let lhs = &word[..eq_pos];
        let lhs = lhs.strip_suffix('+').unwrap_or(lhs);
        // Strip optional [subscript] suffix: name[idx]=value
        let name = if let Some(bracket_pos) = lhs.find('[') {
            if !lhs.ends_with(']') {
                return false;
            }
            &lhs[..bracket_pos]
        } else {
            lhs
        };
        !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    } else {
        false
    }
}

/// Split `VAR=value` into (name_span, value_span) based on the word span.
fn split_assignment(word: &str, span: Span) -> (Span, Span) {
    match word.find('=') {
        Some(eq_pos) => {
            let key = Span::new(span.start, span.start + eq_pos as u32);
            let val = Span::new(span.start + eq_pos as u32 + 1, span.end);
            (key, val)
        }
        None => (span, Span::new(span.end, span.end)),
    }
}

/// Is this word a compound-command keyword?
#[allow(dead_code)]
fn is_compound_keyword(w: &str) -> bool {
    matches!(
        w,
        "if" | "then"
            | "elif"
            | "else"
            | "fi"
            | "while"
            | "until"
            | "for"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "select"
            | "function"
            | "coproc"
            | "time"
            | "in"
            | "{"
            | "}"
    )
}
