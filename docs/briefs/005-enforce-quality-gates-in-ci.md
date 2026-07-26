# Brief 005 — The quality gates are green but unenforced

**Severity**: Medium · **Verified**: 2026-07-26

## What is wrong

The project runs four gates by hand — `cargo build`, `cargo test --workspace`,
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -D warnings`
— plus Smoosh POSIX conformance when `mash` changes. All currently pass.

**Nothing prevents a regression from landing.** Enforcement is a habit
recorded in AGENTS.md, applied by whoever is working.

## Why it matters

The habit is holding, but its cost is that every session re-runs a full
verification manually, and its risk is that one session that skips it lands a
regression the next session inherits without knowing. This project's history
is specifically of work being abandoned after drift accumulated unnoticed.

Two gates in particular are easy to skip because they are conditional:

- **Smoosh** applies only when `mash` changes, so it is exactly the gate a
  contributor is most likely not to realise applies to them.
- **Clippy with `-D warnings`** went green recently. Warnings accumulate
  silently and are tedious to clear in bulk, so the value is in never
  letting them start.

## What done looks like

- A CI workflow running all four gates on every push and pull request.
- Smoosh runs when `mash` or `malt-tools` changes — path-filtered, not
  unconditional, because it takes ~40 s and the point is that it cannot be
  forgotten rather than that it runs always.
- `deny.toml` is already present and should be enforced in the same place.
- A failing gate blocks the merge rather than reporting after the fact.
- AGENTS.md's Session Ritual points at CI as the enforcement and keeps the
  manual run as the local fast path, so the two do not drift.

## Gotchas

- **Windows is the primary target.** A Linux-only CI would pass while the
  platform that matters regressed; Smoosh's expected result differs between
  them (183/3 native Windows, 186/186 WSL).
- Two known flake classes exist and are documented in AGENTS.md
  (shared-process-state races in `mash`; timing in daemon concurrency tests).
  Both were fixed at the root on 2026-07-25 — three workspace passes, 4212
  tests, zero failures — but CI will surface any recurrence as a hard
  failure. That is correct behaviour and should not be met with a retry
  wrapper, which is how a flake becomes permanent.
- Build time on a cold cache is substantial for an 18-crate workspace; cache
  the cargo registry and `target/` or the gate becomes something people want
  to skip.
