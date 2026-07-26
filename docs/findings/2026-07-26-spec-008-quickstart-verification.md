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
| 3: helper states | Partial | Live evidence covers not-installed, stopped, and reachable. The version-mismatch message is unit-tested but no deliberately mismatched helper build was installed. |
| 4: declined UAC leaves no artefact | Not run | This requires deliberately declining a UAC prompt; the operator authorized elevation for this run. No failed-install artefact is claimed as substitute evidence. |
| 5: unauthorised local request | Passed | Real named-pipe test sends a well-formed request from an unenrolled local process and observes refusal. |
| 6: replay | Passed | Real named-pipe test re-sends the authenticated envelope and observes replay refusal. |
| 7: entitlement escape | Passed | Owner, parent-traversal, and symlink-escape refusals are covered against canonicalized paths. |
| 8: privilege boundary | Passed | `elevate_boundary` observed direct `HCS_E_ACCESS_DENIED`; the same entitled helper request was not refused for that reason. |
| 9: teardown absence | Partial | Fake HCS lifecycle test enumerates the removed system; no live helper-created system was available because image layers are out of scope. |
| 10: helper death during session operation | Blocked | A real contained MASH child does not yet exist; see the HCS spawn survey. |
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

It does not establish a contained MALT session. The helper proves that the
privilege boundary changes an HCS outcome, but MASH still launches normal host
children. A helper-owned HCS process, duplicated I/O handles, and a MASH-side
child lifecycle integration are required before `IsolationContext` may be
updated to `Container`.
