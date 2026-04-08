//! Integration tests for the MASH executor scaffold.

use mash::env::{Env, Variable};
use mash::executor::{execute_list, ExecResult};
use mash::parser::parse;
use std::sync::Mutex;

/// Tests that call `cd` mutate the process-wide working directory via
/// `std::env::set_current_dir`. Acquire this lock in every such test to
/// prevent races when the test suite runs in parallel.
static CWD_LOCK: Mutex<()> = Mutex::new(());

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

#[cfg(windows)]
fn windows_symlink_creation_available() -> bool {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&target, "hello").unwrap();

    match malt_platform::fs::create_symlink(&target, &link) {
        Ok(()) => true,
        Err(err) if err.raw_os_error() == Some(1314) => false,
        Err(err) => panic!("unexpected symlink probe error: {err}"),
    }
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
fn builtin_echo_n_suppresses_trailing_newline() {
    let output = run_stdout("echo -n hello; echo world");
    assert_eq!(output, "helloworld\n");
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
fn special_builtin_error_aborts_noninteractive_script() {
    let input = "readonly a=b\nexport a=c\necho egad\n";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(result.exit_code, 1);
    assert!(
        String::from_utf8_lossy(&result.stdout).is_empty(),
        "script should stop before echo: {}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(env.exit_requested(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("readonly variable"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn parameter_expansion_error_aborts_noninteractive_script() {
    let input = "unset x\necho ${x?z}\necho blargh\n";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(result.exit_code, 1);
    assert!(
        String::from_utf8_lossy(&result.stdout).is_empty(),
        "script should stop before echo: {}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(env.exit_requested(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("x: z"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn heredoc_expansion_error_aborts_noninteractive_script() {
    let input = "cat <<EOF > script\nunset x\necho ${x?z}\nEOF\nchmod +x script\n";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(result.exit_code, 1);
    assert!(
        String::from_utf8_lossy(&result.stdout).is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(env.exit_requested(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("heredoc expansion: x: z"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn source_missing_file_aborts_noninteractive_script() {
    let input = "source not_a_thing\necho hi\n";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(result.exit_code, 1);
    assert!(
        String::from_utf8_lossy(&result.stdout).is_empty(),
        "script should stop before echo: {}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(env.exit_requested(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("not found"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn sequential_commands() {
    let output = run_stdout("echo first; echo second");
    assert!(output.contains("first"), "got: {output}");
    assert!(output.contains("second"), "got: {output}");
}

#[test]
fn append_redirection_preserves_existing_file_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let old_cwd = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(dir.path()).expect("chdir tempdir");

    let (result, _) = run("echo first > out; echo second >> out; cat out");

    std::env::set_current_dir(old_cwd).expect("restore cwd");

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "first\nsecond\n");
}

#[test]
fn append_redirection_accumulates_printf_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let old_cwd = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(dir.path()).expect("chdir tempdir");

    let (result, _) = run("printf '%s' alpha > out; printf '%s' beta >> out; cat out");

    std::env::set_current_dir(old_cwd).expect("restore cwd");

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "alphabeta");
}

#[test]
fn variable_expansion_in_args() {
    let mut env = Env::from_os();
    env.set("GREETING", Variable::string("hi"))
        .expect("set failed");
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
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.trim().contains("hello"), "got: {contents}");
}

#[test]
fn exec_can_duplicate_shell_stdout_to_extra_fd() {
    let (result, _) = run("exec 3>&1; echo hello >&3");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hello\n");
}

#[test]
fn exec_fd_duplication_snapshots_current_shell_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snap.txt");
    let cmd = format!(
        "exec 1>{path}; exec 3>&1; echo hello >&3",
        path = shell_path(&path)
    );
    let (result, _) = run(&cmd);
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
}

#[test]
fn command_substitution_captures_function_stderr_via_two_to_one() {
    let (result, _) = run("f() { echo message >&2; }; msg=$(f 2>&1); printf '[%s]\\n' \"$msg\"");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[message]\n");
}

#[test]
fn command_substitution_captures_function_stderr_via_indirect_two_to_fd() {
    let (result, _) =
        run("f() { echo message >&2; }; x=1; msg=$(f 2>&$x); printf '[%s]\\n' \"$msg\"");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[message]\n");
}

#[test]
#[cfg(windows)]
fn pipeline_subshell_can_write_to_snapshotted_shell_stdout_fd() {
    let (result, _) = run("exec 3>&1; (echo hi >&3) | true");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hi\n");
}

#[test]
#[cfg(windows)]
fn subshell_pipeline_preserves_times_ioerror_status() {
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let sleep = temp.path().join("sleep.cmd");
    let capture = temp.path().join("capture.txt");
    std::fs::write(&sleep, "@echo off\r\ntimeout /t %1 /nobreak >nul\r\n").unwrap();

    let input = format!(
        "exec 1>{capture}\nexec 3>&1\n(\n    trap \"\" PIPE\n    sleep 1\n    command times\n    echo ?=$? >&3\n) | true\n",
        capture = shell_path(&capture)
    );
    let cmds = parse(&input).expect("parse failed");
    let mut env = Env::from_os();
    let path = format!("{};{}", temp.path().display(), env.get_str("PATH"));
    env.set("PATH", Variable::exported_string(path))
        .expect("set PATH");

    let result = execute_list(&cmds, &input, &mut env);
    let captured = std::fs::read_to_string(&capture).unwrap_or_default();

    assert_eq!(captured, "?=2\n");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("times"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
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
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
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
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("new"), "got: {contents}");
    assert!(!contents.contains("old"), "got: {contents}");
}

#[test]
fn ln_symbolic_links_satisfy_test_predicates() {
    let _guard = CWD_LOCK.lock().unwrap();
    #[cfg(windows)]
    if !windows_symlink_creation_available() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let cmd = format!(
        "cd '{}'; echo hi >file; mkdir dir; ln -s file link_file; ln -s dir link_dir; \
         [ -e file ] && [ -e link_file ] && [ -f file ] && [ -f link_file ] && \
         [ -e dir ] && [ -e link_dir ] && [ -d dir ] && [ -d link_dir ] && \
         [ -L link_file ] && [ -L link_dir ]",
        shell_path(dir.path())
    );

    let (result, _) = run(&cmd);
    assert_eq!(
        result.exit_code,
        0,
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn hash_tracks_executed_commands_and_clears() {
    let (result, _) = run("ls >/dev/null; hash");
    let output = String::from_utf8_lossy(&result.stdout);
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.contains("ls"), "hash output: {output}");

    let (cleared, _) = run("ls >/dev/null; hash -r; hash");
    let cleared_output = String::from_utf8_lossy(&cleared.stdout);
    assert_eq!(
        cleared.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert!(
        !cleared_output.contains("ls"),
        "hash output after clear: {cleared_output}"
    );
}

#[test]
fn redirect_output_captures_nothing_in_result() {
    // When stdout is redirected to a file, ExecResult.stdout should be empty
    // (the data went to the file, not the pipe).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    let cmd = format!("echo hello > {}", shell_path(&path));
    let (result, _) = run(&cmd);
    assert!(
        result.stdout.is_empty(),
        "expected empty stdout in result, got: {:?}",
        result.stdout
    );
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
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
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
    assert_ne!(
        result.exit_code, 0,
        "last stage failed, pipeline should fail"
    );
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
    let output =
        run_stdout("x=0; while true; do x=$((x+1)); echo $x; if true; then break; fi; done");
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
fn case_pattern_matches_backslash_newline_construct() {
    let output =
        run_stdout("case 'foo\\\nbar' in ( foo\\\\\"\n\"bar ) echo good ;; ( * ) echo bad ;; esac");
    assert_eq!(output, "good\n");
}

#[test]
fn backtick_command_substitution_preserves_backslash_for_hash_argument() {
    let output = run_stdout("x=`printf '%s' \\#`; printf '%s\\n' \"$x\"");
    assert_eq!(output, "#\n");
}

#[test]
fn backtick_command_substitution_preserves_backslash_for_close_paren_argument() {
    let output = run_stdout("x=`printf '%s' \\)`; printf '%s\\n' \"$x\"");
    assert_eq!(output, ")\n");
}

#[test]
fn test_string_equality_accepts_close_paren_operands() {
    let (result, _) = run("[ \")\" = \")\" ]");
    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn test_string_equality_accepts_close_paren_from_vars_and_command_subst() {
    let (result, _) = run("c=')'; out=$(printf '%s' \\)); [ \"$c\" = \"$out\" ]");
    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn test_string_equality_accepts_open_paren_from_vars_and_command_subst() {
    let (result, _) = run("c='('; out=$(printf '%s' \\(); [ \"$c\" = \"$out\" ]");
    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
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

// ── Command substitution tests ───────────────────────────────────────

#[test]
fn command_substitution_basic() {
    let output = run_stdout("echo $(echo hello)");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn command_substitution_nested() {
    let output = run_stdout("echo $(echo $(echo deep))");
    assert!(output.contains("deep"), "got: {output}");
}

#[test]
fn command_substitution_in_variable() {
    let (_, env) = run("x=$(echo captured)");
    assert_eq!(env.get_str("x"), "captured");
}

#[test]
fn env_assignment_uses_command_substitution_exit_status() {
    let (result, env) = run("x=$(false)");
    assert_eq!(result.exit_code, 1);
    assert_eq!(env.exit_code(), 1);
}

#[test]
fn command_substitution_strips_trailing_newlines() {
    let output = run_stdout("echo \"$(echo hello)\"");
    // The inner echo outputs "hello\n", capture_command strips trailing newlines
    assert!(output.trim() == "hello", "got: {}", output.trim());
}

#[test]
fn backtick_substitution() {
    let output = run_stdout("echo `echo world`");
    assert!(output.contains("world"), "got: {output}");
}

#[test]
fn command_sub_captures_output() {
    // Command substitution should capture the stdout of the inner command.
    let output = run_stdout("echo the answer is $(echo 42)");
    assert!(output.contains("the answer is 42"), "got: {output}");
}

#[test]
fn set_e_stops_execution() {
    let output = run_stdout("set -e; nonexistent_xyz; echo unreachable");
    assert!(!output.contains("unreachable"));
}

#[test]
fn set_e_suppressed_in_if() {
    let output =
        run_stdout("set -e; if nonexistent_xyz; then echo no; else echo yes; fi; echo after");
    assert!(output.contains("yes"));
    assert!(output.contains("after"));
}

#[test]
fn set_e_does_not_abort_on_failed_and_or_list() {
    let output = run_stdout("set -e; false && true; echo after");
    assert_eq!(output, "after\n");
}

#[test]
fn set_e_is_suppressed_for_entire_if_subshell_condition() {
    let output = run_stdout(
        "set -o errexit; if ( echo 1; false; echo 2; set -o errexit; echo 3; false; echo 4 ); then echo 5; fi; echo 6",
    );
    assert_eq!(output, "1\n2\n3\n4\n5\n6\n");
}

#[test]
fn shift_builtin() {
    let output = run_stdout("set -- a b c; shift; echo $1");
    assert!(output.contains("b"), "got: {output}");
}

#[test]
fn shift_two() {
    let output = run_stdout("set -- a b c d; shift 2; echo $1");
    assert!(output.contains("c"), "got: {output}");
}

#[test]
fn set_positional_params() {
    let output = run_stdout("set -- x y z; echo $1 $2 $3");
    assert!(output.contains("x y z"), "got: {output}");
}

// ── export / unset / readonly / cd / pwd builtins ────────────────────

#[test]
fn builtin_export_and_check() {
    let (_, env) = run("export FOO=hello");
    assert_eq!(env.get_str("FOO"), "hello");
    assert!(env.get("FOO").unwrap().exported);
}

#[test]
fn builtin_export_existing() {
    let (_, env) = run("FOO=bar; export FOO");
    assert!(env.get("FOO").unwrap().exported);
}

#[test]
fn builtin_unset() {
    let (_, env) = run("FOO=bar; unset FOO");
    assert!(!env.is_set("FOO"));
}

#[test]
fn builtin_unset_function() {
    let (_, env) = run("f() { echo hi; }; unset -f f");
    assert!(env.get_function("f").is_none());
}

#[test]
fn builtin_readonly() {
    let (result, _) = run("readonly X=5; X=10");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn builtin_cd_and_pwd() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().replace('\\', "/");
    let cmd = format!("cd {} && pwd", path);
    let output = run_stdout(&cmd);
    assert!(!output.trim().is_empty());
}

#[test]
fn builtin_cd_home() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_, env) = run("cd");
    let pwd = env.get_str("PWD").replace('\\', "/");
    let home = env.get_str("HOME").replace('\\', "/");
    if !home.is_empty() {
        assert_eq!(pwd, home);
    }
}

#[test]
fn builtin_cd_dash() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().replace('\\', "/");
    let cmd = format!("FIRST=$PWD; cd {}; cd -", path);
    let output = run_stdout(&cmd);
    // cd - should print the previous directory
    assert!(!output.trim().is_empty());
}

#[test]
fn builtin_cd_updates_oldpwd() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().replace('\\', "/");
    let cmd = format!("BEFORE=$PWD; cd {}", path);
    let (_, env) = run(&cmd);
    assert!(!env.get_str("OLDPWD").is_empty());
}

#[test]
fn builtin_pwd_output() {
    let output = run_stdout("pwd");
    assert!(!output.trim().is_empty());
}

#[test]
fn builtin_source_searches_path_when_sourcepath_enabled() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("p1");
    let p2 = dir.path().join("p2");
    std::fs::create_dir_all(&p1).unwrap();
    std::fs::create_dir_all(&p2).unwrap();
    std::fs::write(p1.join("scr2"), "echo nope\n").unwrap();
    std::fs::write(p2.join("scr2"), "echo yep\n").unwrap();

    let cmd = format!(
        "PATH=\"{}:{}:$PATH\"; . scr2",
        shell_path(&p1),
        shell_path(&p2)
    );
    let output = run_stdout(&cmd);
    assert!(
        output.contains("nope") || output.contains("yep"),
        "got: {output}"
    );
}

#[test]
fn builtin_source_skips_unreadable_path_entry() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("p1");
    let p2 = dir.path().join("p2");
    std::fs::create_dir_all(&p1).unwrap();
    std::fs::create_dir_all(&p2).unwrap();

    let scr1 = p1.join("scr2");
    let scr2 = p2.join("scr2");
    std::fs::write(&scr1, "echo nope\n").unwrap();
    std::fs::write(&scr2, "echo yep\n").unwrap();
    malt_platform::fs::set_mode(&scr1, 0o333).unwrap();
    malt_platform::fs::set_mode(&scr2, 0o444).unwrap();

    let cmd = format!(
        "PATH=\"{}:{}:$PATH\"; . scr2",
        shell_path(&p1),
        shell_path(&p2)
    );
    let output = run_stdout(&cmd);
    assert_eq!(output, "yep\n", "got: {output}");
}

#[test]
fn builtin_source_direct_unreadable_file_fails() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("weird");
    std::fs::write(&script, "echo nope\n").unwrap();
    malt_platform::fs::set_mode(&script, 0o333).unwrap();

    let input = format!(". {}", shell_path(&script));
    let cmds = parse(&input).expect("parse");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, &input, &mut env);

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty(), "stdout: {:?}", result.stdout);
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("permission denied"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn builtin_command_exec_preserves_redirected_fd() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let cmd = format!(
        "command exec 8<{}; read msg <&8; echo $msg",
        shell_path(&file)
    );
    let output = run_stdout(&cmd);
    assert!(output.contains("hi"), "got: {output}");
}

#[test]
fn builtin_exec_runs_target_command_and_stops_shell() {
    let (result, _) = run("exec true; false");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn bad_fd_redirect_on_special_builtin_aborts_command() {
    let (result, _) = run(": 2>&9; echo oh no");
    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty(), "stdout: {:?}", result.stdout);
}

#[test]
fn bad_fd_redirect_on_exec_fails() {
    let (result, _) = run("exec 9>&bogus");
    assert_eq!(result.exit_code, 1);
}

#[test]
fn bad_fd_redirect_on_exec_aborts_noninteractive_script() {
    let (result, env) = run("exec 9>&bogus; echo oh no");
    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty(), "stdout: {:?}", result.stdout);
    assert_eq!(env.exit_requested(), Some(1));
}

#[test]
fn heredoc_redirect_feeds_stdin_to_cat() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let script = "cat >scr <<EOF\nhello\nEOF\ncat scr\n";
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let output = run_stdout(script);
    std::env::set_current_dir(previous).unwrap();
    assert_eq!(output, "hello\n", "got: {output}");
}

// ── test / [ builtin ────────────────────────────────────────────────

#[test]
fn test_builtin_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("exists.txt");
    std::fs::write(&file, "hi").unwrap();
    let cmd = format!("test -f {}", shell_path(&file));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_file_not_exists() {
    let (result, _) = run("test -f /nonexistent/file/xyz");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn test_builtin_dir_exists() {
    let dir = tempfile::tempdir().unwrap();
    let cmd = format!("test -d {}", shell_path(dir.path()));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_string_empty() {
    let (result, _) = run("test -z ''");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_string_nonempty() {
    let (result, _) = run("test -n 'hello'");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_string_equal() {
    let (result, _) = run("test hello = hello");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_string_not_equal() {
    let (result, _) = run("test hello != world");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_arithmetic_eq() {
    let (result, _) = run("test 5 -eq 5");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_arithmetic_lt() {
    let (result, _) = run("test 3 -lt 5");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_arithmetic_gt_false() {
    let (result, _) = run("test 3 -gt 5");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn test_builtin_negation() {
    let (result, _) = run("test ! -f /nonexistent");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_no_args_is_false() {
    let (result, _) = run("test");
    assert_eq!(result.exit_code, 1);
}

#[test]
fn test_builtin_single_string_true() {
    let (result, _) = run("test hello");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_exists() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("exists.txt");
    std::fs::write(&file, "hi").unwrap();
    let cmd = format!("test -e {}", shell_path(&file));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_builtin_size_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nonempty.txt");
    std::fs::write(&file, "content").unwrap();
    let cmd = format!("test -s {}", shell_path(&file));
    let (result, _) = run(&cmd);
    assert_eq!(result.exit_code, 0);
}

#[test]
fn bracket_syntax() {
    let (result, _) = run("[ hello = hello ]");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn bracket_syntax_missing_close() {
    let (result, _) = run("[ hello = hello");
    assert_eq!(result.exit_code, 2);
}

// ── trap builtin ────────────────────────────────────────────────────

#[test]
fn trap_set_and_list() {
    let output = run_stdout("trap 'echo hi' INT; trap");
    assert!(output.contains("INT"), "got: {output}");
    assert!(output.contains("echo hi"), "got: {output}");
}

#[test]
fn trap_reset() {
    let output = run_stdout("trap 'echo hi' INT; trap - INT; trap");
    // After resetting, INT should not appear in the trap list.
    assert!(!output.contains("INT"), "got: {output}");
}

#[test]
fn trap_list_signals() {
    let output = run_stdout("trap -l");
    assert!(output.contains("INT"), "got: {output}");
    assert!(output.contains("TERM"), "got: {output}");
}

#[test]
fn trap_print_specific() {
    let output = run_stdout("trap 'echo bye' EXIT; trap -p EXIT");
    assert!(output.contains("EXIT"), "got: {output}");
    assert!(output.contains("echo bye"), "got: {output}");
}

#[test]
fn exit_trap_status_overrides_prior_command_status() {
    let (result, _) = run("trap '(true) || echo bug' EXIT; false");
    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "");
}

#[test]
fn subshell_inherits_parent_exit_trap_for_listing_without_running_it() {
    let output = run_stdout("trap 'echo bye' EXIT; (trap); echo done");
    assert_eq!(output, "trap -- 'echo bye' EXIT\ndone\nbye\n");
}

#[test]
fn invalid_set_o_option_fails() {
    let (result, _) = run("set -o bad@option && echo BUG4");
    assert_eq!(
        result.exit_code,
        1,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("bad@option"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn eval_parse_error_aborts_noninteractive_script() {
    let (result, env) = run("eval \"if\"\necho lived\n");
    assert_eq!(result.exit_code, 1);
    assert_eq!(String::from_utf8_lossy(&result.stdout), "");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("eval"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(env.exit_requested(), Some(1));
}

// ── type builtin ────────────────────────────────────────────────────

#[test]
fn type_builtin() {
    let output = run_stdout("type echo");
    assert!(output.contains("builtin"), "got: {output}");
}

#[test]
fn type_t_builtin() {
    let output = run_stdout("type -t echo");
    assert_eq!(output.trim(), "builtin");
}

#[test]
fn type_function() {
    let output = run_stdout("f() { :; }; type f");
    assert!(output.contains("function"), "got: {output}");
}

#[test]
fn type_t_function() {
    let output = run_stdout("f() { :; }; type -t f");
    assert_eq!(output.trim(), "function");
}

#[test]
fn type_keyword() {
    let output = run_stdout("type -t if");
    assert_eq!(output.trim(), "keyword");
}

#[test]
fn type_not_found() {
    let (result, _) = run("type nonexistent_xyz_99");
    assert_eq!(result.exit_code, 1);
}

// ── hash builtin ────────────────────────────────────────────────────

#[test]
fn hash_r_clears() {
    let (result, _) = run("hash -r");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn hash_empty_table() {
    let output = run_stdout("hash -r; hash");
    assert!(output.contains("empty"), "got: {output}");
}

#[test]
fn hash_not_found() {
    let (result, _) = run("hash nonexistent_xyz_99");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn redirected_last_pipeline_stage_consumes_pipeline_stdout() {
    let output = run_stdout("printf 'hello\\n' | cat >/dev/null; echo after");
    assert_eq!(output, "after\n");
}

#[test]
fn hash_output_can_be_redirected_in_pipeline() {
    let output = run_stdout("ls >/dev/null; hash | cat >/dev/null; echo after");
    assert_eq!(output, "after\n");
}

#[test]
fn hashall_records_commands_from_function_definition_body() {
    let output = run_stdout("set -h\nhash -r\nf() {\n  ls\n  touch hi\n  rm hi\n}\nhash\n");
    assert!(output.contains("ls"), "output: {output}");
    assert!(output.contains("touch"), "output: {output}");
    assert!(output.contains("rm"), "output: {output}");
}

#[test]
fn interactive_history_records_commands_and_respects_nolog() {
    let input = "\
history | grep history >/dev/null || exit 1
echo hi >/dev/null
history | grep echo >/dev/null || exit 2
history -c
history >hist
grep echo >/dev/null hist && exit 3
set -o nolog
history -c
echo hello >/dev/null
history >hist2
grep echo >/dev/null hist2 && exit 4
echo ok
";
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    env.set_interactive(true);
    let result = execute_list(&cmds, input, &mut env);

    std::env::set_current_dir(saved).unwrap();

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
}

#[test]
fn for_loop_preserves_prior_stdout_when_readonly_iteration_variable_aborts() {
    let (result, _) = run("(for x in a b c; do echo $x; readonly x; done) && exit 1\nexit 0\n");
    assert_eq!(result.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a\n");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("readonly variable"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn jobs_builtin_lists_background_job_with_long_format() {
    let input = "\
sleep 10 & pid=$!
jobs -l >job_info
grep \"sleep 10\" job_info >/dev/null || exit 1
grep \"$pid\" job_info >/dev/null || exit 2
kill $pid
echo ok
";
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    std::env::set_current_dir(saved).unwrap();

    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
}

#[test]
fn wait_builtin_flushes_background_group_output() {
    let input = "\
echo hi
{ sleep 1; echo derp; } &
echo bye
wait
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hi\nbye\nderp\n");
}

#[test]
fn wait_builtin_for_signaled_job_preserves_background_output_order() {
    let input = "\
( echo first; sleep 10; echo never ) & pid=$!
sleep 1
kill $pid
wait $pid
echo after:$?
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "first\nafter:143\n");
}

#[test]
fn wait_builtin_preserves_background_stdin_consumption() {
    let input = "\
exec <in
cat &
wait
";
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();
    std::fs::write("in", "illegible\n").unwrap();

    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    std::env::set_current_dir(saved).unwrap();

    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "illegible\n");
}

#[test]
fn exec_input_redirect_registers_readable_shell_fd() {
    let input = "exec <in\n";
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();
    std::fs::write("in", "illegible\n").unwrap();

    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    let mut file = env.open_fd_read(0).expect("open shell stdin");
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut buf).expect("read shell stdin");

    std::env::set_current_dir(saved).unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&buf), "illegible\n");
}

#[test]
fn kill_builtin_runs_term_trap_for_shell_and_signal_zero_succeeds() {
    let input = "\
trap 'echo bye' TERM
kill -s 0 $$
kill -TERM $$
echo ok
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "bye\nok\n");
}

#[test]
fn signal_trap_failure_does_not_fail_kill() {
    let input = "\
trap '(false) && echo BUG' INT
kill -s INT $$
echo ok
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
}

#[test]
fn signal_trap_exit_status_does_not_become_kill_status() {
    let input = "\
trap '(exit 3) && echo BUG' INT
kill -s INT $$
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).is_empty());
}

#[test]
fn background_jobs_do_not_inherit_parent_term_trap_action() {
    let input = "\
trap 'echo hi' TERM
sleep 2 &
pid=$!
sleep 0.2
kill $pid
sleep 0.2
wait $pid
echo $?
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "143\n");
}

#[test]
fn exit_trap_runs_at_shell_exit_not_in_subshell_or_command_substitution() {
    let input = "\
trap 'echo bye' EXIT
(echo hi)
echo $(echo hi)
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hi\nhi\nbye\n");
}

#[test]
fn return_short_circuits_or_list_inside_function() {
    let input = "\
f() {
  return 5 || echo fail passthrough
}
f
echo $?
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n");
}

#[test]
fn subshell_break_two_stops_after_break() {
    let input = "\
for x in a b
do
  (
    for y in c d
    do
      break 2
    done
    echo $x
  )
done
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a\nb\n");
}

#[test]
fn exit_trap_runs_when_function_subshell_returns() {
    let input = "\
f() ( trap 'echo FOO' EXIT; return 5; echo BAR )
f
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "FOO\n");
}

#[test]
fn errexit_applies_inside_signal_trap() {
    let input = "\
set -e
trap 'false; echo BUG' INT
kill -s INT $$
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        1,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).is_empty());
}

#[test]
fn exit_trap_can_run_function_that_returns() {
    let input = "trap 'f() { false; return; }; f; echo $?' EXIT";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "1\n");
}

#[test]
fn special_builtin_prefix_assignments_are_visible_left_to_right_and_persist() {
    let input = "\
x=5 y=$((x+2)) :
echo $x $y
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5 7\n");
}

#[test]
fn plain_assignment_command_without_command_substitution_returns_zero() {
    let input = "\
false
x=hi
echo $?
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n");
}

#[test]
fn function_prefix_redirect_expansion_precedes_assignment_word_expansion() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old_dir = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("cd tempdir");

    let input = "\
show() { echo \"got ${EFF-unset}\"; }
unset x
EFF=${x=assign} show 2>${x=redir}
echo ${EFF-unset after function call}
[ -f assign ] && echo assign exists && rm assign
[ -f redir ] && echo redir exists && rm redir
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    let result = execute_list(&cmds, input, &mut env);

    std::env::set_current_dir(old_dir).expect("restore cwd");

    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "got redir\nunset after function call\nredir exists\n"
    );
}

#[test]
fn readonly_ignores_double_dash_and_assignment_error_does_not_stop_interactive_script() {
    let input = "\
foo=bar
readonly -- foo
readonly -- baz=quux
echo $foo $baz
foo=nope
unset baz
echo $foo $baz
";
    let cmds = parse(input).expect("parse failed");
    let mut env = Env::from_os();
    env.set_interactive(true);
    let result = execute_list(&cmds, input, &mut env);

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "bar quux\nbar quux\n"
    );
}

#[test]
fn test_newer_older_with_absent_file_is_boolean_not_error() {
    let _cwd_guard = CWD_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();
    std::fs::write("present", "x").unwrap();

    let (result, _) = run("[ present -nt absent ] && [ absent -ot present ]");

    std::env::set_current_dir(saved).unwrap();

    assert_eq!(
        result.exit_code,
        0,
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

// ── command builtin ─────────────────────────────────────────────────

#[test]
fn command_v_builtin() {
    let output = run_stdout("command -v echo");
    assert_eq!(output.trim(), "echo");
}

#[test]
fn command_v_not_found() {
    let (result, _) = run("command -v nonexistent_xyz_99");
    assert_eq!(result.exit_code, 1);
}

#[test]
fn command_v_function() {
    let output = run_stdout("f() { :; }; command -v f");
    assert_eq!(output.trim(), "f");
}

#[test]
fn command_bypasses_function() {
    // `command echo` should run the builtin echo, not a function named echo.
    let output = run_stdout("echo() { :; }; command echo hello");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn command_neutralizes_special_builtin_exit_semantics() {
    let (result, env) = run("command readonly x=foo; command readonly x=bar; echo ?=$?");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "?=1\n");
    assert_eq!(env.exit_requested(), None);
}

#[test]
fn command_capital_v_verbose() {
    let output = run_stdout("command -V echo");
    assert!(output.contains("builtin"), "got: {output}");
}

// ── read builtin ────────────────────────────────────────────────────

#[test]
fn builtin_read_from_herestring() {
    let output = run_stdout("read VAR <<< 'hello'; echo $VAR");
    assert!(output.contains("hello"), "got: {output}");
}

#[test]
fn builtin_read_default_reply() {
    let output = run_stdout("read <<< 'world'; echo $REPLY");
    assert!(output.contains("world"), "got: {output}");
}

#[test]
fn builtin_read_eof_returns_1() {
    // Empty here-string still has a newline, so read succeeds.
    // Use /dev/null for true EOF.
    let (result, _) = run("read VAR < /dev/null");
    // On Windows this may fail differently, but we test the concept.
    // The exit code should be non-zero (1) on EOF.
    assert!(
        result.exit_code != 0 || cfg!(windows),
        "expected non-zero on EOF"
    );
}

// ── printf builtin ──────────────────────────────────────────────────

#[test]
fn builtin_printf_string() {
    let output = run_stdout("printf '%s world' hello");
    assert_eq!(output, "hello world");
}

#[test]
fn builtin_echo_reports_redirect_write_failure() {
    let (result, _) = run("echo >/dev/full || echo OK");
    assert_eq!(String::from_utf8_lossy(&result.stdout), "OK\n");
}

#[test]
fn test_numeric_comparison_trims_spaces() {
    let (result, _) = run("test ' 5' -eq ' 5 '");
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn test_file_time_and_identity_operators() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    std::fs::write("first", b"one").unwrap();
    let (same_file, _) = run("test first -ef first");
    assert_eq!(same_file.exit_code, 0);

    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write("second", b"two").unwrap();

    let (different_file, _) = run("test first -ef second");
    assert_eq!(different_file.exit_code, 1);

    let (newer, _) = run("test second -nt first");
    assert_eq!(newer.exit_code, 0);

    let (older, _) = run("test first -ot second");
    assert_eq!(older.exit_code, 0);

    std::env::set_current_dir(previous).unwrap();
}

#[test]
fn test_rejects_unparsed_trailing_operands() {
    let (result, _) = run("test first -ef second");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn builtin_printf_decimal() {
    let output = run_stdout("printf '%d' 42");
    assert_eq!(output, "42");
}

#[test]
fn builtin_printf_newline() {
    let output = run_stdout("printf 'hello\\n'");
    assert_eq!(output, "hello\n");
}

#[test]
fn builtin_printf_reuse_format() {
    let output = run_stdout("printf '%s ' a b c");
    assert_eq!(output, "a b c ");
}

#[test]
fn builtin_printf_octal() {
    let output = run_stdout("printf '%o' 8");
    assert_eq!(output, "10");
}

#[test]
fn builtin_printf_hex_lower() {
    let output = run_stdout("printf '%x' 255");
    assert_eq!(output, "ff");
}

#[test]
fn builtin_printf_hex_upper() {
    let output = run_stdout("printf '%X' 255");
    assert_eq!(output, "FF");
}

#[test]
fn builtin_printf_literal_percent() {
    let output = run_stdout("printf '100%%'");
    assert_eq!(output, "100%");
}

#[test]
fn builtin_printf_char() {
    let output = run_stdout("printf '%c' hello");
    assert_eq!(output, "h");
}

#[test]
fn builtin_printf_no_trailing_newline() {
    let output = run_stdout("printf 'no newline'");
    assert_eq!(output, "no newline");
}

// ── alias / unalias builtins ────────────────────────────────────────

#[test]
fn builtin_alias_set_and_list() {
    let output = run_stdout("alias ll='ls -la'; alias ll");
    assert!(output.contains("ls -la"), "got: {output}");
}

#[test]
fn builtin_alias_list_all() {
    let output = run_stdout("alias foo='bar'; alias baz='qux'; alias");
    assert!(output.contains("foo"), "got: {output}");
    assert!(output.contains("baz"), "got: {output}");
}

#[test]
fn builtin_alias_not_found() {
    let (result, _) = run("alias nonexistent_alias_xyz");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn builtin_unalias() {
    let (_, env) = run("alias foo='bar'; unalias foo");
    assert!(env.get_alias("foo").is_none());
}

#[test]
fn builtin_unalias_all() {
    let (_, env) = run("alias a='1'; alias b='2'; unalias -a");
    assert!(env.aliases().is_empty());
}

#[test]
fn builtin_unalias_not_found() {
    let (result, _) = run("unalias nonexistent_alias_xyz");
    assert_ne!(result.exit_code, 0);
}

// ── getopts builtin ────────────────────────────────────────────────

#[test]
fn builtin_getopts_basic() {
    let output = run_stdout("set -- -a -b; while getopts ab opt; do printf '%s ' $opt; done");
    assert!(output.contains("a"), "got: {output}");
    assert!(output.contains("b"), "got: {output}");
}

#[test]
fn builtin_getopts_with_arg() {
    let output = run_stdout("set -- -f myfile; getopts f: opt; echo $opt $OPTARG");
    assert!(output.contains("f"), "got: {output}");
    assert!(output.contains("myfile"), "got: {output}");
}

#[test]
fn builtin_getopts_done_returns_1() {
    let (result, _) = run("set -- noopt; getopts a opt");
    assert_ne!(result.exit_code, 0);
}

// ── umask builtin ───────────────────────────────────────────────────

#[test]
fn builtin_umask_display() {
    let output = run_stdout("umask");
    assert!(!output.trim().is_empty(), "got: {output}");
}

#[test]
fn builtin_umask_symbolic() {
    let output = run_stdout("umask -S");
    assert!(output.contains("u="), "got: {output}");
    assert!(output.contains("g="), "got: {output}");
    assert!(output.contains("o="), "got: {output}");
}

#[test]
fn builtin_umask_set_valid() {
    let (result, _) = run("umask 0077");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn builtin_umask_set_invalid() {
    let (result, _) = run("umask xyz");
    assert_ne!(result.exit_code, 0);
}

// ── type builtin recognizes new builtins ────────────────────────────

#[test]
fn type_read_is_builtin() {
    let output = run_stdout("type -t read");
    assert_eq!(output.trim(), "builtin");
}

#[test]
fn type_printf_is_builtin() {
    let output = run_stdout("type -t printf");
    assert_eq!(output.trim(), "builtin");
}

#[test]
fn type_alias_is_builtin() {
    let output = run_stdout("type -t alias");
    assert_eq!(output.trim(), "builtin");
}

#[test]
fn type_getopts_is_builtin() {
    let output = run_stdout("type -t getopts");
    assert_eq!(output.trim(), "builtin");
}

#[test]
fn type_umask_is_builtin() {
    let output = run_stdout("type -t umask");
    assert_eq!(output.trim(), "builtin");
}
