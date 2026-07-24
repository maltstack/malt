# Feature Specification: CLI Isolation Flag

**Feature Branch**: `main`

**Created**: 2026-07-24

**Status**: Draft

**Input**: User description: "the `malt new --isolation` CLI flag from the backlog"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create a Session at a Selected Isolation Tier (Priority: P1)

An operator creating a new MALT session can select the desired isolation tier
at the command line, so the session is created with the intended execution
boundary without requiring a separate API call.

**Why this priority**: The primary command-line workflow currently cannot
express an isolation choice even though session creation already supports one.

**Independent Test**: Run `malt new` once for each supported tier and verify
that the created-session result identifies the same tier that was requested.

**Acceptance Scenarios**:

1. **Given** a reachable daemon, **When** an operator runs `malt new --isolation restricted`, **Then** one new session is created and its result identifies the isolation tier as Restricted.
2. **Given** a reachable daemon, **When** an operator runs `malt new --isolation capped --name build`, **Then** one new session named build is created and its result identifies the isolation tier as Capped.
3. **Given** a reachable daemon, **When** an operator selects Bare, Restricted, Capped, or Contained, **Then** the requested tier is passed unchanged to session creation.

---

### User Story 2 - Preserve the Existing Default Session Workflow (Priority: P2)

An operator who does not need an isolated session can continue using the
existing `malt new` command without changing scripts or habits.

**Why this priority**: Adding a security-related option must not make the
default workflow surprising or break existing automation.

**Independent Test**: Run `malt new` without the option and verify that it
creates one session reporting the existing Bare default.

**Acceptance Scenarios**:

1. **Given** a reachable daemon, **When** an operator runs `malt new` without `--isolation`, **Then** a session is created with the Bare tier and the result identifies it as Bare.
2. **Given** a command that combines `--name` with a valid isolation tier, **When** it is run, **Then** both choices are applied to the same newly created session.

---

### User Story 3 - Receive Clear Input and Creation Failures (Priority: P3)

An operator receives a clear failure instead of an unintended session when an
isolation choice is invalid, unavailable, rejected, or not honored.

**Why this priority**: A silent downgrade to Bare would misrepresent the
security boundary the operator requested.

**Independent Test**: Try an unsupported option value and a simulated
creation failure, then verify that each command exits unsuccessfully and does
not print a successful creation result.

**Acceptance Scenarios**:

1. **Given** an unsupported isolation value, **When** an operator invokes `malt new --isolation <value>`, **Then** the command rejects the value before requesting session creation and explains the accepted values.
2. **Given** session creation rejects the requested tier or cannot satisfy it, **When** the command is run, **Then** it exits unsuccessfully with the returned reason and does not claim that a session was created.
3. **Given** session creation reports a tier different from the valid tier requested by the operator, **When** the command receives that result, **Then** it exits unsuccessfully and clearly identifies the requested and reported tiers.

### Edge Cases

- The command is invoked with `--isolation` but no value: it is rejected as invalid command input, with usage guidance and no session-creation request.
- The daemon is unreachable or returns an unreadable creation result: the command reports failure and does not print a successful creation message.
- A platform cannot provide a requested tier: the session-creation authority reports the limitation; the command preserves that failure rather than retrying at Bare.
- The option is omitted by existing interactive use or automation: the established Bare default remains in effect.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST allow `malt new` to accept an optional `--isolation` value.
- **FR-002**: The option MUST accept exactly the canonical tier values `bare`, `restricted`, `capped`, and `contained`.
- **FR-003**: When a valid tier is supplied, the system MUST request creation of the new session using that exact tier.
- **FR-004**: When the option is omitted, the system MUST preserve the existing `malt new` behavior and create a Bare session.
- **FR-005**: The system MUST allow `--name` and `--isolation` to be used together for the same new session.
- **FR-006**: On successful creation, the command MUST identify the created session and its reported isolation tier in its user-visible result.
- **FR-007**: The command MUST reject an unsupported `--isolation` value before attempting to create a session and identify the accepted values.
- **FR-008**: If session creation fails, including because the selected tier is unavailable, the command MUST exit unsuccessfully, preserve the failure reason where available, and MUST NOT report success.
- **FR-009**: If the creation result reports a tier different from the valid tier requested, the command MUST treat the operation as unsuccessful and clearly report both tiers; it MUST NOT silently accept or represent the session as successfully created.
- **FR-010**: The feature MUST NOT change the behavior of other `malt` subcommands or the no-subcommand automatic workflow.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All four supported tier values create a session that reports the exact selected tier when the daemon can satisfy the request.
- **SC-002**: 100% of valid combinations of an optional name and one of the four supported tiers result in one session carrying both requested properties.
- **SC-003**: 100% of unsupported tier values and missing tier values are rejected before a session-creation request is made.
- **SC-004**: Existing `malt new` invocations without `--isolation` continue to create a session reporting Bare.
- **SC-005**: In every creation failure or reported-tier mismatch, the command produces no success message and makes the reason actionable for the operator.

## Assumptions

- Session creation remains the authority for whether a requested tier is available on the current platform and for enforcing the tier after creation.
- The canonical lower-case tier names are the stable command-line contract; alternate spellings are outside this feature's scope.
- The established default when no isolation tier is requested is Bare.
- This feature exposes existing session-creation capability only; it does not change platform isolation semantics, resource policies, or the no-subcommand automatic workflow.
