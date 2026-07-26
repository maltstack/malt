# Implementation Plan: Fail-Closed Session Isolation

**Branch**: `007-fail-closed-isolation` | **Date**: 2026-07-26 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/007-fail-closed-isolation/spec.md`

## Summary

Requesting containment currently produces a session whether or not the
containment was established. `apply_session_isolation` returns `()`, warns
*"session will run without process containment"*, and continues; the session
then reports the tier that was **requested**, so every surface repeats the
question rather than answering it.

The fix is mostly wiring, not platform work. `malt-platform` already contains
14 isolation modules of which 12 have no caller anywhere outside their own
crate. This feature makes session creation consult what was actually
established, gives callers a policy (`required` / `preferred` / `disabled`)
to say how much they care, and replaces the reported tier with a status that
carries what is in force and how it was established.

## Technical Context

**Language/Version**: Rust, edition 2021

**Primary Dependencies**: existing workspace only. No new external
dependencies expected — the OS bindings are already present in
`malt-platform` (`windows-sys`, and the Linux/macOS paths behind `cfg`).

**Storage**: `PersistedSession` already carries an isolation tier; it will
need to carry enough to re-establish or honestly disclaim on restore (FR-014).

**Testing**: `cargo test --workspace`. **Smoosh is not a gate here** —
`mash`'s executor is not modified. The gate that matters instead is that
isolation tests must exercise real OS calls; see the Constitution Check.

**Target Platform**: Windows-native primary. Linux/macOS enforcement is US4;
until then those platforms must *fail* a required request, not pretend.

**Project Type**: Multi-crate Rust workspace.

**Performance Goals**: none specific. Establishing containment happens once
per session; a probe result may be cached per daemon lifetime.

**Constraints**: zero sessions created uncontained under a required policy
(SC-001); every reporting surface agrees (SC-002); teardown verified by
inspection rather than by absence of error (SC-006).

**Scale/Scope**: per-session; a handful of tiers; three request surfaces
(CLI, HTTP, VNP).

## Constitution Check

*GATE: checked before Phase 0 research, re-checked after Phase 1 design.*

| Principle | Assessment |
|---|---|
| **I. VT Codes Confined** | N/A — no VT handling. |
| **II. OS Calls Confined** | **Central to this feature.** Every OS call stays in `malt-platform::isolation`; the daemon consults results and applies policy. Any new verification step (checking a constraint is really in force) is a `malt-platform` function, not a `cfg` block in the daemon. |
| **III. Dependency-Free Foundations** | PASS. The status/policy types shared over the wire go in `malt-protocol`, external deps only. |
| **IV. Safety Is Explicit** | **The governing principle here.** Its own text says two `job_objects.rs` bugs were found only because tests called real Win32 instead of constructing structs — and 12 modules this feature depends on have never been exercised by a running session, with 54 tests passing in 0.01 s. Every tier this feature claims to enforce needs a test that observes the constraint, not one that checks a function returned `Ok`. |
| **V. VNP Only** | PASS. Status travels in existing session messages; no side channel. |
| **VI. Smoosh Gate** | N/A — `mash` is untouched. Stated explicitly so its absence is not mistaken for an oversight. |
| **VII. Layering** | PASS. `malt-platform` (L0) gains verification and richer reports; `malt-daemon` (L2) gains policy. No upward dependency. |
| **VIII. Vendor, Never Depend on Unstable Siblings** | Watch item. vexil-v2 has a Windows AppContainer backend MALT lacks (US4/R4). If used, it is vendored as owned source rewritten to these invariants — never a path or git dependency (ADR-0001). |
| **IX. No Silent Scope-Jumps** | Watch item, two temptations: (a) fixing whichever of the 12 unused modules turn out to be broken, beyond what a tier this feature claims actually needs; (b) adding new tiers or new constraint kinds (network, filesystem) because the plumbing is suddenly there. Both get written down. |
| **X. Commit at Real Checkpoints** | Each story is a checkpoint with gates and a merge. |

**Result: PASS.** No violations to justify; Complexity Tracking omitted.

**Post-Phase-1 re-check: PASS.** The design adds a policy input, a status
type replacing a bare tier, a verified/assumed distinction on the existing
capability report, and a result type on one daemon function. No new crate, no
new layer.

## Project Structure

### Documentation (this feature)

```text
specs/007-fail-closed-isolation/
├── spec.md
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── isolation-status.md
├── checklists/requirements.md
└── tasks.md             # Created by /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── malt-platform/src/isolation/
│   ├── capability_report.rs   # add verified-vs-assumed (research R2)
│   ├── probe.rs               # facets state which they are
│   ├── tier.rs                # per-tier mechanism binding + requirements
│   ├── job_objects.rs         # already wired; verification of what it set
│   ├── hcs.rs                 # unused today; Contained's real mechanism
│   ├── namespaces.rs, cgroups.rs, seccomp.rs,
│   │   rlimit.rs, overlayfs.rs, sandbox.rs, tokens.rs
│   │                          # unused today; US4 and per-tier constraints
│   └── mod.rs
├── malt-protocol/             # IsolationStatus + policy on session messages
├── malt-daemon/src/executor/
│   ├── session_thread.rs      # apply_session_isolation returns a result;
│   │                          #   per-tier mechanism, not one Job Object
│   └── coordinator.rs         # policy decision at creation; status on
│                              #   create/get/list; re-establish on restore
├── malt-gateway/src/          # policy in, status out
└── malt-bin/src/              # `malt new --isolation` gains a policy

schemas/
├── common.vexil               # IsolationStatus, IsolationPolicy
└── session.vexil              # creation carries policy; info carries status
```

**Structure Decision**: No new crate. The OS work is already in
`malt-platform`; the daemon gains the decision and the reporting.

## Implementation Order and Risk

Ordered so the safety guarantee lands before the capability expansion, and so
the riskiest assumption is tested before anything is built on it.

1. **Establish which unused modules actually work** — a throwaway
   verification pass over the modules each tier will depend on, calling real
   OS APIs. Not a deliverable; an input. If a module is broken, that changes
   what the tiers can promise, and it is far cheaper to learn now than after
   the policy layer is built on top of it.
2. **Verified-vs-assumed on the capability report** (R2). Small, and
   everything honest downstream depends on it.
3. **US1 — fail-closed creation.** `apply_session_isolation` returns a
   result; policy decides. This alone removes the trust defect on every
   platform, including the ones with no enforcement at all.
4. **US2 — honest reporting.** Status replaces tier on every surface.
5. **US3 — distinct tiers.** Bind each tier to its own mechanism; `Contained`
   stops being a Job Object under a stronger name.
6. **US4 — other platforms.** Wire the Linux/macOS modules; consider
   AppContainer.

**The known failure mode for this feature** is specific and different from
the last three: not an untested delivery path, but **a test that asserts a
containment function returned `Ok` and concludes the containment exists**.
That is how the present defect survives — the Job Object call *does* succeed,
and nothing checks that anything is actually constrained by it. Every tier
claim needs a test that observes the constraint from outside: a process that
should be inside the container appearing in it, a limit that should bind
actually binding.
