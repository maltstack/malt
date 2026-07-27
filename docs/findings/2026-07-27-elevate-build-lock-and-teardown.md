# Elevated-helper build lock and teardown fault

**Date:** 2026-07-27  
**Host:** Windows, native PowerShell  
**Scope:** Post-Spec-009 full-gate failures in helper installation, HCS test
teardown, and strict Clippy

## What reproduced

Three independent failures were confirmed:

1. `cargo clippy --workspace --all-targets -- -D warnings` stopped in
   `malt-daemon` because `gateway_backend.rs` declared production items after
   a test module.
2. The `malt-elevate` library harness could print a passing test result and
   then exit with `0xc0000005 (STATUS_ACCESS_VIOLATION)`.
3. Installing the helper registered
   `target\debug\malt-elevate.exe` directly with the Service Control Manager.
   The running service therefore held Cargo's build output open and prevented
   replacement or cleanup from the same target directory.

The separate line

```text
memory allocation of 67108864 bytes failed
```

is expected evidence from `malt-platform/tests/isolation_reality.rs`. That
test places a child under a 16 MiB Job Object limit, asks it to allocate
64 MiB, and passes only when the child fails. The line was not the workspace
test failure.

The access violation was timing-dependent, not a deterministic consequence of
running `cargo test -p malt-elevate --lib`. That standalone command also
passed repeatedly before the fix, including twenty consecutive runs in an
isolated target directory. One focused main-target sequence reproduced the
post-test fault. The passing runs do not contradict the defect: they show that
the detached reaper normally completed before the test changed its global fake
backend selection.

## Root cause of the access violation

Fake and native HCS operations were selected by reading the mutable
process-wide `MALT_HCS_FAKE` environment variable at each operation. An
`HcsProcess` or `HcsComputeSystem` handle did not retain which backend created
it. A process reaper could therefore receive a fake handle, outlive the test
that removed `MALT_HCS_FAKE`, and pass that fake value to a native HCS wait or
close operation. The resulting invalid native handle use explains the
post-test access violation more precisely than a generic Rust-static teardown
race. No deterministic scheduler or service-state trigger was established.

The helper also detached one reaper per HCS process and detached client
threads from the service server. Their owners had no shutdown join path.

## Remediation

- `HcsProcess` and `HcsComputeSystem` now retain backend provenance and route
  wait, close, and terminate operations through the backend that created the
  handle.
- The helper container registry owns process reaper join handles, terminates
  compute systems before joining their reapers, and tears down remaining
  containers when the registry is dropped.
- The service server owns and joins client threads before dropping the shared
  container registry.
- The strict-Clippy test module now follows all production items.
- `malt elevate install` copies the helper atomically to
  `%ProgramFiles%\MALT\malt-elevate.exe`, resolved through
  `SHGetKnownFolderPath(FOLDERID_ProgramFiles)` rather than an inherited
  environment variable. SCM registers that administrator-owned copy.
  Uninstall removes both the service and copied executable.

## Verification

Focused validation:

- Backend-provenance regression test passed.
- `cargo test -p malt-elevate --lib -- --nocapture` exited normally.
- Twenty consecutive helper-library runs passed: 500 tests, no teardown
  access violation.
- Service deployment/replacement/removal tests passed.
- Helper-client focused tests passed.

Live installed-helper validation:

- `sc.exe qc MALT-Elevate` reported
  `BINARY_PATH_NAME: "C:\Program Files\MALT\malt-elevate.exe" ...`.
- The service remained `RUNNING` and the authenticated VNP probe remained
  reachable.
- With the service running, `cargo clean -p malt-elevate` removed the Cargo
  artifacts and `cargo build -p malt-elevate` rebuilt them successfully.
  This forced the operation that the previous SCM registration locked.

Final gates:

- `cargo build --workspace` — passed.
- `cargo test --workspace` — passed; the intentional 64 MiB child-allocation
  failure appeared and its enclosing isolation test passed.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo clippy -p malt-platform --features hcs --all-targets -- -D warnings`
  — passed.

## What this did not establish

- The ignored real-image HCS test still requires its explicit image fixture
  and environment opt-in; this gate run did not repeat that scenario.
- This work did not repeat the independent UAC-decline evidence already
  recorded for Spec 008.
- A reachable installed helper is not evidence that an arbitrary daemon
  process has been enrolled; daemon enrollment remains a separate explicit
  operation.
