# ADR-0005: Build the Container Substrate as a Synthesis, and Re-sequence Isolation Ahead of ADR-0003's Order

Date: 2026-07-26
Status: Accepted

Supersedes, on one point only: ADR-0003's placement of "fail-closed
requested isolation" at priority 7. Everything else in ADR-0003 — the
retirement of the phased roadmap, the correctness-first lens, the paused
list — stands unchanged.

## Context

Feature 007 (fail-closed session isolation) is substantially complete: the
policy layer, the single `IsolationStatus` every surface reads, the
verified-versus-assumed basis, and the Windows Job Object tiers all work and
are covered by tests that call real OS APIs. It is stuck on one thing —
`Contained` — and today established why, with evidence rather than inference.

### What today established

1. **The HCS crash was ours, and local.** `HcsStartComputeSystem` and
   `HcsTerminateComputeSystem` were being passed a null `HCS_OPERATION`
   handle, which computecore dereferences unconditionally. Fixed
   (`3c60072`); see `docs/briefs/006-hcs-backend-access-violation.md`. All
   three hypotheses recorded in that brief were wrong, and four `eprintln!`
   statements settled in one run what ordering the candidates had not.

2. **The real blocker is a privilege boundary, and it is not negotiable.**
   With the crash gone: `HCS_E_ACCESS_DENIED` — the daemon is neither an
   administrator nor a member of Hyper-V Administrators. The host is capable.
   This is the HCS API stating a requirement, not a configuration accident.

3. **`vexil-v2` reached the same conclusion independently.** It runs HCS from
   a **LocalSystem Windows service** (`windows_container_service.rs`, 1,222
   lines, installed via `ensure_installed_for_contained`) for exactly this
   reason. Two projects arriving separately at a privileged helper is strong
   evidence it is the shape of the problem.

4. **`malt-elevate` is already that helper, and it is hollow.** Ten typed
   operations — `ManageHcsContainer`, `MountOverlay`, `CreateNamespace`,
   `SetCgroup`, `ApplySeccomp`, `ApplySeatbelt`, `CreateRestrictedToken`,
   `SetupNetns`, `BindPort`, `CreateSymlink` — with a nonce-authenticated
   protocol. **Nine dispatch to `stub_success`** (`dispatch.rs:96`), which
   returns `"stub: operation not yet implemented, returning success"`. Zero
   callers outside the crate. A test named `stub_operations_return_success`
   asserts this behaviour.

   This is the eighth instance of the survey pattern in AGENTS.md and the
   worst variant recorded: it does not merely fail to work, it
   **affirmatively reports success for privileged operations that never
   happened** — the fail-open 007 exists to remove, one layer down, with a
   test holding it in place.

5. **`vexil-v2` has the identical null-handle bug** (`hcs.rs:691`), despite
   its HCS path being reachable from `vexil-bin` and its HTTP routes. Its
   create-and-start cannot ever have completed.

Point 5 corrects a conclusion drawn earlier the same day. Reachability was
used as evidence that vexil-v2's isolation stack is more mature than MALT's.
It is more *wired*. That is not the same thing, and this ADR is written so
the distinction is on the record before any code moves.

### What each project actually has

| Concern | MALT | vexil-v2 |
|---|---|---|
| Privileged protocol | 10 typed ops, auth, dispatch — 9 stubbed | LocalSystem service, named pipe, install/status/uninstall |
| HCS core | 843 lines, non-faulting, decoded errors | 1,378 lines + layer preparation; same null-handle bug |
| Image acquisition | **nothing** | `oci.rs` 778, `image_store.rs` 537 |
| Policy & reporting | 007: policy, status, basis, session-path-honest capabilities | per-facet `ContainedCapability` reasons |
| Tier → mechanism | `TierRequirements` + `TierConstraint` | `ContainedBackend` enum |
| macOS | `sandbox.rs` 359 | `seatbelt.rs` 303 |

MALT has a *slot* for nearly everything vexil-v2 has working. vexil-v2 has a
working implementation for nearly everything MALT stubbed. The complement is
close enough to be worth building deliberately rather than porting
opportunistically.

## Decision

**Build a container substrate as an explicit synthesis of both projects, and
sequence it ahead of the remaining correctness items in ADR-0003's list.**

### Five rules the work is held to

1. **The privilege boundary is the spine.** Everything `Contained` flows
   through `malt-elevate`. This is already MALT's architecture; it is empty.

2. **The stubs are removed before anything is added.** `stub_success`
   becomes an explicit `Err`, and `stub_operations_return_success` is
   inverted. Non-negotiable and first: any caller added before this inherits
   a fail-open, which would build the synthesis on the defect it exists to
   remove.

3. **Nothing vendored is trusted.** Per ADR-0001 and Constitution VIII,
   anything taken from vexil-v2 is **owned source, never a dependency** — and
   beyond that, every ported module earns a test that calls the real OS API
   *before* it is given a caller. vexil-v2's HCS was wired to a binary and to
   HTTP routes and still could not create a container; that is the standard
   of evidence being rejected here.

4. **Capability answers stay session-path-honest.** MALT's
   `session_tier_capabilities()` — what MALT's own spawn path can establish,
   as distinct from what the OS offers — is the better model and survived
   contact with reality today, while the host-primitive `supports_contained()`
   did not. vexil-v2's per-facet reason detail is adopted *into* that shape,
   not instead of it.

5. **Layering holds (Invariant 2, Invariant 9).** OCI is network and archive
   handling, not OS abstraction:
   - **`malt-image` (L1, new)** — registry protocol, manifests,
     content-addressed blob store. No OS calls.
   - **`malt-platform::isolation::layers`** — layer *materialization*
     (`HcsImportLayer`, overlayfs mount). OS calls live here and only here.

### Two decisions the design documents force

A survey of `docs/design/architecture.md` and vexil-v2's
`docs/design-isolation.md` — recorded in
`docs/findings/2026-07-26-isolation-design-doc-survey.md` — settled two
questions this ADR would otherwise have left open.

**A. Pick the spine mechanism before adding backends.** The architecture
document (§"Shell and Isolation", lines 737-749) specifies an
`IsolationContext` token flowing daemon → MASH → platform, where "the
platform layer reads the token and applies the appropriate sandbox". The
token is created (`session_thread.rs:116`) and stored (`mash/src/env.rs:314`)
and **never read** — `isolation_context()` has zero callers. What actually
applies isolation is `job_object: Option<Arc<JobObject>>`, six lines away in
the same struct.

`Env` therefore carries two parallel mechanisms: the documented one, inert,
and the undocumented one, working. This is instance nine of the survey
pattern, in a *half-wired* variant that passes every reachability check
except having a consumer.

`IsolationContext` is the better design — opaque token, MASH free of
isolation logic, extends to non-Windows backends and to a privileged helper.
`Arc<JobObject>` is the working code but is Windows-specific by construction
and cannot carry a container identity. **The substrate resolves this
explicitly, in step 2, before any backend is added to either.** Deleting
`IsolationContext` as dead code would be the wrong fix: it is the only
abstraction already shaped for what comes next.

**B. Images are not deferrable.** vexil-v2's design defines tier 3 as an
"image-backed environment" (lines 22, 47). Images are constitutive of
`Contained`, not an enhancement to it. A `Contained` tier without an image
is a different tier wearing the name — the thing FR-009 exists to forbid. So
step 4 cannot be dropped to make `Contained` land sooner; doing so would
reproduce the aliasing defect fixed today, one level up.

That document also states 007's thesis outright — "expose what is available
on the current host rather than pretending every tier has the same fidelity
everywhere" (lines 24-26) — and vexil-v2 did not implement it either. **This
is the strongest argument for the approach in this ADR: the designs in both
projects are sound and both implementations drifted from them.** Take the
designs; re-verify every line of code.

It additionally supplies one requirement 007 lacks — "tier escalation should
not happen implicitly at runtime" (line 117). FR-017 covers containment
*lost*; nothing covers containment *gained*. Candidate requirement for the
substrate spec.

### Sequence

| # | Work | Rationale |
|---|---|---|
| 1 | `malt-elevate` stubs → explicit errors | Prerequisite; standalone value — removes a live fail-open |
| 2 | Elevated transport and installation (LocalSystem service on Windows) | The boundary itself, no container logic |
| 3 | `ManageHcsContainer` implemented for real | **First point something new is provably true**: `HCS_E_ACCESS_DENIED` should disappear |
| 4 | `malt-image` + layer materialization | The other half `Contained` requires |
| 5 | Contained session spawn | Payoff |

Step 3 is the MVP boundary: it ends with an observable claim that is false
today.

## Consequences

### On feature 007 — the contract is unchanged; the task list shrinks

**007's requirements are mechanism-agnostic and none of them move.** FR-001
(fail closed), FR-004 (visible downgrade), FR-005 (report the mechanism),
FR-006 (verified versus assumed), FR-007 (surfaces agree), FR-009 (a tier is
never satisfied by a weaker mechanism), FR-010 (capabilities discoverable) are
statements about *honesty*, not about which backends exist. The substrate adds
mechanisms; it does not change what honesty means. That 007 needs no
redefinition is a result of it having been specced correctly — as "never claim
what you do not have", not as "implement HCS".

What changes is which tasks belong to it:

- **T028 (HCS contained spawn) leaves 007.** It requires the privilege
  boundary and a base image layer. Both are substrate work.
- **T032 (Linux) and T033 (macOS) leave 007.** Real enforcement on those
  platforms also runs through the elevate boundary — Linux cgroup and
  namespace work needs privilege it does not currently request. Wiring them
  directly now would build a path the substrate immediately reroutes.
- **T027, T037, T042, T043 stay**, scoped to the tiers that exist here:
  `Bare`, `Restricted`, `Capped`.

**007 therefore closes with `Contained` unavailable — and that is 007
succeeding, not 007 falling short.** The entire feature is "refuse rather than
lie". A host that cannot provide `Contained`, says so with a reason, refuses a
`required` request and visibly downgrades a `preferred` one is the success
case demonstrated end to end. Shipping it in that state is a stronger
demonstration of the machinery than shipping it with one more backend.

### Other consequences

- Isolation moves ahead of briefs 001–004 in priority. Those remain open and
  are individually smaller than this work.
- `IsolationMechanism` will grow as backends land; confirm it is
  `#[non_exhaustive]` so that growth is not a breaking change.
- Some capability bases may improve from `Assumed` to `Verified` once the
  boundary can genuinely probe. That is an improvement the substrate enables,
  not a change 007 requires.
- This is realistically 4–6 features. Per Principle X it is committed in
  checkpoints, and per Principle IX any mid-course rethink is written down
  rather than absorbed.

## Alternatives considered

**Port vexil-v2's isolation stack wholesale.** Rejected. Its HCS path carries
a process-faulting bug in wired, reachable code, so its state is unknown
rather than good. Wholesale porting would import that uncertainty at the exact
layer where the failure mode is a crash in a privileged helper.

**Finish 007 by making `Contained` work first.** Rejected. It requires the
privilege boundary and image layers regardless, so this is the substrate work
under a smaller name — precisely the silent scope-jump Principle IX prohibits.

**Leave isolation at priority 7 and continue with briefs 001–004.** Rejected,
but it was close. What decided it: 007 is 85% done and stuck exactly here, the
blocker is now a known architecture rather than an open question, and
`malt-elevate`'s nine success-returning stubs are a live fail-open that should
not sit indefinitely behind a `#[cfg]`-free public binary.

**Treat `AppContainer` as the Windows `Contained` backend instead.** Still
rejected, on 007's research R9 grounds: MALT has no owned AppContainer backend
and no spawn path that could launch under one. It remains a separately scoped
proposal and never a silent fallback.
