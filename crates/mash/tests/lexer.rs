//! Integration tests for the MASH lexer core.

use mash::ast::{LexError, RedirectKind, Span, Spanned, Token};
use mash::lexer::{tokenize, Lexer};

/// Helper: collect just the token nodes from input.
fn nodes(input: &str) -> Vec<Token> {
    tokenize(input)
        .expect("lexer should not error")
        .into_iter()
        .map(|s| s.node)
        .collect()
}

/// Helper: collect spanned tokens.
fn spanned(input: &str) -> Vec<Spanned<Token>> {
    tokenize(input).expect("lexer should not error")
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

#[test]
fn simple_words() {
    let input = "echo hello world";
    let toks = spanned(input);
    assert_eq!(toks.len(), 4); // 3 words + Eof
    assert_eq!(toks[0].span.text(input), "echo");
    assert_eq!(toks[1].span.text(input), "hello");
    assert_eq!(toks[2].span.text(input), "world");
    assert!(matches!(toks[3].node, Token::Eof));
}

#[test]
fn empty_input() {
    let toks = nodes("");
    assert_eq!(toks, vec![Token::Eof]);
}

#[test]
fn whitespace_only() {
    let toks = nodes("   \t  ");
    assert_eq!(toks, vec![Token::Eof]);
}

#[test]
fn single_word() {
    let input = "foo";
    let toks = spanned(input);
    assert_eq!(toks.len(), 2);
    assert_eq!(toks[0].span.text(input), "foo");
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

#[test]
fn pipe() {
    assert_eq!(
        nodes("a | b"),
        vec![word_span(0, 1), Token::Pipe, word_span(4, 5), Token::Eof]
    );
}

#[test]
fn or_or() {
    assert_eq!(
        nodes("a || b"),
        vec![word_span(0, 1), Token::OrOr, word_span(5, 6), Token::Eof]
    );
}

#[test]
fn and_and() {
    assert_eq!(
        nodes("a && b"),
        vec![word_span(0, 1), Token::AndAnd, word_span(5, 6), Token::Eof]
    );
}

#[test]
fn semicolon() {
    assert_eq!(
        nodes("a ; b"),
        vec![word_span(0, 1), Token::Semicolon, word_span(4, 5), Token::Eof]
    );
}

#[test]
fn semi_semi() {
    assert_eq!(
        nodes("a ;; b"),
        vec![word_span(0, 1), Token::SemiSemi, word_span(5, 6), Token::Eof]
    );
}

#[test]
fn ampersand() {
    assert_eq!(
        nodes("a &"),
        vec![word_span(0, 1), Token::Ampersand, Token::Eof]
    );
}

#[test]
fn lparen_rparen() {
    assert_eq!(
        nodes("( a )"),
        vec![Token::LParen, word_span(2, 3), Token::RParen, Token::Eof]
    );
}

#[test]
fn lparen_paren() {
    assert_eq!(
        nodes("(( x ))"),
        vec![Token::LParenParen, word_span(3, 4), Token::RParenParen, Token::Eof]
    );
}

#[test]
fn lbrace_rbrace() {
    assert_eq!(
        nodes("{ a }"),
        vec![Token::LBrace, word_span(2, 3), Token::RBrace, Token::Eof]
    );
}

#[test]
fn double_bracket() {
    assert_eq!(
        nodes("[[ x ]]"),
        vec![Token::LBracketBracket, word_span(3, 4), Token::RBracketBracket, Token::Eof]
    );
}

// ---------------------------------------------------------------------------
// Redirects
// ---------------------------------------------------------------------------

#[test]
fn redirect_input() {
    let toks = nodes("< file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::Input), word_span(2, 6), Token::Eof]);
}

#[test]
fn redirect_output() {
    let toks = nodes("> file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::Output), word_span(2, 6), Token::Eof]);
}

#[test]
fn redirect_append() {
    let toks = nodes(">> file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::Append), word_span(3, 7), Token::Eof]);
}

#[test]
fn redirect_clobber() {
    let toks = nodes(">| file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::Clobber), word_span(3, 7), Token::Eof]);
}

#[test]
fn redirect_input_output() {
    let toks = nodes("<> file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::InputOutput), word_span(3, 7), Token::Eof]);
}

#[test]
fn redirect_dup_input() {
    let toks = nodes("<& 0");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::DupInput), word_span(3, 4), Token::Eof]);
}

#[test]
fn redirect_dup_output() {
    let toks = nodes(">& 1");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::DupOutput), word_span(3, 4), Token::Eof]);
}

#[test]
fn redirect_both() {
    let toks = nodes("&> file");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::Both), word_span(3, 7), Token::Eof]);
}

#[test]
fn redirect_here_string() {
    let toks = nodes("<<< word");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::HereString), word_span(4, 8), Token::Eof]);
}

#[test]
fn redirect_heredoc() {
    let toks = nodes("<< EOF");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::HereDoc), word_span(3, 6), Token::Eof]);
}

#[test]
fn redirect_heredoc_strip() {
    let toks = nodes("<<- EOF");
    assert_eq!(toks, vec![Token::Redirect(RedirectKind::HereDocStrip), word_span(4, 7), Token::Eof]);
}

// ---------------------------------------------------------------------------
// IoNumber
// ---------------------------------------------------------------------------

#[test]
fn io_number_output() {
    let input = "2> err.log";
    let toks = spanned(input);
    assert_eq!(toks.len(), 4); // IoNumber(2), Redirect(Output), Word("err.log"), Eof
    assert!(matches!(toks[0].node, Token::IoNumber(2, _)));
    assert_eq!(toks[0].span.text(input), "2");
    assert_eq!(toks[1].node, Token::Redirect(RedirectKind::Output));
    assert_eq!(toks[2].span.text(input), "err.log");
    assert_eq!(toks[3].node, Token::Eof);
}

#[test]
fn io_number_dup() {
    let input = "2>&1";
    let toks = spanned(input);
    assert!(matches!(toks[0].node, Token::IoNumber(2, _)));
    assert_eq!(toks[1].node, Token::Redirect(RedirectKind::DupOutput));
    assert_eq!(toks[2].span.text(input), "1");
}

#[test]
fn digits_not_before_redirect_is_word() {
    let input = "42 hello";
    let toks = spanned(input);
    // 42 is followed by space, not redirect -- should be a word
    assert!(matches!(toks[0].node, Token::Word(_)));
    assert_eq!(toks[0].span.text(input), "42");
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn comment_skipped() {
    let input = "echo hello # comment";
    let toks = spanned(input);
    // echo, hello, Eof (comment is skipped entirely)
    assert_eq!(toks.len(), 3);
    assert_eq!(toks[0].span.text(input), "echo");
    assert_eq!(toks[1].span.text(input), "hello");
    assert_eq!(toks[2].node, Token::Eof);
}

#[test]
fn comment_at_start() {
    let toks = nodes("# just a comment");
    assert_eq!(toks, vec![Token::Eof]);
}

#[test]
fn comment_before_newline() {
    let input = "echo # comment\nls";
    let toks = spanned(input);
    // echo, Newline (from after comment), ls, Eof
    // Actually comment skips to EOL, then \n is consumed as next token
    assert_eq!(toks[0].span.text(input), "echo");
    assert!(matches!(toks[1].node, Token::Newline));
    assert_eq!(toks[2].span.text(input), "ls");
    assert_eq!(toks[3].node, Token::Eof);
}

// ---------------------------------------------------------------------------
// Newlines
// ---------------------------------------------------------------------------

#[test]
fn newlines() {
    let input = "echo\nls";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "echo");
    assert_eq!(toks[1].node, Token::Newline);
    assert_eq!(toks[2].span.text(input), "ls");
    assert_eq!(toks[3].node, Token::Eof);
}

#[test]
fn crlf_newline() {
    let input = "echo\r\nls";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "echo");
    assert_eq!(toks[1].node, Token::Newline);
    // CRLF span should cover 2 bytes
    assert_eq!(toks[1].span, Span::new(4, 6));
    assert_eq!(toks[2].span.text(input), "ls");
    assert_eq!(toks[3].node, Token::Eof);
}

#[test]
fn bare_cr_is_newline() {
    let input = "a\rb";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "a");
    assert_eq!(toks[1].node, Token::Newline);
    assert_eq!(toks[2].span.text(input), "b");
}

// ---------------------------------------------------------------------------
// Span correctness
// ---------------------------------------------------------------------------

#[test]
fn span_offsets_correct() {
    let input = "echo  hello";
    let toks = spanned(input);
    assert_eq!(toks[0].span, Span::new(0, 4));
    assert_eq!(toks[1].span, Span::new(6, 11));
}

#[test]
fn operator_spans() {
    let input = "a || b";
    let toks = spanned(input);
    // `||` at position 2..4
    assert_eq!(toks[1].span, Span::new(2, 4));
    assert_eq!(toks[1].span.text(input), "||");
}

#[test]
fn redirect_span() {
    let input = ">> file";
    let toks = spanned(input);
    assert_eq!(toks[0].span, Span::new(0, 2));
    assert_eq!(toks[0].span.text(input), ">>");
}

// ---------------------------------------------------------------------------
// Words with special chars (included as-is for now)
// ---------------------------------------------------------------------------

#[test]
fn word_with_equals() {
    let input = "FOO=bar";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "FOO=bar");
}

#[test]
fn word_with_dollar() {
    let input = "$HOME";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "$HOME");
}

#[test]
fn word_with_quotes_included() {
    // Quotes are not yet handled specially -- they're part of the word
    let input = "\"hello\"";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "\"hello\"");
}

// ---------------------------------------------------------------------------
// Iterator interface
// ---------------------------------------------------------------------------

#[test]
fn iterator_yields_none_after_eof() {
    let mut lexer = Lexer::new("a");
    let first = lexer.next().unwrap().unwrap();
    assert!(matches!(first.node, Token::Word(_)));
    let second = lexer.next().unwrap().unwrap();
    assert_eq!(second.node, Token::Eof);
    // After Eof, iterator yields None
    assert!(lexer.next().is_none());
}

// ---------------------------------------------------------------------------
// Mixed expressions
// ---------------------------------------------------------------------------

#[test]
fn pipeline() {
    let input = "cat file | grep pattern | wc -l";
    let toks = spanned(input);
    let texts: Vec<_> = toks.iter().map(|t| match &t.node {
        Token::Word(s) => s.text(input).to_string(),
        Token::Pipe => "|".to_string(),
        Token::Eof => "EOF".to_string(),
        other => format!("{:?}", other),
    }).collect();
    assert_eq!(texts, vec!["cat", "file", "|", "grep", "pattern", "|", "wc", "-l", "EOF"]);
}

#[test]
fn complex_redirect() {
    let input = "cmd 2>&1 >output.log";
    let toks = spanned(input);
    // cmd, IoNumber(2), Redirect(DupOutput), Word(1), Redirect(Output), Word(output.log), Eof
    assert_eq!(toks[0].span.text(input), "cmd");
    assert!(matches!(toks[1].node, Token::IoNumber(2, _)));
    assert_eq!(toks[2].node, Token::Redirect(RedirectKind::DupOutput));
    assert_eq!(toks[3].span.text(input), "1");
    assert_eq!(toks[4].node, Token::Redirect(RedirectKind::Output));
    assert_eq!(toks[5].span.text(input), "output.log");
    assert_eq!(toks[6].node, Token::Eof);
}

#[test]
fn bracket_as_word() {
    // Single `[` should be a word (test command)
    let input = "[ -f foo ]";
    let toks = spanned(input);
    assert_eq!(toks[0].span.text(input), "[");
    assert_eq!(toks[1].span.text(input), "-f");
    assert_eq!(toks[2].span.text(input), "foo");
    assert_eq!(toks[3].span.text(input), "]");
    assert_eq!(toks[4].node, Token::Eof);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a Word token with a specific span for comparison.
fn word_span(start: u32, end: u32) -> Token {
    Token::Word(Span::new(start, end))
}
