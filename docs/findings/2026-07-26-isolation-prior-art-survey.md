# Isolation prior art: what already exists, in MALT and in vexil-v2

**Date:** 2026-07-26
**Context:** survey run before planning `specs/007-fail-closed-isolation/`,
prompted by a recollection that `~/projects/vexil-v2/` had a substantial
containment implementation worth consulting.

AGENTS.md instructs checking vexil-v2 for a working equivalent before
building a feature, so this is that check. It turned up something more
important than the answer to the original question.

## The recollection was right

`~/projects/vexil-v2/` does contain a substantial isolation subsystem:

| File | Lines |
|---|---|
| `vexil-platform/src/isolation.rs` | 3,131 |
| `vexil-platform/src/windows_container_service.rs` | 1,222 |
| `vexil-platform/src/hcs.rs` | 1,378 |
| `vexil-platform/src/seccomp.rs` | 535 |
| `vexil-platform/src/network.rs` | 516 |
| `vexil-platform/src/appcontainer.rs` | 395 |
| `vexil-platform/src/seatbelt.rs` | 303 |

It has a `ContainedBackend` enum spanning `LinuxNativeOverlay`,
`LinuxFuseOverlay`, `WindowsHcs`, `WindowsAppContainer`, and `MacOsSeatbelt`;
a `ContainedCapability` runtime report; and degradation reasons recorded on
the handle (`windows_image_compatibility_reason`, "if the runtime was created
from a degraded path").

## The finding that matters more

**MALT already has an isolation module of comparable size that almost nothing
calls.** `crates/malt-platform/src/isolation/` is **5,954 lines across 14
files**: `capability_report.rs`, `cgroups.rs`, `hcs.rs` (843 lines, including
`create_process`), `job_objects.rs`, `namespaces.rs`, `network.rs`,
`overlayfs.rs`, `probe.rs`, `rlimit.rs`, `sandbox.rs`, `seccomp.rs`,
`tier.rs`, `tokens.rs`.

Callers outside `malt-platform`, by module:

| Module | External callers |
|---|---|
| `job_objects` | 3 |
| `hcs` | **1, and it is a comment** (`session_thread.rs:85`) |
| the other 12 | **0** |

These are not stubs. `probe.rs` exposes `probe()`, `supports_restricted()`,
`supports_capped()` and has 13 tests. `sandbox.rs` has 17.
`capability_report.rs` has `supported()` / `degraded(detail)` /
`unsupported(reason_code, detail)` / `is_usable()` — which is almost exactly
the vocabulary spec 007's FR-002, FR-004, FR-006 and FR-010 ask for.

So the daemon's `apply_session_isolation` creates a Job Object and nothing
else, while a per-tier capability probe, a degradation-reason type, an HCS
process launcher, seccomp, namespaces, cgroups, overlayfs, rlimits and
restricted tokens all sit unused one crate away.

**This is the fifth instance of the same pattern in this feature family**,
after Gateway auth, `AuthorityTracker`, `InputClaim`/`InputAuthorityChanged`,
and `OutputChunk`. It is now the single most reliable prior about this
codebase: *before building a mechanism, check whether it exists and is merely
uncalled.*

## What this means for spec 007

**US1–US3 are mostly wiring and policy, not platform work.** Fail-closed
creation, honest reporting, and distinct tiers can plausibly be built on what
`malt-platform` already has. The plan should start by establishing which of
those 12 modules actually work, not by writing new backends.

**Important caveat, unverified.** Existence and test count are not evidence
of function. Two real bugs in `job_objects.rs` were found in 2026-07 only
because a test called the real Win32 API instead of constructing structs
directly, and that lesson is in AGENTS.md for this reason. The 12 unused
modules have never been exercised by a running session. Some may be correct;
some may have `job_objects`-class defects that nothing has surfaced. **A
first planning task should be to determine which**, because "the module
exists" and "the module contains 13 tests" are exactly the kind of evidence
that looked sufficient in the four previous instances.

## What vexil-v2 genuinely adds

Two backends MALT has no equivalent of at all:

- **macOS Seatbelt** (`seatbelt.rs`, 303 lines) — MALT has
  `probe::has_macos_isolation()` but no backend behind it.
- **Windows AppContainer** (`appcontainer.rs`, 395 lines) — MALT has
  `tokens.rs` for restricted tokens, which is related but not the same
  mechanism. AppContainer is also what vexil-v2 uses as the *imageless*
  contained backend when HCS is unavailable, which is directly relevant to
  spec 007's US3 (a stronger tier must differ from a weaker one) and to
  FR-009 (a level must be provided by the mechanism it denotes, or refused).

Both map to US4, the P3 story. Neither is needed for US1–US3.

## Recommendation

1. **Do not port anything yet.** For US1–US3 the material is already in
   `malt-platform`; porting would add a second unused implementation next to
   the first.
2. **Consult vexil-v2 as reference for US4**, per AGENTS.md — for the logic
   and the OS-call sequences, not verbatim. If anything is taken, it is
   vendored as owned source rewritten to these invariants; never a path or
   git dependency (Constitution VIII / Invariant 11, ADR-0001).
3. **`ContainedCapability`'s shape is worth studying regardless** — a
   capability report carrying a *reason* per degraded facet is close to what
   FR-004 and FR-010 need, and MALT's `CapabilityReport` may or may not
   already carry enough detail.

## What this survey did not establish

- Whether any of MALT's 12 unused isolation modules actually work.
- Whether vexil-v2's backends work; nothing here was built or run. The line
  counts and type shapes come from reading the source.
- Whether `CapabilityReport` in MALT is sufficient for FR-010, or whether it
  needs the per-facet reasons vexil-v2's version carries.
