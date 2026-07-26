# Contained HCS process handoff survey — 2026-07-27

## Why this survey exists

Spec 008 task T040 says that a contained session must route through the
privileged helper and update its one `IsolationContext` only after a
`Performed` outcome. It is not safe to interpret that as *create an HCS
compute system, record its id, and continue launching host children*.
`IsolationContext::Established::Container` is status evidence that MASH
children are inside the container, not a reservation for future work.

This survey establishes the real production path before changing it.

## What exists

| Component | Current evidence | What it proves |
| --- | --- | --- |
| HCS lifecycle | `crates/malt-elevate/src/dispatch.rs:188-347` keeps a helper-owned `HcsComputeSystem` by session id and creates/terminates it from a typed request. | The helper can own compute-system lifetime. |
| HCS process API | `crates/malt-platform/src/isolation/hcs.rs:213-270` wraps `HcsCreateProcess`, HCS process wait, and close. | MALT has a platform abstraction for a process *inside* a compute system. |
| Pipe request | `crates/malt-platform/src/isolation/hcs.rs:31-43` now expresses requested standard streams; the native JSON now emits them. | HCS can return process I/O only when the launch asks for it. |
| Helper entitlement | `crates/malt-elevate/src/entitlement.rs:59-146` re-observes the daemon process and binds sessions to its principal/root. | A helper request can be scoped to the connected daemon instead of caller-supplied authority. |
| Prior-art reference | `C:\Users\mamuk\projects\LEGACY-vexil-v2\vexil-platform\src\windows_container_service.rs:491-547` creates an HCS process, duplicates its handles into the daemon process, then closes service-owned copies. | Handle duplication, not a compute-system id, is the necessary boundary-crossing shape. |

## Production reachability before this implementation

`SessionExecutor::spawn_with_capacity` calls
`apply_session_isolation` in
`crates/malt-daemon/src/executor/session_thread.rs:627-630`. For `Contained`,
that function refuses before creating the session
(`session_thread.rs:113-116`). This is reachable from
`Coordinator::create_session_inner` (`coordinator.rs:338-342`) and is why a
required contained request fails without leaving a session.

The live MASH external command sites are in
`crates/mash/src/executor.rs:1398,1767,3670,5847,6043`; each calls
`malt_platform::process::spawn` directly. No current production caller invokes
`malt_platform::isolation::hcs::create_process`. Therefore HCS lifecycle
tests, capability probes, and a stored container id are not evidence of
contained command execution.

## Required implementation shape

1. Add a typed helper request that names a helper-owned container and describes
   the daemon's command, environment, working directory, and requested standard
   streams. The helper must derive the target daemon PID from the authenticated
   named-pipe peer; it must not accept a target PID from the request.
2. Have the helper validate session ownership, create the HCS process, duplicate
   the HCS process and requested pipe handles into that peer process, and close
   its source copies on every result path.
3. Extend `malt-platform::process::Child` with an HCS process variant that waits
   with `HcsWaitForProcessExit` and closes with `HcsCloseProcess`, while owning
   the duplicated standard handles as synchronous Windows files.
4. Give MASH one injected external-process spawner, and make every production
   external-command call site use it. The daemon supplies the helper-backed
   spawner only for an established contained session. It must preserve normal
   pipe, redirect, and session-input behavior.
5. Only after the helper has created the compute system and the daemon has
   installed the HCS spawner may `IsolationContext::establish_container` run.
   Refused or indeterminate outcomes leave the carrier unchanged; an error after
   creation must request helper teardown before session construction returns.

## Implementation result and live boundary check

The implementation now uses the shape above: the typed request carries a
program/argv/working directory/environment, the helper derives the target PID
from the authenticated pipe peer, HCS process and standard handles are
duplicated into that peer, and `mash` routes all five production external spawn
sites through a session-owned spawner. File redirections, null devices, and
session stdin are relayed through the duplicated HCS pipes; ordinary `Pipe`
outputs stay owned by MASH. Session destruction waits for its worker and then
asks the helper to terminate the entitled compute system.

Live attempt on 2026-07-27:

1. Rebuilt and restarted `MALT-Elevate`; authenticated `malt elevate status`
   reported reachable protocol 2.
2. Started the daemon and UAC-enrolled its PID with
   `malt elevate authorize-daemon 53692`.
3. Ran `malt new --name hcs-live-0727 --isolation contained
   --isolation-policy required`.
4. The helper reached `HcsCreateComputeSystem` but HCS rejected the current
   generated configuration with `HRESULT=0x80071126`, result detail
   `OperationFailure: Construct`. No contained session appeared in daemon
   status.

This proves the UAC/enrollment/typed-helper refusal boundary, and proves the
daemon did not downgrade a required contained request. It does **not** prove a
contained process launch: the generated HCS configuration still has no
validated Windows container image/layer set. Capability reporting remains
unavailable for `Contained` until that host-dependent configuration is
supplied and exercised live.

## What this survey does not establish

It does not establish that the host has compatible Windows image layers, that
HCS can create a process from those layers, or that a live command can pass
through the duplicated streams. Those are final live verification requirements,
not facts inferred from the wrappers or the legacy reference.
