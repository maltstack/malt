//! Native cross-platform Smoosh POSIX conformance runner for MASH.
//!
//! Runs each `.test` file from `tests/shell_suites/smoosh/shell/` against the mash binary,
//! compares stdout and exit code against the corresponding `.out` and `.ec` files.
//!
//! Usage:
//!   cargo test -p mash --test smoosh_runner
//!
//! Override mash path:
//!   MASH=/path/to/mash cargo test -p mash --test smoosh_runner

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Shell,
    Harness,
}

/// Tests that are genuinely impossible on Windows due to missing kernel features.
#[cfg(windows)]
const WINDOWS_UNSUPPORTED_TESTS: &[&str] = &[
    "builtin.exec.modernish.mkfifo.loop",
    "semantics.expansion.quotes.adjacent",
    "semantics.monitoring.ttou",
];

#[cfg(not(windows))]
const WINDOWS_UNSUPPORTED_TESTS: &[&str] = &[];

fn mash_binary() -> PathBuf {
    if let Ok(path) = env::var("MASH") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("CARGO_BIN_EXE_mash") {
        return PathBuf::from(path);
    }
    let mut exe = env::current_exe().expect("could not find current exe");
    exe.pop();
    #[cfg(windows)]
    exe.push("mash.exe");
    #[cfg(not(windows))]
    exe.push("mash");
    exe
}

fn test_data_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("tests/shell_suites/smoosh/shell")
}

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

const HELPER_NAMES: &[&str] = &["getenv", "argv", "readdir", "fds"];

fn helper_inventory() -> &'static [&'static str] {
    HELPER_NAMES
}

fn helper_dir_for(test_tmpdir: &Path) -> PathBuf {
    test_tmpdir.join("test-util")
}

fn staged_script_path(test_tmpdir: &Path, test_name: &str) -> PathBuf {
    test_tmpdir.join(format!("{test_name}.test"))
}

fn helper_executable_name(name: &str) -> String {
    #[cfg(windows)]
    {
        format!("{name}.exe")
    }

    #[cfg(not(windows))]
    {
        name.to_string()
    }
}

fn helper_alias_name(name: &str) -> Option<String> {
    #[cfg(windows)]
    {
        Some(name.to_string())
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

fn helper_env_binary_path(name: &str) -> Option<PathBuf> {
    match name {
        "getenv" => option_env!("CARGO_BIN_EXE_getenv").map(PathBuf::from),
        "argv" => option_env!("CARGO_BIN_EXE_argv").map(PathBuf::from),
        "readdir" => option_env!("CARGO_BIN_EXE_readdir").map(PathBuf::from),
        "fds" => option_env!("CARGO_BIN_EXE_fds").map(PathBuf::from),
        _ => None,
    }
}

fn helper_binary_fallback_candidates(current_test_exe: &Path, helper_name: &str) -> Vec<PathBuf> {
    let helper_executable = helper_executable_name(helper_name);
    let mut candidates = Vec::new();

    if let Some(test_dir) = current_test_exe.parent() {
        candidates.push(test_dir.join(&helper_executable));
        if let Some(target_dir) = test_dir.parent() {
            candidates.push(target_dir.join(&helper_executable));
        }
    }

    candidates
}

fn resolve_helper_binary_path_for(
    helper_name: &str,
    env_candidate: Option<PathBuf>,
    current_test_exe: &Path,
) -> Result<PathBuf, String> {
    let mut checked = Vec::new();

    if let Some(path) = env_candidate {
        checked.push(path.display().to_string());
        if path.exists() {
            return Ok(path);
        }
    }

    for candidate in helper_binary_fallback_candidates(current_test_exe, helper_name) {
        checked.push(candidate.display().to_string());
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "helper {helper_name} could not be resolved from cargo test context; checked {}",
        checked.join(", ")
    ))
}

fn resolve_helper_binary_path(helper_name: &str) -> Result<PathBuf, String> {
    let current_test_exe =
        env::current_exe().map_err(|e| format!("locate current test executable: {e}"))?;
    resolve_helper_binary_path_for(
        helper_name,
        helper_env_binary_path(helper_name),
        &current_test_exe,
    )
}

fn stage_helpers(test_tmpdir: &Path) -> Result<PathBuf, String> {
    let helper_dir = helper_dir_for(test_tmpdir);
    fs::create_dir_all(&helper_dir)
        .map_err(|e| format!("create helper dir {}: {e}", helper_dir.display()))?;

    for helper_name in helper_inventory() {
        stage_helper(&helper_dir, helper_name)?;
    }

    Ok(helper_dir)
}

fn stage_test_script(test_tmpdir: &Path, test_name: &str, script: &str) -> Result<PathBuf, String> {
    let script_path = staged_script_path(test_tmpdir, test_name);
    fs::write(&script_path, script)
        .map_err(|e| format!("write staged test script {}: {e}", script_path.display()))?;
    Ok(script_path)
}

fn stage_helper(helper_dir: &Path, helper_name: &str) -> Result<(), String> {
    let source = resolve_helper_binary_path(helper_name)?;
    let target = helper_dir.join(helper_executable_name(helper_name));
    fs::copy(&source, &target).map_err(|e| {
        format!(
            "copy helper {} from {} to {}: {e}",
            helper_name,
            source.display(),
            target.display()
        )
    })?;

    if let Some(alias) = helper_alias_name(helper_name) {
        let alias_target = helper_dir.join(alias);
        fs::copy(&source, &alias_target).map_err(|e| {
            format!(
                "copy helper alias {} from {} to {}: {e}",
                helper_name,
                source.display(),
                alias_target.display()
            )
        })?;
    }

    Ok(())
}

fn is_windows_unsupported(name: &str) -> bool {
    #[cfg(windows)]
    {
        WINDOWS_UNSUPPORTED_TESTS.contains(&name)
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

fn render_failure_label(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Harness => "HARNESS",
        FailureKind::Shell => "SHELL",
    }
}

fn build_runner_env(
    test_tmpdir: &Path,
    helper_dir: &Path,
    mash: &Path,
    inherited_path: &str,
) -> Vec<(String, String)> {
    vec![
        ("PATH".to_string(), inherited_path.to_string()),
        (
            "HOME".to_string(),
            env::var("HOME").unwrap_or_else(|_| test_tmpdir.to_string_lossy().to_string()),
        ),
        ("TERM".to_string(), "dumb".to_string()),
        (
            "TEST_SHELL".to_string(),
            mash.to_string_lossy().into_owned(),
        ),
        (
            "TEST_UTIL".to_string(),
            helper_dir.to_string_lossy().into_owned(),
        ),
        (
            "PWD".to_string(),
            test_tmpdir.to_string_lossy().to_string().replace('\\', "/"),
        ),
    ]
}

fn render_summary(
    discovered: usize,
    runnable: usize,
    passed: usize,
    skipped_unsupported: usize,
    harness_failures: usize,
    shell_failures: usize,
) -> String {
    format!(
        "\n========================================\n\
         Smoosh Results\n\
         ========================================\n\
         discovered: {discovered}\n\
         runnable: {runnable}\n\
         passed: {passed}\n\
         skipped unsupported: {skipped_unsupported}\n\
         harness failures: {harness_failures}\n\
         shell failures: {shell_failures}\n"
    )
}

enum TestOutcome {
    Passed,
    SkippedUnsupported,
    Failed {
        name: String,
        kind: FailureKind,
        reason: String,
    },
}

impl TestOutcome {
    fn passed() -> Self {
        Self::Passed
    }

    fn skipped_unsupported() -> Self {
        Self::SkippedUnsupported
    }

    fn failed(name: &str, kind: FailureKind, reason: impl Into<String>) -> Self {
        Self::Failed {
            name: name.to_string(),
            kind,
            reason: reason.into(),
        }
    }
}

fn run_test(name: &str, test_dir: &Path, mash: &Path) -> TestOutcome {
    if is_windows_unsupported(name) {
        return TestOutcome::skipped_unsupported();
    }

    let script_path = test_dir.join(format!("{name}.test"));
    let out_path = test_dir.join(format!("{name}.out"));
    let check_stdout = out_path.exists();
    let expected_out = if check_stdout {
        normalize_newlines(&read_optional(&out_path))
    } else {
        String::new()
    };
    let expected_ec: i32 = read_optional(&test_dir.join(format!("{name}.ec")))
        .trim()
        .parse()
        .unwrap_or(0);

    let script = match fs::read_to_string(&script_path) {
        Ok(s) => s,
        Err(e) => {
            return TestOutcome::failed(
                name,
                FailureKind::Harness,
                format!("could not read test file: {e}"),
            );
        }
    };

    let test_tmpdir = match tempfile::Builder::new()
        .prefix(&format!("smoosh_{}_", name.replace('.', "_")))
        .tempdir_in(env::temp_dir())
    {
        Ok(dir) => dir,
        Err(e) => {
            return TestOutcome::failed(
                name,
                FailureKind::Harness,
                format!("could not create tmpdir: {e}"),
            );
        }
    };
    let test_tmpdir_path = test_tmpdir.path().to_path_buf();
    if let Err(e) = fs::create_dir_all(&test_tmpdir_path) {
        return TestOutcome::failed(
            name,
            FailureKind::Harness,
            format!("could not create tmpdir: {e}"),
        );
    }

    let helper_dir = match stage_helpers(&test_tmpdir_path) {
        Ok(dir) => dir,
        Err(e) => {
            return TestOutcome::failed(
                name,
                FailureKind::Harness,
                format!("could not stage helpers: {e}"),
            );
        }
    };

    let staged_script = match stage_test_script(&test_tmpdir_path, name, &script) {
        Ok(path) => path,
        Err(e) => {
            return TestOutcome::failed(name, FailureKind::Harness, e);
        }
    };

    #[cfg(windows)]
    let path_val = env::var("PATH").unwrap_or_default();
    #[cfg(not(windows))]
    let path_val = env::var("PATH").unwrap_or_default();

    let runner_env = build_runner_env(&test_tmpdir_path, &helper_dir, mash, &path_val);
    let mash_str = mash.to_string_lossy().into_owned();

    let mut command = Command::new(&mash_str);
    command
        .arg(staged_script.file_name().unwrap_or_default())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(&test_tmpdir_path)
        .env_clear();

    for (key, value) in runner_env {
        command.env(key, value);
    }

    let child = command.spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return TestOutcome::failed(
                name,
                FailureKind::Harness,
                format!("failed to spawn mash: {e}"),
            );
        }
    };

    let output = wait_with_timeout(child, Duration::from_secs(TIMEOUT_SECS));

    let (got_stdout, got_ec) = match output {
        Some(out) => (
            normalize_newlines(&String::from_utf8_lossy(&out.stdout)),
            out.status.code().unwrap_or(-1),
        ),
        None => {
            return TestOutcome::failed(
                name,
                FailureKind::Harness,
                format!("TIMEOUT after {TIMEOUT_SECS}s"),
            );
        }
    };

    let stdout_ok = !check_stdout || got_stdout == expected_out;
    let passed = stdout_ok && got_ec == expected_ec;
    if passed {
        TestOutcome::passed()
    } else {
        let mut reason = String::new();
        if got_ec != expected_ec {
            reason.push_str(&format!("exit: got={got_ec} want={expected_ec}  "));
        }
        if !stdout_ok {
            reason.push_str(&format!(
                "stdout: got={:?} want={:?}",
                truncate(&got_stdout, 80),
                truncate(&expected_out, 80)
            ));
        }
        TestOutcome::failed(name, FailureKind::Shell, reason)
    }
}

fn wait_with_timeout(
    child: std::process::Child,
    timeout: Duration,
) -> Option<std::process::Output> {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out = child.wait_with_output();
        let _ = tx.send(out);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => Some(out),
        Ok(Err(_)) | Err(_) => None,
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[test]
fn windows_skip_policy_contains_exactly_three_tests() {
    #[cfg(windows)]
    {
        assert_eq!(WINDOWS_UNSUPPORTED_TESTS.len(), 3);
        assert_eq!(
            WINDOWS_UNSUPPORTED_TESTS,
            &[
                "builtin.exec.modernish.mkfifo.loop",
                "semantics.expansion.quotes.adjacent",
                "semantics.monitoring.ttou",
            ]
        );
    }
}

#[test]
fn helper_dependent_tests_are_not_windows_skips() {
    #[cfg(windows)]
    {
        assert!(!is_windows_unsupported("builtin.export.override"));
        assert!(!is_windows_unsupported("semantics.command.argv0"));
    }
}

#[test]
fn redir_devfd_tests_are_not_current_suite_entries() {
    let test_dir = test_data_dir();
    assert!(!test_dir.join("semantics.redir.devfd.input.test").exists());
    assert!(!test_dir.join("semantics.redir.devfd.output.test").exists());
}

#[test]
fn helper_stage_dir_is_under_test_tmpdir() {
    let tmp = std::env::temp_dir().join("mash_runner_helper_stage_dir_test");
    let helper_dir = helper_dir_for(&tmp);
    assert!(helper_dir.starts_with(&tmp));
    assert_eq!(
        helper_dir.file_name().and_then(|s| s.to_str()),
        Some("test-util")
    );
}

#[test]
fn staged_script_is_written_inside_test_tmpdir() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let script_path =
        stage_test_script(tmp.path(), "sample.testcase", "echo ok\n").expect("stage script");
    assert_eq!(script_path, tmp.path().join("sample.testcase.test"));
    assert_eq!(
        fs::read_to_string(&script_path).expect("read staged script"),
        "echo ok\n"
    );
}

#[test]
fn helper_inventory_includes_getenv() {
    let helpers = helper_inventory();
    assert!(helpers.contains(&"getenv"));
}

#[test]
fn helper_inventory_contains_expected_windows_helper_set() {
    let helpers = helper_inventory();
    assert_eq!(helpers, ["getenv", "argv", "readdir", "fds"]);
}

#[test]
fn helper_stage_uses_binary_artifact_names() {
    #[cfg(windows)]
    {
        assert_eq!(helper_executable_name("getenv"), "getenv.exe");
        assert_eq!(helper_alias_name("getenv").as_deref(), Some("getenv"));
    }

    #[cfg(not(windows))]
    {
        assert_eq!(helper_executable_name("getenv"), "getenv");
        assert_eq!(helper_alias_name("getenv"), None);
    }
}

#[test]
fn helper_binary_fallback_candidates_derive_from_current_test_exe() {
    let current_test_exe = PathBuf::from("C:/repo/target/debug/deps/smoosh_runner.exe");
    let candidates = helper_binary_fallback_candidates(&current_test_exe, "getenv");
    assert_eq!(
        candidates,
        vec![
            PathBuf::from("C:/repo/target/debug/deps/getenv.exe"),
            PathBuf::from("C:/repo/target/debug/getenv.exe"),
        ]
    );
}

#[test]
fn helper_resolution_prefers_cargo_bin_exe_path() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let env_helper = temp.path().join(helper_executable_name("getenv"));
    fs::write(&env_helper, b"env").expect("write env helper");

    let current_test_exe = PathBuf::from("C:/repo/target/debug/deps/smoosh_runner.exe");
    let resolved =
        resolve_helper_binary_path_for("getenv", Some(env_helper.clone()), &current_test_exe)
            .expect("resolve helper from cargo env");

    assert_eq!(resolved, env_helper);
}

#[test]
fn helper_resolution_falls_back_to_target_layout_when_env_path_missing() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let target_debug = temp.path().join("target").join("debug");
    let deps_dir = target_debug.join("deps");
    fs::create_dir_all(&deps_dir).expect("create deps dir");

    let current_test_exe = deps_dir.join("smoosh_runner.exe");
    fs::write(&current_test_exe, b"test").expect("write current test exe");

    let helper = target_debug.join(helper_executable_name("getenv"));
    fs::write(&helper, b"helper").expect("write helper");

    let missing_env = temp.path().join("missing-helper.exe");
    let resolved = resolve_helper_binary_path_for("getenv", Some(missing_env), &current_test_exe)
        .expect("resolve helper from target layout");

    assert_eq!(resolved, helper);
}

#[test]
fn helper_resolution_error_mentions_cargo_test_context() {
    let current_test_exe = PathBuf::from("C:/repo/target/debug/deps/smoosh_runner.exe");
    let error = resolve_helper_binary_path_for("getenv", None, &current_test_exe)
        .expect_err("expected unresolved helper");

    assert!(error.contains("cargo test context"));
    assert!(error.contains("getenv.exe"));
}

#[test]
fn harness_failure_reason_is_tagged() {
    assert_eq!(render_failure_label(FailureKind::Harness), "HARNESS");
}

#[test]
fn shell_failure_reason_is_tagged() {
    assert_eq!(render_failure_label(FailureKind::Shell), "SHELL");
}

#[test]
fn test_util_env_points_to_staged_helper_dir() {
    let tmp = std::env::temp_dir().join("mash_runner_test_util_env_test");
    let helper_dir = helper_dir_for(&tmp);
    let envs = build_runner_env(
        &tmp,
        &helper_dir,
        Path::new("C:/mash.exe"),
        "C:/windows/system32",
    );
    let test_util = envs
        .iter()
        .find(|(k, _)| k == "TEST_UTIL")
        .map(|(_, v)| v.clone());

    assert_eq!(
        test_util.as_deref(),
        Some(helper_dir.to_string_lossy().as_ref())
    );
}

#[test]
fn summary_includes_harness_and_shell_failure_counts() {
    let summary = render_summary(186, 183, 90, 3, 2, 91);

    assert!(summary.contains("discovered: 186"));
    assert!(summary.contains("runnable: 183"));
    assert!(summary.contains("passed: 90"));
    assert!(summary.contains("skipped unsupported: 3"));
    assert!(summary.contains("harness failures: 2"));
    assert!(summary.contains("shell failures: 91"));
}

#[test]
fn smoosh_conformance_tests() {
    let test_dir = test_data_dir();
    let mash = mash_binary();

    eprintln!("smoosh_runner: mash     = {}", mash.display());
    eprintln!("smoosh_runner: test_dir = {}", test_dir.display());

    if !mash.exists() {
        eprintln!(
            "ERROR: mash binary not found at {}. Run `cargo build --release -p mash` first.",
            mash.display()
        );
        panic!("mash binary not found");
    }

    if !test_dir.exists() {
        eprintln!(
            "ERROR: test data directory not found at {}.",
            test_dir.display()
        );
        panic!("test data directory not found");
    }

    let mut test_names: Vec<String> = fs::read_dir(&test_dir)
        .expect("cannot read test dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? == "test" {
                Some(p.file_stem()?.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    test_names.sort();

    let total = test_names.len();
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<(String, FailureKind, String)> = Vec::new();

    eprintln!("\nRunning {} Smoosh tests...", total);

    for name in &test_names {
        let outcome = run_test(name, &test_dir, &mash);
        match outcome {
            TestOutcome::Passed => {
                passed += 1;
                eprint!(".");
            }
            TestOutcome::SkippedUnsupported => {
                skipped += 1;
                eprint!("s");
            }
            TestOutcome::Failed { name, kind, reason } => {
                failures.push((name, kind, reason));
                eprint!("F");
            }
        }
    }
    eprintln!();

    let runnable = total - skipped;
    let harness_failures = failures
        .iter()
        .filter(|(_, kind, _)| *kind == FailureKind::Harness)
        .count();
    let shell_failures = failures
        .iter()
        .filter(|(_, kind, _)| *kind == FailureKind::Shell)
        .count();

    print!(
        "{}",
        render_summary(
            total,
            runnable,
            passed,
            skipped,
            harness_failures,
            shell_failures
        )
    );

    if !failures.is_empty() {
        println!("\nFailing tests:");
        for (name, kind, reason) in &failures[..10.min(failures.len())] {
            let label = render_failure_label(*kind);
            println!("  FAIL  [{label}] {name}");
            if !reason.is_empty() {
                println!("        {reason}");
            }
        }
        if failures.len() > 10 {
            println!("  ... and {} more failures", failures.len() - 10);
        }
    }

    assert!(
        failures.is_empty(),
        "{} Smoosh tests failed ({} passed, {} skipped)",
        failures.len(),
        passed,
        skipped
    );
}
