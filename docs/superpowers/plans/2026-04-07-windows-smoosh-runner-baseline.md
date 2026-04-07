# Windows Smoosh Runner Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Windows `smoosh_runner` trustworthy by running 183 Smoosh tests, skipping exactly 3 unsupported tests, staging required helpers into `TEST_UTIL`, and distinguishing harness failures from shell failures.

**Architecture:** Keep all changes localized to the Smoosh runner and runner-adjacent test support. Introduce small helper functions inside the runner for classification, helper staging, and environment construction, then verify behavior with focused tests and one end-to-end run.

**Tech Stack:** Rust test harness (`std`, `cargo test`), existing `mash` binary/test suite, Windows temp directories and environment setup.

---

### Task 1: Extract Runner Outcome and Windows Policy Helpers

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs`

- [ ] **Step 1: Write the failing test for unsupported Windows classification**

Add these tests near the bottom of `crates/mash/tests/smoosh_runner.rs`:

```rust
#[test]
fn windows_skip_policy_contains_exactly_three_tests() {
    #[cfg(windows)]
    {
        assert_eq!(SKIP_ON_WINDOWS.len(), 3);
    }
}

#[test]
fn helper_dependent_tests_are_not_windows_skips() {
    #[cfg(windows)]
    {
        assert!(!SKIP_ON_WINDOWS.contains(&"builtin.export.override"));
        assert!(!SKIP_ON_WINDOWS.contains(&"semantics.command.argv0"));
    }
}
```

- [ ] **Step 2: Run the tests to verify the current policy fails**

Run: `cargo test --package mash --test smoosh_runner windows_skip_policy_contains_exactly_three_tests helper_dependent_tests_are_not_windows_skips`

Expected: FAIL because the current Windows skip list has only 2 entries and helper-dependent coverage is still implicit.

- [ ] **Step 3: Introduce explicit outcome and skip-policy helpers**

Refactor `crates/mash/tests/smoosh_runner.rs` to define small runner-local helpers similar to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Shell,
    Harness,
}

fn is_windows_unsupported(name: &str) -> bool {
    #[cfg(windows)]
    {
        matches!(
            name,
            "builtin.exec.modernish.mkfifo.loop"
                | "semantics.monitoring.ttou"
                | "REPLACE_WITH_THIRD_WINDOWS_UNSUPPORTED_TEST"
        )
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}
```

Update the test-skip path in `run_test()` to call `is_windows_unsupported(name)` instead of directly inspecting `SKIP_ON_WINDOWS`.

- [ ] **Step 4: Run the policy tests to verify they pass**

Run: `cargo test --package mash --test smoosh_runner windows_skip_policy_contains_exactly_three_tests helper_dependent_tests_are_not_windows_skips`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: define explicit windows smoosh skip policy"
```

### Task 2: Add Helper Discovery and Staging into `TEST_UTIL`

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs`

- [ ] **Step 1: Write the failing tests for helper staging**

Add these tests to `crates/mash/tests/smoosh_runner.rs`:

```rust
#[test]
fn helper_stage_dir_is_under_test_tmpdir() {
    let tmp = std::env::temp_dir().join("mash_runner_helper_stage_dir_test");
    let helper_dir = helper_dir_for(&tmp);
    assert!(helper_dir.starts_with(&tmp));
    assert_eq!(helper_dir.file_name().and_then(|s| s.to_str()), Some("test-util"));
}

#[test]
fn helper_inventory_includes_getenv() {
    let helpers = helper_inventory();
    assert!(helpers.iter().any(|helper| helper.name == "getenv"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package mash --test smoosh_runner helper_stage_dir_is_under_test_tmpdir helper_inventory_includes_getenv`

Expected: FAIL because no helper inventory or helper-dir helper exists yet.

- [ ] **Step 3: Implement helper inventory and staging**

In `crates/mash/tests/smoosh_runner.rs`, add small helper types/functions:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct HelperSpec {
    name: &'static str,
    source: &'static str,
}

fn helper_inventory() -> Vec<HelperSpec> {
    vec![
        HelperSpec {
            name: "getenv",
            source: "tests/shell_suites/smoosh/src/getenv",
        },
        HelperSpec {
            name: "argv",
            source: "tests/shell_suites/smoosh/src/argv",
        },
        HelperSpec {
            name: "readdir",
            source: "tests/shell_suites/smoosh/src/readdir",
        },
        HelperSpec {
            name: "fds",
            source: "tests/shell_suites/smoosh/src/fds",
        },
    ]
}

fn helper_dir_for(test_tmpdir: &Path) -> PathBuf {
    test_tmpdir.join("test-util")
}

fn stage_helpers(test_tmpdir: &Path) -> Result<PathBuf, String> {
    let helper_dir = helper_dir_for(test_tmpdir);
    fs::create_dir_all(&helper_dir).map_err(|e| format!("create helper dir: {e}"))?;
    for helper in helper_inventory() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(helper.source);
        let target = helper_dir.join(helper.name);
        fs::copy(&source, &target)
            .map_err(|e| format!("stage helper {} from {}: {e}", helper.name, source.display()))?;
    }
    Ok(helper_dir)
}
```

Use the real helper source paths present in this repository rather than the placeholder paths above. If the helper source filenames differ on Windows, encode that explicitly in `helper_inventory()`.

- [ ] **Step 4: Run the helper tests to verify they pass**

Run: `cargo test --package mash --test smoosh_runner helper_stage_dir_is_under_test_tmpdir helper_inventory_includes_getenv`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: stage smoosh helper utilities for windows runner"
```

### Task 3: Classify Harness Failures Separately from Shell Failures

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs`

- [ ] **Step 1: Write the failing tests for failure classification**

Add these tests:

```rust
#[test]
fn harness_failure_reason_is_tagged() {
    let reason = format_failure_reason(
        FailureKind::Harness,
        "helper staging failed".to_string(),
    );
    assert!(reason.contains("HARNESS"));
}

#[test]
fn shell_failure_reason_is_tagged() {
    let reason = format_failure_reason(
        FailureKind::Shell,
        "stdout mismatch".to_string(),
    );
    assert!(reason.contains("SHELL"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package mash --test smoosh_runner harness_failure_reason_is_tagged shell_failure_reason_is_tagged`

Expected: FAIL because no formatting/classification helper exists yet.

- [ ] **Step 3: Implement failure-kind tagging and harness error paths**

In `crates/mash/tests/smoosh_runner.rs`, add a helper like:

```rust
fn format_failure_reason(kind: FailureKind, detail: String) -> String {
    match kind {
        FailureKind::Harness => format!("HARNESS: {detail}"),
        FailureKind::Shell => format!("SHELL: {detail}"),
    }
}
```

Update `run_test()` so these paths become `FailureKind::Harness`:

- tempdir creation failure
- helper staging failure
- spawn failure
- timeout

Keep stdout/exit-code mismatches as `FailureKind::Shell`.

Extend `TestOutcome` to carry `failure_kind: Option<FailureKind>` and use `format_failure_reason(...)` whenever `failure_reason` is produced.

- [ ] **Step 4: Run the classification tests to verify they pass**

Run: `cargo test --package mash --test smoosh_runner harness_failure_reason_is_tagged shell_failure_reason_is_tagged`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: classify smoosh harness failures separately"
```

### Task 4: Wire Helper Staging into `run_test()` Environment Setup

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs`

- [ ] **Step 1: Write the failing test for `TEST_UTIL` environment construction**

Add this test:

```rust
#[test]
fn test_util_env_points_to_staged_helper_dir() {
    let tmp = std::env::temp_dir().join("mash_runner_test_util_env_test");
    let helper_dir = helper_dir_for(&tmp);
    let envs = build_runner_env(&tmp, &helper_dir, Path::new("C:/mash.exe"), "C:/windows/system32");
    let test_util = envs.iter().find(|(k, _)| k == "TEST_UTIL").map(|(_, v)| v.clone());
    assert_eq!(test_util.as_deref(), Some(helper_dir.to_string_lossy().as_ref()));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --package mash --test smoosh_runner test_util_env_points_to_staged_helper_dir`

Expected: FAIL because there is no centralized environment-construction helper.

- [ ] **Step 3: Extract deterministic environment construction**

Add a helper similar to:

```rust
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
        ("TEST_SHELL".to_string(), mash.to_string_lossy().to_string()),
        ("TEST_UTIL".to_string(), helper_dir.to_string_lossy().to_string()),
        (
            "PWD".to_string(),
            test_tmpdir.to_string_lossy().to_string().replace('\\', "/"),
        ),
    ]
}
```

Update `run_test()` to:

- call `stage_helpers(&test_tmpdir)` before spawning `mash`
- build environment pairs via `build_runner_env(...)`
- apply them to the `Command`

- [ ] **Step 4: Run the environment test to verify it passes**

Run: `cargo test --package mash --test smoosh_runner test_util_env_points_to_staged_helper_dir`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: wire staged test util env into smoosh runner"
```

### Task 5: Update Summary Reporting for Runnable/Skipped/Harness/Shell Counts

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs`

- [ ] **Step 1: Write the failing test for summary rendering**

Add a pure formatting test:

```rust
#[test]
fn summary_includes_harness_and_shell_failure_counts() {
    let summary = render_summary(186, 183, 90, 3, 2, 91);
    assert!(summary.contains("runnable: 183"));
    assert!(summary.contains("skipped unsupported: 3"));
    assert!(summary.contains("harness failures: 2"));
    assert!(summary.contains("shell failures: 91"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --package mash --test smoosh_runner summary_includes_harness_and_shell_failure_counts`

Expected: FAIL because the summary is still printed inline with only pass/skip counts.

- [ ] **Step 3: Extract summary rendering and update the report**

Add a helper like:

```rust
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
```

Update `smoosh_conformance_tests()` to track these counters separately and print the summary via `render_summary(...)`.

- [ ] **Step 4: Run the summary test to verify it passes**

Run: `cargo test --package mash --test smoosh_runner summary_includes_harness_and_shell_failure_counts`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: improve windows smoosh runner summary reporting"
```

### Task 6: Validate the Windows Runner Baseline End-to-End

**Files:**
- Modify: `crates/mash/tests/smoosh_runner.rs` (only if validation exposes runner-only bugs)

- [ ] **Step 1: Build the runner target**

Run: `cargo test --package mash --test smoosh_runner --no-run`

Expected: build succeeds

- [ ] **Step 2: Run the focused runner tests**

Run: `cargo test --package mash --test smoosh_runner windows_skip_policy_contains_exactly_three_tests helper_inventory_includes_getenv harness_failure_reason_is_tagged test_util_env_points_to_staged_helper_dir summary_includes_harness_and_shell_failure_counts`

Expected: PASS

- [ ] **Step 3: Run the full Windows Smoosh runner**

Run: `cargo test --package mash --test smoosh_runner`

Expected:
- exactly 3 unsupported skips reported
- exactly 183 runnable tests reported
- helper-dependent tests no longer fail solely because `TEST_UTIL` is missing
- remaining failures, if any, are labeled as shell failures unless the harness genuinely broke

- [ ] **Step 4: If the end-to-end run shows runner-only defects, fix only those and rerun**

Allowed changes:
- helper path correction
- unsupported-skip list correction to the intended 3 tests
- summary/accounting correction
- harness classification correction

Disallowed in this task:
- shell semantic fixes in `crates/mash/src/*`

- [ ] **Step 5: Commit**

```bash
git add crates/mash/tests/smoosh_runner.rs
git commit -m "test: establish trustworthy windows smoosh baseline"
```

## Self-Review

- Spec coverage: the plan covers explicit Windows unsupported policy, helper staging, deterministic environment setup, harness-vs-shell classification, and end-to-end verification of the 183/3 baseline.
- Placeholder scan: one item remains intentionally marked as `REPLACE_WITH_THIRD_WINDOWS_UNSUPPORTED_TEST`; replace it during Task 1 before implementation proceeds. No other placeholders should remain after that edit.
- Type consistency: all helper names and function signatures referenced in later tasks are introduced in earlier tasks within this plan.
