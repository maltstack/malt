# ADR-0006: macOS Is Unsupported

Date: 2026-07-28
Status: Accepted

## Context

MALT has carried macOS in CI since the workflow was written on 2026-07-26 —
a `Build + test` job and an `Isolation capabilities` job, both advisory. The
tree also carries macOS-specific code: `malt-platform`'s `isolation/sandbox.rs`
(359 lines of `sandbox_init`), macOS facets in the capability probe, and
`#[cfg(target_os = "macos")]` arms throughout the platform layer.

**Nobody working on this project has a macOS machine.** That was tolerable
while the job was thought of as free information. Two days of cross-platform
work showed it is not free:

- The macOS job could not compile at all until 2026-07-27, so nothing it
  reported meant anything before then.
- Once it compiled, it produced two genuine findings —
  `spawn_and_check_exit` failing via `SIGPIPE`, and the compat pane
  delivering no output — and **neither could be debugged.** The first was
  fixed only because it shared a root cause with a Linux failure that *could*
  be reproduced locally (`docs/briefs/007`). The second remains undiagnosed
  for exactly the reason this ADR exists: there is no machine to reproduce it
  on, and CI gives one bit per three minutes with no ability to instrument.
- The capability probe reports macOS facets as `supported` on the strength of
  "macOS always has sandbox_init", which is the *assumed* rather than
  *verified* basis spec 007 exists to keep distinguishable. No one has ever
  watched a macOS session be contained.

The alternative to deciding is drift: a permanently red advisory job, which
AGENTS.md already warns about — *"advisory means nothing forces anyone to
look"* — training everyone to ignore the one signal that might matter.

## Decision

**macOS is an unsupported platform.** Concretely:

1. **Removed from CI**, from both the `cross-platform` and
   `isolation-capabilities` matrices. Supported platforms are **Windows**
   (primary, blocking gates) and **Linux** (advisory, locally reproducible via
   `scripts/wsl-mirror.sh`).
2. **No support claim is made.** Not "works on macOS", not "should work",
   not "untested but probably fine".
3. **macOS code stays in the tree.** It is not deleted. `sandbox.rs`, the
   probe facets and the `#[cfg]` arms remain, compile-gated as they are. They
   are a starting point for whoever revives this, not a liability — and
   deleting them would throw away work to make a point.
4. **Anything that *claims* a macOS capability must say it is unverified.**
   The probe's `Assumed` basis already carries this meaning; it must not be
   promoted to `Verified` for macOS on any basis short of a run.
5. **A test that fails only on macOS may be gated to say so**, naming this
   ADR. That is not the same as hiding a bug on a supported platform, which
   `docs/briefs/007` rightly forbids.

## Consequences

- CI gets faster and its red means something again.
- The open macOS finding — compat-pane output not reaching the grid, with the
  fd inversion already fixed and confirmed by `spawn_and_check_exit` going
  green — is **not fixed and not forgotten**. It is recorded in
  `docs/briefs/007` as a macOS-only gap that this ADR defers rather than
  closes.
- Spec 007's T033 ("wire the macOS sandbox path") is **out of scope** while
  this ADR stands. It should not be picked up as available work.
- `docs/ROADMAP.md`'s macOS entries are deferred on the same basis.

## Reversing this

The bar is a machine or environment where a person can run the suite, attach
a debugger, and instrument a failing path — not a green CI badge. When that
exists:

1. Restore macOS to both CI matrices.
2. Run the full suite and record the result in `docs/findings/`, including
   what it does *not* establish.
3. Re-examine every `Assumed` macOS capability against a real host before
   letting any of them report `Verified`.

Until then, the honest statement is the one in this ADR's title.

## Alternatives considered

**Keep macOS advisory and live with the red.** Rejected: a permanently failing
job is indistinguishable from a broken one, and it degrades the signal from
the Linux job next to it, which *is* actionable.

**Keep the job but skip the failing tests.** Rejected as the worst of both —
it would report green for a platform nobody can vouch for, which is precisely
the false claim specs 007 and 008 were written to remove from this codebase.

**Delete the macOS code.** Rejected: it costs nothing to keep, it is already
`#[cfg]`-gated out of every other build, and it is the obvious foundation if
support is ever revived.
