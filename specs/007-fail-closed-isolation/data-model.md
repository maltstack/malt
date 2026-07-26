# Phase 1 Data Model: Fail-Closed Session Isolation

**Feature**: `specs/007-fail-closed-isolation/`
**Date**: 2026-07-26

---

## IsolationPolicy

How much the caller cares. **Separate from the tier**: the tier says what is
wanted, the policy says what to do when it cannot be had.

| Value | Meaning |
|---|---|
| `Required` | Establish the tier or fail creation. No session is left running. |
| `Preferred` | Establish what is possible; report the downgrade in the creation response. |
| `Disabled` | Attempt nothing. Distinct from requesting the lowest tier — it is "do not try", not "try the weakest". |

**Default**: `Required` when a tier above the baseline is named. This is a
behaviour change (research R5) — callers that today receive a silently
uncontained session will receive an error. The error must name `Preferred` as
the way to opt into the old behaviour, or the change is merely a refusal.

---

## IsolationStatus

What a session actually has. **Replaces the bare tier** on every reporting
surface; it does not sit alongside it (research R6 — two fields that can
disagree is how the present defect arose).

| Field | Type | Notes |
|---|---|---|
| `effective` | `IsolationTier` | The level genuinely in force. Never the requested one unless they match. |
| `requested` | `IsolationTier` | What was asked for. Present so a downgrade is visible without a second call (FR-004). |
| `basis` | `Verified \| Assumed \| None` | How `effective` is known. See below. |
| `mechanism` | optional name | Which mechanism provides it, so `Contained` cannot be a weaker mechanism under a stronger name (FR-009). |
| `detail` | optional string | Why it is not what was requested, or why the basis is not `Verified`. |

**`basis` is the field that makes this feature honest.** Research R2 found
the capability probe already conflates two claims: some facets genuinely
check the host (`hcs_available()` looks for `computecore.dll`;
`linux_seccomp` reads `/proc/self/status`), while others return
`supported()` unconditionally on their platform (`job_objects`,
`restricted_tokens`, `macos_sandbox`, `macos_rlimit`).

- `Verified` — a constraint was observed to be in force.
- `Assumed` — the mechanism reported success, or the platform is documented
  to always provide it, but nothing was observed. Defensible; not the same
  as verified.
- `None` — no containment.

FR-006 requires `Assumed` to be distinguishable from `Verified`, and a
session must never report `Verified` on the strength of an unconditional
`supported()`.

**Invariant**: `effective` never exceeds what `basis` supports. A tier cannot
be reported as in force with `basis: None`.

---

## CapabilityReport (existing type, extended)

`crates/malt-platform/src/isolation/capability_report.rs` already has
`supported()` / `degraded(detail)` / `unsupported(reason_code, detail)` /
`is_usable()` — close to what FR-010 needs, and the reason this feature is
wiring rather than building.

**Required change**: `supported()` must distinguish verified from assumed,
mirroring `IsolationStatus::basis`. Today a caller cannot tell whether
`supported()` came from `hcs_available()` (a real check) or from
`windows_job_objects_report()` (an unconditional return).

Its existing shape is otherwise kept. `CapabilityReasonCode` already covers
`UnsupportedPlatform`, `MissingBinary`, `MissingKernelFeature`.

---

## TierRequirements

What a tier promises, so that "provided by the mechanism it denotes, or
refused" (FR-009) is checkable rather than a matter of naming.

| Field | Notes |
|---|---|
| `tier` | The level. |
| `mechanism` | The named mechanism that provides it on this platform. |
| `constraints` | The specific promises — process containment, memory cap, CPU cap, filesystem or network restriction where a tier claims them. |

**This is what makes SC-004 testable**: for each adjacent pair of tiers, at
least one constraint is enforced at the stronger and not the weaker. Today
`Capped` and `Contained` would produce identical requirement sets, which is
the defect stated as data (research R4).

---

## Relationship to persisted state

`PersistedSession` carries an isolation tier today. It must carry enough to
either **re-establish** on restore or **honestly disclaim** (FR-014).

A restored session that reports its saved tier without re-establishing is the
exact false claim this feature removes, arrived at by a different route — and
a restart is the most plausible way to produce one. Whether restore currently
re-establishes is **unverified** (research R7) and must be checked, not
assumed.

---

## State transitions

```
create(tier, policy)
   ├─ establish attempt
   │     ├─ established        → session, status{effective=tier, basis=Verified|Assumed}
   │     ├─ partial/downgraded → policy Required  → FAIL, no session
   │     │                       policy Preferred → session, status{effective<requested, detail}
   │     └─ unavailable        → policy Required  → FAIL, no session
   │                             policy Preferred → session, status{effective=None, detail}
   └─ policy Disabled          → session, status{effective=None, basis=None}

running → containment lost/detected → status updated, surfaced (FR-017)
restore → re-establish → status reflects what is now held, not what was saved
destroy → containment and everything held inside released (FR-013)
```

**Partial establishment is a failure under `Required`**, not a downgrade to
be reported. Some-constraints-applied is not the tier, and treating it as one
would reintroduce the defect at finer granularity.
