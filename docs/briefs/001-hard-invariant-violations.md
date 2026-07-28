# Brief 001 — Hard invariant violations in production code

**Severity**: High · **Verified**: 2026-07-26 · **Source**: audit A-13

## What is wrong

Two constitutional invariants are violated, and **AGENTS.md marks both
"✅ Clean"**. The false claim is the worst part: someone reading it has no
reason to look.

### Invariant IV — no `unwrap()`/`expect()` outside `#[cfg(test)]`

**39 occurrences in non-test code.** A naive grep reports ~185; most are
inside inline test modules, which are permitted. The real count, by crate:

| Crate | Non-test | Notable |
|---|---|---|
| `malt-platform` | 16 | `isolation/hcs.rs:304` `.expect("compute registry lock poisoned")`; `vfs/fd.rs:471` `.lock().unwrap()` |
| `mash` | 15 | `executor.rs:1305` `tools_registry.get(name).unwrap()`; `expander.rs:384` |
| `malt-tools` | 5 | `chmod.rs:56`, `cp.rs:43` — argument handling |
| `malt-daemon` | 1 | `store/debounce.rs:42` `.expect("failed to spawn debounce flush thread")` |
| `malt-gateway` | 1 | `auth.rs:76` `.expect("OS entropy unavailable…")` |
| `malt-layout` | 1 | `strategies/grid.rs:77` — guarded, `"checked non-empty"` |

They are not equivalent and should not be fixed uniformly:

- **Mutex-poison panics** (`vfs/fd.rs` and 15 siblings) are the dangerous
  class. One panic anywhere poisons the lock and every later call panics —
  a single fault becomes total. This repo already fixed exactly this shape
  once: 9 of 19 `CWD_LOCK` call sites used poison-fragile `.lock().unwrap()`
  and one panic cascaded into unrelated test failures.
  `.unwrap_or_else(|e| e.into_inner())` is the established remedy.
- **Deliberate, documented panics** (`auth.rs:76` — no OS entropy means no
  credential can be minted) may be correct. If so they need a `// SAFETY:`-
  style justification, or the invariant should carve them out explicitly.
  Right now they are indistinguishable from oversights.
- **Argument-handling unwraps** (`malt-tools`) are ordinary bugs: a user
  running `chmod` with the wrong arguments should get an error, not a panic
  that takes the session's tool dispatch with it.

### Invariant II — OS calls only in `malt-platform`

Six files outside it reference `std::os::*`, `libc`, or `windows_sys`:

`malt-daemon/src/supervisor/mod.rs`, `malt-elevate/src/dispatch.rs`,
`malt-gateway/src/auth.rs`, `malt-tools/src/custom/{fds,ln,rm}.rs`

**`malt-elevate` is explicitly outside the layer system** per AGENTS.md, so
it is likely legitimate — confirm and exempt it in the invariant's wording
rather than leaving it as an apparent violation. The other five need either
a move behind `malt-platform` or a stated exemption.

## Why it matters

The invariants are the load-bearing claim of this codebase's architecture,
and AGENTS.md asserting they hold when they do not is worse than the
violations themselves. It means the next person to add an `unwrap` in
`malt-platform` is following what the file says is already normal.

## What done looks like

- Non-test `unwrap`/`expect` count is zero, or every survivor carries a
  written justification and the invariant's wording admits that category.
- Mutex-poison sites recover deliberately (`into_inner`) rather than panic.
- OS-call sites are moved or exempted in writing.
- **AGENTS.md's Hard Invariants section reflects reality**, including a
  standing note that "✅ Clean" means *checked on this date*, not *asserted*.
- A check that keeps it true — see [brief 005](005-enforce-quality-gates-in-ci.md).

## Gotchas

- **Do not bulk-replace.** The three classes above want three different
  fixes; a blanket `unwrap_or_else` would paper over the `malt-tools`
  argument bugs rather than fix them.
- The count must be measured excluding `#[cfg(test)]` blocks, or the work
  looks 5× larger than it is and the wrong sites get attention.
- `mash`'s 15 include parser/expander internals where the invariant is
  genuinely arguable. Decide the policy before editing, or the same site
  will be re-litigated later.

---

## Re-measured 2026-07-28, and the number moved

The brief says **39** non-test `unwrap`/`expect`. Re-counting on 2026-07-28
with test modules excluded gives **90 across 21 files**. It has grown, mostly
from specs 008 and 009 landing.

The count is approximate — the scan tracks `#[cfg(test)] mod` by brace depth
and does not parse strings or comments perfectly — so treat it as "about
ninety, concentrated in a few files", not a target to drive to zero by count.
Re-run it before starting; do not trust either number blind.

## Resolution pass 2026-07-28

The build-script policy is settled: build scripts are in scope for the
invariant. They execute as part of the production build and must report
missing environment, filesystem, or compiler-tool prerequisites as `Result`
errors rather than panic through `unwrap()` or `expect()`. A build-time error
is loud, but it is still an avoidable panic and keeping the wording narrower
would make the invariant ambiguous.

The runtime and build-script sweep is complete. A strict clippy scan over all
workspace libraries and binaries (`clippy::unwrap_used` and
`clippy::expect_used`) reports no non-test uses. The remaining occurrences
are in test modules. Poisoned mutexes in the FD and fake HCS registries now
recover their state with `into_inner()`.

The OS-call survey is also complete. Production references to `std::os::*`,
`libc`, `nix`, and `windows_sys` are confined to `malt-platform`; the
standalone `malt-elevate` helper has no such references and remains exempt by
the architecture wording. Descriptor probing, symlink creation, credential
file permissions, and Windows child-handle conversion were moved behind
platform APIs.

Where they cluster:

| Count | File |
|---|---|
| 19 | `crates/malt-daemon/src/elevate_client.rs` |
| 17 | `crates/malt-config/build.rs` |
| 9 | `crates/malt-protocol/build.rs` |
| 9 | `crates/malt-platform/src/vfs/fd.rs` |
| 7 | `crates/malt-platform/src/isolation/hcs.rs` |
| 6 | `crates/mash/src/expander.rs` |

**26 of the 90 are in `build.rs` files.** Constitution IV says "no `unwrap()`
or `expect()` outside `#[cfg(test)]` code", which was written about runtime
library code. A build script that panics fails the build loudly at build time,
which is arguably the correct behaviour and certainly not the failure mode the
invariant guards against. **Settle that question before touching them** — and
record the answer, because otherwise the next person re-litigates it.

## Tasks (added 2026-07-28, for handoff)

This is three sweeps, not one. They are independent and can land separately.

- [x] T001 Re-run the measurement and record the current figure. The strict production scan is now zero; the dated raw count above remains historical context, not a target.
- [x] T002 Decide whether `build.rs` files are in scope for Constitution IV, and write the answer down. Build scripts are in scope and now return build failures rather than using `unwrap()`/`expect()`.
- [x] T003 Sweep the daemon runtime paths first — the FD registry now recovers poisoned mutexes, and the daemon's fallible startup paths no longer panic. `elevate_client.rs`'s counted occurrences were test-only.
- [x] T004 Sweep `isolation/hcs.rs` (7). Fake backend registry locks now recover deliberately after poisoning.
- [x] T005 Sweep `mash` (`expander.rs`, `executor.rs`). Parser assumptions and tool lookups now have explicit non-panicking paths.
- [x] T006 Re-verify OS calls outside `malt-platform`. Production OS calls are now behind `malt-platform`; test-only platform probes remain in test code, and `malt-elevate` is exempt as a standalone helper.
- [x] T007 Gates per sweep, not once at the end: Windows `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, explicit Smoosh (`183 passed, 3 skipped unsupported`), and the WSL mirror's workspace build/test all pass. **macOS is not a target** (ADR-0006).

**A caution specific to this brief.** Converting an `unwrap` to a `?` changes
a panic into an error path, and an error path nothing handles is worse than a
panic nobody hits — it fails silently. Where you introduce a new error return,
check what the caller does with it. That is the same failure this repo has hit
repeatedly under a different name.
