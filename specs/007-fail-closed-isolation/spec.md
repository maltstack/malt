# Feature Specification: Fail-Closed Session Isolation

**Feature Branch**: `007-fail-closed-isolation`

**Created**: 2026-07-26

**Status**: Draft

**Input**: User description: "When someone asks for a session to be contained, it must either genuinely be contained or the request must fail — never succeed while running uncontained."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A containment request is honoured or refused, never quietly downgraded (Priority: P1)

Someone creates a session and asks for it to be contained, because they are
about to run something they do not fully trust — an agent acting on its own,
an unfamiliar build script, a dependency's install hook. Today, if the
containment cannot be established, the session is created anyway and runs
with no containment at all. The request appears to have succeeded.

Asking for containment must produce containment, or produce a failure. It
must never produce a session that looks contained and is not.

**Why this priority**: This is the whole feature, and it is a trust problem
rather than a missing capability. A missing feature makes someone do
something else; a capability that silently does not work makes them proceed
as though it did. Everything else here supports being able to tell the
difference.

**Independent Test**: Request containment in a situation where it cannot be
established, and confirm the request fails with a reason rather than
returning a usable session.

**Acceptance Scenarios**:

1. **Given** containment is requested and can be established, **When** the
   session is created, **Then** it is created and genuinely contained.
2. **Given** containment is requested and cannot be established, **When**
   creation is attempted, **Then** it fails with a reason naming what could
   not be provided — and no session is left running uncontained.
3. **Given** a caller states it prefers containment but will accept less,
   **When** containment cannot be established, **Then** the session is
   created and the caller is told, in the same response, that it did not get
   what it asked for and what it got instead.
4. **Given** a caller states it does not want containment, **When** the
   session is created, **Then** no containment is attempted and nothing
   fails on account of it.
5. **Given** containment is established but later fails or is torn down,
   **When** that is detected, **Then** it is surfaced rather than left as a
   session that still claims to be contained.

---

### User Story 2 - What is actually in force can be seen (Priority: P2)

Someone with a running session — or a program managing several — needs to
know what containment each one actually has, not what was asked for. Today a
session reports the level that was requested, so the report is a restatement
of the question rather than an answer.

**Why this priority**: Fail-closed creation (US1) protects the moment a
session starts. This protects every moment after, and it is what makes the
guarantee auditable rather than something to take on faith. It is separable:
US1 can ship and be verified before any reporting surface changes.

**Independent Test**: Create sessions at different levels, ask each what
containment it has, and confirm the answer reflects what was established
rather than what was requested — including a session where the request was
downgraded.

**Acceptance Scenarios**:

1. **Given** a session that was granted containment, **When** its isolation
   is queried, **Then** it reports the level actually in force.
2. **Given** a session created under a preference that was downgraded,
   **When** its isolation is queried, **Then** it reports the level actually
   in force, not the level requested.
3. **Given** a session whose containment level cannot be confirmed, **When**
   its isolation is queried, **Then** it reports that it is unverified rather
   than asserting a level. Not knowing must be distinguishable from knowing.
4. **Given** a list of sessions, **When** it is retrieved, **Then** each
   entry's containment reflects what is in force.

---

### User Story 3 - Distinct levels actually differ (Priority: P2)

Someone chooses between containment levels expecting them to mean different
things — a lightly restricted session for ordinary work, a strongly contained
one for something untrusted. Today two of the levels are indistinguishable in
what they enforce, so the choice between them has no effect.

**Why this priority**: A level that does not differ from a weaker one is a
more subtle version of the same trust problem: the choice is offered, made,
and ignored. It is separable from US1 and US2 — a level can be honestly
reported and honestly refused while still being weaker than advertised.

**Independent Test**: Run identical work in a session at each level and
confirm that a constraint enforced at a stronger level is observably not
enforced at a weaker one.

**Acceptance Scenarios**:

1. **Given** two different containment levels, **When** the same work runs
   under each, **Then** at least one constraint is demonstrably enforced at
   the stronger level and not at the weaker one.
2. **Given** the strongest level, **When** a session runs under it, **Then**
   the containment it provides is the one that level names — not a weaker
   mechanism reported under a stronger name.
3. **Given** a level that cannot be provided as specified, **When** it is
   requested as required, **Then** it fails per US1 rather than resolving to
   whatever is available.

---

### User Story 4 - Containment beyond the primary platform (Priority: P3)

Someone runs the product on a platform other than the one it was developed
against and asks for containment. Today nothing is enforced there, and
nothing says so.

Either containment works on that platform, or asking for it fails there —
and which one it is must be discoverable before a session is created, not
after something untrusted has already run.

**Why this priority**: Real, and it decides whether the product's containment
guarantee means anything away from its primary platform. It is last because
US1 already makes the unsafe case *safe* — a required request fails where
nothing can be enforced — so this story is about restoring capability rather
than closing the trust gap.

**Independent Test**: On a platform with no enforcement path, request
required containment and confirm it fails with a clear reason; then confirm
the same request succeeds and enforces where a path does exist.

**Acceptance Scenarios**:

1. **Given** a platform with no enforcement path, **When** containment is
   required, **Then** creation fails and says containment is unavailable on
   this platform.
2. **Given** a platform with no enforcement path, **When** containment is
   merely preferred, **Then** the session is created and reports that it has
   none.
3. **Given** a caller deciding what to ask for, **When** it asks what
   containment this system can provide, **Then** it can find out **before**
   creating a session.

---

### Edge Cases

- **Containment is established for the session but a later process escapes
  it** — started by a different path than the one that was contained, or
  spawned in a way that does not inherit it. The session would still report
  containment truthfully at the session level while something ran outside it.
- **Partial establishment**: some constraints of a level apply and others do
  not. This must not report as the full level, and under a required policy it
  is a failure.
- **A restored session.** A session that comes back after a restart must
  re-establish its containment or report that it no longer has it. Restoring
  the *record* of a contained session without the containment is precisely
  the false claim this feature exists to prevent.
- **A request for a level the system does not recognise.**
- **Containment that cannot be verified even though establishing it seemed to
  succeed** — the mechanism reported success but the constraint is not
  observable.
- **Teardown**: when a session ends, its containment and everything held
  inside it must be released, so a failure to contain is not traded for a
  leak.
- **A caller that never states a policy**, and what the default must be for
  it to be safe.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When containment is required and cannot be established, session
  creation MUST fail. No session may be left running.
- **FR-002**: A failure to establish containment MUST state what was
  requested, what could not be provided, and why.
- **FR-003**: A caller MUST be able to state whether containment is
  **required**, **preferred**, or **not wanted**.
- **FR-004**: Under a preference, a downgrade MUST be reported to the caller
  at creation time — not only discoverable later by asking.
- **FR-005**: A session MUST report the containment actually in force, never
  the level requested.
- **FR-006**: A session whose containment cannot be confirmed MUST report it
  as unverified, distinguishably from both "contained" and "not contained".
- **FR-007**: Every surface that reports a session's containment MUST report
  the same value — creating, querying, and listing MUST NOT disagree.
- **FR-008**: Distinct containment levels MUST enforce distinguishably
  different constraints; two levels MUST NOT be aliases.
- **FR-009**: A level MUST be provided by the mechanism that level denotes,
  or refused; it MUST NOT be satisfied by a weaker mechanism reported under
  its name.
- **FR-010**: A caller MUST be able to discover which containment levels this
  system can actually provide, before creating a session.
- **FR-011**: On a platform where a level cannot be enforced, requesting it
  as required MUST fail rather than appear to succeed.
- **FR-012**: Processes started by a contained session MUST be subject to its
  containment.
- **FR-013**: When a session ends, its containment and the resources held
  within it MUST be released.
- **FR-014**: A restored session MUST re-establish its containment or report
  that it no longer holds it.
- **FR-015**: An unrecognised containment level MUST be rejected with a
  message naming the valid ones.
- **FR-016**: Containment status MUST be observable under the same access
  rules as other session state, so it does not become a way to learn about
  sessions a caller could not otherwise see.
- **FR-017**: Loss of containment detected after creation MUST be surfaced
  rather than leaving the session reporting a level it no longer has.

### Key Entities

- **Containment level**: a named strength of isolation, with the specific
  constraints it promises. Levels are ordered; each must be distinguishable
  from its neighbours by an observable constraint.
- **Containment policy**: how much the caller cares — required, preferred, or
  not wanted. Distinct from the level: it decides what happens when the level
  cannot be provided.
- **Containment status**: what is actually in force for a session — a level,
  or none, or unverified — together with whether it was downgraded from what
  was requested.
- **Platform capability**: which levels this system can currently provide,
  discoverable before a session is created.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100% of attempts where containment is required and cannot be
  established, no session exists afterwards and the caller receives a reason.
  Zero sessions are created uncontained under a required policy.
- **SC-002**: For every session, the containment reported at creation, when
  queried, and when listed is identical — verified across all three surfaces
  for a granted, a downgraded, and a refused request.
- **SC-003**: A downgrade is visible in the creation response itself; no
  caller must make a second request to discover it.
- **SC-004**: For each adjacent pair of containment levels, there is at least
  one constraint demonstrably enforced at the stronger and not at the weaker,
  shown by running the same work under both.
- **SC-005**: A process started inside a contained session is subject to that
  containment, demonstrated by a constraint that applies to it and not to the
  same process in an uncontained session.
- **SC-006**: After a contained session ends, no process or resource it held
  remains, verified by inspection rather than by the absence of an error.
- **SC-007**: A caller can enumerate the containment levels this system can
  provide without creating a session, and the answer matches which requests
  subsequently succeed.
- **SC-008**: A session restored after a restart reports containment that
  matches what it actually holds — never a level inherited from its saved
  record alone.

## Assumptions

- **Requesting a level above the default means "required" unless stated
  otherwise.** Fail-closed is the point of the feature, and a default of
  "preferred" would preserve exactly today's behaviour for every existing
  caller. This is a deliberate behaviour change: callers on platforms or
  systems that cannot provide a level will start seeing failures where they
  previously got a silently uncontained session. That is the intended
  outcome — but it is a breaking change and should be called out in the
  release notes rather than discovered.
- **The default level itself is unchanged.** A caller that asks for nothing
  gets what it gets today, and nothing about that path starts failing.
- **"Verified" means observed, not merely returned.** A mechanism reporting
  success is not sufficient to claim a level; where the constraint can be
  observed it should be, and where it cannot, the status is unverified rather
  than assumed good.
- **Scope excludes new containment levels.** This feature makes the existing
  levels honest; it does not add stronger ones.
- **Scope excludes containment across machines.** Remote and shared
  deployments are out of scope, consistent with the current priority order.
- **Scope excludes non-process resources** such as network and filesystem
  restriction, except where an existing level already promises them. Making
  the current promises true comes before adding new ones.
- **This closes the reopened fail-closed requirement of the CLI isolation
  flag.** That spec's surface already ships; what is missing is exactly this
  guarantee, so it should be closed by this work rather than separately.
