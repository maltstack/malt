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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const TIMEOUT_SECS: u64 = 10;

/// Tests that are genuinely impossible on Windows due to missing kernel features.
#[cfg(windows)]
const SKIP_ON_WINDOWS: &[&str] = &[
    "builtin.exec.modernish.mkfifo.loop",
    "semantics.redir.devfd.input",
    "semantics.redir.devfd.output",
    "semantics.expansion.quotes.adjacent",
    "semantics.monitoring.ttou",
];

#[cfg(not(windows))]
const SKIP_ON_WINDOWS: &[&str] = &[];

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

struct TestOutcome {
    name: String,
    passed: bool,
    skipped: bool,
    failure_reason: Option<String>,
}

fn run_test(name: &str, test_dir: &Path, mash: &Path) -> TestOutcome {
    if SKIP_ON_WINDOWS.contains(&name) {
        return TestOutcome {
            name: name.to_string(),
            passed: false,
            skipped: true,
            failure_reason: None,
        };
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
            return TestOutcome {
                name: name.to_string(),
                passed: false,
                skipped: false,
                failure_reason: Some(format!("could not read test file: {e}")),
            };
        }
    };

    let test_tmpdir = env::temp_dir().join(format!(
        "smoosh_{}_{}",
        std::process::id(),
        name.replace('.', "_")
    ));
    if let Err(e) = fs::create_dir_all(&test_tmpdir) {
        return TestOutcome {
            name: name.to_string(),
            passed: false,
            skipped: false,
            failure_reason: Some(format!("could not create tmpdir: {e}")),
        };
    }

    #[cfg(windows)]
    let path_val = env::var("PATH").unwrap_or_default();
    #[cfg(not(windows))]
    let path_val = env::var("PATH").unwrap_or_default();

    let mash_str = mash.to_string_lossy().into_owned();

    let child = Command::new(&mash_str)
        .arg("-c")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(&test_tmpdir)
        .env_clear()
        .env("PATH", &path_val)
        .env(
            "HOME",
            env::var("HOME").unwrap_or_else(|_| test_tmpdir.to_string_lossy().to_string()),
        )
        .env("TERM", "dumb")
        .env("TEST_SHELL", &mash_str)
        .env(
            "PWD",
            test_tmpdir.to_string_lossy().to_string().replace('\\', "/"),
        )
        .spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_dir_all(&test_tmpdir);
            return TestOutcome {
                name: name.to_string(),
                passed: false,
                skipped: false,
                failure_reason: Some(format!("failed to spawn mash: {e}")),
            };
        }
    };

    let output = wait_with_timeout(child, Duration::from_secs(TIMEOUT_SECS));
    let _ = fs::remove_dir_all(&test_tmpdir);

    let (got_stdout, got_ec) = match output {
        Some(out) => (
            normalize_newlines(&String::from_utf8_lossy(&out.stdout)),
            out.status.code().unwrap_or(-1),
        ),
        None => {
            return TestOutcome {
                name: name.to_string(),
                passed: false,
                skipped: false,
                failure_reason: Some(format!("TIMEOUT after {TIMEOUT_SECS}s")),
            };
        }
    };

    let stdout_ok = !check_stdout || got_stdout == expected_out;
    let passed = stdout_ok && got_ec == expected_ec;
    let failure_reason = if passed {
        None
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
        Some(reason)
    };

    TestOutcome {
        name: name.to_string(),
        passed,
        skipped: false,
        failure_reason,
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
    let mut failures: Vec<(String, String)> = Vec::new();

    eprintln!("\nRunning {} Smoosh tests...", total);

    for name in &test_names {
        let outcome = run_test(name, &test_dir, &mash);
        if outcome.skipped {
            skipped += 1;
            eprint!("s");
        } else if outcome.passed {
            passed += 1;
            eprint!(".");
        } else {
            failures.push((
                outcome.name.clone(),
                outcome.failure_reason.unwrap_or_default(),
            ));
            eprint!("F");
        }
    }
    eprintln!();

    let ran = total - skipped;
    println!("\n========================================");
    println!("Smoosh Results: {passed}/{ran} passed ({skipped} skipped on Windows)");
    println!("========================================");

    if !failures.is_empty() {
        println!("\nFailing tests:");
        for (name, reason) in &failures[..10.min(failures.len())] {
            println!("  FAIL  {name}");
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
