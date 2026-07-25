# Specification Quality Checklist: Authenticated Raw Input with Input Authority

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-07-25 | **Re-validated**: 2026-07-25 after the A-01 revision
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

All items pass. Re-validated after the revision described below.

**The revision, and why it was not a small edit.** The first draft assumed
clients reaching a session were already authenticated, and scoped the
feature to arbitrating between them. That assumption was false for the
transport this feature depends on most — the connection path the
interactive client uses performs no identity check of any kind and
discloses the session inventory during its opening exchange. Verified
directly before revising, and independently reported as finding A-01 in
`docs/findings/2026-07-25-architecture-spec-codebase-audit.md`.

The consequence was not cosmetic. FR-006 requires input to be attributable
to the client that sent it; attribution to a self-asserted identity is not
attribution, so the requirement was unsatisfiable as written. "Exactly one
client holds input authority" would have been a coordination convention
rather than a guarantee, and the spec would have described a security
property it did not deliver. Worse, shipping interactive input first would
have made password prompts injectable by any local process — actively worse
than today, where such prompts simply cannot be answered at all.

Authenticated identity is therefore User Story 1, ahead of the capability
the feature is named for. That ordering matches the audit's own recommended
closure order, which pairs transport authentication with client-scoped
authority enforcement as a single first item.

**On combining what the backlog lists separately.** This spec now covers
three backlog concerns: raw input (ADR-0003 priority 5), input authority
(priority 6), and transport authentication (audit A-01, with the related
pre-identification resource exhaustion A-08 folded in as FR-003/FR-004 —
same connection path, and "an unidentified caller must not harm the daemon"
is the same requirement expressed twice). They are one feature because each
is a constraint on the others' design, not an enhancement: identity has to
be established once, for all of them, and retrofitting it would mean
reworking the same delivery points three times. The four stories remain
independently shippable and independently testable.

**On the confidentiality requirement (FR-010).** This exists because
features 003 and 004 shipped first. Command execution history and the
lifecycle event stream both record command text and are both readable at
Read scope. Routing prompt answers through the ordinary command path —
today's behavior — would publish passwords into two durable, readable
surfaces.

**On FR-009 (byte-for-byte).** Strengthened during revision from the
audit's A-07 detail: the current path decodes lossily *and* trims
whitespace *and* executes the result. Each is an independent way to corrupt
a password, so the requirement names all three explicitly rather than
saying "unmodified".

**On the absence of clarification markers.** Seven points were genuinely
ambiguous and are resolved as documented assumptions, each with a stated
reason a reviewer can overturn: the transport is unauthenticated and
closing that is in scope; authority is claimed rather than granted (a
consent protocol lets a departed holder strand the session); first attach
takes authority; the daemon never echoes input; terminal control concerns
are out of scope; retained type-ahead is not persisted; and identity should
extend the existing permission model rather than create a parallel one.
