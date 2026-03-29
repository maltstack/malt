//! AST types for the MASH POSIX shell.
//!
//! All types use zero-copy [`Span`]s referencing the original input string.
//! The only exception is [`Token::HereDocBody`], which owns its body string
//! (tab stripping and delimiter removal mutate the text).

// ---------------------------------------------------------------------------
// Span
// ---------------------------------------------------------------------------

/// Byte-offset span into the source input. Supports files up to 4 GiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Create a new span from byte offsets.
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Merge two spans into one that covers both.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Extract the referenced text from the source string.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start as usize..self.end as usize]
    }
}

// ---------------------------------------------------------------------------
// Spanned<T>
// ---------------------------------------------------------------------------

/// A value annotated with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/// Shell token — zero-copy for words, allocated only for heredoc bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token {
    /// A word — may contain quotes, expansions. Span references original input.
    Word(Span),
    /// IO number preceding a redirect (e.g., `2` in `2>&1`).
    IoNumber(i32, Span),
    /// `|`
    Pipe,
    /// `||`
    OrOr,
    /// `&&`
    AndAnd,
    /// `;`
    Semicolon,
    /// `;;` (case terminator)
    SemiSemi,
    /// `&`
    Ampersand,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[[`
    LBracketBracket,
    /// `]]`
    RBracketBracket,
    /// `((`
    LParenParen,
    /// `))`
    RParenParen,
    /// A redirect operator.
    Redirect(RedirectKind),
    /// Heredoc body — constructed string (only token that allocates).
    HereDocBody {
        body: String,
        quoted: bool,
    },
    /// Newline (significant in shell grammar).
    Newline,
    /// End of input.
    Eof,
}

// ---------------------------------------------------------------------------
// RedirectKind
// ---------------------------------------------------------------------------

/// All 11 redirect operator variants — full POSIX plus common extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectKind {
    /// `<`
    Input,
    /// `>`
    Output,
    /// `>>`
    Append,
    /// `>|`
    Clobber,
    /// `<>`
    InputOutput,
    /// `<&`
    DupInput,
    /// `>&`
    DupOutput,
    /// `&>`
    Both,
    /// `<<<`
    HereString,
    /// `<<`
    HereDoc,
    /// `<<-`
    HereDocStrip,
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// POSIX shell command AST — 21 variants covering full POSIX parity.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// Simple command: name, arguments, redirects, and environment assignments.
    Simple {
        name: Span,
        args: Vec<Span>,
        redirects: Vec<Spanned<Redirect>>,
        env_assigns: Vec<(Span, Span)>,
    },
    /// Pipeline of commands, optionally negated with `!`.
    Pipeline {
        commands: Vec<Spanned<Command>>,
        negated: bool,
    },
    /// Command list joined by `;`, `&`, `&&`, or `||`.
    List {
        pairs: Vec<(Spanned<Command>, ListOp)>,
        last: Box<Spanned<Command>>,
    },
    /// `if condition; then body; elif ...; else ...; fi`
    If {
        condition: Box<Spanned<Command>>,
        then_body: Vec<Spanned<Command>>,
        elif_clauses: Vec<(Spanned<Command>, Vec<Spanned<Command>>)>,
        else_body: Option<Vec<Spanned<Command>>>,
    },
    /// `while condition; do body; done`
    While {
        condition: Box<Spanned<Command>>,
        body: Vec<Spanned<Command>>,
    },
    /// `until condition; do body; done`
    Until {
        condition: Box<Spanned<Command>>,
        body: Vec<Spanned<Command>>,
    },
    /// `for var in words; do body; done`
    For {
        var: Span,
        words: Vec<Span>,
        body: Vec<Spanned<Command>>,
    },
    /// C-style for: `for (( init; cond; step )); do body; done`
    ForArith {
        init: Span,
        cond: Span,
        step: Span,
        body: Vec<Spanned<Command>>,
    },
    /// `case word in pattern) body;; ... esac`
    Case {
        word: Span,
        items: Vec<CaseItem>,
    },
    /// `select var in words; do body; done`
    Select {
        var: Span,
        words: Vec<Span>,
        body: Vec<Spanned<Command>>,
    },
    /// Function definition: `name() { body }` or `function name { body }`
    FunctionDef {
        name: Span,
        body: Box<Spanned<Command>>,
    },
    /// `{ body; }`
    BraceGroup {
        body: Vec<Spanned<Command>>,
    },
    /// `( body )`
    Subshell {
        body: Vec<Spanned<Command>>,
    },
    /// `(( expr ))`
    Arithmetic {
        expr: Span,
    },
    /// `[[ expr ]]`
    Conditional {
        expr: Span,
    },
    /// `command &`
    Background(Box<Spanned<Command>>),
    /// Bare environment assignments with no command: `FOO=bar BAZ=qux`
    EnvAssign {
        assigns: Vec<(Span, Span)>,
    },
    /// Empty command (e.g., bare newline or trailing `;`).
    Empty,
    /// `coproc [name] command`
    Coproc {
        name: Option<Span>,
        cmd: Box<Spanned<Command>>,
    },
    /// `time [-p] pipeline`
    Time {
        posix_format: bool,
        command: Box<Spanned<Command>>,
    },
    /// Compound command with trailing redirects: `if ...; fi > out`
    Redirected {
        cmd: Box<Spanned<Command>>,
        redirects: Vec<Spanned<Redirect>>,
    },
}

// ---------------------------------------------------------------------------
// Redirect
// ---------------------------------------------------------------------------

/// A redirect operation.
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: Span,
    pub fd: Option<i32>,
    /// Whether the heredoc delimiter was quoted.
    pub quoted: bool,
}

// ---------------------------------------------------------------------------
// ListOp
// ---------------------------------------------------------------------------

/// Operator joining commands in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListOp {
    /// `;`
    Sequential,
    /// `&`
    Background,
    /// `&&`
    AndIf,
    /// `||`
    OrIf,
}

// ---------------------------------------------------------------------------
// CaseItem
// ---------------------------------------------------------------------------

/// A single arm in a `case` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseItem {
    pub patterns: Vec<Span>,
    pub body: Vec<Spanned<Command>>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Lexer error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LexError {
    #[error("unterminated string at byte {pos}")]
    UnterminatedString { pos: u32 },
    #[error("unexpected character {ch:?} at byte {pos}")]
    Unexpected { ch: char, pos: u32 },
    #[error("unterminated heredoc (expected delimiter {delimiter:?})")]
    UnterminatedHeredoc { delimiter: String },
    #[error("unterminated process substitution at byte {pos}")]
    UnterminatedProcessSub { pos: u32 },
}

/// Parser error.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected token at byte {}: {:?}", .span.start, .token)]
    Unexpected { token: Token, span: Span },
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("syntax error at byte {pos}: {message}")]
    SyntaxError { pos: u32, message: String },
}
