# Privileged helper HCS spawn survey — 2026-07-26

## Question

Can Spec 008 truthfully route a `Contained` MALT session through the
privileged helper and mark its `IsolationContext` as `Container`?

## Survey result

No. The helper can create and own an HCS compute system, but MALT's live
external-command path remains `mash::executor` →
`malt_platform::process::spawn`. That path starts normal host processes and
only consults the single isolation carrier for a Windows Job Object. It has
no HCS process-launch branch.

Creating an HCS system and putting its identifier in the carrier would not
constrain any MASH child. It would violate Spec 008 FR-017: reported
isolation and actual process constraint would no longer derive from the same
source. `session_thread::apply_session_isolation` therefore continues to
refuse `Contained` before any session becomes reachable.

## Prior-art result

`C:\Users\mamuk\projects\vexil-v2` has the relevant architectural pattern,
but it is reference material only under this repository's vendor rule:

- `vexil-platform/src/hcs.rs` calls `HcsCreateProcess` with console and
  standard-stream pipe options.
- `vexil-platform/src/windows_container_service.rs` duplicates the resulting
  HCS stdin/stdout handles into the daemon process with `DuplicateHandle`.
- `vexil-daemon/src/spawn_shell.rs` adopts those duplicated handles as the
  shell's reader and writer, rather than spawning the shell normally.

That is the missing mechanism MALT needs: an entitled helper operation that
creates an HCS process and returns only daemon-owned duplicates of its I/O
handles, plus a MASH-side child abstraction that can wait for and manage that
HCS process. The service must retain HCS system/process handles for scoped
termination; the daemon must never receive the service's privileged HCS
handles directly.

## What was verified in MALT

- The named-pipe helper has a live authenticated VNP hello/ack round trip.
- HCS create/start/terminate is helper-owned and session-entitlement scoped.
- Fake HCS coverage verifies that a failed start tears down the newly created
  compute system and that an explicit terminate leaves it absent.
- `mash::Env` has exactly one isolation carrier. It carries a Job Object today
  and is deliberately not upgraded to `Container` without a real HCS child.

## What remains unverified and incomplete

- An HCS process spawned for a MALT session through the helper.
- Daemon-side waiting, cancellation, stdin/stdout/stderr relay, redirections,
  pipelines, and job control for that HCS process.
- A live direct-versus-helper comparison on a host with usable Windows image
  layers, and a helper-death-during-operation test against a real session.
- The UAC-decline scenario and a real protocol-version mismatch scenario.

Those are not environment-only omissions: the first three need the explicit
HCS child transport above. They must remain blocked rather than being
represented as a contained session with no child in the compute system.
