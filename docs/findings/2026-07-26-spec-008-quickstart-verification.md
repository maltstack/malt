# Spec 008 privileged-helper quickstart verification — 2026-07-26

## Host and helper state

Windows helper service `MALT-Elevate` was installed through the explicit
`malt elevate install` UAC flow. `malt elevate status` then reported
`reachable`, protocol 2, and an authenticated VNP hello/ack round trip.
The service was explicitly stopped and started through SCM once each; status
reported `installed, not running` while stopped and `reachable` after restart.
An explicit uninstall in the earlier setup removed the service and status
reported `not installed` before it was installed again.

## Scenario results

| Quickstart scenario | Outcome | Evidence / limit |
|---|---|---|
| 1: unimplemented operations refuse | Passed | `operation_outcomes` enumerates schema operations and rejects a bare success outcome. |
| 2: capability and reality agree | Passed | `malt-elevate` capability/outcome tests passed; current status independently reached the live helper. |
| 3: helper states | Passed | Live evidence covers not-installed, stopped, and reachable. A real temporary named-pipe VNP peer returned another protocol version and the client produced `VersionMismatch`; CLI guidance is distinct for every state. |
| 4: declined UAC leaves no artefact | Not run | This requires deliberately declining a UAC prompt; the operator authorized elevation for this run. No failed-install artefact is claimed as substitute evidence. |
| 5: unauthorised local request | Passed | Real named-pipe test sends a well-formed request from an unenrolled local process and observes refusal. |
| 6: replay | Passed | Real named-pipe test re-sends the authenticated envelope and observes replay refusal. |
| 7: entitlement escape | Passed | Owner, parent-traversal, and symlink-escape refusals are covered against canonicalized paths. |
| 8: privilege boundary | Skipped on revalidation | The opt-in `elevate_boundary` scenario was rerun on 2026-07-27 after the HCS spawn-path change. Both direct and helper routes reached the host configuration failure `HRESULT=0x80071126` (`OperationFailure: Construct`) before a privilege difference could be observed, so the test printed its defined skip instead of claiming a pass. |
| 9: teardown absence | Partial | Fake HCS lifecycle test enumerates the removed system; no live helper-created system was available because image layers are out of scope. |
| 10: helper death during session operation | Partial | A real temporary named-pipe peer accepted the authenticated request then disappeared; the client returned `Indeterminate` and its `IsolationContext` remained unestablished. A live contained MASH child does not yet exist; see the HCS spawn survey. |
| 11: one isolation carrier | Passed | The MASH environment has one `IsolationContext` field and the targeted test passed. |

## Gates run

- `cargo build --workspace` — passed after explicitly stopping the helper so
  Windows released `target/debug/malt-elevate.exe`; the helper was restarted
  and freshly probed afterward.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo clippy -p malt-platform --features hcs --all-targets -- -D warnings`
  — passed.
- Package tests for every workspace crate except `mash` — passed. The
  isolation-reality test prints an expected failed 64 MiB allocation from its
  constrained subprocess while its own result remains pass.
- `cargo test -p mash` and therefore the single `cargo test --workspace`
  gate exceeded the desktop command wrapper's 124-second limit without a test
  failure or diagnostic. The wrapper kills its Cargo descendants, so this is
  not evidence of a passing full suite.

## What this does not establish

It does not establish a contained MALT session. The helper-owned HCS-process,
duplicated-I/O, and MASH child-lifecycle path is now implemented, but this
host has no validated HCS image/layer configuration: HCS rejects compute-system
construction before a process can launch. The declined-UAC scenario likewise
remains unperformed because the operator approved elevation for this run.

## 2026-07-27 containment follow-up

Spec 009 supplied the missing helper-owned Windows image/layer substrate. Its
live evidence now proves required contained session creation, `cmd /c ver`
through the helper-owned HCS process path, teardown, and removal of the
prepared image state. It also proves two-image active-use isolation and
post-workspace HCS-construction rollback. These close Spec 008's helper-backed
containment routing work (T040/T040e), but do not substitute for a declined
UAC prompt.

On 2026-07-27, after stopping the daemon/helper only to release executable
locks, the exact final gate set passed:

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p malt-platform --features hcs --all-targets -- -D warnings`

The expected constrained-subprocess allocation message in the isolation
reality suite was again emitted while the enclosing test passed. The helper
and daemon were restored and the daemon was freshly enrolled afterwards.
