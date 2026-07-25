# ADR-0004: Bounded direct channels for client delivery, not the Bus

**Status**: Accepted
**Date**: 2026-07-25
**Supersedes in part**: `docs/design/architecture.md`'s statement that
Gateway-to-daemon communication is exclusively Bus-based.

## Context

`docs/design/architecture.md` describes the message `Bus` as the delivery
mechanism between the daemon and everything consuming from it. Feature 004
(command lifecycle events) needed exactly that: a way to push
`CommandStarted`/`CommandFinished` to connected clients as they happen.

The obvious implementation was to publish those two messages to the Bus and
have the Gateway drain them. It was not taken, and the reason is not a
preference.

The Bus's `Reliable` priority is documented and implemented to never drop:

> `crates/malt-daemon/src/bus/mod.rs` — "Reliable messages are never
> dropped… The ring buffer grows beyond capacity rather than evict Reliable
> entries."

Both `CommandStarted` and `CommandFinished` are specified `Reliable` in
`schemas/shell.vexil`, matching `malt-protocol`'s priority mapping. So
routing them through the Bus would give every subscriber an unbounded
per-subscriber buffer, fed by an event rate the daemon controls and drained
at a rate an *external client* controls. A client that stops reading — a
hung agent, a stalled network, a laptop lid — grows daemon memory without
limit.

That is the precise failure feature 004's specification exists to prevent
(FR-009, FR-010, SC-007). Using the Bus would have implemented the bug the
feature was written to stop.

Audit finding A-20 (`docs/findings/2026-07-25-architecture-spec-codebase-audit.md`)
correctly identifies the resulting divergence between the shipped code and
the architecture document, and asks for an explicit amendment rather than a
de facto one. This ADR is that amendment.

## Decision

**Client-facing delivery uses bounded per-subscriber channels with an
explicit loss policy, not the Bus.**

Concretely, as implemented in `crates/malt-daemon/src/executor/events.rs`:

- a bounded per-session retention window for reconnecting subscribers
  (1024 events), which evicts oldest-first;
- a bounded per-subscriber channel (256 events, plus one slot reserved for
  the terminal notification);
- non-blocking delivery only — a subscriber that cannot keep up is told it
  fell behind, naming the range it missed, and is then dropped;
- monotonic sequence numbers so a dropped or disconnected subscriber can
  resume from a known position.

This follows a precedent already in the codebase rather than inventing one:
`render_pushers` delivers frames to VNP clients over a bounded
`sync_channel` with `try_send` and sheds clients that fall behind.

**The Bus is not deleted and is not deprecated by this decision.** Its
semantics are appropriate for trusted, in-process consumers whose drain rate
the daemon controls. They are not appropriate for an untrusted external
client. The distinction is the drain side, not the message type.

## Consequences

**Accepted:**

- `architecture.md`'s "exclusively Bus-based" claim is now wrong as written
  and must be amended to distinguish in-daemon delivery from client-facing
  delivery. This ADR is the authority until that edit lands.
- After feature 004, **the Bus still has zero consumers in non-test code.**
  This contradicts how `docs/BACKLOG.md` framed the work ("give the Bus its
  first real consumer") and is recorded there as a deliberate deviation
  rather than quietly re-scoped.
- Two delivery mechanisms now exist. That is a real cost: a future author
  must choose, and choosing wrong is possible. The rule is stated above —
  bounded channels when a consumer outside the daemon controls the drain
  rate, Bus otherwise.

**Left open, deliberately:**

The Bus's own design question is not settled here. A message bus whose
highest reliability tier grows without bound has no safe consumer, so it
needs one of:

1. a bounded `Reliable` policy with explicit gap signalling — essentially
   what `executor/events.rs` now implements, which could be generalized back
   into the Bus; or
2. an honest re-scoping of the Bus to trusted in-daemon consumers only, with
   the unbounded growth documented as intentional for that audience and the
   "zero consumers" status accepted as accurate rather than a gap.

Deciding between those changes shared semantics for every future message
type and deserves its own reasoning and tests. It was explicitly not done
opportunistically inside feature 004 (Constitution IX), and it is not done
here either — this ADR records why the divergence exists, not how the Bus
should end up.

## Alternatives considered

**Publish to the Bus anyway and cap growth in a wrapper.** Rejected: a
wrapper that silently contradicts the Bus's documented `Reliable` guarantee
is worse than not using the Bus, because the next reader would trust the
guarantee and reason from it.

**Fix the Bus's `Reliable` policy as part of feature 004.** Rejected under
Constitution IX. It changes shared semantics for every message type, and
would have expanded a delivery-path feature into a bus redesign mid-flight.

**Downgrade the two messages to a droppable priority.** Rejected: they are
`Reliable` because losing a `CommandFinished` leaves a client believing a
command is still running. The problem is the Bus's unbounded response to
that requirement, not the requirement.
