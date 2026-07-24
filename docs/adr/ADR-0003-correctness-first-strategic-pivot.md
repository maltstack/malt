# ADR-0003: Retire the Phased Feature Roadmap for a Correctness-First Strategy

Date: 2026-07-24
Status: Accepted

## Context

MALT has been abandoned-via-rewrite three times (vexil-v2 → malt →
malt-stack) after multi-day sprints that grew scope faster than they
hardened it. AGENTS.md's "Implementation Roadmap" (Phase A through Phase
I) was written as a sequence of feature phases — plugin lifecycle, full
cross-platform isolation enforcement, observability, layout/theme
completeness, a third-party App SDK — most of them still ahead of where
the project actually is. Today's session did two things that changed the
picture enough to warrant a formal decision rather than a quiet drift:

1. **A round of direct bug fixes** (commits `750fd37..13d298c`) closed six
   concrete, evidence-based backlog items — the P0 rendering "staircase"
   bug, wrong `malt-protocol` codec constants, two dead-code detours, two
   unwired `malt-renderer` safety mechanisms, `exec`'s hardcoded-null exit
   code, and a real duplicate-session-id bug in `malt-session::GroupManager`
   found by writing tests for previously-untested code. Every one of these
   was a case of code that looked done (it compiled, it had some tests, it
   matched a schema) but silently didn't do what its own design intended.
2. **A five-agent audit swarm**, scoped against a 9-step "agent and human
   coexist on one persistent session" demo used only to bound priority (not
   to build against), covering: execution correctness (output shape, exit
   codes, execution IDs, lifecycle events), input and concurrency (raw
   stdin, input authority, multi-client rendering), persistence and
   restore (command history, compat-pane restore, scrollback, env
   round-trip), isolation fail-open safety, and the client/SDK surface.
   Findings are recorded in full, with file:line citations, in:
   - `docs/findings/2026-07-24-audit-execution-correctness.md`
   - `docs/findings/2026-07-24-audit-input-concurrency.md`
   - `docs/findings/2026-07-24-audit-persistence-restore.md`
   - `docs/findings/2026-07-24-audit-isolation-safety.md`
   - `docs/findings/2026-07-24-audit-client-sdk-surface.md`

The audit's headline result: **the "designed but never wired" pattern
found in today's direct fixes is not an isolated occurrence — it is the
dominant shape of the gap between MALT's design and its code**, repeated
independently across five different subsystems:

| Subsystem | Real, tested, unwired mechanism |
|---|---|
| Renderer (fixed today) | `ClientState::should_shed()` — 10s-timeout client eviction |
| mash | `Env::to_snapshot()`/`apply_snapshot()` — full shell-state persistence, built for exactly this, never called by `malt-daemon` |
| malt-session | `CommandBlock`/`push_command_block` — command history ring buffer, zero non-test callers |
| malt-platform (Windows) | `tokens.rs`/`hcs.rs` — real, fail-closed, well-tested isolation primitives, unreferenced from any spawn path |
| malt-gateway | `TokenStore`/`AuthContext`/`RateLimiter` — real auth/rate-limit logic, never attached to `build_router` |
| malt-daemon | `Bus` — has publishers, zero consumers anywhere in non-test code, for any message type |

Beyond that repeated pattern, the audit surfaced findings that change how
several of the demo's steps should be read:

- **The Gateway has no auth enforcement wired in at all today.** Every
  route is Admin-equivalent open to anyone who can reach the port; the
  daemon prints an "API token" that does nothing
  (`docs/findings/2026-07-24-audit-client-sdk-surface.md` §4). This is a
  live gap, not a latent one, and arguably more urgent than the isolation
  question below, since it's about who can talk to the daemon at all.
- **The session executor is single-threaded and blocks on one command
  queue.** A long-running command (the demo's own `cargo test` example)
  blocks attach, output polling, and input processing simultaneously for
  its entire duration; attaching mid-command can hard-fail a TCP
  connection outright rather than degrade gracefully
  (`docs/findings/2026-07-24-audit-input-concurrency.md` §3c). This is the
  root cause behind three separate symptoms the audit found (no live
  stdin, no render notification for gateway-driven execution, attach
  timeouts) — none of the individual symptoms can be fixed correctly
  without addressing the cause first.
- **The isolation concern the project owner raised directly turned out
  broader than its own framing**: Windows Job Objects are fail-open *and*
  tier-blind (Restricted/Capped/Contained all get identical, uncapped
  treatment); Linux and macOS have zero isolation enforcement wired at
  spawn time for any tier, on any platform, success or failure — stronger
  than AGENTS.md's "unverified" language suggested
  (`docs/findings/2026-07-24-audit-isolation-safety.md`).
- **The VNP client (`malt-tui`) — the one path architecture.md treats as
  authoritative — never renders shell output at all.** It doesn't handle
  `RenderCommand::WriteRaw`, regardless of attach timing. The client that
  "happens to work" (`HttpConnection`) does so only by bypassing the
  FrameElement/RenderCommand pipeline entirely
  (`docs/findings/2026-07-24-audit-input-concurrency.md` §3a).
- **Two of the biggest persistence gaps need a schema change, not just
  code wiring**: `PersistedPane` has no field for command history and no
  field for an `EnvSnapshot`, so even after the in-memory wiring lands for
  either, neither survives a daemon restart without also extending
  `schemas/persist/session.vexil`
  (`docs/findings/2026-07-24-audit-persistence-restore.md` §1, §4).

Set against this, continuing to plan around AGENTS.md's Phase C–I roadmap
(plugin lifecycle, full observability, a third-party App SDK, broad
FrameElement variant expansion, remote deployment, exotic isolation tiers)
would mean building new surface area on top of a core that cannot yet
reliably run one command, show its output correctly to one attached
client, or guarantee the isolation boundary it claims to provide. That is
the same failure shape — breadth outpacing hardness — that produced the
vexil-v2 → malt → malt-stack chain.

## Decision

1. **AGENTS.md's phased "Implementation Roadmap" (Phase A through Phase
   I) is retired**, except as historical record of what Phase A/B1
   actually completed. It is replaced by a flat, priority-ordered list of
   correctness and hardening work, with an explicit, equally-durable list
   of paused work — see the rewritten AGENTS.md section this ADR
   accompanies.
2. **Prioritized, in order** (the project owner's own ordering, with two
   audit-discovered structural prerequisites called out ahead of it,
   since they gate several of the items below and were not visible when
   the ordering was first proposed):
   - **0a. Gateway auth actually enforced** — wiring `TokenStore`/
     `AuthContext`/`RateLimiter` into `build_router`. Prerequisite for
     safely exposing the Gateway to any agent at all, and for "human and
     agent coexistence" to mean anything (there's no notion of "who's
     allowed to claim input" without a notion of "who is this request
     from" first).
   - **0b. Decouple command execution from the session's single
     command-dispatch thread.** Prerequisite for genuine raw input, for
     gateway-driven execution to ever notify an attached human, and for
     attach to degrade gracefully instead of hard-timing-out during a
     long command.
   - 1. Correct plain stdout and stderr
   - 2. Real exit codes and execution IDs
   - 3. Command lifecycle events
   - 4. Persistent execution history
   - 5. Genuine raw input
   - 6. Human and agent coexistence
   - 7. Fail-closed requested isolation (new `IsolationPolicy`:
     `required`/`preferred`/`disabled`, plus the underlying per-tier
     enforcement work the audit found missing on every platform)
   - 8. A correct TUI rendering path
   - 9. Session restoration
   - 10. One excellent agent client or Gateway SDK
3. **Explicitly paused, with equal weight to the priority list above** —
   not deferred by default, not silently starved, a deliberate decision:
   the GPU client (`maltty`) beyond basic usability, plugin marketplace
   infrastructure, broad plugin lifecycle features, remote shared
   deployments, exotic isolation tiers beyond what's already built, large
   collections of new FrameElement/UI variants, MCP-specific expansion,
   elaborate observability systems, and maintaining multiple competing
   client experiences (`maltty` and `malt-web` are frozen as-is; `malt-tui`
   in VNP mode is the single reference human client per
   `docs/findings/2026-07-24-audit-client-sdk-surface.md` §5).
4. **The 9-step demo scenario is a scope-setting lens, not a task.** It
   exists to judge whether a given gap matters for genuine human/agent
   coexistence on one session, not as a feature to build toward directly.
   No work item in the resulting backlog should be justified solely by
   "the demo needs it" without also being independently correct/hardening
   work.
5. **ADR-0002 (Gateway canonical, MCP adapter) is unchanged and is now the
   detailed execution plan for priorities 1–4 and 10 above** — this ADR
   sets the overall strategy the phased retirement enables; ADR-0002
   remains the specific migration plan for the Gateway/MCP relationship
   within it. Where this ADR's audit findings extend ADR-0002's own
   scope (the Bus-has-zero-consumers finding, the schema-level persistence
   gaps, the auth-not-wired finding), those are captured in
   `docs/BACKLOG.md`, not by editing ADR-0002 itself.

## Consequences

- `docs/BACKLOG.md` is refreshed in the same change as this ADR to reflect
  all five audits' findings, prioritized against the ordering above, with
  the two audit-discovered structural prerequisites (0a, 0b) called out
  ahead of the project owner's original list rather than silently folded
  into it.
- Historical phase-completion facts (Phase A, Phase B1 done; the crate
  architecture, hard invariants, and code standards) are unaffected — this
  decision retires the *forward-looking* roadmap only, not the record of
  what already shipped.
- Any future request to resume Phase C–I-shaped work (plugin marketplace,
  broad observability, etc.) should be treated as new evidence warranting
  a fresh decision, not a default reached by drift back into the old
  roadmap.
- Some items in the priority list are more expensive than they first
  appear once sequenced correctly: item 5 (genuine raw input) and item 6
  (human/agent coexistence) both depend on 0b; item 4 (persistent
  execution history) depends on both the execution-correctness audit's
  in-memory `CommandBlock` wiring *and* the persistence audit's schema
  extension — treating either half alone as "done" would be the same
  looks-done-but-isn't pattern this ADR exists to stop repeating.
- If the auth-wiring or single-threaded-executor findings turn out to be
  wrong or already superseded by the time work starts on them, that's a
  reason to re-verify against current code before acting — per this
  project's own established practice (see the Session Ritual in
  AGENTS.md) — not a reason to distrust the audit's other findings.
