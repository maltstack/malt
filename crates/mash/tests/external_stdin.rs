//! A registered fd 0 must reach the command that is waiting on it.
//!
//! The daemon registers a session's input pipe at fd 0 so that a command
//! asking for input -- a password prompt, a REPL -- reads from the session's
//! clients rather than from the daemon's own console. These tests drive that
//! at the lowest layer: real commands, real pipe, no daemon and no HTTP.
//!
//! **Choose the command under test carefully.** mash dispatches by *basename*,
//! so `/usr/bin/head`, `/usr/bin/cat`, and `/usr/bin/sed` do not spawn anything
//! -- they resolve to in-process tools in `malt-tools`. Testing "external
//! processes" with those names measures the tool path instead, and produces a
//! confident, entirely wrong conclusion; that is exactly what happened while
//! this was being written. `external_interpreter()` below deliberately picks a
//! program with no in-process equivalent.

use mash::env::Env;
use std::io::Write;

/// Run `script` with an endless pipe at fd 0 pre-loaded with `INPUT`.
fn run(script: &str) -> (i32, String) {
    let mut env = Env::from_os();
    env.set_interactive(true);
    let (read_end, mut write_end) = malt_platform::io::create_pipe().expect("pipe");
    write_end.write_all(b"through-external\n").expect("write");
    write_end.flush().expect("flush");
    // Held open for the whole run, exactly like a session: this must not EOF.
    std::mem::forget(write_end);
    env.register_endless_fd(0, read_end);

    let commands = mash::parser::parse(script).expect("script should parse");
    let result = mash::executor::execute_list(&commands, script, &mut env);
    (
        result.exit_code,
        String::from_utf8_lossy(&result.stdout).into_owned(),
    )
}

/// An interpreter that genuinely spawns, i.e. is not shadowed by a tool.
///
/// Panics rather than skipping if none is present. A test that quietly passes
/// when it could not run is worse than no test -- it reports success for a
/// path it never touched.
fn external_interpreter() -> &'static str {
    for candidate in ["python3", "python", "perl"] {
        let probe = format!("{candidate} --version");
        let commands = mash::parser::parse(&probe).expect("probe should parse");
        let mut env = Env::from_os();
        if mash::executor::execute_list(&commands, &probe, &mut env).exit_code == 0 {
            return candidate;
        }
    }
    panic!("no external interpreter (python3/python/perl) found; this test needs one");
}

/// The motivating case of spec 005: an external program's prompt is answerable.
#[test]
fn an_external_process_reads_a_registered_fd_zero() {
    let interpreter = external_interpreter();
    let script = if interpreter == "perl" {
        "perl -e 'my $l = <STDIN>; chomp $l; print \"got=[$l]\\n\"'".to_string()
    } else {
        format!(
            "{interpreter} -c \"import sys; print('got=[' + sys.stdin.readline().strip() + ']')\""
        )
    };

    let (code, out) = run(&script);
    assert_eq!(code, 0, "interpreter should exit cleanly, got {out:?}");
    assert!(
        out.contains("got=[through-external]"),
        "an external process should read the registered fd 0, got {out:?}"
    );
}

/// The `read` builtin reads the same descriptor in-process.
#[test]
fn the_read_builtin_reads_a_registered_fd_zero() {
    let (_, out) = run("read -r X; echo builtin=[$X]");
    assert!(
        out.contains("builtin=[through-external]"),
        "the read builtin should see client input, got {out:?}"
    );
}

/// A pipeline still feeds an in-process tool, even though fd 0 is endless.
///
/// This is the guard against "fixing" the tool gap by making tools ignore
/// stdin wholesale: an explicit pipeline source must keep working.
#[test]
fn a_pipeline_still_feeds_an_in_process_tool() {
    let (_, out) = run("echo piped-in | /usr/bin/head -n1");
    assert!(
        out.contains("piped-in"),
        "a pipeline should still feed the tool, got {out:?}"
    );
}

/// Known gap, asserted so it is visible and cannot regress silently.
///
/// In-process tools take a pre-read `&[u8]` (`ToolFn = fn(&[String], &[u8])`),
/// so they cannot consume a stream that has not ended. Against a session's
/// endless stdin they are handed an empty buffer instead of hanging. When
/// streaming tools land, this test should be inverted, not deleted.
/// See docs/BACKLOG.md.
#[test]
fn an_in_process_tool_currently_sees_empty_session_stdin() {
    let (_, out) = run("/usr/bin/head -n1");
    assert!(
        !out.contains("through-external"),
        "if a tool now reads session stdin, invert this test and update the backlog; got {out:?}"
    );
}
