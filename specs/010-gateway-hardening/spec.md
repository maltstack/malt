# Feature Specification: Gateway Hardening — Limits an Untrusted Caller Cannot Escape

**Feature Branch**: `010-gateway-hardening`

**Created**: 2026-07-28

**Status**: Draft

**Input**: User description: "Boundary hardening: what an untrusted or compromised peer can send MALT, over both HTTP and VNP, and what happens when they do."

## Scope was cut in half by verification — read this first

The request named four problems. Checking each against code on 2026-07-28
removed two, and the removals are the useful part:

| Claimed | Verified |
|---|---|
| `FrameWriter` casts length to `u32` unchecked (`docs/briefs/004`) | **Already fixed.** `framing.rs:203` bounds `payload_len` against `PROTOCOL_MAX_FRAME_SIZE` before the cast, so both failure modes the brief described are prevented. Brief 004 is stale and should be marked resolved |
| Per-endpoint scope "exists as a concept and is not verified route by route" | **Enforced, and fails closed.** `middleware.rs:30-32` maps `(Method, path)` to a scope with `_ => AuthScope::Admin`. An unmapped route demands the *highest* scope, so a forgotten route is unreachable rather than open |

**This feature is therefore HTTP-only.** The VNP framing boundary is already
symmetric, which also settles the design question the request posed — whether
the two boundaries need the same treatment. They do not, because VNP's is
already done.

The scope mapping living in a table separate from the router is a real
maintainability concern — the "value that re-derives its own truth" shape from
AGENTS.md — but its failure mode is a new route demanding too *much*
privilege, not too little. That is an availability bug, not a security hole,
and it is **out of scope** here.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A throttled caller recovers (Priority: P1)

An agent drives the Gateway hard enough to hit the request limit. Today it is
refused and **stays refused forever** — the limiter counts requests and has no
notion of time. `RateLimiter` exposes `refill()` and `refill_all()`, and
**nothing in production calls either**, so the only thing that restores service
is restarting the daemon.

The limiter is wired into `build_router`, so this is live on every route. An
agent that briefly bursts has permanently removed its own access.

After this story, a caller that exceeds its allowance is refused *for a bounded
period* and then served again, with no operator action.

**Why this priority**: It is the only item here actively breaking a supported
workflow rather than failing to prevent a hypothetical one. It also worsens
with adoption — the more an agent uses MALT, the sooner it bans itself.

**Independent Test**: Exhaust the allowance, confirm refusal, wait the window,
confirm service resumes — with no restart and no manual `refill` call.

**Acceptance Scenarios**:

1. **Given** a client that has exhausted its allowance, **When** the window
   elapses, **Then** its next request is served — **verified by making the
   request**, not by inspecting counters.
2. **Given** a client under the limit, **When** it continues at a sustainable
   rate, **Then** it is never refused.
3. **Given** two clients, **When** one exhausts its allowance, **Then** the
   other is unaffected.
4. **Given** a long-running daemon, **When** many clients have connected and
   gone, **Then** limiter state does not grow without bound.

---

### User Story 2 - An oversized request is refused before it is buffered (Priority: P2)

`POST /sessions/{id}/exec` and `POST /sessions/{id}/send` accept a body with no
size limit, so a caller can make the daemon buffer an arbitrarily large
payload. The daemon holds sessions for other callers; memory exhausted by one
request is denied to all of them.

The refusal must happen **before** the body is read, not after.

**Why this priority**: A single request can degrade the whole daemon, and
unlike US1 it needs no sustained effort. It is P2 rather than P1 only because
it requires deliberate abuse, whereas US1 fires during ordinary heavy use.

**Independent Test**: Send a body larger than the limit; confirm rejection with
a size-related status, and confirm the daemon's memory does not rise by the
payload size while it happens.

**Acceptance Scenarios**:

1. **Given** a request whose declared length exceeds the limit, **When** it
   arrives, **Then** it is refused without the body being read.
2. **Given** a request that declares no length but streams past the limit,
   **When** the limit is crossed, **Then** it is refused mid-stream rather than
   buffered to completion.
3. **Given** a request within the limit, **When** it arrives, **Then** it is
   served normally — the limit must not be so tight that legitimate commands
   fail.

---

### User Story 3 - A refused caller is told how to proceed (Priority: P3)

A refusal today carries no indication of when to retry. A well-behaved agent
cannot distinguish "slow down" from "you are banned", so its only strategy is
to guess — and guessing wrong turns throttling into a retry storm.

There is also no ceiling across *all* clients. Per-client limits do not stop
many clients, or one client holding several identities, from saturating the
daemon together.

**Why this priority**: It makes the other two usable rather than merely
correct. Independently deliverable: retry hints are valuable before a global
ceiling exists.

**Independent Test**: Trigger a refusal, read the response, and confirm a
client can compute when to retry from the response alone.

**Acceptance Scenarios**:

1. **Given** a refused request, **When** the caller reads the response, **Then**
   it can determine when to retry without guessing.
2. **Given** many clients each within their own allowance, **When** their
   combined rate exceeds the daemon's ceiling, **Then** requests are refused
   with the same retry information.
3. **Given** a system-wide refusal, **When** the caller inspects it, **Then** it
   can tell the cause was system-wide rather than its own quota — the two call
   for different responses.

---

### Edge Cases

- The clock moves backwards, or a suspended machine resumes. A window trusting
  wall-clock time can lock a caller out for the duration of the jump.
- A client identifier is caller-controlled: can a caller evade its own limit by
  varying it, or exhaust memory by inventing many?
- Limiter state for clients that never return — does it accumulate forever?
- A request declaring a small length but sending more.
- Concurrent requests from one client arriving together at the boundary of the
  allowance.
- A body under the limit that expands once decoded — is the limit applied to
  what is received or to what is materialised?

## Requirements *(mandatory)*

### Functional Requirements — recovery

- **FR-001**: A caller refused for exceeding its allowance MUST be served again
  after a bounded period, with no operator action.
- **FR-002**: Allowance MUST be restored by the passage of time, not by an
  external caller invoking a method. (`refill`/`refill_all` exist today and
  nothing calls them, which is precisely why the limit is permanent.)
- **FR-003**: One caller exhausting its allowance MUST NOT affect another's.
- **FR-004**: Limiter state MUST NOT grow without bound as clients come and go.

### Functional Requirements — size

- **FR-005**: Request bodies MUST be bounded, and an oversized request MUST be
  refused **before** its body is buffered.
- **FR-006**: The bound MUST NOT be so small that ordinary commands and input
  fail; this is a defence against abuse, not a feature restriction.
- **FR-007**: A request that understates its size and then exceeds the bound
  MUST be refused when the bound is crossed.

### Functional Requirements — legibility

- **FR-008**: A refusal MUST carry enough information for a caller to determine
  when to retry.
- **FR-009**: A per-caller refusal MUST be distinguishable from a system-wide
  one, since a caller should back off differently in each case.
- **FR-010**: The daemon MUST have a ceiling on aggregate request rate, not
  only per-caller limits.

### Functional Requirements — honesty

- **FR-011**: Every limit MUST be enforced where it is claimed to be. A limit
  that exists as a type but is never consulted is the defect this repo has hit
  repeatedly — `refill` is a live example — and MUST NOT be reintroduced by
  this work.
- **FR-012**: Refusals MUST be observable, so an operator can tell a limit from
  an outage.

### Key Entities

- **Allowance**: What one caller may spend before refusal, and when it returns.
  Time-bounded by construction, not by an external reset.
- **Refusal**: Why a request was rejected — quota, size, or system-wide — and
  when the caller may try again. The distinction is the point; a bare status
  code carries none of it.
- **Ceiling**: The aggregate limit across all callers, independent of any one
  caller's allowance.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A client exhausts its allowance, waits, and is served again —
  **without a daemon restart and without any code calling `refill`**. Verified
  by making the request, not by reading counters.
- **SC-002**: A sustained, sustainable request rate is never refused across a
  run of at least several windows.
- **SC-003**: One client's exhaustion leaves a second client unaffected,
  verified by exercising both.
- **SC-004**: An oversized body is refused **and the daemon's memory does not
  rise by the payload size**. Asserting only the status code would pass while
  the buffering defect is fully present.
- **SC-005**: A payload at the documented ceiling for ordinary use succeeds, so
  the limit provably does not break legitimate work.
- **SC-006**: Given only a refusal response, a client can compute a retry time
  and act on it — demonstrated by a client that recovers automatically.
- **SC-007**: A quota refusal and a system-wide refusal are distinguishable by
  a caller that has only the response.
- **SC-008**: Limiter memory, after many distinct clients have connected and
  departed, returns to a bounded level — verified by measurement, not by
  reasoning about the data structure.

## Assumptions

- **The limiter is replaced, not extended.** A fixed count with an external
  reset cannot become time-bounded by addition; the shape is wrong.
  `refill`/`refill_all` are expected to disappear rather than gain a caller — a
  method nothing calls is what produced this bug.
- **Limits are configurable with safe defaults.** An operator may tune them,
  and a daemon started with no configuration is still protected.
- **Windows are approximate.** Precise fairness is not a goal; recovery is. A
  simple scheme whose behaviour is obvious beats a precise one whose behaviour
  is not.
- **Breaking change**: callers that today receive a permanent refusal after N
  requests will begin being served again. Any client working around the bug by
  restarting the daemon or rotating its identifier can stop doing so. Nothing
  that currently succeeds begins failing, except requests exceeding the new
  size bound — which were previously buffered without limit.
- **The clock is monotonic where it matters.** Windows should not be computable
  to a caller's disadvantage by moving wall-clock time.

### Out of scope, and why

- **Brief 004 / VNP frame bounds** — verified fixed 2026-07-28. The write path
  bounds its length before casting. Brief 004 should be marked resolved rather
  than implemented.
- **Per-route scope declaration** — enforcement exists and fails closed to
  `Admin`. The two-sources-of-truth concern is real, but its failure mode is
  excess privilege *demanded*, not granted. Named here so it is not re-derived
  as a security gap; it belongs in the backlog.
- **Authentication** — delivered. This feature assumes an authenticated caller
  and constrains what that caller may then do.
- **The privileged helper's boundary** — spec 008 owns it.
- **Isolation tiers** — specs 007 and 009 own them.
- **VNP peer limits.** VNP peers authenticate before reaching the message loop
  and are bounded by `MAX_PENDING_HANDSHAKES` and the frame size. If
  post-handshake VNP flooding proves a real risk, that is its own feature with
  its own threat model, not an extension of this one.

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
