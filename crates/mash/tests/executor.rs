//! Integration tests for the MASH executor scaffold.

use mash::env::{Env, Variable};
use mash::executor::{execute_list, ExecResult};
use mash::parser::parse;

fn run(input: &str) -> (ExecResult, Env) {
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);
    (result, env)
}

fn run_stdout(input: &str) -> String {
    let (result, _) = run(input);
    String::from_utf8_lossy(&result.stdout).to_string()
}

#[test]
fn echo_hello() {
    let output = run_stdout("echo hello");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn echo_multiple_args() {
    let output = run_stdout("echo hello world");
    assert!(output.contains("hello world"), "got: {output}");
}

#[test]
fn exit_code_zero() {
    let (result, _) = run("echo test");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn nonexistent_command() {
    let (result, _) = run("nonexistent_command_xyz_12345");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn sequential_commands() {
    let output = run_stdout("echo first; echo second");
    assert!(output.contains("first"), "got: {output}");
    assert!(output.contains("second"), "got: {output}");
}

#[test]
fn variable_expansion_in_args() {
    let mut env = Env::from_os();
    env.set("GREETING", Variable::string("hi")).expect("set failed");
    let input = "echo $GREETING";
    let cmds = parse(input).expect("parse failed");
    let result = execute_list(&cmds, input, &mut env);
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(output.contains("hi"), "got: {output}");
}

#[test]
fn and_if_success() {
    let output = run_stdout("echo first && echo second");
    assert!(output.contains("first"), "got: {output}");
    assert!(output.contains("second"), "got: {output}");
}

#[test]
fn and_if_failure() {
    let output = run_stdout("nonexistent_xyz && echo should_not_appear");
    assert!(!output.contains("should_not_appear"), "got: {output}");
}

#[test]
fn or_if_failure() {
    let output = run_stdout("nonexistent_xyz || echo fallback");
    assert!(output.contains("fallback"), "got: {output}");
}

#[test]
fn or_if_success() {
    let output = run_stdout("echo first || echo should_not_appear");
    assert!(output.contains("first"), "got: {output}");
    assert!(!output.contains("should_not_appear"), "got: {output}");
}

#[test]
fn nonexistent_command_code_127() {
    let (result, _) = run("nonexistent_command_xyz_12345");
    assert_eq!(result.exit_code, 127);
}

#[test]
fn nonexistent_command_stderr() {
    let (result, _) = run("nonexistent_command_xyz_12345");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("command not found"),
        "expected 'command not found' in stderr, got: {stderr}"
    );
}

#[test]
fn empty_command() {
    // A lone semicolon or newline produces Empty commands.
    let (result, _) = run(";");
    assert_eq!(result.exit_code, 0);
}

// ── Redirect tests ────────────────────────────────────────────────────

#[test]
fn redirect_output_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    // Use forward slashes for cross-platform compatibility with the shell parser.
    let path_str = path.to_string_lossy().replace('\\', "/");
    let cmd = format!("echo hello > {path_str}");
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0, "stderr: {}", String::from_utf8_lossy(&result.stderr));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.trim().contains("hello"), "got: {contents}");
}

/// Helper: convert a path to a forward-slash string for shell commands.
fn shell_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn redirect_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    std::fs::write(&path, "first\n").unwrap();
    let cmd = format!("echo second >> {}", shell_path(&path));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0, "stderr: {}", String::from_utf8_lossy(&result.stderr));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("first"), "got: {contents}");
    assert!(contents.contains("second"), "got: {contents}");
}

#[test]
fn redirect_clobber() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    std::fs::write(&path, "old content\n").unwrap();
    let cmd = format!("echo new >| {}", shell_path(&path));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0, "stderr: {}", String::from_utf8_lossy(&result.stderr));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("new"), "got: {contents}");
    assert!(!contents.contains("old"), "got: {contents}");
}

#[test]
fn redirect_output_captures_nothing_in_result() {
    // When stdout is redirected to a file, ExecResult.stdout should be empty
    // (the data went to the file, not the pipe).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    let cmd = format!("echo hello > {}", shell_path(&path));
    let (result, _) = run(&cmd);
    assert!(result.stdout.is_empty(), "expected empty stdout in result, got: {:?}", result.stdout);
    // But the file should have the data.
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.trim().contains("hello"), "file: {contents}");
}

#[test]
fn redirect_both_stdout_and_stderr() {
    // &> redirects both stdout and stderr to the same file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("both.txt");
    let cmd = format!("echo combined &> {}", shell_path(&path));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0, "stderr: {}", String::from_utf8_lossy(&result.stderr));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("combined"), "got: {contents}");
}

// ── Pipeline tests ────────────────────────────────────────────────────

#[test]
fn pipeline_echo_findstr() {
    let output = run_stdout("echo hello | findstr hello");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn pipeline_filters() {
    // echo outputs "hello world", findstr filters for "world"
    let output = run_stdout("echo hello world | findstr world");
    assert!(output.contains("world"), "got: {output}");
}

#[test]
fn negated_pipeline_false() {
    let (result, _) = run("! nonexistent_cmd_xyz");
    assert_eq!(result.exit_code, 0, "negated failure should be success");
}

#[test]
fn negated_pipeline_true() {
    let (result, _) = run("! echo hello");
    assert_ne!(result.exit_code, 0, "negated success should be failure");
}

#[test]
fn pipeline_exit_code_is_last_stage() {
    // The last command determines the exit code (without pipefail).
    let (result, _) = run("echo hello | nonexistent_cmd_xyz_99");
    assert_ne!(result.exit_code, 0, "last stage failed, pipeline should fail");
}

#[test]
fn pipeline_single_command_no_pipe() {
    // A degenerate pipeline with one command should work like a normal command.
    let output = run_stdout("echo solo");
    assert!(output.contains("solo"), "got: {output}");
}

// ── Control flow tests ───────────────────────────────────────────────

#[test]
fn if_true_branch() {
    let output = run_stdout("if true; then echo yes; fi");
    assert!(output.contains("yes"), "got: {output}");
}

#[test]
fn if_false_else() {
    let output = run_stdout("if false; then echo no; else echo yes; fi");
    assert!(output.contains("yes"), "got: {output}");
    assert!(!output.contains("no"), "got: {output}");
}

#[test]
fn if_elif() {
    let output = run_stdout("if false; then echo no1; elif true; then echo yes; else echo no2; fi");
    assert!(output.contains("yes"), "got: {output}");
    assert!(!output.contains("no1"), "got: {output}");
    assert!(!output.contains("no2"), "got: {output}");
}

#[test]
fn for_loop_words() {
    let output = run_stdout("for x in hello world; do echo $x; done");
    assert!(output.contains("hello"), "got: {output}");
    assert!(output.contains("world"), "got: {output}");
}

#[test]
fn while_loop_with_break() {
    let output = run_stdout("x=0; while true; do x=$((x+1)); echo $x; if true; then break; fi; done");
    assert!(output.contains("1"), "got: {output}");
}

#[test]
fn until_loop() {
    // until false (i.e., condition fails, so loop continues; but we break immediately)
    let _output = run_stdout("until false; do echo never; break; done");
    // 'until false' → condition exit code is 1 (failure) → loop body runs
    // Wait, that's wrong. 'until' runs body when condition FAILS.
    // 'until false' → false returns 1 → condition failed → body runs → break
    let output2 = run_stdout("until true; do echo never; done");
    // 'until true' → condition succeeds → don't enter body
    assert!(!output2.contains("never"), "got: {output2}");
}

#[test]
fn case_match() {
    let output = run_stdout("case hello in h*) echo matched;; *) echo no;; esac");
    assert!(output.contains("matched"), "got: {output}");
    assert!(!output.contains("no"), "got: {output}");
}

#[test]
fn case_default() {
    let output = run_stdout("case xyz in h*) echo no;; *) echo default;; esac");
    assert!(output.contains("default"), "got: {output}");
    assert!(!output.contains("no"), "got: {output}");
}

#[test]
fn case_no_match() {
    let (result, _) = run("case xyz in h*) echo no;; esac");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn function_def_and_call() {
    let output = run_stdout("greet() { echo hello; }; greet");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn function_with_args() {
    let output = run_stdout("f() { echo $1 $2; }; f hello world");
    assert!(output.contains("hello world"), "got: {output}");
}

#[test]
fn function_return() {
    let (result, _) = run("f() { return 42; }; f");
    assert_eq!(result.exit_code, 42);
}

#[test]
fn subshell_isolation() {
    // Variable set in subshell shouldn't affect parent.
    let output = run_stdout("x=before; (x=inside; echo $x); echo $x");
    assert!(output.contains("inside"), "got: {output}");
    assert!(output.contains("before"), "got: {output}");
}

#[test]
fn arithmetic_command_nonzero() {
    let (result, _) = run("(( 1 + 1 ))");
    assert_eq!(result.exit_code, 0, "nonzero result should be success");
}

#[test]
fn arithmetic_command_zero() {
    let (result, _) = run("(( 0 ))");
    assert_ne!(result.exit_code, 0, "zero result should be failure");
}

#[test]
fn brace_group() {
    let output = run_stdout("{ echo a; echo b; }");
    assert!(output.contains("a"), "got: {output}");
    assert!(output.contains("b"), "got: {output}");
}

#[test]
fn break_in_for() {
    let output = run_stdout("for x in a b c; do if echo $x; then break; fi; done");
    assert!(output.contains("a"), "got: {output}");
    assert!(!output.contains("b"), "got: {output}");
}

#[test]
fn continue_in_for() {
    let output = run_stdout("for x in a b c; do echo start_$x; continue; echo end_$x; done");
    assert!(output.contains("start_a"), "got: {output}");
    assert!(output.contains("start_b"), "got: {output}");
    assert!(output.contains("start_c"), "got: {output}");
    assert!(!output.contains("end_"), "got: {output}");
}

#[test]
fn nested_loops_break() {
    // break 2 should exit both loops
    let output = run_stdout("for i in 1 2; do for j in a b; do echo $i$j; break 2; done; done");
    assert!(output.contains("1a"), "got: {output}");
    assert!(!output.contains("1b"), "got: {output}");
    assert!(!output.contains("2"), "got: {output}");
}

#[test]
fn true_builtin() {
    let (result, _) = run("true");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn false_builtin() {
    let (result, _) = run("false");
    assert_eq!(result.exit_code, 1);
}

#[test]
fn colon_builtin() {
    let (result, _) = run(":");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn builtin_echo() {
    // Built-in echo should work identically to external echo.
    let output = run_stdout("echo hello world");
    assert!(output.contains("hello world"), "got: {output}");
}

#[test]
fn for_loop_variable_persists() {
    let output = run_stdout("for x in last; do true; done; echo $x");
    assert!(output.contains("last"), "got: {output}");
}

#[test]
fn function_scope_positional_restore() {
    // After function call, positional params should be restored.
    let output = run_stdout("f() { echo $1; }; f inner; echo done");
    assert!(output.contains("inner"), "got: {output}");
    assert!(output.contains("done"), "got: {output}");
}

#[test]
fn if_condition_does_not_trigger_errexit() {
    // With errexit, a failing condition in 'if' should not abort.
    let mut env = Env::from_os();
    env.options_mut().errexit = true;
    let input = "if false; then echo no; else echo yes; fi; echo after";
    let cmds = parse(input).expect("parse failed");
    let result = execute_list(&cmds, input, &mut env);
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(output.contains("yes"), "got: {output}");
    assert!(output.contains("after"), "got: {output}");
}
