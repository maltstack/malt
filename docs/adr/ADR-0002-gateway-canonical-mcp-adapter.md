# ADR-0002: API Gateway Is the Canonical Agent Control Plane; MCP Is a Replaceable Adapter

Date: 2026-07-24
Status: Accepted

## Context

`malt-mcp` is a thin, hand-written JSON-RPC wrapper around six HTTP
operations on `malt-gateway` (list/create/exec/get_output/send_input/
destroy). Investigating it directly (2026-07-24 live-daemon session, see
`docs/findings/2026-07-24-live-daemon-session.md`) and via code reading
surfaced real, confirmed problems with treating MCP as MALT's primary
agent-interaction model:

- **`exec` never returns a real execution identity or exit code.**
  `gateway_backend.rs`'s `exec_command` hardcodes `command_id: 0` and
  `exit_code: None` as literals — not a missed read of data the daemon
  already captures. The daemon's `RunCommand` reply channel only carries a
  plain `output: String`; there is no exit-code capture anywhere in that
  path today. Verified by reading the code, not inferred from behavior.
- **`malt-session::pane::CommandBlock` — a fully-built command-history
  ring buffer with exactly the fields an execution resource needs
  (`command_id`, `started_at`, `finished_at`, `exit_code`) — is completely
  unwired.** `grep` for `CommandBlock {` (construction) and
  `push_command_block` (its only mutator) outside tests returns zero hits
  anywhere in `malt-daemon` or `malt-session`. Sessions do not track
  command history today despite having the data model built for it.
- **`get_output` returns `StyledGrid`** — character cells with RGB/bold
  flags, the representation built for `malt-tui`/`maltty` — not something
  built for a program to parse. Confirmed firsthand: reading a session's
  output as an agent required a throwaway script to flatten grid cells
  back into text. Already tracked in `docs/BACKLOG.md` P1 before this ADR.
- **`send_input`'s actual forwarding semantics are unconfirmed and
  suspected wrong.** It may dispatch another `RunCommand` rather than
  writing raw bytes to an already-running process's stdin, which would
  explain why simple cases (a standalone echo) work in testing while
  genuinely interactive cases (a REPL, a password prompt) might not. Not
  yet verified against the code — flagged here as a claim requiring
  confirmation before Phase 3 work begins, not treated as settled fact.
- **MCP's own protocol is mid-overhaul.** The 2026-07-28 spec revision
  ("largest since launch") moves to a stateless core and drops the
  session-handshake model. Building MALT's primary agent contract directly
  around MCP would make MALT dependent on external protocol revisions that
  don't necessarily match MALT's actor model (see the landscape discussion
  in this repo's conversation history, 2026-07-24).
- Separately, comparative evidence suggests plain structured CLI/HTTP
  output is more reliable and far cheaper in tokens than MCP for
  agent-driven tasks (a 75-test study found MCP failing ~28% of the time
  vs. near-100% for direct CLI, and 10-32x more expensive in tokens) — an
  argument for the gateway-first design being right, not just convenient.

MALT already has the internal concepts a proper agent execution model
needs: `CommandStarted`/`CommandFinished`/`PromptReady`/`OutputChunk` exist
as real VNP message types. The gap is that the external HTTP/MCP surface
collapses all of this into a single blocking request returning a string.

## Decision

1. **`malt-gateway` is the canonical external control plane** for agents,
   automation, IDE integrations, and CI — not MCP. MCP may remain
   available as an optional compatibility adapter, but it does not own
   terminal semantics, execution state, authorization, output
   representation, or lifecycle behavior. Those belong to the Gateway,
   backed by VNP.
2. **The current `malt-mcp` is classified experimental/legacy.** No
   investment in migrating its current implementation while the
   Gateway's own agent contract remains incomplete — see Phase 2 below.
3. **The Gateway should expose MALT domain concepts** — sessions,
   executions with stable identity and lifecycle state, separate
   plain/structured/render output views, explicit input authority — rather
   than mirroring individual daemon methods or MCP tool conventions.
4. **First-party clients (`malt exec`, integration tests, agent clients)
   consume the same public contract** agents will, so the contract can't
   become an ornamental side entrance nobody's own tooling actually uses.
5. **MCP gets rebuilt last**, as a thin, disposable adapter over the
   stabilized Gateway contract, using the official SDK — small enough to
   replace again whenever MCP's spec changes without requiring
   architectural work inside MALT.

### Migration phases (see `docs/BACKLOG.md` for current status of each)

1. Record this decision (this document).
2. Quarantine `malt-mcp` — mark experimental, no further investment until
   Phase 4 lands.
3. Correct current Gateway defects: real exit codes, stdout/stderr
   separation, remove the hardcoded `command_id: 0`, verify and fix
   `send_input`'s actual forwarding behavior, add an agent-readable
   (plain/structured) output representation alongside the existing grid
   one, return errors for non-2xx cases.
4. Introduce execution resources: stable IDs, lifecycle state,
   cancellation, event sequencing, cursor-based reads. See the
   implementation addendum below for where this data actually lives.
5. Harden identity/authority: auth scopes actually enforced on routes
   (they exist but aren't consistently wired in), rate/payload limits
   enforced, input leases for exclusive/shared terminal control between
   multiple agents or humans on one session.
6. Make first-party clients consume the same contract.
7. Rebuild MCP as a thin adapter over the now-stable Gateway contract.

### Implementation addendum: where do execution events live?

Investigated 2026-07-24 by reading the actual code rather than assuming.

- **`malt-daemon::bus::{Bus, RingBuffer}` is the wrong home for the durable
  execution/event log.** `RingBuffer::drain()` is destructive — it empties
  the buffer as it returns messages, so there's no way to re-read from a
  cursor once consumed. Non-`Reliable` messages can be silently dropped
  under pressure (flow-control rejection, or priority eviction with no
  record it happened) — a defensible tradeoff for frame rendering (a
  dropped stale frame is harmless, a newer one supersedes it) and a
  correctness bug for command output (a silently dropped stdout chunk is
  corrupted data). There is no sequence-number field on `BusMessage` at
  all. Bus solves a genuinely different problem: ephemeral, drop-tolerant,
  live-only fan-out to already-connected clients.
- **`malt-session::pane::{PaneRuntime, CommandBlock}` is the right shape
  for execution *metadata and lifecycle* — extend it, don't replace it.**
  Its `command_blocks()` accessor returns a `&VecDeque` reference, not a
  draining consumer — durable, re-readable, indexable, exactly what
  cursor-based resumption needs. It's already bounded (1000 entries,
  evicts oldest), which already satisfies "durable or bounded execution
  history." It is, however, currently **completely unwired** (see
  Context above) — this is a real gap to close, not a reuse-for-free win.
- **Bus is still the right tool for one specific job: live notification.**
  Use `Priority::Reliable` publishes on the Bus to tell already-connected
  SSE clients "new data landed, go read it" — the bus is the wake-up
  signal, `CommandBlock` (extended) is the source of truth clients
  actually read from.
- **The byte-level output stream (`stdout.delta`/`stderr.delta`/
  `structured_output` events) has nowhere to live yet — this is genuinely
  new engineering**, not a build-vs-reuse question with an existing
  answer. Needs a real decision: extend `CommandBlock` with a captured-
  output buffer, or a companion structure keyed by `command_id`. Not
  decided here — flagged as the concrete open design question for
  Phase 4.
- Two things worth checking before Phase 4, not confirmed as bugs: the
  Bus's `Critical` inbox (`Vec<BusMessage>`) has no bound or eviction —
  worth knowing if Critical priority is ever considered for execution
  events, since an absent consumer could accumulate it unboundedly.
  `Bus::publish` discards the return value of
  `flow.try_publish_reliable()` — plausibly intentional per its own doc
  comment, unverified against `flow_control.rs`.

## Consequences

- MALT is insulated from MCP's breaking changes — the stable contract
  agents actually depend on lives in the Gateway/VNP, not in whatever MCP
  looks like this quarter.
- Real new engineering is required before MCP is "done" again — this is
  accepted, not treated as scope creep, per the Phase 3→7 sequencing:
  small, cheap, already-diagnosed defect fixes ship first and
  independently of the larger execution-resource/event-stream/lease work.
- `CommandBlock` needing to be wired up (not just extended) means Phase 3
  and Phase 4 are less separable than they first looked — capturing a real
  exit code and populating command history are the same piece of work,
  discovered by checking the code rather than assuming the Gateway layer
  alone was the gap.
- If a genuinely urgent need for full MCP-spec compliance shows up before
  Phase 7, that's new evidence and warrants revisiting this ADR — not a
  default reached by drift.
