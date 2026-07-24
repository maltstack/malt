# ADR-0001: Revert the malt-stack Dependency; Vendor, Never Depend

Date: 2026-07-24
Status: Accepted

## Context

MALT sat dormant for ~3.5 months (last commit `1296955`, 2026-04-10). The
working tree was left mid-refactor and uncommitted, not at a clean stopping
point. Investigation on resuming found that `malt-platform` (a foundational
L0 crate — PTY, process, signals, sockets, fs, isolation) and `malt-elevate`
had been rewritten, uncommitted, to depend on `malt-stack`'s `carboy-core`,
`carboy-types`, `carboy-isolation`, and `keg` crates via relative path
dependencies (`../../../malt-stack/...`), dated 2026-04-11 — one day after
the last real commit.

`malt-stack` is a sibling "Orix substrate" project (Carboy/Keg/Kettle/Hops/
Cask/Tap) that was itself abandoned shortly after starting: zero commits,
4 of 6 planned rings still placeholders by its own README. Depending on it
would make MALT's own "done" bar hostage to an even-less-finished sibling —
the same failure shape that produced the vexil-v2 → malt → malt-stack
rewrite chain in the first place. The dependency was introduced and then
work stopped; it's plausibly connected, though not provably causal.

Comparing the two isolation implementations directly (not by reputation)
found a mixed picture, not a clean "carboy is better":
- Shared primitives (e.g. `job_objects.rs`) are near-identical — carboy's
  copy is literally derived from malt's/vexil-v2's, per malt-stack's own
  extraction plan ("neutralize comments"), and malt's original kept better
  SAFETY-comment discipline.
- carboy-isolation adds real capability malt lacked: HCS (Windows Host
  Compute System — genuine Win32 API bindings, a real `computecore.dll`
  runtime check, feature-gated with a fake-mode test backend).
- carboy-isolation also adds capability that *looks* real but isn't:
  `appcontainer_available()` always returns `true` (not a real check), and
  its restricted-token function is documented in its own code comment as
  "currently a compatibility stub (process token fallback)" — it returns the
  unrestricted process token, providing zero actual isolation.
- carboy's CRIU backend is a 5-line placeholder: one function, unconditional
  `Err(Unsupported)`.

## Decision

1. **`malt-platform` and `malt-elevate` are reverted to their last committed,
   self-contained state.** No crate in MALT has a path or git dependency on
   `malt-stack`. The full uncommitted carboy/keg-dependent state is preserved
   on branch `checkpoint/pre-carboy-revert-2026-07-24` for reference — not
   deleted, not depended on.
2. **Policy going forward: vendor, never depend.** If something in
   `malt-stack` (or any sibling project) is genuinely useful, port the code
   in as owned source under `malt-platform`, rewritten to match MALT's own
   invariants (SAFETY comments, doc comments, MALT's error types). Never add
   a live dependency on an unstable, unversioned sibling project.
3. **Ported on this basis:** `hcs.rs` (real HCS bindings + `hcs_available()`
   runtime probe + fake-mode test backend), and the `CapabilityReport`/
   `CapabilityStatus`/`CapabilityReasonCode` model (as owned types, adapted
   into the existing tested `IsolationCapabilities`/`tier_available()`
   machinery rather than replacing it).
4. **Not ported:** AppContainer (stub — would add a false sense of security)
   and CRIU (placeholder — nothing there to port). If a real AppContainer or
   CRIU implementation is written later, it should be original work assessed
   on its own merits, not adopted because carboy already has scaffolding for
   it.

## Consequences

- MALT's isolation subsystem gains a real second Windows containment backend
  (HCS) and richer capability diagnostics, without importing malt-stack's
  unfinished ring model or its documentation-discipline regressions.
- No crate in the workspace can silently reintroduce the malt-stack coupling
  without a `Cargo.toml` diff being obviously visible in review.
- `malt-daemon` still does not call any isolation code — sessions remain
  unsandboxed today. Wiring that in is separate follow-up work, not covered
  by this decision.
- If malt-stack's carboy ever matures into something worth depending on
  directly (versioned releases, its own CI, stable API), that would be a new
  ADR superseding this one — not a default reached by drift.
