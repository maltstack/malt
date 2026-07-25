//! Lowest-layer probe: does an external process spawned by mash read a
//! registered fd 0? No daemon involved.
use mash::env::Env;
use std::io::Write;

fn run(script: &str) -> String {
    let mut env = Env::from_os();
    env.set_interactive(true);
    let (read_end, mut write_end) = malt_platform::io::create_pipe().expect("pipe");
    write_end.write_all(b"through-external\n").expect("write");
    write_end.flush().expect("flush");
    // Held open, exactly like a session: the pipe must not EOF.
    std::mem::forget(write_end);
    env.register_endless_fd(0, read_end);

    let cmds = mash::parser::parse(script).expect("parse");
    let r = mash::executor::execute_list(&cmds, script, &mut env);
    format!(
        "code={} out={:?} err={:?}",
        r.exit_code,
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    )
}

/// Documents an unmet requirement of spec 005/US2, executably.
///
/// Ignored because it fails today. It is kept rather than deleted so the gap
/// is reproducible in one command at the lowest layer -- no daemon, no HTTP:
///
/// ```text
/// cargo test -p mash --test external_stdin_probe -- --ignored --nocapture
/// ```
///
/// Expected output when this is fixed: all three lines produce their input.
/// Today the middle one is empty. See docs/BACKLOG.md.
#[test]
#[ignore = "unmet: external processes do not read a registered fd 0 (spec 005/US2)"]
fn an_external_process_reads_a_registered_fd_zero() {
    // `head -n1` exits after one line, so it terminates even on an endless pipe.
    for s in [
        "echo piped-in | /usr/bin/head -n1",
        "/usr/bin/head -n1",
        "read -r X; echo builtin=[$X]",
    ] {
        println!("{s:>28} -> {}", run(s));
    }
}
