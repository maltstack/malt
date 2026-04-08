//! Correctness-focused conformance checks for mash.
//!
//! This test suite complements `smoosh_runner` with:
//! - differential fixed-script comparison against a reference POSIX shell
//! - generated (property-style) script differential checks
//! - Modernish capability probes (`shell_suites/modernish/cap/*.t`)
//! - optional full Modernish runner when a populated checkout is available
#![cfg_attr(windows, allow(dead_code, unused_imports))]

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const MAX_FAILURE_REPORTS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunResult {
    status: i32,
    stdout: String,
    stderr: String,
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn run_shell(shell: &Path, script: &str) -> Result<RunResult, String> {
    let output = Command::new(shell)
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("spawn {}: {e}", shell.display()))?;
    Ok(RunResult {
        status: output.status.code().unwrap_or(128),
        stdout: normalize_newlines(&String::from_utf8_lossy(&output.stdout)),
        stderr: normalize_newlines(&String::from_utf8_lossy(&output.stderr)),
    })
}

fn run_program<S: AsRef<OsStr>>(program: S, args: &[&str], cwd: &Path) -> Result<RunResult, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("spawn failed: {e}"))?;
    Ok(RunResult {
        status: output.status.code().unwrap_or(128),
        stdout: normalize_newlines(&String::from_utf8_lossy(&output.stdout)),
        stderr: normalize_newlines(&String::from_utf8_lossy(&output.stderr)),
    })
}

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

fn env_enabled(name: &str) -> bool {
    matches!(
        env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn require_correctness_enable() -> bool {
    if env_enabled("MASH_CORRECTNESS_ENABLE") {
        true
    } else {
        eprintln!(
            "set MASH_CORRECTNESS_ENABLE=1 to run correctness differential/property/modernish lanes"
        );
        false
    }
}

fn default_reference_shell() -> Option<PathBuf> {
    if let Ok(path) = env::var("MASH_DIFF_REF_SHELL") {
        return Some(PathBuf::from(path));
    }
    if cfg!(windows) {
        None
    } else {
        Some(PathBuf::from("dash"))
    }
}

fn sample_scripts() -> &'static [&'static str] {
    &[
        "a=1; echo $a",
        "unset a; echo ${a-unset}",
        "readonly a=1; a=2; echo $?",
        "set -- 'a b' c; printf '[%s]\\n' \"$1\" \"$2\"",
        "x=' a  b '; IFS=' '; set -- $x; echo $#",
        "printf '%s\\n' \"$(printf 'x\\n')\"",
        "f(){ echo hi; }; f",
        "echo one | cat",
        "x=abc; echo ${x%bc}",
        "case x in x) echo ok;; *) echo no;; esac",
        "for i in 1 2 3; do echo $i; done",
        "(echo subshell); echo parent",
    ]
}

fn compare_script_pair(mash: &Path, reference: &Path, script: &str) -> Result<(), String> {
    let m = run_shell(mash, script)?;
    let r = run_shell(reference, script)?;
    let compare_stderr = env_enabled("MASH_DIFF_COMPARE_STDERR");
    let same = if compare_stderr {
        m == r
    } else {
        m.status == r.status && m.stdout == r.stdout
    };
    if !same {
        return Err(format!(
            "script:\n{script}\n--- mash ---\nstatus={}\nstdout={:?}\nstderr={:?}\n--- ref ({}) ---\nstatus={}\nstdout={:?}\nstderr={:?}",
            m.status,
            m.stdout,
            m.stderr,
            reference.display(),
            r.status,
            r.stdout,
            r.stderr
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }
    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        let idx = (self.next() as usize) % items.len();
        items[idx]
    }
}

fn gen_script(seed: u64) -> String {
    let mut rng = Lcg(seed);
    let vars = ["a", "b", "c", "x", "y"];
    let values = ["0", "1", "2", "foo", "bar", "x y"];
    let ops = [
        "echo \"$a\"",
        "echo ${a-}",
        "echo ${a:-d}",
        "set -- $a $b; echo $#",
        "case \"$a\" in foo) echo f;; *) echo n;; esac",
        "for i in $a $b; do echo $i; done",
        "printf '%s\\n' \"$a\"",
    ];

    let assign1 = format!("{}='{}'", rng.pick(&vars), rng.pick(&values));
    let assign2 = format!("{}='{}'", rng.pick(&vars), rng.pick(&values));
    let op1 = rng.pick(&ops);
    let op2 = rng.pick(&ops);
    format!("{assign1}; {assign2}; {op1}; {op2}")
}

fn modernish_cap_dir() -> PathBuf {
    if let Ok(path) = env::var("MASH_MODERNISH_CAP_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/shell_suites/modernish/cap")
}

#[cfg(not(windows))]
#[test]
fn differential_fixed_scripts_against_dash() {
    if !require_correctness_enable() {
        return;
    }
    let mash = mash_binary();
    let Some(reference) = default_reference_shell() else {
        eprintln!("no reference shell configured; skipping");
        return;
    };
    let mut failures = Vec::new();
    for script in sample_scripts() {
        if let Err(e) = compare_script_pair(&mash, &reference, script) {
            failures.push(e);
            if failures.len() >= MAX_FAILURE_REPORTS {
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "differential fixed-script mismatches (first {}):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[cfg(not(windows))]
#[test]
fn differential_generated_scripts_against_dash() {
    if !require_correctness_enable() {
        return;
    }
    let mash = mash_binary();
    let Some(reference) = default_reference_shell() else {
        eprintln!("no reference shell configured; skipping");
        return;
    };

    let rounds = env::var("MASH_DIFF_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200);

    let mut failures = Vec::new();
    for i in 0..rounds {
        let script = gen_script(i as u64 + 1);
        if let Err(e) = compare_script_pair(&mash, &reference, &script) {
            failures.push(format!("seed={}: {}", i + 1, e));
            if failures.len() >= MAX_FAILURE_REPORTS {
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "differential generated-script mismatches (first {}):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[cfg(not(windows))]
#[test]
fn modernish_capability_probes() {
    if !require_correctness_enable() || !env_enabled("MASH_MODERNISH_CAP_ENABLE") {
        eprintln!("set MASH_MODERNISH_CAP_ENABLE=1 to run modernish cap probes");
        return;
    }
    let mash = mash_binary();
    let cap_dir = modernish_cap_dir();
    if !cap_dir.is_dir() {
        eprintln!("modernish cap dir not found at {}; skipping", cap_dir.display());
        return;
    }

    let mut tests: Vec<PathBuf> = fs::read_dir(&cap_dir)
        .expect("read modernish cap dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "t"))
        .collect();
    tests.sort();

    let mut failures = Vec::new();
    for t in &tests {
        let wrapper = format!(
            "set -eu; DEFPATH=\"$PATH\"; MSH_SHELL='{}'; . '{}' ",
            mash.display(),
            t.display()
        );
        match run_program("dash", &["-c", &wrapper], &cap_dir) {
            Ok(res) if res.status == 0 => {}
            Ok(res) => failures.push(format!(
                "{} => status={} stdout={:?} stderr={:?}",
                t.file_name().unwrap_or_default().to_string_lossy(),
                res.status,
                res.stdout,
                res.stderr
            )),
            Err(e) => failures.push(format!(
                "{} => harness error: {}",
                t.file_name().unwrap_or_default().to_string_lossy(),
                e
            )),
        }
        if failures.len() >= MAX_FAILURE_REPORTS {
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "modernish cap failures (first {}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[cfg(not(windows))]
#[test]
fn modernish_upstream_optional_smoke() {
    if !require_correctness_enable() {
        return;
    }
    let Ok(dir) = env::var("MASH_MODERNISH_UPSTREAM_DIR") else {
        eprintln!("MASH_MODERNISH_UPSTREAM_DIR not set; skipping");
        return;
    };
    let modernish_dir = PathBuf::from(dir);
    let install = modernish_dir.join("install.sh");
    if !install.is_file() {
        eprintln!(
            "install.sh not found at {}; skipping",
            modernish_dir.display()
        );
        return;
    }

    let mash = mash_binary();
    let tmp_home = TempDir::new().expect("create temp home");
    let timeout_secs = env::var("MASH_MODERNISH_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(180);

    let cmd = format!(
        "set -e; HOME='{}' timeout {}s bash -lc \"yes n | '{}' -s '{}'\"",
        tmp_home.path().display(),
        timeout_secs,
        install.display(),
        mash.display()
    );
    let res = run_program("bash", &["-lc", &cmd], &modernish_dir)
        .expect("run modernish install/test smoke");

    if res.status == 124 {
        panic!(
            "modernish upstream smoke timed out after {}s.\nstdout={}\nstderr={}",
            timeout_secs, res.stdout, res.stderr
        );
    }

    // install.sh exits non-zero after answering "n", so we only assert it reached
    // the test execution section and produced diagnostic output.
    assert!(
        res.stdout.contains("Running modernish test suite")
            || res.stderr.contains("Running modernish test suite"),
        "modernish upstream smoke did not reach test execution.\nstatus={}\nstdout={}\nstderr={}",
        res.status,
        res.stdout,
        res.stderr
    );
}

#[cfg(windows)]
#[test]
fn correctness_runner_windows_note() {
    eprintln!(
        "correctness_runner differential/modernish lanes are Linux-focused; run them in WSL/CI linux."
    );
}
