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
| 4: declined UAC leaves no artefact | Passed (2026-07-27) | From a normal PowerShell process, a successful elevated uninstall removed the helper; a subsequent declined `malt elevate install` returned Windows error 1223. `malt elevate status` reported `not installed` and `sc.exe query MALT-Elevate` returned `1060` (service does not exist). |
| 5: unauthorised local request | Passed | Real named-pipe test sends a well-formed request from an unenrolled local process and observes refusal. |
| 6: replay | Passed | Real named-pipe test re-sends the authenticated envelope and observes replay refusal. |
| 7: entitlement escape | Passed | Owner, parent-traversal, and symlink-escape refusals are covered against canonicalized paths. |
| 8: privilege boundary | Passed (2026-07-27) | From a normal, non-admin PowerShell process, `MALT_RUN_ELEVATE_BOUNDARY=1 cargo test -p malt-daemon --test elevate_boundary privilege_boundary_changes_the_outcome -- --ignored --nocapture` passed in 1.46 seconds. The test asserts the direct HCS request is denied for privilege and the helper-routed request is not denied for that reason. |
| 9: teardown absence | Passed (2026-07-27) | The live contained session was destroyed, `hcsdiag list` contained no MALT compute system, and the prepared image was removable afterward. The two-image run additionally proved only the active image was protected. |
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

It does not establish that every Windows host can run contained sessions: the
successful evidence is tied to this host, its validated Windows base images,
and its HCS configuration. It also does not replace recurring real-machine
coverage with a one-time manual UAC decision.

## 2026-07-27 containment follow-up

Spec 009 supplied the missing helper-owned Windows image/layer substrate. Its
live evidence now proves required contained session creation, `cmd /c ver`
through the helper-owned HCS process path, teardown, and removal of the
prepared image state. It also proves two-image active-use isolation and
post-workspace HCS-construction rollback. These close Spec 008's helper-backed
containment routing work (T040/T040e). The independently declined install
later supplied the separate UAC evidence.

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
