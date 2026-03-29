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
