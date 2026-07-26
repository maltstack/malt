# ADR-0006: Establish a User-Approved, Helper-Owned Session Entitlement Authority

Date: 2026-07-26
Status: Accepted

## Context

Spec 008 has a live LocalSystem helper service, a SID-restricted named pipe,
OS-attributed client PIDs, explicit-UAC install/uninstall, and generated VNP
messages. Those are necessary transport controls, but they are not an
entitlement authority.

The helper must decide whether a request may affect a session's process or
files. The required evidence is: the session owner, the canonical storage
root, and the live process identities belonging to that session. The current
persisted schema records only session ID, layout, panes, working directory,
and requested isolation (`schemas/persist/session.vexil:7-18,27-31`). It has
no owner, storage root, PID set, signed enrollment record, or other
helper-verifiable authority. The daemon's current runtime data cannot become
that authority merely by being sent over the pipe: an unprivileged daemon may
be compromised or impersonated.

A pipe ACL and peer SID establish only which Windows user opened the pipe.
They cannot establish that the client is an operator-approved MALT daemon or
that it owns an arbitrary `session_id`, path, or PID. Dispatching a privileged
request on that basis would make the service a same-user escalation surface.
The helper therefore currently returns `Refused{NotEntitled}` for every
operation after the authenticated handshake.

## Decision required

Adopt an explicit **user-approved daemon enrollment** as the trust anchor,
then let the helper own an append-only entitlement registry for the lifetime
of that enrollment.

1. `malt elevate authorize-daemon <pid>` is an explicit UAC action. The
   elevated helper validates the PID exists, records its owner SID, creation
   time, image identity, and a random enrollment ID, and grants access only
   to that exact live process. PID reuse is rejected by comparing creation
   time; a different SID or image is rejected.
2. The enrolled daemon creates/removes session entitlement records through a
   narrow, typed registration channel. The helper rejects registrations that
   do not match the enrolled process identity and owner SID. A record contains
   session ID, owner SID, canonical storage root, and observed child process
   identities (PID plus creation time).
3. Before every privileged operation, the helper rechecks the caller process
   identity and resolves every requested path before verifying containment in
   the record's canonical root. It rechecks PID identity against the OS and
   the record. The typed HCS document is rendered by the helper from the
   entitlement and operation fields; callers never supply an opaque document.
4. Enrollment and all session records are revoked when the enrolled process
   exits, the service is uninstalled, or an explicit revoke action succeeds.
   Registry state must be owned by the service and protected from the
   unprivileged user's write access.

This is deliberately an operator approval of a concrete daemon process, not
a daemon-supplied registry. It is the narrowest Windows-native anchor that
does not require introducing code-signing/TPM infrastructure that MALT does
not currently possess.

## Alternatives rejected

- **Trust a daemon-supplied in-memory or persisted registry.** It does not
  independently validate anything at the privilege boundary and fails
  FR-012/FR-013 by construction.
- **Treat pipe SID as daemon identity.** Any same-user process has the SID;
  it cannot distinguish the daemon from a local attacker.
- **Allow arbitrary image paths or PIDs after a one-time service install.**
  This moves the arbitrary-privileged-action input from the request union to
  an equally untrusted registration API.
- **Require code signing now.** This can become a stronger distribution-time
  control, but MALT has no signing identity or release-key process. Adding it
  here would turn a required authority decision into unscoped release
  infrastructure.

## Consequences

- `malt elevate install` remains an explicit service lifecycle operation; it
  does not silently authorize a daemon.
- A separate explicit authorization/revocation UX and a helper-owned registry
  are required before T024, T031-T033, T038-T043 can become true.
- The current fail-closed service is retained until this ADR is accepted and
  implemented. No request path may bypass it.
