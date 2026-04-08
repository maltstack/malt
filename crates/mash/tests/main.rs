use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn shell_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> std::process::Output {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("poll child") {
            Some(_) => return child.wait_with_output().expect("wait with output"),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("collect timed out child");
                panic!(
                    "child timed out after {:?}\nstdout: {}\nstderr: {}",
                    timeout,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[test]
fn interactive_flag_executes_script_without_repl_banner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("script.sh");
    std::fs::write(&script, "echo ok\n").expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg("-i")
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("MASH 0.1.0"),
        "unexpected interactive banner in stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn interactive_flag_with_piped_stdin_suppresses_repl_banner_and_fails_on_parse_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mash");

    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b")\n")
        .expect("write input");

    let output = child.wait_with_output().expect("wait mash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("parse error"), "stderr: {stderr}");
    assert!(!stderr.contains("MASH 0.1.0"), "stderr: {stderr}");
}

#[test]
fn parse_error_stderr_can_be_captured_via_two_to_one_before_stdout_redirect() {
    let mash = shell_path(std::path::Path::new(env!("CARGO_BIN_EXE_mash")));
    let script = format!(
        "err=$({mash} -c ': ${{}}' 2>&1 >/dev/null); if [ \"$err\" ]; then printf '%s' \"$err\"; else printf 'EMPTY'; fi"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args(["-c", &script])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("EMPTY"),
        "stdout leaked instead of being captured: {stdout}"
    );
    assert!(
        stdout.contains("bad substitution"),
        "stdout: {} stderr: {}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg(not(windows))]
fn command_substitution_captures_modernish_style_putln_pipeline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("driver.sh");
    let out = dir.path().join("captured.txt");
    std::fs::write(
        &script,
        format!(
            "SIGPIPESTATUS=141\n\
             die() {{ :; }}\n\
             putln() {{\n\
               case $# in\n\
               ( 0 ) PATH=/bin command printf '\\n' ;;\n\
               ( * ) PATH=/bin command printf '%s\\n' \"$@\" ;;\n\
               esac || {{ let \"$? > 125 && $? != SIGPIPESTATUS\" && die \"putln: internal error\"; }}\n\
             }}\n\
             x=$(putln abcxyz | tr '[:lower:]' '[:upper:]' 2>/dev/null)\n\
             printf '%s' \"$x\" > {}\n",
            shell_path(&out)
        ),
    )
    .expect("write script");

    let child = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mash");
    let output = wait_with_timeout(child, Duration::from_secs(2));

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&out).expect("read captured output"),
        "ABCXYZ"
    );
}

#[test]
fn script_file_heredoc_before_output_redirect_executes_without_running_delimiter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("driver.sh");
    let child = dir.path().join("child.sh");
    std::fs::write(
        &script,
        format!(
            "cat <<EOF > {}\necho ok\nEOF\n{} {}\n",
            shell_path(&child),
            shell_path(std::path::Path::new(env!("CARGO_BIN_EXE_mash"))),
            shell_path(&child)
        ),
    )
    .expect("write driver");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[test]
#[cfg(not(windows))]
fn exec_with_args_in_pipeline_runs_target_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args(["-c", "printf 'foo\\n' | exec sed -n p"])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "foo\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
#[cfg(windows)]
fn interactive_child_inherits_shell_fd_redirections_and_extra_fds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let driver = dir.path().join("driver.test");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    std::fs::write(
        &driver,
        format!(
            "cat >scr <<'EOF'\nfoo=bar\nreadonly -- foo\nreadonly -- baz=quux\necho $foo $baz >&3\nfoo=nope\nunset baz\necho $foo $baz >&3\nEOF\nexec 3>&1 1>/dev/null 2>/dev/null\n{} -i scr\n",
            shell_path(&mash)
        ),
    )
    .expect("write driver");

    let output = Command::new(&mash)
        .arg(shell_path(&driver))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "bar quux\nbar quux\n"
    );
}

#[test]
#[cfg(windows)]
fn path_looked_up_command_preserves_shell_token_as_argv0() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    let argv = std::path::PathBuf::from(env!("CARGO_BIN_EXE_argv"));
    let helper_dir = argv.parent().expect("helper dir");
    let path = format!(
        "{};{}",
        helper_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(&mash)
        .args(["-c", "argv"])
        .env_clear()
        .env("PATH", path)
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "argv[0] = \"argv\";\n"
    );
}

fn explicit_posix_bin_tool_path_runs_builtin_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));

    let output = Command::new(&mash)
        .args(["-c", "/bin/echo hello"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello\n");
}

#[test]
#[cfg(windows)]
fn path_looked_up_shell_script_runs_via_mash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    let script = dir.path().join("cmd.sh");
    std::fs::write(&script, "echo hi\n").expect("write script");

    let output = Command::new(&mash)
        .args(["-c", "cmd.sh"])
        .env_clear()
        .env("PATH", shell_path(dir.path()))
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hi\n");
}

#[test]
#[cfg(windows)]
fn path_looked_up_shell_script_does_not_leave_capture_tempfiles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    let script = dir.path().join("cmd.sh");
    std::fs::write(&script, "echo hi\n").expect("write script");

    let output = Command::new(&mash)
        .args(["-c", "cmd.sh"])
        .env_clear()
        .env("PATH", shell_path(dir.path()))
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let leaks: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("mash-shell-capture-"))
        .collect();
    assert!(leaks.is_empty(), "capture files leaked: {leaks:?}");
}

#[test]
#[cfg(windows)]
fn shell_capture_tempfiles_are_not_visible_in_working_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    std::fs::write(dir.path().join("visible.txt"), "ok\n").expect("write visible file");

    let output = Command::new(&mash)
        .args(["-c", "ls"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("visible.txt"), "stdout: {stdout}");
    assert!(!stdout.contains("mash-shell-capture-"), "stdout: {stdout}");
}

#[test]
#[cfg(windows)]
fn unreadable_script_file_fails_before_execution() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    let driver = dir.path().join("driver.sh");
    std::fs::write(
        &driver,
        format!(
            "echo 'echo nope' >scr\nchmod -r scr\n{} ./scr && exit 1\n",
            shell_path(&mash)
        ),
    )
    .expect("write driver");

    let output = Command::new(&mash)
        .arg(shell_path(&driver))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(126),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
#[cfg(windows)]
fn child_shell_reports_parent_shell_pid_via_ppid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let driver = dir.path().join("driver.sh");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    std::fs::write(
        &driver,
        format!(
            "echo $$ > shellpid\n{} -c 'echo $PPID' > ppid\ncat shellpid ppid\n",
            shell_path(&mash)
        ),
    )
    .expect("write driver");

    let output = Command::new(&mash)
        .arg(shell_path(&driver))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lines: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        lines[0],
        lines[1],
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
#[cfg(windows)]
fn backgrounded_child_shell_pid_matches_bang() {
    let dir = tempfile::tempdir().expect("tempdir");
    let driver = dir.path().join("driver.sh");
    let mash = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mash"));
    let child = dir.path().join("showpid.sh");
    std::fs::write(&child, "echo $$ > pid.out\n").expect("write child");
    std::fs::write(
        &driver,
        format!(
            "{} {} &\nsleep 1\ncat pid.out\necho $!\n",
            shell_path(&mash),
            shell_path(&child)
        ),
    )
    .expect("write driver");

    let output = Command::new(&mash)
        .arg(shell_path(&driver))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| dir.path().display().to_string()),
        )
        .env("TERM", "dumb")
        .env("PWD", shell_path(dir.path()))
        .env("TEMP", dir.path())
        .env("TMP", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lines: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        lines[0],
        lines[1],
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}


#[test]
fn c_option_sets_positional_arg0_and_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args([
            "-c",
            "printf '%s|%s|%s\\n' \"$0\" \"$1\" \"$#\"",
            "./scr",
            "arg1",
        ])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "./scr|arg1|1\n");
}

#[test]
fn script_file_sets_special_parameter_zero_to_script_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("scr");
    std::fs::write(&script, "printf '%s\\n' \"$0\"\n").expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{}\n", shell_path(&script))
    );
}

#[test]
#[cfg(not(windows))]
fn script_file_sets_ppid_to_nonzero_parent_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("ppid.sh");
    std::fs::write(
        &script,
        "case ${PPID-} in\n( '' | 0* | *[!0123456789]* ) printf 'BAD:%s\\n' \"${PPID-}\" ;;\n( * ) printf '%s\\n' \"$PPID\" ;;\nesac\n",
    )
    .expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    assert!(
        !trimmed.starts_with("BAD:"),
        "stdout: {} stderr: {}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    let ppid: u32 = trimmed
        .parse()
        .expect("PPID should be a non-zero decimal integer");
    assert!(ppid > 0, "stdout: {stdout}");
}

#[test]
#[cfg(not(windows))]
fn stale_mash_ppid_env_does_not_override_actual_parent_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer.sh");
    let parent_file = dir.path().join("parent.txt");
    let child_file = dir.path().join("child.txt");
    std::fs::write(
        &outer,
        format!(
            "printf '%s\\n' \"$$\" > \"{}\"\n{} -c 'printf \"%s\\n\" \"$PPID\"' > \"{}\"\n",
            shell_path(&parent_file),
            shell_path(std::path::Path::new(env!("CARGO_BIN_EXE_mash"))),
            shell_path(&child_file),
        ),
    )
    .expect("write outer script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .env("MASH_PPID", "11111")
        .arg(shell_path(&outer))
        .output()
        .expect("run outer mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = std::fs::read_to_string(&parent_file)
        .expect("read parent pid")
        .trim()
        .to_string();
    let child = std::fs::read_to_string(&child_file)
        .expect("read child ppid")
        .trim()
        .to_string();

    assert_eq!(child, parent, "child should see the actual outer shell pid");
    assert_ne!(child, "11111", "stale MASH_PPID should not win on unix");
}

#[test]
#[cfg(not(windows))]
fn pipeline_spawn_overrides_stale_mash_ppid_for_child_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer.sh");
    let parent_file = dir.path().join("parent.txt");
    let child_file = dir.path().join("child.txt");
    std::fs::write(
        &outer,
        format!(
            "printf '%s\\n' \"$$\" > \"{}\"\nprintf x | {} -c 'printf \"%s\\n\" \"$PPID\"' {} > \"{}\"\n",
            shell_path(&parent_file),
            shell_path(std::path::Path::new(env!("CARGO_BIN_EXE_mash"))),
            shell_path(std::path::Path::new(env!("CARGO_BIN_EXE_mash"))),
            shell_path(&child_file),
        ),
    )
    .expect("write outer script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .env("MASH_PPID", "11111")
        .arg(shell_path(&outer))
        .output()
        .expect("run outer mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = std::fs::read_to_string(&parent_file)
        .expect("read parent pid")
        .trim()
        .to_string();
    let child = std::fs::read_to_string(&child_file)
        .expect("read child ppid")
        .trim()
        .to_string();

    assert_eq!(child, parent, "pipeline child should see the actual outer shell pid");
    assert_ne!(child, "11111", "stale MASH_PPID should not leak through pipeline spawn");
}

#[test]
#[cfg(not(windows))]
fn cd_physical_option_with_double_dash_is_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args(["-c", "cd -P -- / && pwd", "mash"])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "/\n");
}

#[test]
#[cfg(not(windows))]
fn modernish_min_posix_gate_succeeds() {
    let min_posix =
        "cd -P -- / && ! { ! case x in ( x ) : ${0##*/} || : $( : ) ;; esac; }";
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args(["-c", min_posix, "mash"])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg(not(windows))]
fn allexport_option_exports_subsequent_assignments() {
    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .args([
            "-c",
            "set -o allexport; FOO=bar; export -p",
            "mash",
        ])
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("export FOO=\"bar\""),
        "stdout: {} stderr: {}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sourced_function_wrapper_can_set_caller_variable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let helper = dir.path().join("helper.sh");
    let script = dir.path().join("script.sh");
    std::fs::write(&helper, "_Msh_testFn() { DEFPATH=ok; }\n_Msh_testFn\n").expect("helper");
    std::fs::write(
        &script,
        format!(
            ". {}\nprintf '<%s>\\n' \"$DEFPATH\"\n",
            shell_path(&helper)
        ),
    )
    .expect("script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<ok>\n");
}

#[test]
fn script_file_trap_zero_runs_at_shell_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("driver.sh");
    std::fs::write(&script, "trap 'echo ZERO_OK' 0\n").expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ZERO_OK\n");
}

#[test]
fn sourced_trap_zero_is_visible_as_exit_and_runs_at_shell_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sourced = dir.path().join("trap.sh");
    let driver = dir.path().join("driver.sh");
    std::fs::write(&sourced, "trap 'echo ZERO_OK' 0\n").expect("write sourced script");
    std::fs::write(
        &driver,
        format!(
            ". {}\necho AFTER\ntrap -p EXIT\n",
            shell_path(&sourced)
        ),
    )
    .expect("write driver");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg(shell_path(&driver))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "AFTER\ntrap -- 'echo ZERO_OK' EXIT\nZERO_OK\n"
    );
}
