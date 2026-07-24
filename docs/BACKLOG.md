# MALT Backlog

Living document. Update it as work lands or priorities change — this is the
"what's next and why" record, distinct from `docs/design/architecture.md` (target
design) and `docs/adr/` (decisions already made and their reasoning).

Each item should be concrete enough to act on without re-deriving context.
Link to a findings doc or ADR for the "how do we know this" evidence instead
of restating it here.

For larger, multi-week feature phases (plugin system completion, full
cross-platform isolation enforcement, etc.), see AGENTS.md's "Implementation
Roadmap" (Phase B2 through Phase I) — this backlog tracks concrete,
near-term, evidence-based items, not the whole product roadmap.

## P0 — blocks daily-driver usability

- **Terminal grid rendering: cursor-position "staircase" bug.** Successive
  output lines render with growing left-padding instead of resetting to
  column 0. Shell logic is correct; the rendered grid is not. This is
  plausibly the single biggest gap between "the shell works" and "this is
  usable day to day." Not yet root-caused — candidates are
  `malt-renderer`'s `FrameWalker`/`DirtyTracker`, `malt-compat`'s VT-to-grid
  translation, or `mash`'s own cursor tracking.
  See: `docs/findings/2026-07-24-live-daemon-session.md#1-terminal-grid-rendering-cursor-position-staircase-bug--high-priority`

## P1 — real gaps, worth fixing soon

- **Backgrounded commands (`&`) don't appear to survive through the daemon's
  `/exec` route.** `ping -n 30 127.0.0.1 &` returned empty output and no
  process was found afterward, even though `mash`'s own background-job
  tests pass standalone. Not yet root-caused — could be daemon-side
  process/job bookkeeping, or a client-side timing artifact against a
  short-lived check. Needs a proper trace before concluding either way.
  See: `docs/findings/2026-07-24-live-daemon-session.md#2-backgrounded-commands--dont-appear-to-survive-through-exec--medium-priority-not-root-caused`
- **`malt new` has no `--isolation` flag.** The gateway API and daemon-side
  wiring both support it (confirmed working via direct gateway calls); the
  primary CLI just doesn't expose it. Small, mechanical fix.
- **`exec` responses always show `"exit_code": null`.** Observed on every
  call this session, including successful ones. Might be intentional
  (async result delivered elsewhere) — confirm before treating as a bug.

## P2 — deliberately deferred, not forgotten

- **PTY/compat supervisor spawn path has no isolation wiring.** Only
  `mash`'s own external-command spawn path (`malt_platform::process::spawn`)
  got wired to session isolation tiers today; `ProcessSupervisor::spawn` →
  `spawn_with_pty` (daemon-managed compat/legacy processes) did not.
  Deliberately scoped out to keep that day's change small and provable —
  see ADR-0001.
- **`malt-elevate` has 9 of 10 privileged operations stubbed.** Only
  `CreateSymlink` is real; `CreateNamespace`, `MountOverlay`, `SetCgroup`,
  `SetupNetns`, `ApplySeccomp`, `CreateRestrictedToken`,
  `ManageHcsContainer`, `ApplySeatbelt`, `BindPort` all return stub success
  without performing the operation.
- **HCS's real native path is unverified.** `hcs.rs` was ported from carboy
  with real Win32 bindings, but only fake-mode has actually been tested —
  the real `HcsCreateComputeSystem` path requires Windows Containers
  actually installed, which this dev machine doesn't have. Flagged
  explicitly rather than assumed working; see ADR-0001.
- **AppContainer and CRIU were not ported from carboy-isolation, on
  purpose.** Both were found to be stubs/placeholders in carboy itself
  (AppContainer's restricted-token function is a documented fallback with
  zero real isolation; CRIU is a 5-line `Err(Unsupported)` placeholder).
  Porting them would have added a false sense of security. If real
  implementations are ever wanted, they should be built as original work,
  not adopted because scaffolding already exists somewhere.
- **`vexil.std` types (Duration/IpAddr/SemVer/Url) are still missing in
  vexil-lang.** External dependency — vexil-lang's own docs defer this to
  their package-manager milestone. Tracked, not actionable from this repo.

## Done (for context — remove once stale)

- 2026-07-24: Reverted the uncommitted malt-stack (carboy/keg) dependency
  from `malt-platform`/`malt-elevate`; established vendor-not-depend policy
  (ADR-0001).
- 2026-07-24: Doc audit and fixes — `CLAUDE.md`, `AGENTS.md`,
  `docs/design/architecture.md`, `VEXIL_GAPS.md`, `docs/REFACTORING_PLAN.md` all
  brought back in line with verified reality.
- 2026-07-24: Ported HCS + capability-report model from carboy-isolation as
  owned source.
- 2026-07-24: Wired session `IsolationTier` through to real Job Object
  process containment in `mash`'s external-command spawn path (all 5 spawn
  call sites), with a real end-to-end test.
- 2026-07-24: Test rigor pass — found and fixed 2 real bugs in
  `job_objects.rs` (undersized `IO_COUNTERS` struct silently failing every
  job creation; hardcoded active-process-limit of 1 silently rejecting the
  second process in any job). Implemented real `query_active_processes`
  (was a hardcoded stub). Audited `tokens.rs` and `pty/windows.rs` — both
  genuinely solid. Added missing success-path tests to `signals/windows.rs`.
  Fixed a `CWD_LOCK` poison-cascade bug and unguarded env-var races causing
  intermittent test failures in `mash`'s and `malt-platform`'s test suites.
