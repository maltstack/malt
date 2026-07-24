# Deprecated 2026-07-24 — historical record only

This directory (`specs/` and `plans/` subdirectories) held Phase 3-4 build
specs and implementation plans (2026-03 through 2026-04), written using the
`superpowers` skill system. That system is no longer used on this project —
see `AGENTS.md`.

On 2026-07-24 every document here was cross-referenced against the actual
current code by a dedicated audit. Result: the large majority accurately
describe what was built and is still there — safe to treat as a reliable
historical record. The specific gaps and discrepancies that audit found
(including a confirmed, fixable bug in `malt-compat`'s VT translator, and a
confirmed stub in compat-pane session restore) were folded into
`docs/BACKLOG.md`, not left implicit in these files.

Do not add new content here. For current work, see `docs/BACKLOG.md`. For
new feature specs, use GitHub Spec Kit (`specs/`, via `/speckit-specify`).

Full audit: `docs/findings/2026-07-24-plan-implementation-audit.md`
