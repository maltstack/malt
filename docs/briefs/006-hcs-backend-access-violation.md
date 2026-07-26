# Brief 006 — The HCS backend access-violates on a real call

**Severity**: High · **Verified**: 2026-07-26 · **Source**: probing 007's T028

## What is wrong

Building `malt-platform` with `--features hcs` and calling
`hcs::create_compute_system` with a structurally valid minimal Windows
container document terminates the process with
**`STATUS_ACCESS_VIOLATION` (0xc0000005)**.

Reproduce:

```
cargo test -p malt-platform --features hcs --test isolation_reality hcs_probe -- --ignored --nocapture
```

Output on this host:

```
hcs_available() = true
ensure_hcs_runtime() = Ok
error: ... (exit code: 0xc0000005, STATUS_ACCESS_VIOLATION)
```

It crashes **after** `ensure_hcs_runtime()` succeeds — so the prerequisites
resolve and the fault is inside `native::create_compute_system`
(`crates/malt-platform/src/isolation/hcs.rs`, the `mod native` block), in or
around `HcsCreateComputeSystem` / `operation_result_string`.

The cause is not yet identified. Candidates worth checking first, in order:

- `HcsCreateComputeSystem` is asynchronous and typically returns
  `HCS_E_OPERATION_PENDING`; `operation_result_string(operation)` is then
  called on an operation that has not completed.
- `HcsCreateOperation(null, None)` — confirm the context/callback pair is the
  documented null form for this API version.
- The out-parameter and lifetime handling of `id_wide` / `cfg_wide` across
  the call.

## Why it matters

It is the reason feature 007's T028 could not be completed, and it was
misattributed. The task was recorded as **blocked on an unavailable external
platform**. It is not: `computecore.dll` is present, `vmcompute` and `hns` are
running, the `hcs` feature compiles, and `ensure_hcs_runtime()` returns `Ok`.
The host is fine. **The binding is broken.**

That distinction matters because "blocked on missing platform" implies waiting
for hardware or CI, while "crashes on a real call" is work that can be done
now, on this machine.

It is also the third defect of its exact shape found here: two in
`job_objects.rs` in 2026-07, and now this — all in code whose tests never
called the real OS API. AGENTS.md already records that lesson; this is it
recurring in the neighbouring module.

## What done looks like

- `create_compute_system` returns an `Err` for an unusable configuration and
  never faults. **No input should be able to access-violate the daemon.**
- A test that calls it for real, with the `hcs` feature on, and asserts a
  clean error — the `hcs_probe` test in
  `crates/malt-platform/tests/isolation_reality.rs` is the starting point;
  promote it from `#[ignore]` once it cannot crash.
- If a valid compute system *can* be created on a properly configured host,
  a test that does so and tears it down, which is what 007's T028 needs.
- The `hcs` feature is exercised somewhere in CI or a documented local run;
  today nothing builds with it, which is why this survived.

## Gotchas

- **Do not enable the `hcs` feature by default to "fix" this.** A crash
  reachable from a session request is worse than a missing tier. The feature
  gate is currently the only thing keeping the fault unreachable.
- The fake mode (`MALT_HCS_FAKE=1`) bypasses the native path entirely. A test
  that passes under fake mode proves nothing about this bug — check which
  mode a test runs in before trusting it.
- Fixing the crash does not by itself deliver `Contained`: creating a usable
  Windows container also needs a base image layer. Those are separate, and
  conflating them will make the fix look incomplete when it is not.
