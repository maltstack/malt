# Findings: Full Plan-vs-Code Implementation Audit

Date: 2026-07-24
Context: 44 historical planning documents from the original build
(2026-03-26 through 2026-04-07), spread across `docs/design/legacy-specs/`,
`docs/superpowers/specs/`, `docs/superpowers/plans/`, and top-level `plans/`,
were cross-referenced against the actual current code by 6 parallel research
agents, one per topic area. This is the evidence record; `docs/BACKLOG.md`
carries the resulting prioritized action items.

Method: each agent read its assigned plan+spec documents in full, then
grepped/read the actual corresponding crate source and tests, and classified
every major claim as IMPLEMENTED (cited), PARTIAL, NOT IMPLEMENTED, or
SUPERSEDED (design changed later, with the change noted). Research only — no
code was modified during the audit itself.

## Headline result

The overwhelming majority of the 44 documents describe work that was
genuinely built, close to spec, and is still there and working. This is not
a "the plans were fiction" finding — three of six topic areas (protocol/VNP,
mash shell core, clients+Smoosh) came back essentially clean. The valuable
findings are concentrated in a handful of specific, real gaps and one
concrete bug, listed below. All 44 documents are safe to treat as accurate
historical record; see "Retirement" at the end.

## Real, actionable findings (fed into `docs/BACKLOG.md`)

### 1. `CompatTranslator::feed()` accumulates instead of replacing — strong lead for the P0 rendering bug

`crates/malt-compat/src/translator.rs:36` does
`self.last_data.extend_from_slice(data)` on every `feed()` call. Both the
original design doc and its own implementation plan specify
`self.last_data = data.to_vec()` (replace, not accumulate). The buffer grows
forever and `frame_element()` returns the whole thing every time, so
`DirtyTracker` sees it as changed every frame and every `RenderBatch`
re-sends the session's entire raw VT history instead of just new bytes.

This is a concrete, self-contained, fixable bug — not a hypothesis. Directly
relevant to the P0 "staircase" rendering bug from
`docs/findings/2026-07-24-live-daemon-session.md`, though not proven
identical: `/sessions/{id}/output`'s `StyledGrid` route may read
`TerminalGrid` cells directly rather than going through this pipeline, in
which case a second, complementary lead applies —

### 2. Possible missing PTY ONLCR translation (secondary lead, same bug)

`TerminalGrid::execute()` correctly separates `0x0A` (linefeed, vertical
only) from `0x0D` (carriage return, column reset) per VT100 spec — this is
*not* a bug in malt-compat. But if the byte stream feeding the translator
carries bare `\n` without `\r` (no ONLCR translation upstream), this
spec-correct code would reproduce exactly the reported growing-offset
pattern. Needs checking at the mash/PTY layer, outside malt-compat/renderer.

### 3. Compat-pane session restore is a confirmed stub

`coordinator.rs:547-551` explicitly returns
`DaemonError::RestoreFailed(id, "compat pane restore not yet implemented")`
for any Dormant session containing a Compat pane. The design's `spawn_compat`
function was never written (zero grep hits). By contrast, **shell-session
restore is real and tested** — 26 tests in `coordinator.rs` cover it
end-to-end (`restore_shell_session_from_dormant`,
`shutdown_graceful_saves_all_active_sessions`, etc.). Phase B2 is more done
than its "in design" status in the roadmap suggested — just not for Compat
panes specifically.

### 4. Session persistence API exists, but not where/how originally specced

`phase2-malt-session.md` called for `SessionRuntime::to_persisted()` /
`from_persisted()` methods and a `PersistedSession` type living in
`malt-session`. Grepping `malt-session` alone for these found nothing —
looks like a flat "never built." But it *was* built, just relocated:
`PersistedSession`/`PersistedPane` are schema-generated types from
`malt_protocol::persist::session`, and the actual conversion logic
(`build_persisted_session`, `SessionCommand::Snapshot`) lives in
`malt-daemon`'s `session_thread.rs`, not as methods on `SessionRuntime`. This
is SUPERSEDED, not NOT IMPLEMENTED — the discrepancy only became visible by
running the platform/session audit and the daemon-core audit in parallel and
comparing notes. Worth noting as a case for why the swarm approach caught
something a single linear read might have taken at face value in either
direction.

### 5. Dead code: `builtins.rs`'s `Builtin` trait / `BuiltinRegistry`

`crates/mash/src/builtins.rs` defines a trait/registry architecture that is
never referenced anywhere else in the crate. All 15+ builtins actually run
through an inline `match` in `executor.rs::try_execute_builtin()` instead —
functionally complete, just via a different, simpler architecture than this
one file's abandoned detour. Either remove the dead scaffold or migrate to
it; leaving both is confusing for anyone reading the code cold.

### 6. Dead code: second VT parser API in malt-compat

`crates/malt-compat/src/parser.rs` contains a full second parser API
(`VtEvent`, `CsiParams`, `Intermediates`, `VtParser`/`EventCollector`, ~150
lines) that's `pub` but not re-exported from `lib.rs` and not used by
`CompatTranslator` (which still drives `GridPerformer` via a bare
`vte::Parser`). Neither document mentions it. Likely scaffolding for the
Phase H out-of-process compat worker that was never wired up. Use it or
remove it.

### 7. `crates/malt-protocol/src/codec.rs` has wrong constants and tautological tests

Its `MSG_*`/`DOMAIN_*` constants don't match the real schema `@type()`
values (e.g. `MSG_COMMAND_OUTPUT=0x01` vs. the real `OutputChunk@0x04`), and
some referenced message types (`MSG_PING`/`MSG_PONG`/`MSG_SHUTDOWN`) don't
exist in the System domain schema at all. Its own tests in `tests/codec.rs`
just assert the constants equal themselves — they would never catch this.
Needs a fix-or-delete decision; nothing currently appears to depend on it
(unverified — worth a grep before deleting).

### 8. `max_output_bytes` and slow-client shedding defined but not enforced

`malt-renderer`: `WalkConfig.max_output_bytes` (1 MiB cap, per design) is
defaulted but never read in `walker.rs` — no enforcement exists despite the
design doc calling it "Enforced." Separately, `ClientState::should_shed()`
(10s-no-ack disconnect) exists and is unit-tested, but `RendererHost::
process_frame` never calls it — the logic is real but not wired in.

### 9. Test-coverage gaps on functionally-complete code

`malt-session`'s `GroupManager` is functionally complete (create_group,
add_session with max_sessions enforcement, on_oom, etc.) but has zero tests
anywhere in the crate. `malt-platform`'s `env.rs` has zero tests despite
being fully implemented. Neither is broken as far as this audit could tell —
both are coverage gaps, not confirmed bugs, but given today's track record
(two real bugs in `job_objects.rs` found specifically by writing real tests
for under-tested code), these are worth treating as real risk, not just
tidiness.

### 10. Backgrounded-command bug: confirmed to need new design, not a regression fix

The process-supervisor's original design (Phase 3E) never covered detached
background jobs (`&`) surviving past their pane's lifecycle at all — it's
scoped to interactive foreground processes tied to a PTY. The backgrounding
gap tracked in `docs/BACKLOG.md` P1 needs original design work, not a "find
where this broke" investigation against an existing spec.

## Lower-priority / informational

- `frame_element.vexil` grew from a "minimal, expand later" design to 21
  variants including all originally-deferred rich widgets (Table, List,
  Tree, etc.) — undocumented scope expansion, but it's a real, working
  superset, not a gap.
- `schemas/config/*.vexil` (daemon.vexil, user.vexil) use a `config` schema
  construct never mentioned in any Phase 1 planning doc — works, just
  undocumented.
- Session store format changed from the original `.vx`/Value-converter
  design to `.vxb` bitpack — already correctly reflected in the newer plan
  doc, only the older design doc is stale on this specific point.
- `orix/vexil-lang/` is no longer a local sibling workspace; `malt-protocol`
  now pulls `vexil-runtime` via a git dependency. Almost certainly
  intentional (matches vexil-lang's externalization, tracked elsewhere) but
  flagged in case it was a workspace-layout accident.

## What checked out clean (no material findings)

- Phase 0/1 protocol and VNP schema design — implemented closely, `vnp-
  protocol.md` in particular is accurate enough to be treated as the live
  protocol reference rather than a historical doc.
- Mash's env, expander, lexer/parser, and the builtins themselves (as
  opposed to the unused registry architecture) — implemented closely,
  often exceeding the original test-count targets.
- Platform config (PTY, process, signals, sockets), malt-layout (all 5
  strategies + resolve/ops/focus), malt-term — implemented closely.
- Daemon message bus/executor, session store (API, not format), process
  supervisor (for its actual documented scope), gateway core REST surface —
  all implemented as designed. Gateway's own "deferred to Phase 5" list
  (MCP transport, WebSocket, TLS auth) is confirmed still not built, which
  is expected, not drift.
- malt-bin and malt-tui — both implemented and substantially extended
  beyond their original designs (real daemon start/stop, real VNP
  connection, HTTP polling fallback — none of which were in the original
  "deferred" lists as done yet, but all now are).
- Windows Smoosh runner baseline — not just met, exceeded (more robust
  helper-staging than originally designed).

## Retirement

All 44 documents are confirmed accurate historical records (not
misleading — the earlier doc audit already handled the one genuinely
misleading document, `docs/REFACTORING_PLAN.md`). None need content
corrections. They're marked retired via a header note pointing here rather
than deleted, archived, or rewritten — see the directories themselves.
