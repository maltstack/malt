# Phase 0 Research: Fail-Closed Session Isolation

**Feature**: `specs/007-fail-closed-isolation/`
**Date**: 2026-07-26

Every finding below was checked against running code, not inferred from the
audit or from the architecture documents. Where a first answer turned out to
be wrong, the correction is kept rather than the conclusion silently swapped —
two of them happened here, and both are instructive.

---

## R1. Almost all the platform machinery exists and is called by nothing

**Finding.** `crates/malt-platform/src/isolation/` is 5,954 lines across 14
modules. Callers outside that crate: `job_objects` has 3; `hcs` has 1 **and
it is a comment** (`session_thread.rs:85`); the other **12 have none**.

The session path (`apply_session_isolation`, `session_thread.rs:93`) creates
a Job Object and does nothing else. `probe`, `capability_report`, `tier`,
`sandbox`, `tokens`, `hcs`, `seccomp`, `namespaces`, `cgroups`, `overlayfs`,
`rlimit`, `network` are all unreferenced from the daemon.

**Decision.** This feature is primarily **wiring and policy**, not new
platform work. The plan does not budget for writing backends.

**Correction kept visible.** The first version of the survey
(`docs/findings/2026-07-26-isolation-prior-art-survey.md`) claimed MALT had
no macOS backend and recommended vexil-v2's `seatbelt.rs` as a source. False:
`isolation/sandbox.rs` is 359 lines of `sandbox_init` under
`#[cfg(target_os = "macos")]` — *larger* than vexil-v2's 303. The error came
from labelling a grep's output "(empty above)" with a hardcoded `echo` while
the grep had printed three files. macOS is therefore not a coverage gap; it
is a twelfth instance of the same finding.

---

## R2. The capability probe is real in places and assumed in others

This is the most consequential finding for the design, and it took three
passes to get right. Recording all three, because the first two are the
answers a shallower look produces:

- **First look**: `IsolationCapabilities::probe()` is `Self::default()` →
  "the probe is a stub, it examines nothing."
- **Second look**: `Default` is a *custom* impl delegating to per-facet
  `*_report()` functions → "so it does probe after all."
- **Third look, correct**: the facets differ from each other.

| Facet | What it actually does |
|---|---|
| `windows_hcs` | **Real check** — calls `hcs::hcs_available()`, looks for `computecore.dll` |
| `linux_seccomp` | **Real check** — reads `/proc/self/status` for a `Seccomp:` line |
| `linux_namespaces`, `linux_cgroups`, `linux_overlayfs` | to be confirmed per-facet; same shape |
| `windows_job_objects` | **Assumed** — returns `supported()` unconditionally, with a stated rationale: Job Objects exist on every supported Windows version |
| `windows_restricted_tokens` | **Assumed** — unconditional `supported()` |
| `macos_sandbox`, `macos_rlimit` | **Assumed** — unconditional `supported()` |

**Decision.** `CapabilityReport::supported()` currently conflates two
different claims: *"this host was checked and has it"* and *"this platform
always has it, so we did not check"*. **FR-006 requires those to be
distinguishable**, so the report type needs a third state — verified versus
assumed — rather than a boolean-ish supported/unsupported.

**Rationale.** This is the whole feature in miniature. A tier reported as
applied because a facet was *assumed* present is exactly the false claim
US1 and US2 exist to prevent, one level down. The unconditional Windows
`supported()` values are defensible as *assumptions* and indefensible as
*verifications*, and today nothing distinguishes them.

**Alternative considered.** *Treat assumed-supported as good enough.*
Rejected: it is how the current defect is phrased in the first place. It also
fails SC-002, since a session could report a level whose basis was never
checked.

---

## R3. Where the fail-open lives

**Finding.** `apply_session_isolation(env, session_id, isolation)` returns
`()`. On failure it warns — the message is literally *"failed to create job
object for session isolation; session will run without process containment"* —
and returns. The caller cannot tell, because there is nothing to tell it
with. On non-Windows the entire body is `let _ = session_id;`.

**Decision.** The function must return a result carrying **what was
established**, and session creation must consult it against the requested
policy before returning a session. This is the single change that makes
FR-001 true; everything in US2 is reporting what it returns.

**Note.** The warning text is honest — the code says exactly what it is doing.
The defect is that it says it to a log rather than to the caller.

---

## R4. `Capped` and `Contained` are aliases today

**Finding.** `job_object_limits_for_tier` (`session_thread.rs:50`) maps
`Bare | Restricted → (0, 0)` and `Capped | Contained → (CAPPED_MEMORY_LIMIT_MB,
CAPPED_CPU_RATE_PERCENT)`. Two tiers, one behaviour. HCS is never invoked, so
`Contained` is a Job Object wearing a stronger name — precisely what FR-009
forbids.

**Decision.** Each tier binds to a named mechanism, and a tier that cannot be
provided by its own mechanism is refused rather than silently resolved to a
weaker one. The mechanism-per-tier mapping is a design decision for Phase 1,
constrained by what R1/R2 show is available.

**Open sub-question for the design**: whether `Contained` on Windows means
HCS only, or HCS with AppContainer as a *named, reported* alternative.
vexil-v2 uses AppContainer as the imageless contained backend when HCS is
unavailable — but under this spec that is only acceptable if the session
reports which one it got (FR-005), never as a silent substitution.

---

## R5. Where policy enters, and the breaking change

**Finding.** The isolation tier reaches session creation from three surfaces:
the CLI (`malt new --isolation`), the HTTP create-session route, and VNP
`CreateSession`. `SessionInfo` carries `isolation @3 : IsolationTier` — a
tier, with no room for a policy or a status.

**Decision.** Policy (`required` / `preferred` / `disabled`) is a separate
input from the tier, defaulting to **required** when a tier above the default
is named — per the spec's stated assumption.

**This is a breaking change and the plan must treat it as one.** Callers that
today receive a silently-uncontained session will begin receiving failures.
That is the intended outcome, but it must appear in release notes, and the
error must say how to opt into the old behaviour (`preferred`) rather than
merely refusing.

---

## R6. Reported tier versus verified status

**Finding.** `SessionInfo.isolation` is populated from the requested tier at
creation. Nothing recomputes or verifies it, so every surface that reports it
is repeating the request.

**Decision.** Sessions carry a **status**, not a tier: the level in force, how
it was established (verified or assumed, per R2), and whether it was
downgraded from what was requested. Every reporting surface reads that one
value, which is what makes FR-007 (creating, querying and listing must not
disagree) structural rather than a convention.

**Alternative considered.** *Keep the tier field and add a separate
"verified" flag.* Rejected: two fields that can disagree is how the current
problem arose. One value with an explicit state cannot drift.

---

## R7. Restore is a live hole

**Finding.** A dormant session's `PersistedSession` carries its isolation
tier, and `spawn_with_cwd` restores it. Whether re-establishment happens on
restore has not been traced.

**Decision.** Treat as unverified and check during implementation. FR-014
requires a restored session to re-establish containment or report that it no
longer holds it. Restoring the *record* of a contained session without the
containment is the exact false claim this feature exists to remove, and a
restart is the most plausible way to produce one.

---

## R8. Non-Windows enforcement is absent from the daemon, not from the tree

**Finding.** The daemon's non-Windows path is empty, but `namespaces`,
`cgroups`, `seccomp`, `overlayfs`, `rlimit` and `sandbox` all exist, unused.

**Decision.** US4 is wiring plus verification, not implementation — with the
same caveat as R1: existence is not function, and none of these has ever run
under a session. US1 makes the unsafe case safe on those platforms
immediately (a required request fails where nothing is established), which is
why US4 is P3.

---

## Open questions carried into planning

None blocking. Two must be settled by evidence during implementation rather
than by assumption:

- **Which of the 12 unused modules actually work.** Their tests pass in
  0.01 s for 54 tests, which is consistent with pure-logic tests that never
  touch the OS. `job_objects` — the one module in real use — had two real
  bugs found only when a test finally called Win32 instead of constructing
  structs. Assume nothing about the other twelve.
- **The per-tier mechanism binding** (R4), which depends on the above.
