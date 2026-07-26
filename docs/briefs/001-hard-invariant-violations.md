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
