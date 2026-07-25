//! Tests for the output sink seam (`Env::set_output_sink`): a command's
//! top-level, unredirected output reaches the sink, and the two contexts
//! that must never leak into it -- command substitution and pipeline
//! stages -- genuinely stay silent.
//!
//! Assertions are on sink *contents*, not call counts: a count-based
//! assertion would still pass if the wrong bytes arrived.

use mash::env::{Env, OutputSink};
use mash::executor::execute_list;
use mash::parser::parse;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingSink {
    stdout: Mutex<Vec<u8>>,
    stderr: Mutex<Vec<u8>>,
}

impl RecordingSink {
    fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout.lock().unwrap()).to_string()
    }

    fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).to_string()
    }

    fn is_empty(&self) -> bool {
        self.stdout.lock().unwrap().is_empty() && self.stderr.lock().unwrap().is_empty()
    }
}

impl OutputSink for RecordingSink {
    fn write_stdout(&self, data: &[u8]) {
        self.stdout.lock().unwrap().extend_from_slice(data);
    }

    fn write_stderr(&self, data: &[u8]) {
        self.stderr.lock().unwrap().extend_from_slice(data);
    }
}

fn env_with_sink() -> (Env, Arc<RecordingSink>) {
    let mut env = Env::from_os();
    let sink = Arc::new(RecordingSink::default());
    env.set_output_sink(sink.clone());
    (env, sink)
}

#[test]
fn sink_receives_a_commands_output_progressively() {
    let (mut env, sink) = env_with_sink();
    let cmds = parse("echo first; echo second").expect("parse failed");
    let result = execute_list(&cmds, "echo first; echo second", &mut env);

    assert_eq!(result.exit_code, 0);
    assert_eq!(sink.stdout_string(), "first\nsecond\n");
}

#[test]
fn sink_receives_stderr_separately_from_stdout() {
    let (mut env, sink) = env_with_sink();
    let cmds = parse("echo out; echo err 1>&2").expect("parse failed");
    let result = execute_list(&cmds, "echo out; echo err 1>&2", &mut env);

    assert_eq!(result.exit_code, 0);
    assert_eq!(sink.stdout_string(), "out\n");
    assert_eq!(sink.stderr_string(), "err\n");
}

#[test]
fn command_substitution_captures_the_value_and_sends_nothing_to_the_sink() {
    let (mut env, sink) = env_with_sink();
    let cmds = parse("result=$(echo hi); echo \"got:$result\"").expect("parse failed");
    let result = execute_list(&cmds, "result=$(echo hi); echo \"got:$result\"", &mut env);

    assert_eq!(result.exit_code, 0);
    // The substitution's own "hi" must never reach the sink -- only the
    // final echo, which reports the captured value as a string.
    assert_eq!(sink.stdout_string(), "got:hi\n");
}

#[test]
fn command_substitution_alone_sends_nothing_to_the_sink() {
    let (mut env, sink) = env_with_sink();
    let cmds = parse("x=$(echo hi)").expect("parse failed");
    let result = execute_list(&cmds, "x=$(echo hi)", &mut env);

    assert_eq!(result.exit_code, 0);
    assert!(
        sink.is_empty(),
        "a bare assignment with no real top-level output must not reach the sink, got stdout={:?} stderr={:?}",
        sink.stdout_string(),
        sink.stderr_string()
    );
}

#[test]
fn pipeline_sends_its_data_once_not_twice() {
    let (mut env, sink) = env_with_sink();
    let cmds = parse("echo a | cat").expect("parse failed");
    let result = execute_list(&cmds, "echo a | cat", &mut env);

    assert_eq!(result.exit_code, 0);
    // The pipeline's own captured stdout (the last stage's ExecResult) flows
    // through the ordinary accumulation path and reaches the sink exactly
    // once, at the top-level list -- not once per stage.
    assert_eq!(sink.stdout_string(), "a\n");
}

#[test]
fn redirected_command_does_not_stream_to_the_sink() {
    let (mut env, sink) = env_with_sink();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out.txt");
    // Single-quoted so a Windows path's backslashes are not treated as
    // shell escapes (which would otherwise mangle the path).
    let cmd = format!("echo redirected > '{}'", file.to_string_lossy());
    let cmds = parse(&cmd).expect("parse failed");
    let result = execute_list(&cmds, &cmd, &mut env);

    assert_eq!(result.exit_code, 0);
    assert!(
        sink.is_empty(),
        "output redirected to a file must not also stream to the sink, got stdout={:?}",
        sink.stdout_string()
    );
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(written, "redirected\n");
}

#[test]
fn no_sink_installed_means_no_panic_and_normal_accumulation() {
    let mut env = Env::from_os();
    let cmds = parse("echo hello").expect("parse failed");
    let result = execute_list(&cmds, "echo hello", &mut env);
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hello\n");
}
