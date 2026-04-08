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
fn invalid_ampersand_less_redirect_is_rejected() {
    assert!(
        parse("exec 9&<-").is_err(),
        "parser accepted invalid '&<' redirect form"
    );
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
            assert_eq!(redirects[0].node.heredoc_body.as_deref(), Some("hello\n"));
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn heredoc_and_output_redirect_stay_on_same_command() {
    let input = "cat >scr <<EOF\nhello\nEOF\n";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple {
            name,
            args,
            redirects,
            ..
        } => {
            assert_eq!(name.text(input), "cat");
            assert!(args.is_empty());
            assert_eq!(redirects.len(), 2);
            assert!(matches!(redirects[0].node.kind, RedirectKind::Output));
            assert_eq!(redirects[0].node.target.text(input), "scr");
            assert!(matches!(redirects[1].node.kind, RedirectKind::HereDoc));
            assert_eq!(redirects[1].node.heredoc_body.as_deref(), Some("hello\n"));
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn heredoc_before_output_redirect_stays_on_same_command() {
    let input = "cat <<EOF >scr\nhello\nEOF\n";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple {
            name,
            args,
            redirects,
            ..
        } => {
            assert_eq!(name.text(input), "cat");
            assert!(args.is_empty());
            assert_eq!(redirects.len(), 2);
            assert!(matches!(redirects[0].node.kind, RedirectKind::HereDoc));
            assert_eq!(redirects[0].node.heredoc_body.as_deref(), Some("hello\n"));
            assert!(matches!(redirects[1].node.kind, RedirectKind::Output));
            assert_eq!(redirects[1].node.target.text(input), "scr");
        }
        other => panic!("expected Simple, got {:?}", other),
    }
}

#[test]
fn heredoc_script_preserves_following_command() {
    let input = "cat >scr <<EOF\nhello\nEOF\ncat scr\n";
    let cmds = parse(input).unwrap();
    assert_eq!(cmds.len(), 2);
    match &cmds[1].node {
        Command::Simple { name, args, .. } => {
            assert_eq!(name.text(input), "cat");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0].text(input), "scr");
        }
        other => panic!("expected second Simple, got {:?}", other),
    }
}

#[test]
fn adjacent_braces_can_be_simple_command_arguments() {
    let input = "find . -exec echo {} +";
    let cmds = parse(input).unwrap();
    match &cmds[0].node {
        Command::Simple { name, args, .. } => {
            assert_eq!(name.text(input), "find");
            assert_eq!(args.len(), 5);
            assert_eq!(args[0].text(input), ".");
            assert_eq!(args[1].text(input), "-exec");
            assert_eq!(args[2].text(input), "echo");
            assert_eq!(args[3].text(input), "{}");
            assert_eq!(args[4].text(input), "+");
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

// ---------------------------------------------------------------------------
// Regression tests — complex POSIX constructs ported from vexil-shell suite
// ---------------------------------------------------------------------------
//
// The vexil-shell posix_regression.rs tests are all execution tests (they call
// execute_list + Env).  These regression tests are parse-only: every case below
// must parse without error.  If a test panics here the parser has a bug.

mod regression {
    use super::*;

    // Complex real-world patterns that must parse successfully

    #[test]
    fn nested_command_substitution_in_assignment() {
        parse("x=$(echo $(date +%Y))").unwrap();
    }

    #[test]
    fn heredoc_in_pipeline() {
        parse("cat <<EOF | grep foo\nhello foo\nbar\nEOF\n").unwrap();
    }

    #[test]
    fn multiline_pipeline() {
        parse("echo hello |\n  grep h |\n  wc -l").unwrap();
    }

    #[test]
    fn case_with_complex_patterns() {
        parse("case $1 in\n  -h|--help) echo help ;;\n  -v|--version) echo 1.0 ;;\n  *) echo unknown ;;\nesac").unwrap();
    }

    #[test]
    fn function_with_redirects() {
        parse("log() { echo \"$@\" >> /tmp/log; }").unwrap();
    }

    #[test]
    fn arithmetic_for_with_nested_arith() {
        parse("for (( i=0; i < $((n*2)); i++ )); do echo $i; done").unwrap();
    }

    #[test]
    fn conditional_with_regex() {
        parse("[[ $x =~ ^[0-9]+$ ]]").unwrap();
    }

    #[test]
    fn subshell_with_env() {
        parse("(export FOO=bar; echo $FOO)").unwrap();
    }

    #[test]
    fn complex_redirect_chain() {
        parse("cmd 2>&1 1>/dev/null").unwrap();
    }

    #[test]
    fn nested_if_for_case() {
        parse("if true; then\n  for x in a b c; do\n    case $x in\n      a) echo first ;;\n      *) echo other ;;\n    esac\n  done\nfi").unwrap();
    }

    #[test]
    fn while_with_redirect_on_done() {
        parse("while read line; do echo $line; done < input.txt").unwrap();
    }

    #[test]
    fn brace_group_in_pipeline() {
        parse("{ echo a; echo b; } | grep a").unwrap();
    }

    #[test]
    fn multiple_heredocs_with_process_sub() {
        parse("diff <(cat <<A\nfoo\nA\n) <(cat <<B\nbar\nB\n)").unwrap();
    }

    #[test]
    fn env_assign_array() {
        parse("arr=(one two three)").unwrap();
    }

    #[test]
    fn function_def_with_local() {
        parse("f() { local x=1; echo $x; }").unwrap();
    }

    #[test]
    fn coproc_with_brace_group() {
        parse("coproc myproc { read line; echo got: $line; }").unwrap();
    }

    #[test]
    fn time_complex_brace_group() {
        parse("time -p { cat largefile | sort | uniq -c | sort -rn | head; }").unwrap();
    }

    #[test]
    fn nested_double_quotes_with_expansion() {
        parse(r#"echo "hello $(echo "world")""#).unwrap();
    }

    #[test]
    fn here_string() {
        parse("grep foo <<< 'hello foo bar'").unwrap();
    }

    #[test]
    fn complex_parameter_expansion() {
        parse("echo ${var:-${OTHER:-default}}").unwrap();
    }

    #[test]
    fn semicolon_separated_in_brace() {
        parse("{ cmd1; cmd2; cmd3; }").unwrap();
    }

    #[test]
    fn background_pipeline() {
        parse("cmd1 | cmd2 | cmd3 &").unwrap();
    }

    #[test]
    fn select_with_case_body() {
        parse("select opt in start stop restart; do\n  case $opt in\n    start) echo starting ;;\n    stop) echo stopping ;;\n    *) echo unknown ;;\n  esac\ndone").unwrap();
    }

    #[test]
    fn empty_case_arm_body() {
        parse("case $x in a) ;; b) echo b ;; esac").unwrap();
    }

    #[test]
    fn if_with_pipeline_condition() {
        parse("if cmd1 | cmd2; then echo yes; fi").unwrap();
    }

    #[test]
    fn for_without_in_newline_do() {
        parse("for x\ndo\n  echo $x\ndone").unwrap();
    }

    #[test]
    fn consecutive_redirects() {
        parse("cmd >file1 2>file2 3>&1").unwrap();
    }

    // ----- Additional POSIX edge cases -----

    #[test]
    fn heredoc_strip_tabs() {
        parse("cat <<-EOF\n\thello\n\tEOF\n").unwrap();
    }

    #[test]
    fn heredoc_quoted_delimiter_no_expansion() {
        parse("cat <<'NOEXP'\n$HOME\nNOEXP\n").unwrap();
    }

    #[test]
    fn double_negated_pipeline() {
        // Two separate negated pipelines separated by semicolon
        parse("! grep foo f; ! grep bar f").unwrap();
    }

    #[test]
    fn function_def_keyword_form_with_parens() {
        parse("function greet() { echo hello; }").unwrap();
    }

    #[test]
    fn and_or_chain_three_ops() {
        parse("a && b || c && d").unwrap();
    }

    #[test]
    fn redirect_input_output() {
        parse("cmd <>file").unwrap();
    }

    #[test]
    fn redirect_clobber() {
        parse("cmd >|file").unwrap();
    }

    #[test]
    fn redirect_append() {
        parse("cmd >> log.txt").unwrap();
    }

    #[test]
    fn deeply_nested_subshells() {
        parse("(( (echo inner) ))").unwrap();
    }

    #[test]
    fn for_arith_empty_clauses() {
        parse("for (( ; ; )); do break; done").unwrap();
    }

    #[test]
    fn case_first_pattern_with_open_paren() {
        parse("case $x in (a) echo a ;; (b) echo b ;; esac").unwrap();
    }

    #[test]
    fn while_pipeline_condition() {
        parse("while read -r line | grep foo; do echo $line; done").unwrap();
    }

    #[test]
    fn until_compound_condition() {
        parse("until [ $i -ge 10 ]; do i=$((i+1)); done").unwrap();
    }

    #[test]
    fn pipeline_with_negation_in_list() {
        parse("! cmd1 | cmd2 && cmd3").unwrap();
    }

    #[test]
    fn coproc_unnamed_simple() {
        parse("coproc sort").unwrap();
    }

    #[test]
    fn time_pipeline() {
        parse("time cat file | sort | uniq").unwrap();
    }

    #[test]
    fn multiline_function_body() {
        parse("setup() {\n  mkdir -p /tmp/work\n  cd /tmp/work\n  export WORK=/tmp/work\n}")
            .unwrap();
    }

    #[test]
    fn nested_arithmetic_in_condition() {
        parse("[[ $((a+b)) -gt $((c*d)) ]]").unwrap();
    }

    #[test]
    fn here_string_double_quoted() {
        parse(r#"cat <<< "hello world""#).unwrap();
    }

    #[test]
    fn redirected_while_loop() {
        parse("while IFS= read -r line; do echo \"$line\"; done < file.txt > out.txt").unwrap();
    }

    #[test]
    fn subshell_background() {
        parse("(long_running_cmd arg1 arg2) &").unwrap();
    }

    #[test]
    fn brace_group_with_redirects() {
        parse("{ echo header; cat body.txt; echo footer; } > result.txt").unwrap();
    }

    #[test]
    fn escaped_open_paren_word_parses() {
        parse("printf '%s' \\(").unwrap();
    }

    #[test]
    fn case_glob_patterns() {
        parse("case $file in\n  *.tar.gz) echo tgz ;;\n  *.zip)    echo zip ;;\n  *)        echo other ;;\nesac").unwrap();
    }

    #[test]
    fn nested_param_expansion_assign() {
        parse("echo ${X:=${Y:=default}}").unwrap();
    }

    #[test]
    fn multiple_env_assigns_before_command() {
        parse("A=1 B=2 C=3 cmd arg").unwrap();
    }

    #[test]
    fn process_substitution_in_command() {
        parse("diff <(sort file1) <(sort file2)").unwrap();
    }

    #[test]
    fn output_process_substitution() {
        parse("tee >(grep error > errors.log) < input").unwrap();
    }

    #[test]
    fn function_body_followed_by_plus_parameter_expansion_parses() {
        parse("f() { echo $# ; }\nunset -v nonesuch\nf ${nonesuch+nonempty} a b\nx=foo\nf ${x+hi} a b\n")
            .unwrap();
    }

    #[test]
    fn command_substitution_with_unset_parameter_word_elision_parses() {
        parse("count() { echo $#; }\n[ $(count a $nonesuch b) -eq 2 ]\n").unwrap();
    }
}
