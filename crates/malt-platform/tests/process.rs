//! Integration tests for process spawning.

use malt_platform::process::{spawn, ExitStatus, Io, ProcessGroup, SpawnConfig, SpawnError};
use std::io::Read;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Platform-appropriate echo command that writes "hello" to stdout.
fn echo_config() -> SpawnConfig {
    #[cfg(windows)]
    {
        SpawnConfig::new("cmd.exe")
            .args(["/C", "echo hello"])
            .stdout(Io::Pipe)
            .stderr(Io::Null)
    }
    #[cfg(unix)]
    {
        SpawnConfig::new("/bin/echo")
            .arg("hello")
            .stdout(Io::Pipe)
            .stderr(Io::Null)
    }
}

/// Platform-appropriate long-running command (sleeps for 60 seconds).
fn sleep_config() -> SpawnConfig {
    #[cfg(windows)]
    {
        SpawnConfig::new("cmd.exe")
            .args(["/C", "timeout /T 60 /NOBREAK >nul"])
            .stdin(Io::Null)
            .stdout(Io::Null)
            .stderr(Io::Null)
    }
    #[cfg(unix)]
    {
        SpawnConfig::new("/bin/sleep")
            .arg("60")
            .stdin(Io::Null)
            .stdout(Io::Null)
            .stderr(Io::Null)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[test]
fn spawn_echo_capture_stdout() {
    let mut child = spawn(echo_config()).expect("failed to spawn echo");
    assert!(child.pid() > 0, "pid should be positive");

    let mut stdout = child.take_stdout().expect("stdout pipe should exist");
    let status = child.wait().expect("wait should succeed");
    assert!(status.success(), "echo should exit 0, got {status}");

    let mut buf = String::new();
    stdout.read_to_string(&mut buf).expect("read stdout");
    assert!(
        buf.trim().contains("hello"),
        "stdout should contain 'hello', got: {buf:?}"
    );
}

#[test]
fn spawn_nonexistent_binary() {
    let config = SpawnConfig::new("/nonexistent/binary/that/does/not/exist/xyzzy")
        .stdout(Io::Null)
        .stderr(Io::Null);

    let result = spawn(config);
    assert!(result.is_err(), "spawning nonexistent binary should fail");
    match result.unwrap_err() {
        SpawnError::NotFound { path } => {
            assert!(
                path.to_string_lossy().contains("xyzzy"),
                "error should contain program path"
            );
        }
        other => panic!("expected NotFound, got: {other}"),
    }
}

#[test]
fn spawn_with_null_io() {
    #[cfg(windows)]
    let config = SpawnConfig::new("cmd.exe")
        .args(["/C", "echo hi"])
        .stdin(Io::Null)
        .stdout(Io::Null)
        .stderr(Io::Null);
    #[cfg(unix)]
    let config = SpawnConfig::new("/bin/echo")
        .arg("hi")
        .stdin(Io::Null)
        .stdout(Io::Null)
        .stderr(Io::Null);

    let mut child = spawn(config).expect("spawn with null IO should succeed");
    assert!(child.stdin.is_none(), "stdin should be None for Io::Null");
    assert!(child.stdout.is_none(), "stdout should be None for Io::Null");
    assert!(child.stderr.is_none(), "stderr should be None for Io::Null");

    let status = child.wait().expect("wait should succeed");
    assert!(status.success(), "process should exit 0");
}

#[test]
fn try_wait_returns_none_for_running() {
    let mut child = spawn(sleep_config()).expect("failed to spawn sleep");

    // The process just started — it should still be running.
    let result = child.try_wait().expect("try_wait should not error");
    assert!(
        result.is_none(),
        "try_wait should return None for a running process"
    );

    // Kill the process so we don't leave it dangling.
    #[cfg(windows)]
    {
        // On Windows, use TerminateProcess via the signals module or just drop.
        // We can use the platform signal to terminate.
        let _ = malt_platform::signals::send_signal(
            child.pid(),
            malt_platform::signals::SignalKind::Term,
        );
    }
    #[cfg(unix)]
    {
        let _ = malt_platform::signals::send_signal(
            child.pid(),
            malt_platform::signals::SignalKind::Term,
        );
    }

    // Wait for cleanup.
    let _ = child.wait();
}

#[test]
fn spawn_new_process_group() {
    #[cfg(windows)]
    let config = SpawnConfig::new("cmd.exe")
        .args(["/C", "echo pg"])
        .stdout(Io::Null)
        .stderr(Io::Null)
        .process_group(ProcessGroup::New);
    #[cfg(unix)]
    let config = SpawnConfig::new("/bin/echo")
        .arg("pg")
        .stdout(Io::Null)
        .stderr(Io::Null)
        .process_group(ProcessGroup::New);

    let mut child = spawn(config).expect("spawn with new process group should succeed");
    let status = child.wait().expect("wait should succeed");
    assert!(status.success(), "process should exit 0");
}

#[test]
fn exit_status_properties() {
    let success = ExitStatus::from_raw(0);
    assert!(success.success());
    assert_eq!(success.code(), 0);

    let failure = ExitStatus::from_raw(1);
    assert!(!failure.success());
    assert_eq!(failure.code(), 1);

    #[cfg(unix)]
    {
        let signal_kill = ExitStatus::from_raw(137); // 128 + 9 (SIGKILL)
        assert!(!signal_kill.success());
        assert_eq!(signal_kill.signal(), Some(9));

        let normal = ExitStatus::from_raw(42);
        assert_eq!(normal.signal(), None);
    }
}

#[test]
fn spawn_with_env() {
    #[cfg(windows)]
    let config = SpawnConfig::new("cmd.exe")
        .args(["/C", "echo %MALT_TEST_VAR%"])
        .env("MALT_TEST_VAR", "test_value_42")
        .stdout(Io::Pipe)
        .stderr(Io::Null);
    #[cfg(unix)]
    let config = SpawnConfig::new("/bin/sh")
        .args(["-c", "echo $MALT_TEST_VAR"])
        .env("MALT_TEST_VAR", "test_value_42")
        .stdout(Io::Pipe)
        .stderr(Io::Null);

    let mut child = spawn(config).expect("spawn with env should succeed");
    let mut stdout = child.take_stdout().expect("stdout pipe");
    let status = child.wait().expect("wait");
    assert!(status.success());

    let mut buf = String::new();
    stdout.read_to_string(&mut buf).expect("read stdout");
    assert!(
        buf.contains("test_value_42"),
        "stdout should contain env var value, got: {buf:?}"
    );
}

#[test]
fn spawn_config_builder() {
    let config = SpawnConfig::new("/bin/echo")
        .arg("hello")
        .args(["world", "foo"])
        .env("K", "V")
        .env_clear()
        .cwd("/tmp")
        .stdin(Io::Null)
        .stdout(Io::Pipe)
        .stderr(Io::Inherit)
        .process_group(ProcessGroup::New);

    assert_eq!(config.program.to_str(), Some("/bin/echo"));
    assert_eq!(config.args.len(), 3);
    assert!(config.env_clear);
    assert_eq!(config.cwd.as_ref().and_then(|p| p.to_str()), Some("/tmp"));
    assert_eq!(config.process_group, ProcessGroup::New);
}
