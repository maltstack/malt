# Work briefs

Between a backlog line and a Spec Kit feature. A backlog entry says *what is
wrong*; a brief says *what to do, what done looks like, and what will go
wrong on the way* — enough to pick up and act on without re-deriving context,
but without the ceremony of `/speckit-specify`.

Use a brief when the work is a bounded fix or hardening pass. Use Spec Kit
when it is a feature with user stories worth slicing.

**Every claim in a brief is verified against code at the stated date, with
file:line.** That is the difference between these and the backlog entries
they came from: several audit findings turned out to be partly stale when
checked, and one — A-08 — was already closed. A brief that restates an audit
without re-checking is worth less than nothing, because it reads as
confirmation.

| Brief | Severity | Verified | One line |
|---|---|---|---|
| [001 — hard invariant violations](001-hard-invariant-violations.md) | High | 2026-07-26 | 39 non-test `unwrap`/`expect` and 6 files calling OS APIs outside `malt-platform`, while AGENTS.md marks both invariants clean |
| [002 — `kill` does not kill](002-process-kill-does-not-kill.md) | High | 2026-07-26 | `ProcessSupervisor::kill` removes a map entry and returns `Ok`; the process keeps running |
| [003 — rate limiter has no window](003-rate-limiter-has-no-window.md) | High | 2026-07-26 | A "per window" limiter with no window: a client is banned permanently after N requests |
| [004 — writer lacks the reader's frame bound](004-frame-writer-size-bound.md) | Medium | 2026-07-26 | `FrameWriter` casts length to `u32` unchecked while `FrameReader` enforces a 16 MiB max |
| [005 — the lint gate is green but unenforced](005-enforce-quality-gates-in-ci.md) | Medium | 2026-07-26 | Four gates pass locally and nothing prevents a regression landing |
| [006 — HCS backend access-violates](006-hcs-backend-access-violation.md) | High | 2026-07-26 | **RESOLVED** — null `HCS_OPERATION` handle passed to `HcsStartComputeSystem`/`HcsTerminateComputeSystem`. All three recorded hypotheses were wrong; underneath it, `HCS_E_ACCESS_DENIED`. vexil-v2 has the identical bug |

## Not written up, and why

- **A-02 (isolation)** — being delivered now as `specs/007-fail-closed-isolation/`.
- **Helper-owned privileged-operation entitlement** — discovered while
  implementing `specs/008-privileged-helper/`. The service boundary is live,
  but a helper-verifiable session owner/PID/storage-root authority is a
  security architecture decision, not a bounded brief. It remains explicitly
  fail-closed and is recorded in `docs/BACKLOG.md`.
- **A-08 (pre-handshake exhaustion)** — already closed by spec 005; the
  backlog says so. Listed here because a stale reading of the backlog
  headings suggested otherwise.
- **A-10, A-11, A-15, A-16** and the smaller `mash`/`vexilc` items — real,
  but each is a single-file fix better handled when someone is next in that
  file than as a standalone effort. They stay in `docs/BACKLOG.md`.
- **P2 items** — deliberately paused per ADR-0003. Not stale, not forgotten,
  not actionable now.
