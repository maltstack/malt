# Phase 2: `mash` Sub-Project 1 — Lexer + Parser

## Goal

Build the lexer and parser for the MASH POSIX shell. Tokenizes shell input with zero-copy spans, parses into a span-annotated AST. Full POSIX parity with the vexil-shell reference — no conformance regression.

## Architecture

Single `mash` crate with `ast`, `lexer`, and `parser` modules. The lexer is a streaming iterator producing span-based tokens (zero-copy — tokens reference the input string). The parser is recursive descent with one-token lookahead, producing `Vec<Spanned<Command>>`. All AST types live in `ast.rs` shared by both lexer and parser.

## Reference Implementation

`C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\` — `lexer.rs` (1,345 lines) and `parser.rs` (1,790 lines). Port the POSIX logic. Rewrite with zero-copy spans, streaming iteration, and proper quality.

## Spec References

- `malt/specs/architecture.md` §5 (Shell)
- `C:\Users\mamuk\projects\vexil-v2\CLAUDE.md` (POSIX conformance, Smoosh suite)

---

## Crate Structure

```
orix/malt/crates/mash/
  Cargo.toml
  src/
    lib.rs              # pub mod ast, lexer, parser
    ast.rs              # Span, Spanned<T>, Token, Command, Redirect, all types
    lexer.rs            # Lexer iterator: &str → Token stream
    parser.rs           # Parser: token stream → Vec<Spanned<Command>>
  tests/
    lexer.rs            # Token-level tests with span assertions
    parser.rs           # AST structure tests
```

---

## Span Type

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self;
    pub fn merge(self, other: Span) -> Span;
    pub fn text<'a>(&self, source: &'a str) -> &'a str;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}
```

`u32` offsets — supports files up to 4 GiB.

---

## Token Type

Zero-copy: `Word` stores a `Span` referencing the input, not an owned `String`. The only exception is `HereDocBody` which constructs its body string (tab stripping, delimiter removal).

```rust
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
```

Operator tokens (`Pipe`, `Semicolon`, etc.) don't need spans stored in the variant — the `Lexer` yields `Spanned<Token>` so the span is always available.

---

## RedirectKind

All 11 variants — full POSIX + common extensions:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectKind {
    Input,         // <
    Output,        // >
    Append,        // >>
    Clobber,       // >|
    InputOutput,   // <>
    DupInput,      // <&
    DupOutput,     // >&
    Both,          // &>
    HereString,    // <<<
    HereDoc,       // <<
    HereDocStrip,  // <<-
}
```

---

## Command AST

All 21 variants from reference — full POSIX parity. Every variant carries source spans via `Spanned<Command>`.

```rust
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Command {
    Simple {
        name: Span,
        args: Vec<Span>,
        redirects: Vec<Spanned<Redirect>>,
        env_assigns: Vec<(Span, Span)>,  // (name_span, value_span)
    },
    Pipeline {
        commands: Vec<Spanned<Command>>,
        negated: bool,
    },
    List {
        pairs: Vec<(Spanned<Command>, ListOp)>,
        last: Box<Spanned<Command>>,
    },
    If {
        condition: Box<Spanned<Command>>,
        then_body: Vec<Spanned<Command>>,
        elif_clauses: Vec<(Spanned<Command>, Vec<Spanned<Command>>)>,
        else_body: Option<Vec<Spanned<Command>>>,
    },
    While {
        condition: Box<Spanned<Command>>,
        body: Vec<Spanned<Command>>,
    },
    Until {
        condition: Box<Spanned<Command>>,
        body: Vec<Spanned<Command>>,
    },
    For {
        var: Span,
        words: Vec<Span>,
        body: Vec<Spanned<Command>>,
    },
    ForArith {
        init: Span,
        cond: Span,
        step: Span,
        body: Vec<Spanned<Command>>,
    },
    Case {
        word: Span,
        items: Vec<CaseItem>,
    },
    Select {
        var: Span,
        words: Vec<Span>,
        body: Vec<Spanned<Command>>,
    },
    FunctionDef {
        name: Span,
        body: Box<Spanned<Command>>,
    },
    BraceGroup {
        body: Vec<Spanned<Command>>,
    },
    Subshell {
        body: Vec<Spanned<Command>>,
    },
    Arithmetic {
        expr: Span,
    },
    Conditional {
        expr: Span,
    },
    Background(Box<Spanned<Command>>),
    EnvAssign {
        assigns: Vec<(Span, Span)>,
    },
    Empty,
    Coproc {
        name: Option<Span>,
        cmd: Box<Spanned<Command>>,
    },
    Time {
        posix_format: bool,
        command: Box<Spanned<Command>>,
    },
    Redirected {
        cmd: Box<Spanned<Command>>,
        redirects: Vec<Spanned<Redirect>>,
    },
}
```

Note: `Simple.name` and `Simple.args` are `Span`s — the actual text is retrieved via `span.text(source)`. The expander (sub-project 3) will process these spans into expanded strings.

---

## Redirect

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: Span,
    pub fd: Option<i32>,
    pub quoted: bool,   // for heredoc delimiter quoting
}
```

---

## ListOp

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListOp {
    Sequential,   // ;
    Background,   // &
    AndIf,        // &&
    OrIf,         // ||
}
```

---

## CaseItem

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct CaseItem {
    pub patterns: Vec<Span>,
    pub body: Vec<Spanned<Command>>,
}
```

---

## Lexer Design

Streaming iterator over the input string. Zero-copy for word tokens.

### Public API

```rust
pub struct Lexer<'a> { /* ... */ }

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self;
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token>, LexError>;
}
```

### Internal State

```rust
struct Lexer<'a> {
    input: &'a str,
    chars: Peekable<CharIndices<'a>>,
    pending_heredocs: Vec<PendingHeredoc>,
}

struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
}
```

### Key Behaviors

1. **Words are spans** — `Token::Word(Span { start, end })` points into the original input. No allocation.

2. **Heredoc two-phase** — After `<<`/`<<-`, next word is delimiter. Body accumulated after next newline, emitted as `Token::HereDocBody { body, quoted }`. This is the one token that allocates (tab stripping, delimiter removal mutate the text).

3. **Balanced tracking** — Command substitution `$(...)`, arithmetic `$((...))`, parameter expansion `${...}` tracked with depth counters. Content stays in the Word span.

4. **Line continuation** — `\<newline>` consumed silently (joins lines).

5. **IoNumber detection** — Digits immediately before `<` or `>` emit `IoNumber(n, span)`.

6. **Quote preservation** — Quotes are part of the Word span. The expander handles quote removal.

7. **Process substitution** — `<(...)` and `>(...)` tracked with balanced parens.

### Error Type

```rust
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
```

---

## Parser Design

Recursive descent, one-token lookahead, no backtracking.

### Public API

```rust
pub fn parse(input: &str) -> Result<Vec<Spanned<Command>>, ParseError>
```

### Internal Structure

```rust
struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    current: Option<Spanned<Token>>,
}
```

### Grammar Productions

Each POSIX grammar rule is a method:
- `parse_complete_command()` → top-level entry
- `parse_list()` → handles `;`, `&`
- `parse_and_or()` → handles `&&`, `||`
- `parse_pipeline()` → handles `|`, `!` negation
- `parse_command()` → dispatches to compound or simple
- `parse_simple_command()` → word + args + redirects + env assigns
- `parse_compound_command()` → if/while/until/for/case/select/brace/subshell
- `parse_if()`, `parse_while()`, `parse_for()`, `parse_case()`, etc.
- `parse_redirect()` → redirect operator + target word
- `parse_function_def()` → `name() { body }` or `function name { body }`

### Reserved Word Recognition

Keywords (`if`, `then`, `else`, `fi`, `while`, `do`, `done`, `for`, `case`, `in`, `esac`, `select`, `function`) are recognized by matching `Word` span text at specific parse positions. No special token type — the parser checks `span.text(self.input) == "if"` etc.

### Operator Precedence

1. `||` — lowest
2. `&&`
3. `;` / `&`
4. `|` — highest

### Span Propagation

Each parse method returns `Spanned<Command>`. Spans are constructed by merging the first and last token spans of each production. For example, `parse_if()` merges the span of `if` keyword with the span of `fi` keyword.

### Error Type

```rust
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
```

---

## Dependencies

```toml
[dependencies]
thiserror = "2"
```

No other dependencies. The lexer and parser are pure computation — no I/O, no platform, no protocol. Later sub-projects (env, expander, executor) will add `malt-platform` and `malt-protocol`.

---

## Testing Strategy

### Lexer Tests

Input string → assert exact `Spanned<Token>` sequence.

Coverage:
- Simple words and whitespace
- All operators (`|`, `||`, `&&`, `;`, `;;`, `&`, `(`, `)`, `{`, `}`)
- All redirect operators (11 kinds)
- IoNumber detection (`2>`, `0<`)
- Single quotes, double quotes, backslash escapes
- ANSI-C quotes (`$'...'`)
- Line continuation (`\<newline>`)
- Heredocs: `<<EOF`, `<<-EOF`, quoted delimiter `<<'EOF'`
- Command substitution: `$(cmd)`, `` `cmd` ``
- Arithmetic: `$((expr))`
- Process substitution: `<(cmd)`, `>(cmd)`
- Nested constructs: `"$(echo "hello")"`, `${var:-$(default)}`
- Span correctness: verify start/end offsets match source text
- Error cases: unterminated string, unterminated heredoc, unterminated process sub

### Parser Tests

Input string → assert `Spanned<Command>` structure.

Coverage:
- Simple command: `echo hello world`
- Pipeline: `cat file | grep foo | wc -l`
- Pipeline negation: `! grep -q error log`
- List operators: `cmd1 && cmd2`, `cmd1 || cmd2`, `cmd1 ; cmd2`, `cmd1 & cmd2`
- If/elif/else/fi
- While/do/done, until/do/done
- For/do/done (word list), for arithmetic `(( ))`
- Case/esac with multiple patterns per item
- Select/do/done
- Function definition: `f() { body; }`, `function f { body; }`
- Brace group: `{ cmd1; cmd2; }`
- Subshell: `(cmd1; cmd2)`
- Arithmetic: `(( x + 1 ))`
- Conditional: `[[ -f file ]]`
- Background: `cmd &`
- Coproc: `coproc name cmd`
- Time: `time -p pipeline`
- Redirected compound: `if ...; fi > out`, `{ cmd; } 2>&1`
- Environment assignments: `FOO=bar cmd`, `FOO=bar` (no command)
- Nested structures: `if cmd; then for x in a b; do echo $x; done; fi`
- Span correctness: verify command spans cover the full source range
- Error cases: unexpected token, unexpected EOF, missing `fi`/`done`/`esac`

### Ported Regression Tests

Port parser-level tests from `vexil-shell/tests/posix_regression.rs` that only verify parse success (no execution). These tests call `parse(input)` and assert it returns `Ok` — verifying the parser accepts all POSIX constructs that the reference parser accepted.
