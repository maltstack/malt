# Brief 006 — The HCS backend access-violates on a real call

**Severity**: High · **Verified**: 2026-07-26 · **Source**: probing 007's T028
**Status**: **RESOLVED 2026-07-26.** Root cause found, fixed, and covered by a
test that runs unconditionally. Kept as the record, because how it was
diagnosed matters more than the fix.

---

## What was wrong

Building `malt-platform` with `--features hcs` and calling
`hcs::create_compute_system` terminated the process with
**`STATUS_ACCESS_VIOLATION` (0xc0000005)**.

**Root cause:** `HcsStartComputeSystem` was called with a **null
`HCS_OPERATION` handle**:

```rust
// crates/malt-platform/src/isolation/hcs.rs, before the fix
let hr = unsafe {
    HcsStartComputeSystem(handle, std::ptr::null_mut() as HCS_OPERATION, std::ptr::null())
};
```

Every mutating HCS API is asynchronous. It takes an operation handle, returns
once the request is *queued*, and reports the real outcome through that
operation. computecore dereferences the handle unconditionally, so a null one
faults before any HRESULT can come back.

`terminate_compute_system` had the identical defect, meaning teardown faulted
for the same reason as startup.

The comment above the call claimed this was "the synchronous-start pattern
used throughout this module". **There is no such pattern** — the same function
creates a real operation for the create call twenty lines earlier. The comment
asserted a convention that the code it described did not follow, which is why
it read as deliberate.

## How it was found, and why that matters

All three hypotheses recorded in the original version of this brief were
**wrong**:

| Hypothesis | Reality |
|---|---|
| `HcsCreateComputeSystem` returns `HCS_E_OPERATION_PENDING`; the result is read too early | It returned **`S_OK`** with a valid non-null handle |
| `HcsCreateOperation(null, None)` is the wrong null form | Correct as written; returned a valid operation |
| `id_wide`/`cfg_wide` lifetime or out-parameter handling | Fine |

They were plausible, ordered by likelihood, and none survived contact with
four `eprintln!` statements. The fault was two calls later than every one of
them predicted. **Reasoning about which FFI call is wrong is not a substitute
for printing where execution stops.**

## What the fix was

A single `run_operation(name, call)` helper in the `native` module now owns the
whole lifecycle — create operation, invoke, **wait for the result**, close on
every path — and all three async call sites go through it or follow its shape:

- `HcsCreateComputeSystem`, `HcsStartComputeSystem` and
  `HcsTerminateComputeSystem` all get a valid operation handle.
- Results are collected with `HcsWaitForOperationResult` rather than assumed
  from the call's own HRESULT. `S_OK` from the call means *accepted*, not
  *done* — `HcsCreateComputeSystem` returned `S_OK` on a host that cannot run
  containers at all.
- `create_process` moved from `HcsGetOperationResultAndProcessInfo` to
  `HcsWaitForOperationResultAndProcessInfo`. The non-waiting variant returns
  whatever state the operation is in, so process launch succeeded or failed
  by timing.
- Result documents are `LocalFree`d. Every one of them leaked before.
- HRESULTs are decoded to names via the `windows_sys` constants.

## What it revealed underneath

With the crash gone, the real answer appeared:

```
create_compute_system() = Err: HcsCreateComputeSystem failed asynchronously:
  HRESULT=0x8037011b (HCS_E_ACCESS_DENIED -- the caller is not an administrator
  and not a member of the Hyper-V Administrators group)
```

So the host **can** run containers; the daemon lacks the rights. This is the
same conclusion vexil-v2 reached independently — it runs HCS from a
**LocalSystem Windows service** (`windows_container_service.rs`, installed via
`ensure_installed_for_contained`) precisely because an unprivileged daemon
cannot make these calls. A privileged-helper design is therefore not optional
for `Contained` on Windows; it is a requirement the API imposes.

The error decoder was added for this reason and is asserted by the test: a
bare `0x8037011b` is unreadable, and it is what would have hidden this next.

## vexil-v2 has the identical bug

`vexil-platform/src/hcs.rs:691-697` passes the same null operation handle to
`HcsStartComputeSystem`. Its HCS path is wired all the way to `vexil-bin` and
its HTTP routes, and it still cannot have completed a create-and-start.

**This is the single most important finding for the vendoring decision.**
Reachability was the evidence used to argue vexil-v2's isolation stack is more
mature than MALT's. It is more *wired*, which is not the same thing, and this
is the counter-example. Anything taken from it needs the same treatment this
module just got: run it, print where it stops, and do not trust the comments.

## Regression cover

`hcs_create_never_faults_whatever_this_host_supports` in
`crates/malt-platform/tests/isolation_reality.rs`, no longer `#[ignore]`d.
It asserts the call *returns* — no host configuration makes a fault
acceptable, and a faulting daemon takes every other session with it. It does
not assert `Ok`, because creating a compute system legitimately depends on
host rights, the Containers feature, and a base layer.

Note it previously carried `#[ignore]`, which is why the crash survived: an
ignored test cannot catch a regression.

## What remains (not this brief)

Fixing the crash does not deliver `Contained`. Still needed:

- **Privileged execution** — a LocalSystem helper or equivalent, per the
  `HCS_E_ACCESS_DENIED` above.
- **A base image layer** — no compute system is useful without one, and MALT
  has no OCI/image-store concept at all.

Both are properly scoped work, not follow-ups to this fix. Conflating them
with the crash is what made T028 look like it was blocked on hardware.
