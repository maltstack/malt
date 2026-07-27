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

/// Run `script` with a never-EOF pipe at fd 0 pre-loaded with input.
fn run_with_stdin_mode(script: &str, close_external_stdin: bool) -> (i32, String, String) {
    let mut env = Env::from_os();
    env.set_interactive(true);
    env.set_close_external_stdin(close_external_stdin);
    let (read_end, mut write_end) = malt_platform::io::create_pipe().expect("pipe");
    write_end.write_all(b"through-external\n").expect("write");
    write_end.flush().expect("flush");
    // Held open for the whole run, exactly like a session: this must not EOF,
    // so anything that reads it to the end would block forever.
    std::mem::forget(write_end);
    env.register_fd(0, read_end);

    let commands = mash::parser::parse(script).expect("script should parse");
    let result = mash::executor::execute_list(&commands, script, &mut env);
    (
        result.exit_code,
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
    )
}

fn run(script: &str) -> (i32, String, String) {
    run_with_stdin_mode(script, false)
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

    let (code, out, err) = run(&script);
    assert_eq!(
        code, 0,
        "interpreter should exit cleanly, got stdout={out:?}, stderr={err:?}"
    );
    assert!(
        out.contains("got=[through-external]"),
        "an external process should read the registered fd 0, got {out:?}"
    );
}

/// Gateway executions are one-shot, so their external children must not keep
/// the session input relay open. The preloaded pipe makes this a positive
/// assertion: the child must see EOF, not merely happen to finish.
#[test]
fn a_one_shot_external_process_receives_eof_instead_of_session_input() {
    let interpreter = external_interpreter();
    let script = if interpreter == "perl" {
        "perl -e 'my $l = <STDIN>; $l //= q{}; chomp $l; print \"got=[$l]\\n\"'".to_string()
    } else {
        format!(
            "{interpreter} -c \"import sys; data = '' if sys.stdin is None else sys.stdin.readline().strip(); print('got=[' + data + ']')\""
        )
    };

    let (code, out, err) = run_with_stdin_mode(&script, true);
    assert_eq!(
        code, 0,
        "one-shot interpreter should exit cleanly, got stdout={out:?}, stderr={err:?}"
    );
    assert!(
        out.contains("got=[]"),
        "a one-shot external process must receive EOF instead of session input, got {out:?}"
    );
}

/// Pipeline stage zero has no pipeline input of its own. It must therefore
/// receive the same one-shot EOF as an ordinary external command, while later
/// stages remain free to consume the pipe.
#[test]
fn a_one_shot_pipeline_first_stage_receives_eof() {
    let interpreter = external_interpreter();
    let script = if interpreter == "perl" {
        "perl -e 'my $l = <STDIN>; $l //= q{}; chomp $l; print \"got=[$l]\\n\"' | /usr/bin/head -n1"
            .to_string()
    } else {
        format!(
            "{interpreter} -c \"import sys; data = '' if sys.stdin is None else sys.stdin.readline().strip(); print('got=[' + data + ']')\" | /usr/bin/head -n1"
        )
    };

    let (code, out, err) = run_with_stdin_mode(&script, true);
    assert_eq!(
        code, 0,
        "one-shot pipeline should exit cleanly, got stdout={out:?}, stderr={err:?}"
    );
    assert!(
        out.contains("got=[]"),
        "a one-shot pipeline's first external stage must receive EOF, got {out:?}"
    );
}

/// The `read` builtin reads the same descriptor in-process.
#[test]
fn the_read_builtin_reads_a_registered_fd_zero() {
    let (_, out, _) = run("read -r X; echo builtin=[$X]");
    assert!(
        out.contains("builtin=[through-external]"),
        "the read builtin should see client input, got {out:?}"
    );
}

/// An explicit pipeline source still wins over the session's fd 0.
///
/// The guard against "fixing" stdin resolution by preferring one source
/// everywhere: a pipeline must still feed the tool.
#[test]
fn a_pipeline_still_feeds_an_in_process_tool() {
    let (_, out, _) = run("echo piped-in | /usr/bin/head -n1");
    assert!(
        out.contains("piped-in"),
        "a pipeline should still feed the tool, got {out:?}"
    );
}

/// In-process tools read session stdin too, and stop when they have enough.
///
/// This was the inverse assertion until tools became reader-based: `ToolFn`
/// took a finished `&[u8]`, so dispatch had to slurp fd 0 to EOF, and a
/// session's stdin has no EOF. Tools were handed an empty buffer to keep them
/// from hanging.
///
/// `head -n1` is the sharp case. It must return on its first line without
/// waiting for a second, which is only true because `head_reader` takes
/// exactly `num_lines` rather than reading one more and then breaking.
#[test]
fn an_in_process_tool_reads_session_stdin_and_stops_early() {
    let (_, out, _) = run("/usr/bin/head -n1");
    assert!(
        out.contains("through-external"),
        "an in-process tool should read client input, got {out:?}"
    );
}
