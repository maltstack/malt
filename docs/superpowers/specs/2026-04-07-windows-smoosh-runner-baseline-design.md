# Windows Smoosh Runner Baseline Design

**Date:** 2026-04-07

**Goal**

Make `cargo test --package mash --test smoosh_runner` trustworthy on Windows before changing shell semantics. The runner must execute 183 runnable Smoosh tests, skip exactly 3 tests that are genuinely unpassable on Windows, and clearly separate unsupported-platform skips from real shell failures and harness/setup failures.

**Context**

`mash` currently reports a large number of Windows Smoosh failures, but the failure set is polluted by runner defects:

- required helper utilities are not staged into `TEST_UTIL`
- the runner currently classifies only a small subset of environment-dependent tests as skipped
- setup and platform mismatches are reported as ordinary shell failures

This milestone does not attempt to improve shell correctness directly. Its purpose is to establish a reliable Windows conformance baseline so later shell work is measured against real failures.

## Scope

This milestone covers only the native Smoosh runner in `crates/mash/tests/smoosh_runner.rs` and any small test-support additions required to make the runner self-contained on Windows.

Included:

- helper utility staging into each temporary test workspace
- explicit Windows unsupported-test policy for exactly 3 skipped tests
- deterministic test environment setup for `PATH`, `PWD`, `HOME`, `TERM`, `TEST_SHELL`, and `TEST_UTIL`
- harness-level error classification so setup faults are not misreported as shell conformance regressions
- runner output that distinguishes pass, skip, shell failure, and harness failure

Excluded:

- shell parser, expander, executor, builtin, or path-resolution fixes
- Linux-specific runner redesign
- broad Smoosh-suite curation beyond the Windows skip policy for the 3 known unpassable tests

## Non-Goals

- Raising the Windows pass count in this milestone
- Matching `vexil-v2` implementation details
- Refactoring the runner for style alone
- Adding speculative skips for tests that should be made runnable by fixing the harness

## Constraints

- MALT remains the architectural authority; `vexil-v2` may be consulted only as a behavioral reference
- The runner must stay native Rust-based and integrated with `cargo test`
- Unsupported Windows skips must be explicit and auditable in code
- If the harness cannot set up a required helper or environment precondition, the test run must fail as a harness problem rather than silently skip or misclassify the result

## Design

### 1. Test Classification Model

Each Smoosh test on Windows will fall into exactly one of four outcomes:

- `Passed`: test executed and matched expected artifacts
- `SkippedUnsupported`: test is on the fixed Windows unsupported list
- `FailedShell`: harness setup succeeded, test ran, and `mash` behavior differed from expected output or exit status
- `FailedHarness`: the runner could not prepare or execute the test correctly due to setup, staging, or orchestration failure

This classification replaces the current binary pass/fail model for internal accounting. Final test output may still aggregate failures, but it must preserve the distinction between shell and harness failures in the textual report.

### 2. Windows Unsupported Policy

The runner will maintain one explicit Windows-only skip list containing exactly 3 tests. This list is the project’s current policy for “genuinely not passable on Windows” and must be documented inline with one short reason per test.

No helper-dependent test may remain skipped solely because the runner failed to stage utilities. Those become runnable as part of this milestone.

If a future review changes the count away from 3, that requires an intentional policy change with rationale in code and tests.

### 3. Helper Utility Staging

The runner currently sets `TEST_UTIL=.` inside a fresh temporary directory but does not stage the helper programs expected by Smoosh tests. This creates false shell failures.

The runner will instead populate a dedicated helper directory inside the temporary workspace and set `TEST_UTIL` to that directory.

The helper staging mechanism must:

- identify the concrete helper files needed by the Windows-runnable Smoosh set
- copy or materialize them into the temp workspace before executing the test
- fail the test as `FailedHarness` if any required helper cannot be staged

The staging implementation should be minimal and explicit. It should stage only the utilities required for the runnable Windows set, not reconstruct the entire upstream Smoosh helper environment without need.

### 4. Launch Contract

The runner must create a deterministic process environment for `mash` on Windows:

- `PATH` inherited from the host, unless test support requires prepending staged helper paths
- `HOME` set to the temp workspace when the host environment does not provide one
- `PWD` set to the temp workspace using the shell’s expected slash normalization
- `TERM=dumb`
- `TEST_SHELL` set to the resolved `mash` binary path
- `TEST_UTIL` set to the staged helper directory

The runner must continue to execute each test from a unique temp workspace and clean it up after completion.

### 5. Failure Reporting

The textual report printed by `smoosh_runner` must distinguish:

- unsupported skips
- harness/setup failures
- shell conformance failures

At minimum, the summary must include separate counts for:

- total tests discovered
- runnable tests
- passed tests
- skipped unsupported tests
- harness failures
- shell failures

Detailed failure lines must indicate whether the failing item is a harness or shell problem.

### 6. Verification Expectations

This milestone is complete only when:

- the runner executes 183 tests on Windows
- the runner skips exactly 3 tests on Windows
- helper-dependent tests are no longer skipped solely because helpers are missing
- a deliberately broken helper-staging path produces a harness failure classification rather than a shell-failure classification
- `cargo test --package mash --test smoosh_runner` completes with the updated accounting and reporting

## Acceptance Criteria

1. On Windows, the Smoosh runner reports exactly 3 skipped tests due to explicit unsupported policy.
2. On Windows, the Smoosh runner reports 183 runnable tests.
3. Tests requiring `TEST_UTIL` helpers run against staged helpers instead of failing because the helpers are absent from the temp workspace.
4. Harness setup errors are reported distinctly from shell conformance failures.
5. No `mash` source files outside the runner/support surface are modified unless required for runner self-containment and explicitly justified during implementation.

## Risks

- Some Smoosh helper expectations may be implicit rather than listed centrally, so helper discovery must be verified against the runnable Windows set.
- Windows path normalization may still expose shell bugs after the harness is fixed; those should remain classified as shell failures, not folded back into the runner.
- The current suite may contain tests that appear unsupported but are actually runnable once helpers are staged; the implementation must resist adding speculative skips.

## Test Strategy

- Add focused unit coverage for any new helper-discovery or classification helpers extracted from the runner.
- Add runner-level tests where practical for:
  - unsupported skip classification
  - helper-staging path selection
  - harness-failure reporting
- Run `cargo test --package mash --test smoosh_runner` on Windows as the primary end-to-end validation.

## Implementation Notes

- Keep the runner changes localized and auditable.
- Prefer small helper functions inside the runner module over introducing a broad new test-support framework.
- Consult `vexil-v2` only when validating intended Windows runner behavior; do not port its structure directly.
