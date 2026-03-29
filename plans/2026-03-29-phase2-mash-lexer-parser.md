# `mash` Lexer + Parser — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the POSIX shell lexer and parser for mash — zero-copy span-based tokenization and recursive descent parsing producing a span-annotated AST. Full POSIX parity with vexil-shell reference.

**Architecture:** Streaming lexer iterator (`&str → Iterator<Spanned<Token>>`) with zero-copy `Word(Span)` tokens. Recursive descent parser with one-token lookahead producing `Vec<Spanned<Command>>`. All AST types in `ast.rs` shared between lexer and parser. Reference code at `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\` — port the POSIX logic, rewrite with span-based zero-copy quality.

**Tech Stack:** Rust, thiserror (errors only — no other deps)

**Spec:** `malt/specs/phase2-mash-lexer-parser.md`

**Reference:** `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\lexer.rs` (1,345 lines), `parser.rs` (1,790 lines)

---

## File Structure

```
orix/malt/crates/mash/
  Cargo.toml
  src/
    lib.rs              # pub mod ast, lexer, parser
    ast.rs              # Span, Spanned<T>, Token, Command, Redirect, all AST types
    lexer.rs            # Lexer struct implementing Iterator<Item = Result<Spanned<Token>, LexError>>
    parser.rs           # parse(input) → Result<Vec<Spanned<Command>>, ParseError>
  tests/
    lexer.rs            # Token-level tests with span assertions
    parser.rs           # AST structure tests
```

---

## Task 1: Crate Scaffold + AST Types

**Files:**
- Modify: `orix/malt/Cargo.toml` (add workspace member)
- Create: `orix/malt/crates/mash/Cargo.toml`
- Create: `orix/malt/crates/mash/src/lib.rs`
- Create: `orix/malt/crates/mash/src/ast.rs`
- Create: `orix/malt/crates/mash/src/lexer.rs` (stub)
- Create: `orix/malt/crates/mash/src/parser.rs` (stub)

- [ ] **Step 1: Add mash to workspace**

Add `"crates/mash"` to workspace members in `orix/malt/Cargo.toml`.

- [ ] **Step 2: Create Cargo.toml**

Create `orix/malt/crates/mash/Cargo.toml`:

```toml
[package]
name = "mash"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "MASH — POSIX shell for MALT"

[dependencies]
thiserror = "2"
```

- [ ] **Step 3: Write ast.rs with all types**

Create `orix/malt/crates/mash/src/ast.rs` with ALL types from the spec:

- `Span { start: u32, end: u32 }` with `new()`, `merge()`, `text()`
- `Spanned<T> { node: T, span: Span }`
- `Token` enum — all variants with `Word(Span)`, operators, `HereDocBody { body: String, quoted: bool }`, `IoNumber(i32, Span)`, `Redirect(RedirectKind)`, `Newline`, `Eof`
- `RedirectKind` — all 11 variants
- `Command` — all 21 variants using `Span` for string data, `Spanned<Command>` for children
- `Redirect { kind, target: Span, fd: Option<i32>, quoted: bool }`
- `ListOp` — Sequential, Background, AndIf, OrIf
- `CaseItem { patterns: Vec<Span>, body: Vec<Spanned<Command>> }`
- `LexError` — UnterminatedString, Unexpected, UnterminatedHeredoc, UnterminatedProcessSub (positions as u32)
- `ParseError` — Lex, Unexpected, UnexpectedEof, SyntaxError (with spans)

All types: `#[derive(Debug, Clone, PartialEq)]`, `#[non_exhaustive]` on Token, Command, RedirectKind, ListOp, LexError, ParseError.

**IMPORTANT:** Use exact type signatures from `specs/phase2-mash-lexer-parser.md`. Every Command variant's fields must match the spec.

- [ ] **Step 4: Create lib.rs and stubs**

Create `orix/malt/crates/mash/src/lib.rs`:
```rust
//! MASH — POSIX shell for MALT.
pub mod ast;
pub mod lexer;
pub mod parser;
```

Create `orix/malt/crates/mash/src/lexer.rs`:
```rust
//! POSIX shell lexer — streaming zero-copy tokenization.
use crate::ast::*;
```

Create `orix/malt/crates/mash/src/parser.rs`:
```rust
//! POSIX shell parser — recursive descent, span-annotated AST.
use crate::ast::*;
```

- [ ] **Step 5: Verify it compiles**

Run: `cd orix/malt && cargo check -p mash`

- [ ] **Step 6: Commit**

```bash
cd orix/malt
git add Cargo.toml crates/mash/
git commit -m "feat(mash): scaffold crate with AST types — Span, Token, Command (21 variants)"
```

---

## Task 2: Lexer Core

**Files:**
- Modify: `orix/malt/crates/mash/src/lexer.rs`
- Create: `orix/malt/crates/mash/tests/lexer.rs`

Implement the `Lexer` struct as an `Iterator<Item = Result<Spanned<Token>, LexError>>`. This task covers: the main character dispatch loop, whitespace/comment skipping, operator recognition, word reading (basic — no quoting/expansion yet), IoNumber detection, newline handling.

**Reference:** `vexil-shell/src/lexer.rs` lines 188-352 (main loop), 374-443 (redirect operators), 815-820 (read_word), 323-334 (IoNumber).

- [ ] **Step 1: Implement Lexer struct and Iterator**

```rust
pub struct Lexer<'a> {
    input: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    pending_heredocs: Vec<PendingHeredoc>,
    finished: bool,
}

struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self;

    fn peek_char(&mut self) -> Option<char>;
    fn next_char(&mut self) -> Option<(usize, char)>;
    fn make_span(&self, start: usize, end: usize) -> Span;
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token>, LexError>;
    fn next(&mut self) -> Option<Self::Item>;
}
```

The `next()` method is the main dispatch loop. Port the character dispatch table from the reference's `run()` method, but:
- Return one token at a time (iterator) instead of pushing to a Vec
- Use `Span` for positions instead of bare `usize`
- `Word` tokens store `Span` referencing input, not owned `String`

- [ ] **Step 2: Implement operator recognition**

Port `lex_less_than()` and `lex_greater_than()` — redirect operator dispatch. These return `Spanned<Token>` with `Token::Redirect(kind)`.

Multi-character operators: peek ahead to distinguish `<` vs `<<` vs `<<<` vs `<<-` vs `<&` vs `<>`, and `>` vs `>>` vs `>&` vs `>|`.

Also: `|` vs `||`, `&` vs `&&` vs `&>`, `;` vs `;;`, `(` vs `((`, `)` vs `))`, `[` vs `[[`, `]` vs `]]`.

- [ ] **Step 3: Implement basic word reading**

Port `read_word()` and `continue_word()` — but only the non-quoting, non-expansion parts for now. Words break on whitespace, operators, and newlines. The word span covers start to end position.

Handle IoNumber: if all digits and followed by `<` or `>`, emit `IoNumber(n, span)` instead of `Word(span)`.

- [ ] **Step 4: Write lexer tests for core functionality**

Create `orix/malt/crates/mash/tests/lexer.rs` with tests:

```rust
use mash::ast::*;
use mash::lexer::Lexer;

fn tokens(input: &str) -> Vec<Token> {
    Lexer::new(input)
        .map(|r| r.unwrap().node)
        .collect()
}

fn tokens_with_spans(input: &str) -> Vec<Spanned<Token>> {
    Lexer::new(input)
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn simple_words() {
    let toks = tokens("echo hello world");
    // Word, Word, Word, Eof
    assert_eq!(toks.len(), 4);
    assert!(matches!(&toks[0], Token::Word(_)));
    assert!(matches!(&toks[3], Token::Eof));
}

#[test]
fn operators() {
    let toks = tokens("a | b && c || d ; e & f");
    // Verify Pipe, AndAnd, OrOr, Semicolon, Ampersand at correct positions
}

#[test]
fn redirects() {
    let toks = tokens("echo hello > out.txt 2>&1");
    // Word, Word, Redirect(Output), Word, IoNumber(2), Redirect(DupOutput), Word, Eof
}

#[test]
fn io_number() {
    let toks = tokens("2> err.log");
    assert!(matches!(&toks[0], Token::IoNumber(2, _)));
}

#[test]
fn comments_skipped() {
    let toks = tokens("echo hello # this is a comment");
    // Only: Word(echo), Word(hello), Eof
    assert_eq!(toks.len(), 3);
}

#[test]
fn newlines_preserved() {
    let toks = tokens("echo\nls");
    // Word, Newline, Word, Eof
}

#[test]
fn spans_are_correct() {
    let input = "echo hello";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[0].span.text(input), "echo");
    assert_eq!(toks[1].span.text(input), "hello");
}

#[test]
fn double_parens() {
    let toks = tokens("(( x + 1 ))");
    assert!(matches!(&toks[0], Token::LParenParen));
}

#[test]
fn double_brackets() {
    let toks = tokens("[[ -f file ]]");
    assert!(matches!(&toks[0], Token::LBracketBracket));
}

#[test]
fn semicolon_semicolon() {
    let toks = tokens("pattern) body ;;");
    assert!(matches!(&toks[toks.len() - 2], Token::SemiSemi));
}
```

- [ ] **Step 5: Run tests**

Run: `cd orix/malt && cargo test -p mash --test lexer`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
cd orix/malt
git add crates/mash/src/lexer.rs crates/mash/tests/lexer.rs
git commit -m "feat(mash): implement lexer core — operators, words, comments, IoNumber, spans"
```

---

## Task 3: Lexer Quoting

**Files:**
- Modify: `orix/malt/crates/mash/src/lexer.rs`
- Modify: `orix/malt/crates/mash/tests/lexer.rs`

Add quoting support: single quotes, double quotes, ANSI-C quotes (`$'...'`), backslash escapes, line continuation.

**Reference:** `vexil-shell/src/lexer.rs` lines 828-886 (`read_quoted_segment`), 485-563 (`continue_word` quoting paths).

- [ ] **Step 1: Implement quoting in continue_word**

In `continue_word()`, add handling for:
- `'` → single quote: read until closing `'`, no escaping. Check for `$'` prefix (ANSI-C).
- `"` → double quote: read until closing `"`, with `\`, `$`, and backtick processing inside.
- `\` → backslash: if followed by newline, consume both (line continuation). Else, include both `\` and next char in word span.

**Key difference from reference:** Since we use spans (not owned strings), the word span just extends to cover the quoted content. The quotes are part of the span — the expander strips them later.

For double quotes with nested `$(...)` or `${...}`, the span still covers the full extent. The balanced tracking (Task 4) will handle depth counting.

- [ ] **Step 2: Implement backtick substitution**

Port `read_backtick_subst()` — read until closing backtick, handling `\` escapes inside. Extend the word span to cover the backtick content.

- [ ] **Step 3: Write quoting tests**

Add to `tests/lexer.rs`:

```rust
#[test]
fn single_quoted_word() {
    let input = "echo 'hello world'";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "'hello world'");
}

#[test]
fn double_quoted_word() {
    let input = r#"echo "hello world""#;
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "\"hello world\"");
}

#[test]
fn ansi_c_quote() {
    let input = "echo $'hello\\nworld'";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "$'hello\\nworld'");
}

#[test]
fn backslash_escape() {
    let input = "echo hello\\ world";
    let toks = tokens(input);
    // "hello\ world" is one word (escaped space)
    assert_eq!(toks.len(), 3); // Word, Word, Eof — wait, escaped space joins
}

#[test]
fn line_continuation() {
    let input = "echo hel\\\nlo";
    let toks = tokens(input);
    // "hel" + continuation + "lo" = one word "hello"
}

#[test]
fn unterminated_single_quote() {
    let input = "echo 'hello";
    let result: Vec<_> = Lexer::new(input).collect();
    assert!(result.last().unwrap().is_err());
}

#[test]
fn backtick_substitution() {
    let input = "echo `date`";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "`date`");
}

#[test]
fn nested_quotes_in_double() {
    let input = r#"echo "it's a test""#;
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "\"it's a test\"");
}
```

- [ ] **Step 4: Run tests**

Run: `cd orix/malt && cargo test -p mash --test lexer`

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/mash/src/lexer.rs crates/mash/tests/lexer.rs
git commit -m "feat(mash): lexer quoting — single, double, ANSI-C, backslash, backtick, line continuation"
```

---

## Task 4: Lexer Expansions

**Files:**
- Modify: `orix/malt/crates/mash/src/lexer.rs`
- Modify: `orix/malt/crates/mash/tests/lexer.rs`

Add balanced tracking for command substitution `$(...)`, arithmetic `$((...))`, parameter expansion `${...}`, and process substitution `<(...)` / `>(...)`.

**Reference:** `vexil-shell/src/lexer.rs` lines 642-767 (`read_balanced_parens`), 567-638 (`read_balanced_arith`), 771-794 (`read_balanced_braces`), 447-467 (`read_process_sub`).

- [ ] **Step 1: Implement balanced paren tracking for $()**

Port `read_balanced_parens()` — tracks depth, handles nested quotes and `case`/`esac` inside command substitution. The word span extends to cover the closing `)`.

**Critical:** Must track case statement depth to avoid matching pattern `)` as command subst close.

- [ ] **Step 2: Implement balanced arithmetic tracking for $(())**

Port `read_balanced_arith()` — distinguishes `$((...))` from `$(...)`. Handles nested `$(...)` and `${...}` inside arithmetic.

- [ ] **Step 3: Implement balanced brace tracking for ${}**

Port `read_balanced_braces()` — depth tracking for `${...}`. Handles nested quotes inside.

- [ ] **Step 4: Implement process substitution**

Port `read_process_sub()` — `<(...)` and `>(...)`. Tracked with balanced parens. Emitted as part of a `Word` span.

- [ ] **Step 5: Write expansion tests**

Add to `tests/lexer.rs`:

```rust
#[test]
fn command_substitution() {
    let input = "echo $(date)";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "$(date)");
}

#[test]
fn nested_command_substitution() {
    let input = "echo $(echo $(date))";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "$(echo $(date))");
}

#[test]
fn arithmetic_expansion() {
    let input = "echo $((1 + 2))";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "$((1 + 2))");
}

#[test]
fn parameter_expansion() {
    let input = "echo ${var:-default}";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "${var:-default}");
}

#[test]
fn process_substitution_input() {
    let input = "diff <(sort a) <(sort b)";
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "<(sort a)");
    assert_eq!(toks[2].span.text(input), "<(sort b)");
}

#[test]
fn command_subst_with_case_inside() {
    let input = "echo $(case x in a) echo a ;; esac)";
    let toks = tokens(input);
    // Should produce: Word(echo), Word($(...)), Eof
    // The case ) inside should NOT close the $()
    assert_eq!(toks.len(), 3);
}

#[test]
fn dollar_brace_in_double_quotes() {
    let input = r#"echo "${var}""#;
    let toks = tokens_with_spans(input);
    assert_eq!(toks[1].span.text(input), "\"${var}\"");
}
```

- [ ] **Step 6: Run tests and commit**

Run: `cd orix/malt && cargo test -p mash --test lexer`

```bash
cd orix/malt
git add crates/mash/src/lexer.rs crates/mash/tests/lexer.rs
git commit -m "feat(mash): lexer expansions — $(), $(()),  ${}, <(), >() with balanced tracking"
```

---

## Task 5: Lexer Heredocs

**Files:**
- Modify: `orix/malt/crates/mash/src/lexer.rs`
- Modify: `orix/malt/crates/mash/tests/lexer.rs`

Implement the heredoc state machine: `<<`/`<<-` detection → delimiter extraction → body accumulation after newline.

**Reference:** `vexil-shell/src/lexer.rs` lines 890-994 (heredoc resolution, delimiter extraction, body reading).

- [ ] **Step 1: Implement heredoc state tracking**

After emitting `Redirect(HereDoc)` or `Redirect(HereDocStrip)`, push to `pending_heredocs` with `strip_tabs` flag. The next `Word` token is the delimiter — extract via `extract_heredoc_delimiter()`.

- [ ] **Step 2: Implement heredoc body accumulation**

After emitting `Newline`, if `pending_heredocs` is non-empty, read lines until delimiter found. Emit `HereDocBody { body, quoted }`. This is the one token that allocates (tab stripping mutates text).

Port `read_heredoc_body()` and `extract_heredoc_delimiter()` from reference.

- [ ] **Step 3: Write heredoc tests**

```rust
#[test]
fn heredoc_basic() {
    let input = "cat <<EOF\nhello\nworld\nEOF\n";
    let toks = tokens(input);
    // Should contain HereDocBody with body "hello\nworld\n"
    let body = toks.iter().find_map(|t| match t {
        Token::HereDocBody { body, quoted } => Some((body.clone(), *quoted)),
        _ => None,
    });
    assert_eq!(body, Some(("hello\nworld\n".to_string(), false)));
}

#[test]
fn heredoc_strip_tabs() {
    let input = "cat <<-EOF\n\thello\n\tworld\nEOF\n";
    let toks = tokens(input);
    let body = toks.iter().find_map(|t| match t {
        Token::HereDocBody { body, .. } => Some(body.clone()),
        _ => None,
    });
    assert_eq!(body, Some("hello\nworld\n".to_string()));
}

#[test]
fn heredoc_quoted_delimiter() {
    let input = "cat <<'EOF'\nhello $var\nEOF\n";
    let toks = tokens(input);
    let body = toks.iter().find_map(|t| match t {
        Token::HereDocBody { body, quoted } => Some((body.clone(), *quoted)),
        _ => None,
    });
    assert_eq!(body, Some(("hello $var\n".to_string(), true)));
}

#[test]
fn heredoc_unterminated() {
    let input = "cat <<EOF\nhello\n";
    let result: Vec<_> = Lexer::new(input).collect();
    assert!(result.iter().any(|r| r.is_err()));
}
```

- [ ] **Step 4: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/lexer.rs crates/mash/tests/lexer.rs
git commit -m "feat(mash): lexer heredocs — <<, <<-, quoted delimiter, tab stripping, body accumulation"
```

---

## Task 6: Parser Core

**Files:**
- Modify: `orix/malt/crates/mash/src/parser.rs`
- Create: `orix/malt/crates/mash/tests/parser.rs`

Implement the parser infrastructure and simple command parsing: `parse()` entry point, `Parser` struct with peek/advance/expect, `parse_command_list`, `parse_and_or`, `parse_pipeline`, `parse_simple_command`.

**Reference:** `vexil-shell/src/parser.rs` lines 172-376 (entry, list, and_or, pipeline), 913-1050 (simple command), 1077-1147 (helpers).

- [ ] **Step 1: Implement Parser struct and infrastructure**

```rust
pub fn parse(input: &str) -> Result<Vec<Spanned<Command>>, ParseError> {
    let mut parser = Parser::new(input);
    parser.parse_command_list()
}

struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    current: Option<Spanned<Token>>,
}
```

Helper methods: `peek()`, `advance()`, `expect_word()`, `expect_keyword()`, `skip_newlines()`, `skip_terminators()`, `peek_keyword()`.

**Span propagation:** Each `parse_*` method returns `Spanned<Command>`. Spans are built by merging the first and last token spans of each production.

- [ ] **Step 2: Implement parse_command_list, parse_and_or, parse_pipeline**

Port the list/and-or/pipeline parsing. These handle `;`, `&`, `&&`, `||`, `|`, and `!` negation.

`parse_command_list_until()` takes a closure predicate for the end condition — used by compound commands to stop at `fi`, `done`, `esac`, etc.

- [ ] **Step 3: Implement parse_simple_command**

Port `parse_simple_command()` — handles words, arguments, environment assignments (`VAR=val`), redirects, IoNumber-prefixed redirects, and HereDocBody token integration.

Port `is_assignment()` and `split_assignment()` helpers. The assignment check uses `span.text(input)` instead of owned strings.

- [ ] **Step 4: Implement parse_command dispatch**

The dispatch method checks the current token and routes to compound command parsers (Task 7) or `parse_simple_command()`. For now, compound commands return `ParseError::SyntaxError` — they'll be implemented in Task 7.

Also implement function definition detection: lookahead for `name ( )`.

- [ ] **Step 5: Write parser tests**

Create `orix/malt/crates/mash/tests/parser.rs`:

```rust
use mash::ast::*;
use mash::parser::parse;

#[test]
fn simple_command() {
    let cmds = parse("echo hello world").unwrap();
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0].node, Command::Simple { .. }));
}

#[test]
fn pipeline() {
    let cmds = parse("cat file | grep foo | wc -l").unwrap();
    assert_eq!(cmds.len(), 1);
    match &cmds[0].node {
        Command::Pipeline { commands, negated } => {
            assert_eq!(commands.len(), 3);
            assert!(!negated);
        }
        _ => panic!("expected Pipeline"),
    }
}

#[test]
fn negated_pipeline() {
    let cmds = parse("! grep -q error log").unwrap();
    match &cmds[0].node {
        Command::Pipeline { negated, .. } => assert!(negated),
        _ => panic!("expected Pipeline"),
    }
}

#[test]
fn and_or_list() {
    let cmds = parse("cmd1 && cmd2 || cmd3").unwrap();
    assert!(matches!(&cmds[0].node, Command::List { .. }));
}

#[test]
fn sequential_list() {
    let cmds = parse("echo a; echo b; echo c").unwrap();
    assert_eq!(cmds.len(), 3);
}

#[test]
fn background_command() {
    let cmds = parse("sleep 10 &").unwrap();
    assert!(matches!(&cmds[0].node, Command::Background(_)));
}

#[test]
fn env_assign_only() {
    let cmds = parse("FOO=bar BAZ=qux").unwrap();
    assert!(matches!(&cmds[0].node, Command::EnvAssign { .. }));
}

#[test]
fn env_assign_with_command() {
    let input = "FOO=bar echo hello";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { env_assigns, .. } => assert_eq!(env_assigns.len(), 1),
        _ => panic!("expected Simple"),
    }
}

#[test]
fn redirect_output() {
    let cmds = parse("echo hello > out.txt").unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects.len(), 1);
            assert!(matches!(redirects[0].node.kind, RedirectKind::Output));
        }
        _ => panic!("expected Simple"),
    }
}

#[test]
fn redirect_with_fd() {
    let cmds = parse("cmd 2>&1").unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects[0].node.fd, Some(2));
            assert!(matches!(redirects[0].node.kind, RedirectKind::DupOutput));
        }
        _ => panic!("expected Simple"),
    }
}

#[test]
fn empty_input() {
    let cmds = parse("").unwrap();
    assert!(cmds.is_empty());
}

#[test]
fn spans_cover_full_command() {
    let input = "echo hello world";
    let cmds = parse(input).unwrap();
    let span = cmds[0].span;
    assert_eq!(span.text(input), "echo hello world");
}
```

- [ ] **Step 6: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/parser.rs crates/mash/tests/parser.rs
git commit -m "feat(mash): parser core — simple commands, pipelines, lists, redirects, env assigns"
```

---

## Task 7: Parser Compound Commands

**Files:**
- Modify: `orix/malt/crates/mash/src/parser.rs`
- Modify: `orix/malt/crates/mash/tests/parser.rs`

Implement all compound command parsers: if, while, until, for (word and arith), case, select, function def, brace group, subshell, arithmetic, conditional, coproc, time, and the `Redirected` wrapper for compound commands with trailing redirects.

**Reference:** `vexil-shell/src/parser.rs` lines 443-869 (all compound commands), 877-911 (trailing redirects).

- [ ] **Step 1: Implement if/elif/else/fi**

Port `parse_if()`. Uses `parse_command_list_until()` with keyword predicates. Merge spans from `if` token to `fi` token.

- [ ] **Step 2: Implement while/until/for/select**

Port `parse_while()`, `parse_until()`, `parse_for()`, `parse_for_arith()`, `parse_select()`. For C-style `for (( ))`, collect tokens between `((` and `))`, split on `;`.

- [ ] **Step 3: Implement case/esac**

Port `parse_case()`. Handle multiple patterns per item (`pat1 | pat2`), optional leading `(`, body until `;;` or `esac`.

- [ ] **Step 4: Implement function def, brace group, subshell**

Port `parse_function_keyword()` (keyword form), and the `name () { body }` detection from `parse_command()`. Port `parse_brace_group()` and `parse_subshell()`.

- [ ] **Step 5: Implement arithmetic, conditional, coproc, time**

Port `parse_arithmetic()` (`(( expr ))`), `parse_conditional()` (`[[ expr ]]`), `parse_coproc()`, `parse_time()`.

- [ ] **Step 6: Implement trailing redirects for compound commands**

Port `collect_trailing_redirects()`. After any compound command parse, check for trailing `>`, `>>`, `2>&1`, etc. and wrap in `Command::Redirected`.

- [ ] **Step 7: Write compound command tests**

Add to `tests/parser.rs`:

```rust
#[test]
fn if_then_fi() {
    let cmds = parse("if true; then echo yes; fi").unwrap();
    assert!(matches!(&cmds[0].node, Command::If { .. }));
}

#[test]
fn if_elif_else_fi() {
    let cmds = parse("if false; then echo no; elif true; then echo maybe; else echo yes; fi").unwrap();
    match &cmds[0].node {
        Command::If { elif_clauses, else_body, .. } => {
            assert_eq!(elif_clauses.len(), 1);
            assert!(else_body.is_some());
        }
        _ => panic!("expected If"),
    }
}

#[test]
fn while_loop() {
    let cmds = parse("while true; do echo loop; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::While { .. }));
}

#[test]
fn for_loop_with_words() {
    let cmds = parse("for x in a b c; do echo $x; done").unwrap();
    match &cmds[0].node {
        Command::For { words, .. } => assert_eq!(words.len(), 3),
        _ => panic!("expected For"),
    }
}

#[test]
fn for_arith() {
    let cmds = parse("for (( i=0; i<10; i++ )); do echo $i; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::ForArith { .. }));
}

#[test]
fn case_statement() {
    let cmds = parse("case $x in a) echo a ;; b|c) echo bc ;; esac").unwrap();
    match &cmds[0].node {
        Command::Case { items, .. } => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[1].patterns.len(), 2); // b|c
        }
        _ => panic!("expected Case"),
    }
}

#[test]
fn function_def_posix() {
    let cmds = parse("greet() { echo hello; }").unwrap();
    assert!(matches!(&cmds[0].node, Command::FunctionDef { .. }));
}

#[test]
fn function_def_keyword() {
    let cmds = parse("function greet { echo hello; }").unwrap();
    assert!(matches!(&cmds[0].node, Command::FunctionDef { .. }));
}

#[test]
fn brace_group() {
    let cmds = parse("{ echo a; echo b; }").unwrap();
    assert!(matches!(&cmds[0].node, Command::BraceGroup { .. }));
}

#[test]
fn subshell() {
    let cmds = parse("(echo a; echo b)").unwrap();
    assert!(matches!(&cmds[0].node, Command::Subshell { .. }));
}

#[test]
fn arithmetic_command() {
    let cmds = parse("(( x + 1 ))").unwrap();
    assert!(matches!(&cmds[0].node, Command::Arithmetic { .. }));
}

#[test]
fn conditional_command() {
    let cmds = parse("[[ -f file ]]").unwrap();
    assert!(matches!(&cmds[0].node, Command::Conditional { .. }));
}

#[test]
fn redirected_compound() {
    let cmds = parse("if true; then echo yes; fi > out.txt").unwrap();
    assert!(matches!(&cmds[0].node, Command::Redirected { .. }));
}

#[test]
fn nested_if_in_while() {
    let cmds = parse("while true; do if x; then echo y; fi; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::While { .. }));
}

#[test]
fn coproc_named() {
    let cmds = parse("coproc myproc { cat; }").unwrap();
    match &cmds[0].node {
        Command::Coproc { name, .. } => assert!(name.is_some()),
        _ => panic!("expected Coproc"),
    }
}

#[test]
fn time_pipeline() {
    let cmds = parse("time -p ls -la | wc -l").unwrap();
    match &cmds[0].node {
        Command::Time { posix_format, .. } => assert!(posix_format),
        _ => panic!("expected Time"),
    }
}
```

- [ ] **Step 8: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/parser.rs crates/mash/tests/parser.rs
git commit -m "feat(mash): parser compound commands — if, while, for, case, function, subshell, arithmetic, coproc, time"
```

---

## Task 8: Integration and Regression Tests

**Files:**
- Modify: `orix/malt/crates/mash/tests/parser.rs`

Port parser-level regression tests from `vexil-shell/tests/posix_regression.rs` that only test parse success (no execution). These verify the parser accepts all POSIX constructs the reference accepted.

- [ ] **Step 1: Read reference regression tests**

Read `C:\Users\mamuk\projects\vexil-v2\vexil-shell\tests\posix_regression.rs`. Identify all tests that only call `parse()` and assert success (no `execute_list`, no `Env`). Port these.

- [ ] **Step 2: Write parse-only regression tests**

Add to `tests/parser.rs` a `mod regression` section with ported tests. Each test calls `parse(input).unwrap()` for inputs that must parse successfully.

Include complex real-world patterns:
- Nested command substitution in assignments
- Heredocs with variable expansion
- Multi-line pipelines
- Case statements with complex patterns
- Function definitions with redirects
- Arithmetic for loops
- Conditional expressions with regex

- [ ] **Step 3: Run full test suite**

Run: `cd orix/malt && cargo test -p mash`
Expected: All lexer and parser tests pass.

- [ ] **Step 4: Run workspace tests**

Run: `cd orix/malt && cargo test --workspace`
Expected: All tests across all crates pass. No regressions.

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/mash/tests/
git commit -m "test(mash): add parse-level regression tests ported from vexil-shell conformance suite"
```

---

## Verification

After all tasks complete:

1. `cargo test -p mash` — all lexer and parser tests pass
2. `cargo test --workspace` — 64+ existing tests still pass, new mash tests pass
3. `cargo clippy -p mash -- -D warnings` — clean
4. Every Token variant is covered by at least one lexer test
5. Every Command variant is covered by at least one parser test
6. Span assertions verify zero-copy: `span.text(input)` matches expected source text
7. Error tests verify unterminated quotes, heredocs, and unexpected tokens produce correct error variants with correct positions
