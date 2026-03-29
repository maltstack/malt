use mash::ast::*;
use mash::parser::parse;

#[test]
fn simple_command() {
    let cmds = parse("echo hello world").unwrap();
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0].node, Command::Simple { .. }));
}

#[test]
fn simple_command_name_and_args() {
    let input = "ls -la /tmp";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { name, args, .. } => {
            assert_eq!(name.text(input), "ls");
            assert_eq!(args.len(), 2);
            assert_eq!(args[0].text(input), "-la");
            assert_eq!(args[1].text(input), "/tmp");
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn pipeline_two_commands() {
    let cmds = parse("cat file | grep foo").unwrap();
    assert_eq!(cmds.len(), 1);
    match &cmds[0].node {
        Command::Pipeline { commands, negated } => {
            assert_eq!(commands.len(), 2);
            assert!(!negated);
        }
        other => panic!("expected Pipeline, got {:?}", other),
    }
}

#[test]
fn pipeline_three_commands() {
    let cmds = parse("cat file | grep foo | wc -l").unwrap();
    match &cmds[0].node {
        Command::Pipeline { commands, .. } => assert_eq!(commands.len(), 3),
        other => panic!("expected Pipeline, got {:?}", other),
    }
}

#[test]
fn negated_pipeline() {
    let cmds = parse("! grep -q error log").unwrap();
    match &cmds[0].node {
        Command::Pipeline { negated, .. } => assert!(negated),
        other => panic!("expected Pipeline, got {:?}", other),
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
        Command::Simple {
            env_assigns, name, ..
        } => {
            assert_eq!(env_assigns.len(), 1);
            assert_eq!(name.text(input), "echo");
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn redirect_output() {
    let input = "echo hello > out.txt";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects.len(), 1);
            assert!(matches!(redirects[0].node.kind, RedirectKind::Output));
            assert_eq!(redirects[0].node.target.text(input), "out.txt");
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn redirect_with_fd() {
    let input = "cmd 2>&1";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects[0].node.fd, Some(2));
            assert!(matches!(redirects[0].node.kind, RedirectKind::DupOutput));
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn empty_input() {
    let cmds = parse("").unwrap();
    assert!(cmds.is_empty());
}

#[test]
fn multiple_redirects() {
    let input = "cmd < in.txt > out.txt 2> err.txt";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects.len(), 3);
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn heredoc_in_simple_command() {
    let input = "cat <<EOF\nhello\nEOF\n";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { redirects, .. } => {
            assert_eq!(redirects.len(), 1);
            assert!(matches!(redirects[0].node.kind, RedirectKind::HereDoc));
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Compound command tests
// ---------------------------------------------------------------------------

#[test]
fn if_then_fi() {
    let cmds = parse("if true; then echo yes; fi").unwrap();
    assert!(matches!(&cmds[0].node, Command::If { .. }));
}

#[test]
fn if_else_fi() {
    let cmds = parse("if false; then echo no; else echo yes; fi").unwrap();
    match &cmds[0].node {
        Command::If { else_body, .. } => assert!(else_body.is_some()),
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn if_elif_else_fi() {
    let cmds = parse("if false; then echo 1; elif true; then echo 2; else echo 3; fi").unwrap();
    match &cmds[0].node {
        Command::If {
            elif_clauses,
            else_body,
            ..
        } => {
            assert_eq!(elif_clauses.len(), 1);
            assert!(else_body.is_some());
        }
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn while_loop() {
    let cmds = parse("while true; do echo loop; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::While { .. }));
}

#[test]
fn until_loop() {
    let cmds = parse("until false; do echo loop; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::Until { .. }));
}

#[test]
fn for_loop_with_words() {
    let input = "for x in a b c; do echo $x; done";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::For { var, words, .. } => {
            assert_eq!(var.text(input), "x");
            assert_eq!(words.len(), 3);
        }
        other => panic!("expected For, got {:?}", other),
    }
}

#[test]
fn for_loop_no_in() {
    let cmds = parse("for x; do echo $x; done").unwrap();
    match &cmds[0].node {
        Command::For { words, .. } => assert!(words.is_empty()),
        other => panic!("expected For, got {:?}", other),
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
            assert_eq!(items[1].patterns.len(), 2);
        }
        other => panic!("expected Case, got {:?}", other),
    }
}

#[test]
fn select_statement() {
    let cmds = parse("select x in a b c; do echo $x; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::Select { .. }));
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
fn coproc_unnamed() {
    let cmds = parse("coproc cat").unwrap();
    match &cmds[0].node {
        Command::Coproc { name, .. } => assert!(name.is_none()),
        other => panic!("expected Coproc, got {:?}", other),
    }
}

#[test]
fn time_command() {
    let cmds = parse("time ls -la").unwrap();
    match &cmds[0].node {
        Command::Time { posix_format, .. } => assert!(!posix_format),
        other => panic!("expected Time, got {:?}", other),
    }
}

#[test]
fn time_posix_format() {
    let cmds = parse("time -p ls").unwrap();
    match &cmds[0].node {
        Command::Time { posix_format, .. } => assert!(posix_format),
        other => panic!("expected Time, got {:?}", other),
    }
}

#[test]
fn redirected_if() {
    let cmds = parse("if true; then echo yes; fi > out.txt").unwrap();
    assert!(matches!(&cmds[0].node, Command::Redirected { .. }));
}

#[test]
fn nested_if_in_while() {
    let cmds = parse("while true; do if x; then echo y; fi; done").unwrap();
    assert!(matches!(&cmds[0].node, Command::While { .. }));
}

#[test]
fn nested_for_in_if() {
    let cmds = parse("if true; then for x in a b; do echo $x; done; fi").unwrap();
    assert!(matches!(&cmds[0].node, Command::If { .. }));
}
