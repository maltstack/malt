//! POSIX shell parser — recursive descent, span-annotated AST.
//!
//! Implements the full POSIX grammar: command lists, and-or lists, pipelines,
//! simple commands with redirects and environment assignments, and all compound
//! commands (if/while/until/for/case/select/function/brace group/subshell/
//! arithmetic/conditional/coproc/time).

use crate::ast::*;
use crate::lexer::Lexer;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a shell input string into a list of commands.
pub fn parse(input: &str) -> Result<Vec<Spanned<Command>>, ParseError> {
    let mut parser = Parser::new(input)?;
    parser.parse_command_list()
}

/// Expand aliases before parsing so that grammar-introducing aliases like
/// Modernish LOOP/DO/DONE macros are expanded into real syntax tokens.
///
/// This is a "preparse" step: it only expands aliases in command position
/// (including after `|`, `&&`, `||`, `;`, newline). It does NOT expand
/// inside compound commands (if/while/for/case/function bodies) because
/// those contexts do not support aliases.
///
/// Iterates until no more substitutions are made, supporting aliases that
/// expand to text containing other alias references. A recursion guard
/// prevents infinite loops (an alias expanding to itself).
pub fn preparse_expanded(input: &str, aliases: &HashMap<String, String>) -> String {
    if aliases.is_empty() {
        return input.to_string();
    }
    let mut current = input.to_string();
    for _ in 0..100 {
        let next = preparse_expanded_pass(&current, aliases);
        if next == current {
            break;
        }
        current = next;
    }
    current
}

fn preparse_expanded_pass(input: &str, aliases: &HashMap<String, String>) -> String {
    let mut lexer = Lexer::new(input);
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut in_command_position = true;
    while let Some(tok) = lexer.next() {
        let tok = match tok {
            Ok(t) => t,
            Err(_) => break,
        };
        let s = tok.span.start as usize;
        let e = tok.span.end as usize;
        if s > last_end {
            result.push_str(&input[last_end..s]);
        }
        match &tok.node {
            Token::Word(span) if in_command_position => {
                let word = span.text(input);
                if let Some(replacement) = aliases.get(word) {
                    result.push_str(replacement);
                    let ends_with_sep = replacement
                        .as_bytes()
                        .last()
                        .map(|b| b" \t\n;|&".contains(b))
                        .unwrap_or(false);
                    in_command_position = ends_with_sep;
                } else {
                    result.push_str(&input[s..e]);
                    in_command_position = false;
                }
            }
            _ => {
                result.push_str(&input[s..e]);
                in_command_position = matches!(
                    tok.node,
                    Token::Semicolon
                        | Token::Newline
                        | Token::Pipe
                        | Token::AndAnd
                        | Token::OrOr
                        | Token::Ampersand
                        | Token::LBrace
                        | Token::LParen
                );
            }
        }
        last_end = e;
    }
    if last_end < input.len() {
        result.push_str(&input[last_end..]);
    }
    result
}

/// Collect alias definitions from script text before execution.
/// This scans for lines like `alias NAME='VALUE'` and builds a map
/// that can be used for preparse expansion before the script executes.
/// Collects ALL aliases (not just grammar macros) so that source/eval
/// can pick up runtime aliases like LOOP/DO/DONE and others.
pub fn collect_grammar_aliases_from_script(input: &str) -> HashMap<String, String> {
    collect_aliases_from_script(input)
}

pub fn collect_aliases_from_script(input: &str) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for line in input.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("alias ") {
            // Support: alias NAME=VALUE
            // Find the first unquoted '=' sign
            let mut in_single = false;
            let mut in_double = false;
            let mut eq_pos = None;
            let bytes = rest.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                match b {
                    b'\'' if !in_double => in_single = !in_single,
                    b'"' if !in_single => in_double = !in_double,
                    b'=' if !in_single && !in_double => {
                        eq_pos = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(pos) = eq_pos {
                let name = rest[..pos].trim();
                let value = &rest[pos + 1..];
                // Strip surrounding quotes from value
                let value = if (value.starts_with('\'') && value.ends_with('\''))
                    || (value.starts_with('"') && value.ends_with('"'))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };
                if !name.is_empty() {
                    aliases.insert(name.to_string(), value.to_string());
                }
            }
        }
    }
    aliases
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
        let tok = self.current.take().unwrap_or_else(eof_sentinel);
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

    /// Drain any pending HereDocBody tokens at statement boundaries.
    ///
    /// When a heredoc is part of a pipeline (e.g. `cat <<EOF | grep foo\n...`),
    /// the lexer emits the HereDocBody AFTER the pipeline's terminating newline,
    /// because it can only read the body once a newline is seen. By the time
    /// `parse_command_list_until` consumes the terminating newline for a
    /// pipeline command, the HereDocBody token is already buffered in the lexer
    /// and will be the next token peeked. These tokens have already been
    /// accounted for by the `<<` redirect in the pipeline; draining them here
    /// prevents the parser from treating them as a new command.
    fn drain_heredoc_bodies(&mut self) -> Result<(), ParseError> {
        while matches!(self.peek().node, Token::HereDocBody { .. }) {
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
            // Skip stray terminators between commands (including any HereDocBody
            // tokens that were buffered after a newline for pipeline heredocs).
            if matches!(
                self.peek().node,
                Token::Semicolon | Token::Newline | Token::HereDocBody { .. }
            ) {
                self.skip_terminators()?;
                self.drain_heredoc_bodies()?;
                continue;
            }

            let before_span = self.peek().span;
            let cmd = self.parse_and_or()?;

            match &self.peek().node {
                Token::Semicolon | Token::Newline => {
                    self.advance()?;
                    // Drain any HereDocBody tokens buffered after the newline;
                    // they belong to a heredoc redirect on the preceding command.
                    self.drain_heredoc_bodies()?;
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
        let cmd = match &self.peek().node {
            Token::Word(span) => {
                let w = span.text(self.input).to_owned();
                match w.as_str() {
                    "if" => self.parse_if()?,
                    "while" => self.parse_while()?,
                    "until" => self.parse_until()?,
                    "for" => self.parse_for()?,
                    "case" => self.parse_case()?,
                    "select" => self.parse_select()?,
                    "function" => self.parse_function_keyword()?,
                    "coproc" => self.parse_coproc()?,
                    "time" => self.parse_time()?,
                    _ => {
                        // Simple command or POSIX function def (name() { body }).
                        // Function def detection happens inside parse_simple_command
                        // after consuming the first word and seeing `(`.
                        return self.parse_simple_command();
                    }
                }
            }
            Token::LBrace => self.parse_brace_group()?,
            Token::LParen => self.parse_subshell()?,
            Token::LParenParen => self.parse_arithmetic()?,
            Token::LBracketBracket => self.parse_conditional()?,
            // Everything else is a simple command.
            _ => return self.parse_simple_command(),
        };

        // Collect trailing I/O redirections on compound commands.
        let trailing = self.collect_trailing_redirects()?;
        if trailing.is_empty() {
            Ok(cmd)
        } else {
            let span = cmd
                .span
                .merge(trailing.last().map(|r| r.span).unwrap_or(cmd.span));
            Ok(Spanned {
                node: Command::Redirected {
                    cmd: Box::new(cmd),
                    redirects: trailing,
                },
                span,
            })
        }
    }

    // -- Compound command parsers -----------------------------------------

    /// Parse `if condition; then body [elif ...] [else ...] fi`
    fn parse_if(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("if")?;
        self.skip_newlines()?;

        let condition = self.parse_command_list_as_one(
            |t| matches!(t, Token::Word(s) if s.text(self.input) == "then"),
        )?;
        self.expect_keyword("then")?;
        self.skip_newlines()?;

        let then_body = self.parse_body_until(&["elif", "else", "fi"])?;

        let mut elif_clauses = Vec::new();
        while self.peek_keyword("elif") {
            self.advance()?;
            self.skip_newlines()?;
            let elif_cond = self.parse_command_list_as_one(
                |t| matches!(t, Token::Word(s) if s.text(self.input) == "then"),
            )?;
            self.expect_keyword("then")?;
            self.skip_newlines()?;
            let elif_body = self.parse_body_until(&["elif", "else", "fi"])?;
            elif_clauses.push((elif_cond, elif_body));
        }

        let else_body = if self.peek_keyword("else") {
            self.advance()?;
            self.skip_newlines()?;
            Some(self.parse_body_until(&["fi"])?)
        } else {
            None
        };

        let end = self.expect_keyword("fi")?;

        Ok(Spanned {
            node: Command::If {
                condition: Box::new(condition),
                then_body,
                elif_clauses,
                else_body,
            },
            span: start.merge(end),
        })
    }

    /// Parse `while condition; do body; done`
    fn parse_while(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("while")?;
        self.skip_newlines()?;

        let condition = self.parse_command_list_as_one(
            |t| matches!(t, Token::Word(s) if s.text(self.input) == "do"),
        )?;
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        let body = self.parse_body_until(&["done"])?;
        let end = self.expect_keyword("done")?;

        Ok(Spanned {
            node: Command::While {
                condition: Box::new(condition),
                body,
            },
            span: start.merge(end),
        })
    }

    /// Parse `until condition; do body; done`
    fn parse_until(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("until")?;
        self.skip_newlines()?;

        let condition = self.parse_command_list_as_one(
            |t| matches!(t, Token::Word(s) if s.text(self.input) == "do"),
        )?;
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        let body = self.parse_body_until(&["done"])?;
        let end = self.expect_keyword("done")?;

        Ok(Spanned {
            node: Command::Until {
                condition: Box::new(condition),
                body,
            },
            span: start.merge(end),
        })
    }

    /// Parse `for var [in words]; do body; done` or `for (( ... )); do body; done`
    fn parse_for(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("for")?;
        self.skip_newlines()?;

        // Check for C-style: `for (( init; cond; step ))`
        if matches!(self.peek().node, Token::LParenParen) {
            return self.parse_for_arith(start);
        }

        let var_tok = self.expect_word()?;
        let var = Self::word_span(&var_tok.node).unwrap_or(var_tok.span);

        self.skip_newlines()?;
        // Optional `in words...` clause.
        let words = if self.peek_keyword("in") {
            self.advance()?; // consume "in"
            let mut ws = Vec::new();
            let mut loop_count = 0u64;
            loop {
                let before_span = self.peek().span;
                match &self.peek().node {
                    Token::Word(span) => {
                        let text = span.text(self.input);
                        if text == "do" {
                            break;
                        }
                        let s = *span;
                        self.advance()?;
                        ws.push(s);
                        // Guard: verify we advanced to prevent infinite loop.
                        if self.peek().span.start == before_span.start
                            && self.peek().span.end == before_span.end
                        {
                            return Err(ParseError::Unexpected {
                                token: self.peek().node.clone(),
                                span: self.peek().span,
                            });
                        }
                    }
                    _ => break,
                }
                // Iteration count guard
                loop_count += 1;
                if loop_count > 10000 {
                    return Err(ParseError::Unexpected {
                        token: self.peek().node.clone(),
                        span: self.peek().span,
                    });
                }
            }
            ws
        } else {
            Vec::new()
        };

        self.skip_terminators()?;
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        let body = self.parse_body_until(&["done"])?;
        let end = self.expect_keyword("done")?;
        Ok(Spanned {
            node: Command::For { var, words, body },
            span: start.merge(end),
        })
    }

    /// Parse the C-style arithmetic for loop: `for (( init; cond; step )); do body; done`
    /// Called after "for" keyword has been consumed.
    fn parse_for_arith(&mut self, start: Span) -> Result<Spanned<Command>, ParseError> {
        self.advance()?; // consume `((`

        // Collect tokens until `))`. The three clauses are separated by `;`.
        let mut clause_spans: Vec<(u32, u32)> = Vec::new();
        let mut clause_start = self.peek().span.start;

        loop {
            match &self.peek().node {
                Token::RParenParen => {
                    let end_pos = self.peek().span.start;
                    clause_spans.push((clause_start, end_pos));
                    self.advance()?;
                    break;
                }
                Token::Eof => return Err(ParseError::UnexpectedEof),
                Token::Semicolon => {
                    let end_pos = self.peek().span.start;
                    clause_spans.push((clause_start, end_pos));
                    self.advance()?;
                    clause_start = self.peek().span.start;
                }
                _ => {
                    self.advance()?;
                }
            }
        }

        // Pad to exactly 3 clauses.
        let fallback = self.peek().span.start;
        while clause_spans.len() < 3 {
            clause_spans.push((fallback, fallback));
        }

        let init = Span::new(clause_spans[0].0, clause_spans[0].1);
        let cond = Span::new(clause_spans[1].0, clause_spans[1].1);
        let step = Span::new(clause_spans[2].0, clause_spans[2].1);

        self.skip_terminators()?;
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        let body = self.parse_body_until(&["done"])?;
        let end = self.expect_keyword("done")?;

        Ok(Spanned {
            node: Command::ForArith {
                init,
                cond,
                step,
                body,
            },
            span: start.merge(end),
        })
    }

    /// Parse `case word in (pattern|pattern) body ;; ... esac`
    fn parse_case(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("case")?;
        self.skip_newlines()?;

        let word_tok = self.expect_word()?;
        let word = Self::word_span(&word_tok.node).unwrap_or(word_tok.span);
        self.skip_newlines()?;

        self.expect_keyword("in")?;
        self.skip_newlines()?;

        let mut items = Vec::new();

        while !self.peek_keyword("esac") && !matches!(self.peek().node, Token::Eof) {
            // Optional leading `(`
            if matches!(self.peek().node, Token::LParen) {
                self.advance()?;
            }

            // Parse patterns separated by `|`.
            let mut patterns = Vec::new();
            let pat_tok = self.expect_word()?;
            patterns.push(Self::word_span(&pat_tok.node).unwrap_or(pat_tok.span));

            while matches!(self.peek().node, Token::Pipe) {
                self.advance()?;
                let pat_tok = self.expect_word()?;
                patterns.push(Self::word_span(&pat_tok.node).unwrap_or(pat_tok.span));
            }

            // Expect `)`
            if !matches!(self.peek().node, Token::RParen) {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "expected ')' after case pattern".to_string(),
                });
            }
            self.advance()?;
            self.skip_newlines()?;

            // Parse body until `;;` or `esac`.
            let body = self.parse_command_list_until(|t| {
                matches!(t, Token::SemiSemi)
                    || matches!(t, Token::Word(s) if s.text(self.input) == "esac")
            })?;

            items.push(CaseItem { patterns, body });

            if matches!(self.peek().node, Token::SemiSemi) {
                self.advance()?;
                self.skip_newlines()?;
            }
        }

        let end = self.expect_keyword("esac")?;

        Ok(Spanned {
            node: Command::Case { word, items },
            span: start.merge(end),
        })
    }

    /// Parse `select var in words; do body; done`
    fn parse_select(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("select")?;
        self.skip_newlines()?;

        let var_tok = self.expect_word()?;
        let var = Self::word_span(&var_tok.node).unwrap_or(var_tok.span);

        self.skip_newlines()?;
        let words = if self.peek_keyword("in") {
            self.advance()?;
            let mut ws = Vec::new();
            loop {
                let before_span = self.peek().span;
                match &self.peek().node {
                    Token::Word(span) => {
                        let text = span.text(self.input);
                        if text == "do" {
                            break;
                        }
                        let s = *span;
                        self.advance()?;
                        ws.push(s);
                        // Guard: verify we advanced to prevent infinite loop.
                        if self.peek().span.start == before_span.start
                            && self.peek().span.end == before_span.end
                        {
                            return Err(ParseError::Unexpected {
                                token: self.peek().node.clone(),
                                span: self.peek().span,
                            });
                        }
                    }
                    _ => break,
                }
            }
            ws
        } else {
            Vec::new()
        };

        self.skip_terminators()?;
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        let body = self.parse_body_until(&["done"])?;
        let end = self.expect_keyword("done")?;

        Ok(Spanned {
            node: Command::Select { var, words, body },
            span: start.merge(end),
        })
    }

    /// Parse `function name [()]; { body }` or `function name compound_command`
    fn parse_function_keyword(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("function")?;
        self.skip_newlines()?;

        let name_tok = self.expect_word()?;
        let name = Self::word_span(&name_tok.node).unwrap_or(name_tok.span);

        // Optional `()` after function name.
        if matches!(self.peek().node, Token::LParen) {
            self.advance()?;
            if !matches!(self.peek().node, Token::RParen) {
                return Err(ParseError::SyntaxError {
                    pos: self.peek().span.start,
                    message: "expected ')' after '(' in function definition".to_string(),
                });
            }
            self.advance()?;
        }

        self.skip_newlines()?;
        let body = self.parse_command()?;
        let span = start.merge(body.span);

        Ok(Spanned {
            node: Command::FunctionDef {
                name,
                body: Box::new(body),
            },
            span,
        })
    }

    /// Parse `coproc [NAME] command`
    fn parse_coproc(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("coproc")?;
        self.skip_newlines()?;

        // If the next token is a word that is NOT a compound-command keyword,
        // check if it might be a named coproc. A named coproc has a name
        // followed by a command. If the word after the name starts a command,
        // treat it as named. Otherwise, treat the word as the start of a simple command.
        let (name, cmd) = match &self.peek().node {
            Token::Word(span) => {
                let text = span.text(self.input).to_owned();
                if !is_compound_keyword(&text) {
                    // Could be named coproc — but we need lookahead.
                    // Simple heuristic: if the word is not a compound keyword
                    // and could be a name, consume it as name and parse command.
                    // But we can't distinguish `coproc cat` (unnamed, command=cat)
                    // from `coproc MYNAME cat` (named) without more lookahead.
                    //
                    // POSIX/bash rule: if next-next token starts a compound command,
                    // treat current word as name. Otherwise, parse current word as
                    // the start of a simple command (unnamed coproc).
                    //
                    // Since we lack multi-token lookahead, we parse as unnamed and
                    // let the current word become part of the command.
                    let c = self.parse_command()?;
                    (None, c)
                } else {
                    let c = self.parse_command()?;
                    (None, c)
                }
            }
            _ => {
                let c = self.parse_command()?;
                (None, c)
            }
        };

        let span = start.merge(cmd.span);
        Ok(Spanned {
            node: Command::Coproc {
                name,
                cmd: Box::new(cmd),
            },
            span,
        })
    }

    /// Parse `time [-p] pipeline`
    fn parse_time(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.expect_keyword("time")?;
        self.skip_newlines()?;

        let posix_format = if self.peek_keyword("-p") {
            self.advance()?;
            self.skip_newlines()?;
            true
        } else {
            false
        };

        let cmd = self.parse_pipeline()?;
        let span = start.merge(cmd.span);

        Ok(Spanned {
            node: Command::Time {
                posix_format,
                command: Box::new(cmd),
            },
            span,
        })
    }

    /// Parse a brace group: `{ command_list; }`
    fn parse_brace_group(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.peek().span;
        self.advance()?; // consume `{`
        self.skip_newlines()?;

        let body = self.parse_command_list_until(|t| matches!(t, Token::RBrace))?;

        if !matches!(self.peek().node, Token::RBrace) {
            return Err(ParseError::SyntaxError {
                pos: self.peek().span.start,
                message: "expected '}'".to_string(),
            });
        }
        let end = self.advance()?.span;

        Ok(Spanned {
            node: Command::BraceGroup { body },
            span: start.merge(end),
        })
    }

    /// Parse a subshell: `( command_list )`
    fn parse_subshell(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.peek().span;
        self.advance()?; // consume `(`
        self.skip_newlines()?;

        let body = self.parse_command_list_until(|t| matches!(t, Token::RParen))?;

        if !matches!(self.peek().node, Token::RParen) {
            return Err(ParseError::SyntaxError {
                pos: self.peek().span.start,
                message: "expected ')'".to_string(),
            });
        }
        let end = self.advance()?.span;

        Ok(Spanned {
            node: Command::Subshell { body },
            span: start.merge(end),
        })
    }

    /// Parse `(( expr ))` arithmetic command.
    fn parse_arithmetic(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.peek().span;
        self.advance()?; // consume `((`

        let expr_start = self.peek().span.start;
        let mut expr_end = expr_start;

        loop {
            match &self.peek().node {
                Token::RParenParen => {
                    let end = self.advance()?.span;
                    return Ok(Spanned {
                        node: Command::Arithmetic {
                            expr: Span::new(expr_start, expr_end),
                        },
                        span: start.merge(end),
                    });
                }
                Token::Eof => return Err(ParseError::UnexpectedEof),
                _ => {
                    let tok = self.advance()?;
                    expr_end = tok.span.end;
                }
            }
        }
    }

    /// Parse `[[ expr ]]` conditional command.
    fn parse_conditional(&mut self) -> Result<Spanned<Command>, ParseError> {
        let start = self.peek().span;
        self.advance()?; // consume `[[`

        let expr_start = self.peek().span.start;
        let mut expr_end = expr_start;

        loop {
            match &self.peek().node {
                Token::RBracketBracket => {
                    let end = self.advance()?.span;
                    return Ok(Spanned {
                        node: Command::Conditional {
                            expr: Span::new(expr_start, expr_end),
                        },
                        span: start.merge(end),
                    });
                }
                Token::Eof => return Err(ParseError::UnexpectedEof),
                _ => {
                    let tok = self.advance()?;
                    expr_end = tok.span.end;
                }
            }
        }
    }

    // -- Compound command helpers ------------------------------------------

    /// Parse a command list until one of the given keywords is found.
    /// Returns the list of commands (body).
    fn parse_body_until(&mut self, keywords: &[&str]) -> Result<Vec<Spanned<Command>>, ParseError> {
        self.parse_command_list_until(
            |t| matches!(t, Token::Word(s) if keywords.iter().any(|kw| s.text(self.input) == *kw)),
        )
    }

    /// Parse a command list and collapse it into a single `Spanned<Command>`.
    /// Used for conditions in if/while/until where a single command is expected.
    fn parse_command_list_as_one(
        &mut self,
        is_end: impl Fn(&Token) -> bool,
    ) -> Result<Spanned<Command>, ParseError> {
        let cmds = self.parse_command_list_until(is_end)?;
        Ok(list_to_command(cmds))
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

                        // Detect POSIX function def: first word followed by `(` `)`.
                        if words.is_empty()
                            && env_assigns.is_empty()
                            && matches!(self.peek().node, Token::LParen)
                        {
                            // Consume `(`
                            self.advance()?;
                            if matches!(self.peek().node, Token::RParen) {
                                // Consume `)` — this is a function def.
                                self.advance()?;
                                self.skip_newlines()?;
                                let body = self.parse_command()?;
                                let span = start_span.merge(body.span);
                                return Ok(Spanned {
                                    node: Command::FunctionDef {
                                        name: ws,
                                        body: Box::new(body),
                                    },
                                    span,
                                });
                            } else {
                                // Not a function def — this was `word(` without `)`.
                                // This shouldn't happen in well-formed shell, but
                                // treat as error.
                                return Err(ParseError::SyntaxError {
                                    pos: self.peek().span.start,
                                    message: "expected ')' after '(' in function definition"
                                        .to_string(),
                                });
                            }
                        }

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
                        let target_span =
                            Self::word_span(&target_tok.node).unwrap_or(target_tok.span);
                        let redir_span = fd_tok.span.merge(target_tok.span);
                        redirects.push(Spanned {
                            node: Redirect {
                                kind,
                                target: target_span,
                                fd: Some(fd_val),
                                quoted: false,
                                heredoc_body: None,
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
                    let target_span = Self::word_span(&target_tok.node).unwrap_or(target_tok.span);
                    let redir_span = redir_tok.span.merge(target_tok.span);
                    redirects.push(Spanned {
                        node: Redirect {
                            kind,
                            target: target_span,
                            fd: None,
                            quoted: false,
                            heredoc_body: None,
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
                Token::HereDocBody { body, quoted } => {
                    let body_val = body.clone();
                    let quoted_val = *quoted;
                    let body_tok = self.advance()?;
                    end_span = body_tok.span;
                    let heredoc_index = redirects.iter().rposition(|redir| {
                        matches!(
                            redir.node.kind,
                            RedirectKind::HereDoc | RedirectKind::HereDocStrip
                        ) && redir.node.heredoc_body.is_none()
                    });
                    if let Some(index) = heredoc_index {
                        let prior = redirects[index].clone();
                        redirects[index] = Spanned {
                            node: Redirect {
                                kind: prior.node.kind,
                                target: body_tok.span,
                                fd: prior.node.fd,
                                quoted: quoted_val,
                                heredoc_body: Some(body_val),
                            },
                            span: prior.span.merge(body_tok.span),
                        };
                    } else {
                        redirects.push(Spanned {
                            node: Redirect {
                                kind: RedirectKind::HereDoc,
                                target: body_tok.span,
                                fd: None,
                                quoted: quoted_val,
                                heredoc_body: Some(body_val),
                            },
                            span: body_tok.span,
                        });
                    }
                    if !matches!(self.peek().node, Token::HereDocBody { .. }) {
                        break;
                    }
                }
                Token::Newline => {
                    let awaiting_heredoc_body = redirects.iter().any(|redir| {
                        matches!(
                            redir.node.kind,
                            RedirectKind::HereDoc | RedirectKind::HereDocStrip
                        ) && redir.node.heredoc_body.is_none()
                    });
                    if awaiting_heredoc_body {
                        self.advance()?;
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }

        let full_span = start_span.merge(end_span);

        // Classification.
        if words.is_empty() && !env_assigns.is_empty() && redirects.is_empty() {
            return Ok(Spanned {
                node: Command::EnvAssign {
                    assigns: env_assigns,
                },
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
                        let target_span =
                            Self::word_span(&target_tok.node).unwrap_or(target_tok.span);
                        let redir_span = fd_tok.span.merge(target_tok.span);
                        redirects.push(Spanned {
                            node: Redirect {
                                kind,
                                target: target_span,
                                fd: Some(fd_val),
                                quoted: false,
                                heredoc_body: None,
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
                    let target_span = Self::word_span(&target_tok.node).unwrap_or(target_tok.span);
                    let redir_span = redir_tok.span.merge(target_tok.span);
                    let is_heredoc =
                        matches!(kind, RedirectKind::HereDoc | RedirectKind::HereDocStrip);
                    redirects.push(Spanned {
                        node: Redirect {
                            kind,
                            target: target_span,
                            fd: None,
                            quoted: false,
                            heredoc_body: None,
                        },
                        span: redir_span,
                    });
                    if is_heredoc {
                        if matches!(self.peek().node, Token::Newline) {
                            self.advance()?;
                        }
                        if let Token::HereDocBody { body, quoted } = &self.peek().node {
                            let body_val = body.clone();
                            let quoted_val = *quoted;
                            let body_tok = self.advance()?;
                            if let Some(index) = redirects.iter().rposition(|redir| {
                                matches!(
                                    redir.node.kind,
                                    RedirectKind::HereDoc | RedirectKind::HereDocStrip
                                ) && redir.node.heredoc_body.is_none()
                            }) {
                                let prior = redirects[index].clone();
                                redirects[index] = Spanned {
                                    node: Redirect {
                                        kind: prior.node.kind,
                                        target: body_tok.span,
                                        fd: prior.node.fd,
                                        quoted: quoted_val,
                                        heredoc_body: Some(body_val),
                                    },
                                    span: prior.span.merge(body_tok.span),
                                };
                            }
                        }
                    }
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

/// Collapse a list of commands into a single `Spanned<Command>`.
/// 0 commands → Empty, 1 → return as-is, N → List with Sequential ops.
fn list_to_command(mut cmds: Vec<Spanned<Command>>) -> Spanned<Command> {
    match cmds.len() {
        0 => Spanned {
            node: Command::Empty,
            span: Span::new(0, 0),
        },
        1 => cmds.remove(0),
        _ => {
            let span = cmds[0]
                .span
                .merge(cmds.last().map(|c| c.span).unwrap_or(cmds[0].span));
            let Some(last) = cmds.pop() else {
                unreachable!("len >= 2 checked by match arm")
            };
            let pairs = cmds.into_iter().map(|c| (c, ListOp::Sequential)).collect();
            Spanned {
                node: Command::List {
                    pairs,
                    last: Box::new(last),
                },
                span,
            }
        }
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
