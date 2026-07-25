//! An endless fd 0 must not stall commands that never read stdin.
//!
//! A terminal session's stdin is a pipe whose write end the daemon holds open
//! for the session's lifetime, so it never reaches EOF. In-process tools take
//! a pre-read `&[u8]`, so their dispatch slurps fd 0 -- which against that pipe
//! never returns, hanging every tool in the session.
//!
//! This drives real commands and compares wall-clock time with and without an
//! endless fd 0 registered. It is written that way deliberately: the bug is a
//! *hang*, so a test that merely inspected `is_fd_endless` would have passed
//! while two of the three tool-dispatch sites were still unguarded -- which is
//! exactly what happened the first time.

use mash::env::Env;
use std::time::{Duration, Instant};

/// Run `script`, optionally with an endless pipe at fd 0, and time it.
fn run(script: &str, endless_stdin: bool) -> Duration {
    let mut env = Env::from_os();
    env.set_interactive(true);
    if endless_stdin {
        let (read_end, write_end) = malt_platform::io::create_pipe().expect("pipe");
        // Hold the write end open for the whole run so the pipe never EOFs,
        // which is precisely the session case.
        std::mem::forget(write_end);
        env.register_endless_fd(0, read_end);
        assert!(env.is_fd_endless(0), "fd 0 should be marked endless");
    }
    let commands = mash::parser::parse(script).expect("script should parse");
    let start = Instant::now();
    let _ = mash::executor::execute_list(&commands, script, &mut env);
    start.elapsed()
}

/// Commands that do not read stdin must be unaffected by an endless fd 0.
///
/// `which` and `sleep` are in-process tools reached by different dispatch
/// routes; `echo` is a builtin, and is here as the control that kept passing
/// even while every tool site was broken.
#[test]
fn an_endless_fd_zero_does_not_stall_commands_that_ignore_stdin() {
    for script in ["echo hi", "which ls", "sleep 1"] {
        let plain = run(script, false);
        let endless = run(script, true);
        assert!(
            endless < plain + Duration::from_secs(2),
            "`{script}` took {endless:?} with an endless fd 0 but only {plain:?} without it; \
             a tool dispatch site is slurping stdin to EOF"
        );
    }
}

/// An explicit redirect still wins: marking fd 0 endless must not make the
/// shell ignore stdin the script actually asked for.
#[test]
fn an_explicit_stdin_redirect_is_still_honoured() {
    let elapsed = run("cat < /dev/null", true);
    assert!(
        elapsed < Duration::from_secs(2),
        "an explicit redirect should be read normally, took {elapsed:?}"
    );
}
