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
