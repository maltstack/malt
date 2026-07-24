# Responsive Session Control During Execution — Baseline and Verification

## 2026-07-25 baseline

- Commit: `6ac4391031abd828bb288163c49f8959db2bd2b5`
- Toolchain: `rustc 1.97.0 (2d8144b78 2026-07-07)`; `cargo 1.97.0 (c980f4866 2026-06-30)`
- Registered tests: 1,292 (`cargo test --workspace -- --list`)
- `cargo build --workspace`: passed.
- `cargo test --workspace`: passed.
- Pre-existing failures: none observed.

This is the clean baseline captured before the responsive session-control
implementation changes begin.

## 2026-07-25 implementation evidence

- Final registered-test count: 1,311.
- Focused responsive-control suites passed: daemon worker, coordinator,
  session-thread, Gateway backend, VNP listener, and Gateway route contracts.
- Protocol and persistence compatibility passed: `malt-protocol`, daemon VNP,
  and daemon store suites. `schemas/` has no diff.
- Native Windows Smoosh passed: 183 passed, 3 unsupported skipped.
- VNP attach test: 100 consecutive attaches during a busy command each
  received `InitialState` within one second.
- Cross-session soak: 600.20 seconds of repeated observation of an
  independent session while another session ran `sleep 600`; no observation
  exceeded one second. The Gateway initiator timed out at its unchanged
  30-second result wait, which does not cancel accepted execution.
- A full workspace build/test gate passed during implementation. After the
  final test-only additions, the changed daemon and Gateway suites passed and
  `cargo build --workspace` passed; two fresh full-workspace test retries
  exceeded this session's 64-second command wrapper before producing a test
  failure. The repository-wide formatter check remains failing because it
  reports pre-existing formatting drift in unrelated files; that drift was
  not mechanically rewritten as part of this feature.
