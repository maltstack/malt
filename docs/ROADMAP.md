# Roadmap — the remaining shape of the work

**Created**: 2026-07-28 · **Every status below was verified against code on
that date**, not carried over from the phase plan this replaces.

## What this is, and what it is not

This is a **map of coherent bodies of work**, grouped by domain. It exists so
that a session can see the shape of what remains without reading the whole
architecture document.

**It is not a schedule, and it does not decide what happens next.**
`docs/adr/ADR-0003-correctness-first-strategic-pivot.md` retired the phased
roadmap (Phase A–I) on 2026-07-24 precisely because using phases as a
sequence grew scope faster than it hardened anything. That decision stands.
What governs order is, in this order:

1. `docs/BACKLOG.md` — prioritized, evidence-based, and the thing to read
   before picking up work
2. `docs/briefs/` — items already surveyed and scoped
3. ADR-0003's correctness-first priority list, as amended by ADR-0005

The domains below keep their old phase letters in parentheses only so that
older documents and commit messages remain findable. **The letters are names,
not an order.** Nothing here should be started because it is "next"; it should
be started because the backlog says it matters.

## What checking it changed

The phase plan was written before 2026-07-24. Re-verifying every item on
2026-07-28 moved most of it:

| Claim in the old plan | Reality on 2026-07-28 |
|---|---|
| B2 "in design" — graceful shutdown, dormancy, restore, `DetachSession` | **Done.** `shutdown_graceful`, `SessionLifecycle::Dormant`, restore-on-attach, and a `DetachSession` handler all exist |
| C "add 11 missing FrameElement variants" | **All 11 already in `schemas/frame_element.vexil`** — Table, List, Tree, Diff, ProgressBar, Sparkline, Badge, KeyValue, Tabs, Modal, StatusBar |
| C "MASH FrameElement emission (currently absent)" | **Built and unwired** — see below; absent in effect, but not for the stated reason |
| C "wire Smoosh into CI" | **Done** — blocking job, path-filtered to `crates/mash\|malt-tools` |
| C "IsolationContext token injection" | **Done** by spec 008 |
| D "wire rate limiter into route handlers" | **Done** — `malt-gateway/src/middleware.rs`. But `docs/briefs/003` says the limiter has no window, so it is wired *and* wrong |
| H "isolation enforcement" | **Largely superseded** by specs 007/008/009 and ADR-0005 |
| E, G | **Explicitly paused** by ADR-0003 — listed here for completeness, not as available work |

**The most useful correction**, and a good argument for re-checking rather
than resuming: `crates/mash/src/frame_element.rs` already composes `Table`,
`Badge`, `KeyValue` and `ProgressBar` elements via `compose_prompt`,
`compose_stdout`, `compose_stderr` and `compose_command_output` — and every
one of those has **zero callers outside its own file.**

That is the **seventh** instance of AGENTS.md failure class 1 ("it exists, it
is tested, and nothing calls it"). The old plan said to build this. The work
is largely done; what is missing is a caller.

---

## Structured output (C)

The largest genuinely-unbuilt domain, and the one closest to MALT's premise —
a daemon that emits typed output rather than escape codes.

- **Wire MASH's FrameElement composers.** `mash/src/frame_element.rs` exists
  with zero external callers. Establish first whether the composers are
  *correct* (their tests construct types, so class 4 applies) before wiring.
- **Emit the unused variants.** `Tree`, `Diff`, `Sparkline`, `Tabs`, `Modal`
  and `StatusBar` have no constructor anywhere in the workspace. `Table`,
  `Badge`, `KeyValue` and `ProgressBar` are constructed only in the unwired
  file above.
- **Theme token resolution** (F). `malt-renderer/src/theme.rs:7` — "Currently
  a stub returning default colors." Structured output that cannot be themed is
  half a feature.

## Shell robustness (C)

None of these exist; all were checked on 2026-07-28.

- `catch_unwind` at every MASH poll point. Two exist in `executor.rs`; the
  plan called for keystroke, execution and expansion.
- Alias expansion depth limit (1024) and subshell recursion limit (256) — no
  limit constants found.
- Per-session watchdog with heartbeat and an SLA — nothing found.

## Gateway hardening (D)

**Now specced as `specs/010-gateway-hardening/`.** Scoping it on 2026-07-28
verified each item and removed two:

- **Rate limiter has no window** (`docs/briefs/003`) — still true, and worse
  than written: `refill`/`refill_all` have **zero production callers**, so a
  client is refused until the daemon restarts. Spec 010 US1.
- **No request body limit** on `/exec` and `/send` — still true. Spec 010 US2.
- **No global ceiling, no retry hints** on refusals — still true. Spec 010 US3.
- ~~Per-endpoint scope enforcement~~ — **not a gap.** `middleware.rs` maps
  `(Method, path)` to a scope and defaults `_ => AuthScope::Admin`, so an
  unmapped route demands the *highest* scope. Two sources of truth is a
  maintainability concern with a fail-closed mode, not a hole.
- ~~VNP frame writer bound~~ (`docs/briefs/004`) — **already fixed**;
  `framing.rs:203` bounds the length before the cast. Brief marked resolved.

## Persistence (B3)

- **Scrollback.** Not built. mmap append-only log per pane, ring buffer
  header, disk budget, per-client scroll offsets.

## Isolation and containment (H)

Mostly superseded. What ADR-0005 leaves open:

- **Linux session backends** (007 T032) — the platform modules exist and are
  unwired; class 1 applies.
- ~~macOS session backends (007 T033)~~ — **out of scope** while ADR-0006
  stands. macOS is unsupported; do not pick this up as available work.
- ~~The Unix PTY is inverted~~ — **fixed 2026-07-28** (`docs/briefs/007`).
  The child now gets the slave with `setsid`/`TIOCSCTTY`; compat panes deliver
  output on Linux for the first time. A macOS-only delivery gap remains,
  deferred by ADR-0006.
- Contained-tier completion: helper-owned HCS spawning plus a validated image
  layer configuration for session compute systems.

## Layout (F)

- Focus layer segmentation — tiled base layer vs float overlays, directional
  navigation staying within a layer. No layer concept found in `malt-layout`.

## Paused — not available work (E, G)

Listed so nobody re-derives them as gaps. ADR-0003 paused these deliberately:

- **Observability (E)** — metrics registry, Prometheus endpoint, real
  `/health`, diagnostics channel, structured log rotation.
- **Plugin system (G)** — startup lifecycle, user overrides, output filtering,
  `malt plugin audit`, latency SLOs, and the `malt-app-sdk` crate (which does
  not exist; note it is *not* `malt-tui`, and the two are easy to conflate
  because both were described as dual-mode runners).

Also paused: the GPU client (`maltty`) beyond basic usability, remote shared
deployments, MCP-specific expansion, and maintaining competing clients.
`malt-tui` in VNP mode is the single reference human client.

---

## Keeping this honest

Every entry above carries a verified date because the version this replaces
did not, and roughly half of it had silently become false. When picking
anything up here:

1. **Re-verify before starting.** This document is a map, and maps go stale.
2. **Ask class-1 first.** Three items in the old plan called for building
   something that already existed. Search schemas and type definitions, then
   grep for callers excluding the defining crate and tests.
3. **Update this file when a domain moves**, or it becomes the next thing
   someone has to re-check from scratch.
