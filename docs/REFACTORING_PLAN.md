# Lexer/Parser Refactoring Plan: Streaming → Pre-tokenized

## Goal
Replace streaming Iterator-based lexer/parser with pre-tokenized Vec<Token> approach (like vexil-shell) to fix timing-sensitive infinite loops and simplify code.

## Status
- Current: Streaming lexer with `buffered_tokens`, `pending_heredocs`, `awaiting_heredoc_count`, `finished` flags
- Target: Pre-tokenized `Vec<Spanned<Token>>` with simple index-based parser
- Reference: `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\lexer.rs` and `parser.rs`

## Why This Fix Is Correct

### Root Cause Analysis
The infinite loop in `parse_for` (and likely elsewhere) is caused by:
1. **Streaming state machine complexity** - Lexer maintains internal state that parser mutates via `peek()`
2. **Heredoc two-phase confusion** - Buffered tokens, pending heredocs, and finished flags interact in complex ways
3. **Fallible peek** - Each `peek()` can trigger re-lexing, which can fail or change state unexpectedly

### Why Pre-tokenizing Fixes It
1. **Deterministic** - One pass: input → Vec<Token>. No mutation during parsing.
2. **Simple indexing** - Parser just does `tokens[pos]` and `pos += 1`. No state machines.
3. **No heredoc interleaving** - Heredoc bodies are collected during tokenization, not during parsing.

### We Keep Zero-Copy
Both approaches use `Spanned<Token>` with spans referencing the original input. The only difference is when tokenization happens (streaming vs upfront).

## Refactoring Steps

### Phase 1: Lexer Changes (crates/mash/src/lexer.rs)

#### Current State
```rust
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    heredoc_queue: VecDeque<HeredocSpec>,
    pending_heredocs: Vec<PendingHeredoc>,
    buffered_tokens: Vec<Spanned<Token<'a>>>,
    awaiting_heredoc_count: usize,
    finished: bool,
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token<'a>>, LexerError>;
    // Complex next() method with heredoc handling
}
```

#### Target State
```rust
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    // Simplified: just track position during tokenization
}

impl<'a> Lexer<'a> {
    /// New: Tokenize entire input upfront
    pub fn tokenize(input: &'a str) -> Result<Vec<Spanned<Token<'a>>>, LexerError> {
        let mut lexer = Self::new(input);
        let mut tokens = Vec::new();
        
        loop {
            match lexer.next_token()? {
                Some(token) => tokens.push(token),
                None => break,
            }
        }
        
        Ok(tokens)
    }
    
    fn next_token(&mut self) -> Result<Option<Spanned<Token<'a>>>, LexerError> {
        // Simplified: just produce next token, no heredoc queue management
    }
}
```

#### Key Changes
1. Remove `heredoc_queue`, `pending_heredocs`, `buffered_tokens`, `awaiting_heredoc_count`, `finished`
2. Replace `Iterator::next()` with `Lexer::tokenize()` that produces `Vec<Token>`
3. Heredoc handling: When we see `<<` or `<<-`, immediately read and tokenize the heredoc body
4. Remove all `Peekable` wrapper usage from parser

### Phase 2: Parser Changes (crates/mash/src/parser.rs)

#### Current State
```rust
pub struct Parser<'a, I: Iterator<Item = LexerResult<Token<'a>>>> {
    lexer: Peekable<I>,
    current: Option<Spanned<Token<'a>>>,
    // Error recovery state...
}

impl<'a, I: Iterator<Item = LexerResult<Token<'a>>>> Parser<'a, I> {
    fn peek(&mut self) -> Result<&Spanned<Token<'a>>, ParseError> {
        // Complex: may trigger lexer state changes
    }
}
```

#### Target State
```rust
pub struct Parser<'a> {
    tokens: Vec<Spanned<Token<'a>>>,
    pos: usize,
    // Error recovery state...
}

impl<'a> Parser<'a> {
    pub fn new(tokens: Vec<Spanned<Token<'a>>>) -> Self {
        Self { tokens, pos: 0 }
    }
    
    fn peek(&self) -> Option<&Spanned<Token<'a>>> {
        self.tokens.get(self.pos)
    }
    
    fn advance(&mut self) {
        self.pos += 1;
    }
    
    fn consume(&mut self) -> Option<Spanned<Token<'a>>> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }
}
```

#### Key Changes
1. Remove generic `I: Iterator`, use concrete `Vec<Spanned<Token>>`
2. Replace `Peekable<I>` with `Vec` and `usize` index
3. Make `peek()` infallible and non-mutating: `&self -> Option<&Token>`
4. Make `advance()` infallible: `&mut self`
5. Update all call sites: remove `?` from `peek()`, handle `None` as EOF

### Phase 3: Heredoc Handling Strategy

**Current (problematic):**
- Parser encounters `<<FOO`, lexer queues heredoc for later
- Parser continues, eventually reads newline
- Lexer magically switches mode and produces heredoc body tokens

**New (deterministic):**
- Lexer encounters `<<FOO`, immediately scans ahead for herdoc body
- Lexer produces: `[Word("<<"), Word("FOO"), Newline, HeredocBody("..."), Word("FOO")]`
- Parser sees complete sequence, no special handling needed

### Phase 4: API Changes

#### Entry Points
- `mash::parse(input: &str) -> Result<Program, ParseError>` - top-level
- `parse_command_list(input: &str)` - for command substitution
- `parse_word_list(input: &str)` - for array contexts

All should: `Lexer::tokenize(input)?` → `Parser::new(tokens).parse()`

### Phase 5: Test Migration

1. **Smoosh tests** (186 tests): Should pass unchanged - these test semantics, not implementation
2. **Unit tests**: Update any that relied on streaming behavior
3. **Modernish**: The target - must pass `--test -eqq`

## Checkpoints

| Checkpoint | Test | Commit Message |
|------------|------|----------------|
| 1 | Lexer compiles | `refactor(lexer): remove streaming state, add tokenize()` |
| 2 | Parser compiles | `refactor(parser): index-based peek/advance` |
| 3 | Smoosh 186/186 | `test: Smoosh passes with new lexer/parser` |
| 4 | local.mm parses | `fix: Modernish local.mm parses without hang` |
| 5 | Modernish --test | `feat: Modernish test suite passes` |

## Risk Mitigation

### If Refactoring Is Too Large
1. **Fallback**: Apply minimal fix to streaming parser (pattern-matching fix for `do` keyword)
2. **Alternative**: Add `eprintln!()` workaround as temporary unblocker

### If Tests Fail
1. Compare token-by-token with vexil-shell output
2. Check span positions (byte offsets should match)
3. Verify heredoc body inclusion

### Reference Implementation
- `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\lexer.rs`
- `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\parser.rs`
- These are proven working with Modernish

## Success Criteria

1. Smoosh: 186/186 tests pass (WSL), 183/183 (Windows)
2. Modernish: `timeout 10s mash -n local.mm` exits 0 (not 124)
3. Modernish: `mash modernish --test -eqq` passes
4. No timing-sensitive hangs on any input
5. Clean `git status` (no debug artifacts)

## Notes

- Keep spans as `Span { start: usize, end: usize }` - zero-copy is preserved
- REPL efficiency: Pre-tokenizing per-line is fine; lines are short
- Error messages: Should improve (deterministic token positions)
- Future: If we need streaming for huge scripts, we can chunk; but for now, simplicity wins
