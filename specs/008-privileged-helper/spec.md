# Feature Specification: A Privileged Helper That Performs Privileged Operations

**Feature Branch**: `008-privileged-helper`

**Created**: 2026-07-26

**Status**: Draft

**Input**: User description: "A privileged helper that actually performs privileged operations, so that isolation MALT cannot establish unprivileged is either genuinely established or honestly refused."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A privileged operation that did not happen reports failure (Priority: P1)

Someone asks MALT for isolation that requires privilege the daemon does not
hold. Today the privileged helper answers `"stub: operation not yet
implemented, returning success"` for nine of its ten operations — it reports
success for work it never performed. Any caller wired to it would be told its
session was contained when nothing was done.

After this story, every operation the helper cannot perform returns an error
naming the operation and why, and the helper's own capability surface reports
which operations it can actually carry out — never inferring availability from
the protocol accepting a message.

**Why this priority**: This is a fail-open, and it is the same defect spec 007
exists to eliminate, one layer down. It must be removed *before* anything is
built on the helper, because every caller added first inherits it. It is also
the only story that delivers value with no new infrastructure: it can ship
alone, and it makes the system more honest immediately.

**Independent Test**: Invoke every operation the helper exposes on a host
where none can be performed. Assert each returns an error identifying the
operation. Assert none reports success. Fully testable with no privileged
context, no installation, and no container.

**Acceptance Scenarios**:

1. **Given** a helper operation with no implementation for this platform,
   **When** it is invoked, **Then** it returns an error naming the operation
   and the reason, and does not report success.
2. **Given** the helper's capability surface, **When** it is queried, **Then**
   it lists only operations that can actually be carried out here — an
   operation the protocol can encode but that nothing implements is reported
   unavailable.
3. **Given** the existing test asserting stubs return success, **When** the
   suite runs, **Then** that expectation has been inverted rather than
   deleted, so the old behaviour cannot return unnoticed.

---

### User Story 2 - The helper runs with real privilege and its state is visible (Priority: P2)

An operator needs MALT to perform operations the daemon cannot: the container
API refuses any caller that is not an administrator or a member of the
virtualization administrators group. The helper must therefore run in a
privileged context the unprivileged daemon can reach over an authenticated
channel, and the operator must be able to see and reverse that arrangement.

Installation requires elevation and never happens silently or as a side effect
of another command. Status distinguishes *not installed*, *installed but not
running*, and *reachable*, because "isolation unavailable" for those three
reasons calls for three different actions.

**Why this priority**: The privilege boundary is what makes any privileged
operation possible, so it precedes every backend. It is also independently
valuable: an operator can install, inspect and remove the helper and see a
truthful status before any container work exists.

**Independent Test**: Produce all three states on one host — uninstalled,
installed and stopped, installed and running — and confirm status reports each
distinctly. Confirm a request from a process that is not the authorised daemon
is refused. No container operation required.

**Acceptance Scenarios**:

1. **Given** no helper installed, **When** status is queried, **Then** it
   reports not installed, and any operation requiring it fails naming that as
   the cause rather than silently doing nothing.
2. **Given** an install is requested, **When** elevation is declined,
   **Then** installation does not occur and the outcome says so — no partial
   install, no success message.
3. **Given** an installed and running helper, **When** the daemon sends an
   authenticated request, **Then** it is accepted; **and when** an
   unauthorised local process sends the same request, **Then** it is refused.
4. **Given** an installed helper, **When** uninstall is requested and
   elevation granted, **Then** the helper and everything installation created
   are removed, verified by inspection rather than by absence of an error.
5. **Given** a helper whose protocol version does not match the daemon's,
   **When** the daemon connects, **Then** the mismatch is reported and no
   operation is attempted.

---

### User Story 3 - One operation works end to end across the boundary (Priority: P3)

Proving the protocol and the transport is not proving the path. This story
takes the single operation that motivated the boundary — managing a container
compute system — and drives it from the daemon, through the helper, into the
OS, and back.

The observable change: the same call that fails from the daemon with an
access-denied error gets *past* the privilege boundary when routed through the
helper.

**Why this priority**: The first point at which something new is provably true
rather than merely better structured. It depends on both stories above and
delivers the evidence that the boundary is real.

**Independent Test**: On a host with the container feature present, make the
same request directly and through the helper in one run, and assert the two
outcomes differ in exactly the expected way — the direct call refused for lack
of privilege, the helper-routed call not refused for that reason.

**Acceptance Scenarios**:

1. **Given** a host where the daemon's direct call fails for lack of
   privilege, **When** the same operation is routed through an installed
   helper, **Then** it no longer fails for that reason. Any remaining failure
   names a different cause (see Assumptions on images).
2. **Given** a compute system created through the helper, **When** it is torn
   down, **Then** nothing it held remains, verified by enumerating the system
   and finding it absent — not by the teardown call returning success.
3. **Given** the helper becomes unreachable while an operation is in flight,
   **When** the daemon observes this, **Then** it reports the outcome as
   unknown-or-failed and never as succeeded.
4. **Given** a host without the container feature, **When** the operation is
   requested, **Then** it is refused with that as the stated reason, distinct
   from a privilege refusal.

---

### Edge Cases

- **The helper is a privilege-escalation surface.** It performs operations its
  callers cannot. What stops any local process from asking it to create a
  restricted token, mount a filesystem, or bind a port? Covered by FR-010 to
  FR-013; the highest-consequence risk in this feature.
- A captured request is replayed later by another process.
- Two daemons, run by different users, address the same machine-wide helper —
  can one act on the other's sessions?
- The helper is uninstalled while sessions still hold resources it created.
- The daemon restarts while the helper holds resources: can it re-associate
  them, and if not, are they leaked or reported?
- The helper crashes mid-operation, leaving a partially created resource.
- An operation is requested that the protocol can encode but no platform
  implements.
- The daemon is already running elevated — is the helper bypassed, and if so
  does the reported mechanism still say what actually happened?
- Elevation is granted but the privileged context still cannot perform the
  operation (feature absent, group membership missing).

## Requirements *(mandatory)*

### Functional Requirements — honest reporting

- **FR-001**: An operation the helper does not perform MUST return an error
  identifying the operation and the reason. It MUST NOT report success.
- **FR-002**: The helper MUST expose which operations it can actually carry
  out on the current host, and MUST NOT report an operation as available on
  the grounds that the protocol can encode it.
- **FR-003**: A caller MUST be able to distinguish *helper not installed*,
  *helper installed but not reachable*, *operation unsupported here*, and
  *operation attempted and failed*. These call for different actions and MUST
  NOT collapse into one failure.
- **FR-004**: When the helper is unavailable, an operation depending on it
  MUST fail naming that as the cause. It MUST NOT be silently skipped and MUST
  NOT be silently downgraded to something weaker.
- **FR-005**: An operation whose outcome cannot be determined — the helper
  became unreachable in flight — MUST be reported as unknown or failed, never
  as succeeded.

### Functional Requirements — the privilege boundary

- **FR-006**: The helper MUST run in a privileged context able to perform
  operations the daemon cannot.
- **FR-007**: Installation MUST require explicit elevation, MUST be initiated
  by an explicit operator action, and MUST NOT occur as a side effect of any
  other command.
- **FR-008**: Declined or failed elevation MUST leave nothing installed and
  MUST report that outcome.
- **FR-009**: The helper MUST be removable, and removal MUST take away
  everything installation created, verifiable by inspection.

### Functional Requirements — the helper must not become an escalation vector

- **FR-010**: The helper MUST authenticate every request and MUST refuse any
  request it cannot attribute to an authorised MALT daemon.
- **FR-011**: The helper MUST reject replayed requests.
- **FR-012**: Each operation MUST be scoped to resources belonging to the
  requesting principal's own sessions; the helper MUST refuse a request that
  would act on another user's session, or on an arbitrary path or resource of
  the caller's choosing.
- **FR-013**: The helper MUST refuse any operation it cannot validate rather
  than passing unvalidated parameters to a privileged OS call. Operations
  accepting a path, identifier or configuration document MUST validate it
  against what the requesting session is entitled to.
- **FR-014**: Protocol version mismatch between daemon and helper MUST be
  detected before any operation is attempted, and reported.

### Functional Requirements — one mechanism carries isolation

- **FR-015**: Isolation MUST reach a spawned process through exactly one
  mechanism. Today there are two — a token that is created, stored and never
  read, and a working platform-specific handle — and this feature MUST resolve
  that to one rather than adding a third.
- **FR-016**: The surviving mechanism MUST be able to carry a container
  identity, not only a process-grouping handle, so a container backend does
  not require a parallel path.
- **FR-017**: What a session reports about its isolation and what actually
  constrains that session MUST derive from the same source, so the two cannot
  drift apart.
- **FR-018**: Isolation MUST NOT increase implicitly at runtime. A session
  MUST NOT end up more privileged or more contained than it was granted.
  (Spec 007's FR-017 covers containment lost; nothing covered containment
  gained.)

### Key Entities

- **Privileged operation**: A named unit of work the helper performs on behalf
  of a session — its parameters, the session it is scoped to, and whether this
  host can perform it at all.
- **Helper state**: Whether the helper is installed, running and reachable,
  and its protocol version — what status reports and callers branch on.
- **Operation outcome**: Performed, refused with a reason, or indeterminate.
  Distinct from "the call returned", which is what the current stubs report.
- **Isolation carrier**: The single value conveying a session's isolation from
  the daemon to a spawned process, and that reporting reads from.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every operation the helper exposes, invoked on a host that
  cannot perform it, returns an error naming the operation. **Verified by
  invoking all of them and asserting on each outcome**, not by inspecting the
  dispatch code — the current defect is code that reads as deliberate.
- **SC-002**: Zero operations report success without the operation having
  occurred. **Verified by observing the effect** — the resource exists, the
  constraint binds — not by the call returning success.
- **SC-003**: All three helper states (not installed, installed and not
  running, reachable) are produced on one host and reported distinctly, with
  distinct guidance.
- **SC-004**: A request from a local process that is not an authorised daemon
  is refused. **Verified by making the request**, not by reviewing the
  authentication design.
- **SC-005**: A replayed request is refused, verified by capturing a valid
  request and re-sending it.
- **SC-006**: A request naming another user's session, or a resource outside
  the requesting session's entitlement, is refused. Both requests made.
- **SC-007**: On a host with the container feature present, the operation
  refused for lack of privilege when called directly is **not** refused for
  that reason through the helper. Both calls made in the same run, outcomes
  compared.
- **SC-008**: A resource created through the helper leaves nothing behind
  after teardown, **verified by enumerating the resource and finding it
  absent** — not by teardown returning success (mirrors 007 SC-006).
- **SC-009**: Isolation reaches a spawned process through exactly one
  mechanism. Count of distinct carriers is 1; it is 2 today. Verified by User
  Story 3 landing without introducing a parallel path.
- **SC-010**: Uninstall removes everything installation created, verified by
  inspecting for each artefact rather than by uninstall reporting success.

## Assumptions

- **Non-Windows hosts report the helper as unavailable, honestly.** Linux and
  macOS privileged-helper mechanisms differ substantially from a Windows
  service and are out of scope here. On those platforms the helper reports
  itself unavailable and dependent operations are refused with that reason. A
  deliberate limit, not an oversight: it satisfies FR-001 to FR-004 everywhere
  while implementing FR-006 only where it is needed now.
- **The helper is machine-wide, and every operation is scoped to the
  requesting user's own sessions.** A machine-wide privileged context is what
  the platform offers; the risk it creates — one user's daemon acting on
  another's sessions — is answered by FR-012 rather than by per-user helpers.
  A judgement call; the alternative (a helper per user) is worth revisiting in
  planning if scoping proves hard to enforce.
- **The daemon does not gain privilege; only the helper has it.** The
  alternative — running the daemon elevated — would remove the need for this
  feature and is rejected: the daemon handles untrusted input, hosts sessions,
  and executes untrusted commands.
- **Getting past the privilege boundary is the success condition for User
  Story 3, not creating a usable container.** A compute system is image-backed;
  without a base layer the operation may still fail for a different reason
  after privilege is resolved. Acceptance is therefore "no longer refused for
  lack of privilege", which is falsifiable and does not over-promise something
  a separate feature controls.
- **The existing helper protocol shape is kept.** Its ten operations and
  authenticated envelope are the right decomposition; this feature makes them
  real, adds capability reporting, and hardens authentication. Redesigning the
  protocol is not in scope.
- **Breaking change**: callers that today receive success from a stubbed
  operation begin receiving errors. That is the point of User Story 1. No such
  callers exist outside the helper's own crate today, so the practical impact
  falls on the helper's tests — which is why FR-001's acceptance requires the
  existing expectation be *inverted* rather than removed. A caller that needs
  the old behaviour has no alternative and is not meant to: reporting success
  for work not done is the defect.

### Out of scope, and why

- **Image acquisition and layer materialization.** Constitutive of a usable
  container tier, and a separate feature (ADR-0005). Named here because User
  Story 3 makes it tempting to absorb: once privilege is resolved, "just add an
  image" looks like the last mile. It is not — it is a registry client, a
  content-addressed store, and platform-specific materialization.
- **Linux and macOS backends** beyond making their refusals honest.
- **Checkpointing**, including the per-tier restore fidelity the predecessor
  design describes. Adjacent and interesting; not required by anything here.
- **Group-level resource ceilings.** MALT's group policy carries session counts
  and lifecycle only. A known divergence from the predecessor design, recorded
  so it is not mistaken for something this feature broke.
- **Wiring the remaining eight helper operations to real implementations.**
  This feature makes them refuse honestly (User Story 1) and proves the path
  with one of them (User Story 3). Implementing the rest is per-backend work
  that follows.

---

## MALT standing rules for specifications

*Appended by the `malt` preset. Not part of this feature — rules every feature
inherits. Each was learned from a specific failure in this repo.*

### Success criteria must be able to fail

A criterion that would pass on inspection while the defect is fully present is
not a criterion. "Isolation works correctly" and "output appears promptly" both
read fine and prove nothing.

State the measurement **and how it is verified** where that is where the trap
lies:

- by **content**, not by count — a count matches when the wrong bytes arrive
- **byte-for-byte**, including invalid UTF-8 and multi-byte characters split
  across a boundary
- by **inspection**, not by the absence of an error — "no error was raised" is
  how a silent failure stays silent
- by **observing the constraint**, not by a call returning `Ok`

### State breaking changes; do not bury them

If a default changes, or a wire shape changes, or previously-succeeding calls
begin to fail, say so in Assumptions with its consequence, and say what a
caller does instead. A refusal that does not name the alternative converts a
silent problem into a dead end.

### Record judgement calls as assumptions, not silent defaults

Where a decision could reasonably go two ways and you picked one, write it in
Assumptions with the reasoning. Planning can then revisit it as a decision
rather than inherit it as something nobody noticed was chosen.

Reserve `[NEEDS CLARIFICATION]` for choices with no defensible default. Three
maximum.

### Name what is out of scope, and why

Especially the adjacent thing this feature makes more visible or more
tempting. Constitution IX: a scope-jump written down is a proposal; one
absorbed mid-feature is how this project was abandoned three times.
