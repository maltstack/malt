# Specification Quality Checklist: Gateway Hardening

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-07-28
**Feature**: [spec.md](../spec.md)

## Content Quality

- [X] No implementation details (languages, frameworks, APIs)
- [X] Focused on user value and business needs
- [X] Written for non-technical stakeholders
- [X] All mandatory sections completed

## Requirement Completeness

- [X] No [NEEDS CLARIFICATION] markers remain
- [X] Requirements are testable and unambiguous
- [X] Success criteria are measurable
- [X] Success criteria are technology-agnostic (no implementation details)
- [X] All acceptance scenarios are defined
- [X] Edge cases are identified
- [X] Scope is clearly bounded
- [X] Dependencies and assumptions identified

## Feature Readiness

- [X] All functional requirements have clear acceptance criteria
- [X] User scenarios cover primary flows
- [X] Feature meets measurable outcomes defined in Success Criteria
- [X] No implementation details leak into specification

## Notes

**Half the requested scope was removed by verification**, which is the most
useful thing this pass produced. Both removals are recorded at the top of the
spec rather than dropped silently:

1. **Brief 004 (VNP frame writer bound) is stale.** `framing.rs:203` already
   bounds `payload_len` against `PROTOCOL_MAX_FRAME_SIZE` before the cast, so
   both failure modes the brief describes are prevented. Specifying it would
   have meant building something that exists — the exact trap the roadmap
   survey caught three times on 2026-07-28. **Brief 004 should be marked
   RESOLVED in `docs/briefs/README.md`.**
2. **Per-endpoint scope enforcement is not a hole.** `middleware.rs` maps
   `(Method, path)` to a scope and defaults `_ => AuthScope::Admin`, so an
   unmapped route demands the highest scope. The concern is real but is a
   maintainability issue with a fail-closed mode; it is named as out of scope
   rather than quietly kept in.

**A consequence worth noting**: removing the VNP half also settled the design
question the request posed — whether the two boundaries need the same
treatment. They do not, because VNP's is already symmetric. That decision
therefore needs no separate resolution during planning.

**Deliberate spec-level choices carried into planning:**

- SC-004 and SC-008 assert *memory* behaviour, not status codes. That is
  awkward to measure and deliberately so: asserting only the response would
  pass while the buffering and unbounded-state defects are fully present.
  Planning must decide how to observe it, not whether to.
- FR-002 forbids restoring allowance via an external call. This is stated as a
  requirement rather than left to design because `refill`/`refill_all` already
  exist, are correct in isolation, and are called by nothing — the shape that
  produced the bug. Re-adding a method someone must remember to call would
  reproduce it.

**One item intentionally left imprecise**: the size limits themselves. FR-006
says the bound must not break legitimate use, and SC-005 requires proving that
with a payload at the documented ceiling — but the number belongs to planning,
where real command and input sizes can be measured, not to a spec guessing at
it.
