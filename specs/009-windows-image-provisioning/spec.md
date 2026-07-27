# Feature Specification: Windows contained-image provisioning

**Feature Branch**: `009-windows-image-provisioning`

**Created**: 2026-07-27

**Status**: Complete

**Input**: Provide MALT-owned Windows image and layer provisioning so a contained session can use a verified image, then return to and complete the privileged-helper containment proof.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Provision a Windows base image (Priority: P1)

An operator obtains a supported Windows base image for MALT and can inspect
its identity, platform, operating-system version, layer set, and preparation
status before allowing it to be used for contained sessions.

**Why this priority**: A contained session cannot exist without a trustworthy
base image. This is the prerequisite that Spec 008 intentionally left out.

**Independent Test**: On a Windows host with the Containers feature, provision
a public Windows base image and verify that MALT reports its immutable identity
and whether it is ready for the current host.

**Acceptance Scenarios**:

1. **Given** no local copy of a supported public Windows base image, **When**
   the operator provisions it by reference, **Then** MALT records its immutable
   manifest identity, ordered layer set, platform metadata, and source
   reference.
2. **Given** a provisioned image, **When** the operator inspects it, **Then**
   MALT reports whether it is ready for use on this host and explains any
   refusal without claiming that a merely downloaded image is runnable.
3. **Given** a non-Windows image, an unsupported platform variant, or data
   whose digest does not match its manifest, **When** it is provisioned,
   **Then** MALT refuses it and leaves no selectable image record.

---

### User Story 2 - Start an actually contained Windows session (Priority: P1)

An operator requests a required contained session using a ready image. MALT
creates a private writable workspace over the approved image and runs the
session's external commands inside that containment boundary.

**Why this priority**: This is the user-visible outcome that makes the
privileged helper useful rather than merely reachable.

**Independent Test**: Create a required contained session from a ready image,
run an external command that identifies its Windows environment, and verify
that MALT reports containment as established. Destroy the session and verify
the compute system and writable workspace are gone.

**Acceptance Scenarios**:

1. **Given** a ready image and a reachable enrolled helper, **When** the
   operator creates a required contained session, **Then** the request creates
   one contained session with a private writable workspace and reports the
   selected image identity.
2. **Given** that session, **When** an external command is run, **Then** it is
   executed inside the contained environment rather than by the host process
   path.
3. **Given** contained setup fails after it begins, **When** MALT reports the
   failure, **Then** no session, compute system, or writable workspace remains.

---

### User Story 3 - Operate and retire provisioned images safely (Priority: P2)

An operator can list, inspect, and remove provisioned images without deleting
an image still required by an active contained session. MALT retains enough
evidence to diagnose why an image is unavailable after a host or image change.

**Why this priority**: Image preparation has privileged host state and can be
large. Operators need a safe lifecycle rather than an opaque cache.

**Independent Test**: Provision two images, use one for a contained session,
attempt to remove both, and verify that only the unused image can be removed.

**Acceptance Scenarios**:

1. **Given** multiple provisioned images, **When** the operator lists them,
   **Then** each shows its immutable identity, readiness, and active-use state.
2. **Given** an image used by an active contained session, **When** removal is
   requested, **Then** MALT refuses and names the dependent session.
3. **Given** an unused image, **When** removal is requested, **Then** its
   MALT-owned prepared state and metadata are removed without affecting any
   other image or session.

### Edge Cases

- A download, extraction, verification, or preparation failure leaves no
  partially selectable image or orphaned writable workspace.
- The host changes after an image was prepared; MALT re-evaluates readiness
  before the next contained-session request and refuses stale state honestly.
- An image has multiple platform variants; MALT selects only the applicable
  Windows variant or refuses with an actionable reason.
- The helper stops during preparation or session construction; the caller sees
  an indeterminate result and MALT does not report containment as established.
- Docker Desktop runs Linux containers or is absent; image provisioning and
  contained-session availability do not depend on Docker's active engine mode.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: MALT MUST let an operator provision a public Windows OCI image
  by reference and retain its immutable manifest identity, ordered layer
  identities, platform metadata, and source reference.
- **FR-002**: MALT MUST accept only the Windows platform variant applicable to
  the current host and refuse image data whose declared or downloaded identity
  cannot be verified.
- **FR-003**: MALT MUST distinguish an image that was acquired from one that
  was successfully prepared and proven usable for a contained session on the
  current host.
- **FR-004**: MALT MUST prepare the selected read-only layer chain and create
  a distinct writable workspace for each contained session without allowing a
  daemon request to supply arbitrary privileged layer paths or configuration.
- **FR-005**: MALT MUST pass only a helper-owned, verified image selection to
  the privileged containment boundary; the boundary MUST refuse raw image
  paths, arbitrary storage documents, and unrecorded layer identities.
- **FR-006**: MALT MUST create a required contained session only after its
  image, writable workspace, helper state, and containment runtime have all
  been established; otherwise it MUST leave no session or containment state.
- **FR-007**: MALT MUST preserve the existing preferred-policy behaviour: an
  unavailable contained environment visibly downgrades only when the operator
  explicitly requested preferred policy, and required policy refuses.
- **FR-008**: MALT MUST provide operator commands to provision, list, inspect,
  and remove provisioned images, with clear reasons for unavailable or
  in-use images.
- **FR-009**: MALT MUST prevent removal of an image, prepared layer set, or
  writable workspace while it is required by an active contained session.
- **FR-010**: MALT MUST clean up MALT-owned partial provisioning state and
  writable workspaces after every failed or terminated lifecycle path, while
  never deleting data it does not own.
- **FR-011**: MALT MUST re-evaluate an image's readiness after relevant host
  state changes and report the concrete reason when a previously provisioned
  image can no longer be used.
- **FR-012**: MALT MUST support this workflow independently of Docker
  Desktop's active Linux or Windows container engine.
- **FR-013**: MALT MUST record live evidence of a successful contained command
  and of cleanup after destruction, then use that evidence to complete the
  blocked Spec 008 containment scenarios.

### Key Entities

- **Provisioned Image**: An operator-selected Windows image with immutable
  identity, platform details, source reference, readiness state, and ordered
  layer set.
- **Prepared Layer Set**: The MALT-owned, host-ready representation of one
  verified image's read-only layers, including preparation evidence and
  ownership boundaries.
- **Writable Workspace**: A private, session-scoped writable environment built
  over a prepared layer set and removed when its contained session ends.
- **Image Readiness Assessment**: The current host's verdict on whether a
  provisioned image can be used, including the reason and evidence basis.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can provision and inspect a supported public Windows
  base image in one workflow, with 100% of displayed image identities matching
  the recorded manifest identity and layer set.
- **SC-002**: A required contained-session request using a ready image creates
  one contained session and executes a verification command inside it; MALT
  reports the selected image identity and established containment.
- **SC-003**: In each tested setup, process launch, and teardown failure path,
  inspection finds zero orphaned sessions, compute systems, or MALT-owned
  writable workspaces.
- **SC-004**: 100% of tested unsupported, mismatched, tampered, stale, and
  helper-unavailable image requests are refused with a concrete reason and
  never reported as contained.
- **SC-005**: The same Windows-image provisioning workflow succeeds with
  Docker Desktop in Linux-container mode or with Docker Desktop unavailable.
- **SC-006**: An image in active use cannot be removed; after its last
  contained session is destroyed, the operator can remove it and inspection
  finds none of its MALT-owned prepared state.

## Assumptions

- The first release supports publicly accessible Windows OCI images; private
  registry authentication and importing arbitrary archive files are out of
  scope.
- Windows containment remains the first implementation target. Linux and
  macOS image provisioning are separate work and continue to report their
  actual availability.
- Vexil-v2 is reference material only. MALT will vendor no code or runtime
  dependency from it and will not rely on its existing image cache.
- The privileged helper from Spec 008 remains the sole owner of privileged
  containment operations. This feature supplies verified image state to that
  boundary; it does not weaken its entitlement model.
