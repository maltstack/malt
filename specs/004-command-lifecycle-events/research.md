# Research: Command Lifecycle Event Delivery

**Feature**: 004-command-lifecycle-events | **Date**: 2026-07-25

The spec deliberately left the transport open (it is a HOW). This document settles it, and records a second decision that turned out to matter more.

## R1: Transport — Gateway SSE, with VNP deferred

**Decision**: `GET /sessions/{id}/events` as a Server-Sent Events stream on the Gateway. VNP frame forwarding to `malt-tui` is a deliberate follow-up, recorded in `docs/BACKLOG.md`, not attempted here.

**Rationale**:

- ADR-0002 makes the Gateway canonical and MCP an adapter over it. A new client-facing capability that skipped the Gateway would contradict a decision already made.
- The spec's P1 story is explicitly an AI agent, and agents reach MALT over HTTP. Serving them first delivers the feature's stated value soonest.
- SSE is a better fit than WebSockets here: the stream is strictly server→client, needs no client framing, reconnects natively in every HTTP client, and — decisively — carries a built-in resume mechanism (`Last-Event-ID`) that maps exactly onto FR-006/FR-007 rather than requiring a bespoke handshake.
- Humans are not left out in practice: `malt-tui` already renders command output live via the render path. Lifecycle events add most for programmatic consumers, which is precisely who the Gateway serves.

**Alternatives considered**: *VNP frames to `malt-tui` first* — rejected as the first target, not as a bad idea: the two schema messages exist and it will be the right way to give the human client structured state, but doing it first serves the lower-value audience while leaving ADR-0002's canonical surface without the capability. *Both at once* — rejected as scope: the daemon-side event source below is transport-neutral, so adding VNP later is additive, not a redesign. *WebSockets* — rejected: bidirectional framing this feature does not need, and no native resume story.

## R2: Delivery does NOT go through the existing `Bus` — the decisive finding

**Decision**: Do not use `crate::bus::Bus` as the delivery mechanism. Use a bounded per-subscriber channel plus a bounded per-session event log, both owned by the session control actor.

**Rationale**: The Bus's `Reliable` priority is *specified and implemented* to never drop:

> `crates/malt-daemon/src/bus/mod.rs`: "Reliable messages are never dropped… The ring buffer grows beyond capacity rather than evict Reliable entries."

And `schemas/shell.vexil` documents both `CommandStarted` and `CommandFinished` as `# Priority: Reliable`, matching `malt-protocol`'s `priority.rs` mapping. So the obvious implementation — publish lifecycle events to the Bus, have the SSE handler drain them — builds an unbounded per-subscriber buffer for a consumer the daemon does not control. A subscriber that stops reading grows daemon memory without limit.

That is a direct contradiction of **FR-009** ("MUST NOT cause unbounded resource growth") and **FR-010**, which are in this spec precisely because that failure mode is the risk. Using the Bus would implement the bug the feature exists to prevent.

The codebase already contains the correct pattern for pushing to an untrusted client: `render_pushers: HashMap<u64, SyncSender<RenderBatch>>` with `sync_channel(4)` and `try_send`, paired with `shed_stale_clients`. That is bounded, non-blocking, and has an explicit policy for a client that falls behind. This feature follows it.

**Consequence to record honestly**: `docs/BACKLOG.md` frames this work as "giving the Bus its first real consumer." After this feature ships, **the Bus will still have zero consumers**. That is a deliberate deviation from the recorded plan (Constitution IX) and must be written into the backlog as part of this work, along with the underlying question it exposes: a message bus whose highest reliability tier grows without bound has no safe consumer, so the Bus needs either a bounded `Reliable` policy with explicit gap signalling, or an honest re-scoping to in-daemon trusted consumers only. Resolving that is its own change, with its own tests — not something to fix opportunistically inside this feature.

**Alternatives considered**: *Fix the Bus's `Reliable` policy as part of this feature* — rejected under Constitution IX: it changes shared semantics for every future message type, deserves its own reasoning and tests, and would expand this feature from "delivery path" to "bus redesign" mid-flight. *Use the Bus with a wrapper that caps growth* — rejected: a wrapper that silently contradicts the Bus's documented `Reliable` guarantee is worse than not using it, because the next reader would trust the guarantee.

## R3: Channel type — `tokio::sync::mpsc`, not `std::sync::mpsc`

**Decision**: Per-subscriber `tokio::sync::mpsc::channel(256)`. The session control actor (a plain OS thread) pushes with `try_send`; the SSE handler (async) awaits `recv()`.

**Rationale**: The existing Gateway idiom — `reply_rx.recv_timeout(...)` on a `std::sync::mpsc` receiver inside an async handler — blocks a tokio worker thread for the call's duration. `exec_command` does this for up to 30 s, which is already a wart but survivable because it is bounded. An SSE stream lives for minutes or hours; blocking a worker per subscriber would exhaust the runtime after a handful of clients. `tokio::sync::mpsc` solves both ends cleanly: its bounded `Sender::try_send` is callable from a sync thread without a runtime handle, and its `Receiver::recv()` is awaitable. `malt-daemon` already depends on `tokio` with the `sync` feature, so this adds no dependency.

**Alternatives considered**: *`std::sync::mpsc` + `spawn_blocking` bridge* — rejected: one blocking task per subscriber, same exhaustion problem in a different pool, plus a shutdown-coordination problem. *`tokio::sync::broadcast`* — genuinely tempting (it is built for fan-out and has lagged-receiver semantics), rejected because its lag behavior drops the *oldest* messages for a slow receiver and reports a count, while FR-008 requires resuming from a *client-supplied position* — the per-session event log in R4 has to exist regardless, and once it does, broadcast adds a second overlapping buffer.

## R4: Catch-up — bounded per-session event log + monotonic sequence

**Decision**: The control actor owns an `EventLog`: a `VecDeque<LifecycleEvent>` capped at 1024 entries, each stamped with a monotonic per-session `sequence: u64`. A subscriber sends its last-seen sequence (via SSE `Last-Event-ID`); the log replays everything after it, then the subscriber joins the live stream. If the requested sequence is older than the oldest retained entry, the server emits an explicit gap event *first*, then replays what it still has.

**Rationale**: FR-006/FR-007 need a stable resume position, and a monotonic sequence is the simplest one that survives reconnection without server-side per-client state. SSE's `Last-Event-ID` header carries it natively, so no bespoke resume handshake is needed. The 1024 cap matches the spirit of the 1000-entry command-history bound while staying independent of it — this window exists to make reconnection correct, not to be an audit log (spec Assumptions).

The gap event is the load-bearing part and satisfies **FR-008** and **SC-007**. A stream that silently skips events while looking healthy is worse than one that refuses to start, because the client draws confident wrong conclusions. Emitting the gap *before* the replay means the client learns about the hole before receiving data that would otherwise look contiguous.

**Alternatives considered**: *Replay from persisted command history* — rejected: history stores completed commands, not the start/finish transitions with their sequence positions, and conflating them would make the event stream's retention accidentally depend on persistence semantics. *No catch-up at all* — rejected: FR-007 exists because an agent that reconnects and silently resumes from "now" can conclude a command never ran.

## R5: Slow-consumer policy — lag, signal, then drop

**Decision**: On publish, `try_send` to each subscriber. If the channel is full, the subscriber is marked **lagged** and its channel is *not* grown. The daemon then sends it a single terminal gap event describing what it missed (best-effort, replacing any queued content it can) and closes the subscription. The client sees a clean end-of-stream with a stated reason and can reconnect with its last-seen sequence — which lands it on the R4 catch-up path.

**Rationale**: This satisfies FR-009 (no unbounded growth, no execution delay: `try_send` never blocks the control actor), FR-010 (never silently starved — the subscriber is told), and FR-011 (resources released). It also degrades usefully: a dropped subscriber's natural next move is to reconnect from its last sequence, which is exactly the mechanism R4 already provides. Closing rather than limping matches the renderer's existing `shed_stale_clients` precedent, so the codebase has one story for "client cannot keep up," not two.

**Alternatives considered**: *Drop individual events and keep the subscription alive* — rejected: it produces a subscriber that believes it has a complete stream, the exact SC-007 violation. *Block until the subscriber catches up* — rejected outright: it makes an external client able to stall command execution, inverting the daemon-authoritative model.

## R6: Event payload — JSON over SSE, mirroring the schema messages

**Decision**: SSE `data:` frames carry JSON objects whose fields mirror the schema-defined `CommandStarted`/`CommandFinished` messages, with the SSE `event:` field naming the type and `id:` carrying the sequence.

**Rationale**: Consistent with how every other Gateway route already renders payloads to JSON (`get_output`, `exec`, `history`), so agents parse it with the same tooling. Constitution V is satisfied because the Gateway is the sanctioned external boundary, not an inter-component side-channel (see the Constitution Check table for the full argument). Keeping the field names aligned with the schema messages means the deferred VNP path (R1) carries the same information under the same names rather than diverging.

**Alternatives considered**: *Bitpack-encoded VNP frames over SSE* — rejected: SSE is a text protocol, and forcing binary through it would need base64, giving agents something strictly harder to consume for no gain on a boundary that is already JSON by design.

## R7: Access control — `Read` scope, and a middleware-ordering check

**Decision**: `(GET, "/sessions/{id}/events")` → `AuthScope::Read` in `required_scope`. Unknown session → 404 before the stream opens.

**Rationale**: Same sensitivity class as `/output` and `/history` — command text can contain paths, arguments, and secrets typed at the prompt (FR-012). `required_scope` fails closed to `Admin` for unrecognized routes, so a forgotten entry denies rather than leaks; the entry makes Read-scoped clients work.

**Verification needed during implementation, not assumed**: auth and rate-limit middleware must run *before* the stream is established, and the rate limiter must not be applied per-event to a long-lived connection. Feature 0a's `with_auth` wraps routes registered before it is called, and `malt-bin` applies it last — the events route must therefore be registered inside `build_router` like every other route, which it will be. tasks.md carries an explicit test for the 401/403 cases rather than trusting this reasoning.

## R8: `malt-mcp` is deliberately not touched

**Decision**: No MCP tool for event subscription in this feature.

**Rationale**: MCP tools in this codebase are request/response JSON-RPC over stdio; a long-lived event stream does not fit that shape, and inventing a polling tool over the top of a streaming endpoint would hand agents a worse interface than the SSE endpoint they can already consume directly. An MCP-native subscription belongs with a real MCP notifications design, which is its own decision. Recorded in `docs/BACKLOG.md` rather than half-built.

## R9: Relationship to features 002 and 003

**Decision**: Publish from the control actor's existing `ExecutionStarted` and `ExecutionCompleted` handlers — the same two points that already drive command history.

**Rationale**: Those handlers are already the authoritative "a command started / a command finished" moments, already carry `command_id`, command text, start timestamp, and exit code, and are already ordered correctly relative to each other by 002's finalization barrier. Publishing there means events and history cannot disagree — they are derived from one source, at one point, rather than instrumented twice.

One consequence worth stating: `ExecutionCompleted` is handled when the result is *committed* (after the finalization slices complete), not when the worker finishes. That is the correct moment — it is when the session's observable state actually includes the command's effects — and it means a finish event never arrives before the output that produced it is visible to a subsequent `get_output` call.

## R10: A first-party consumer ships with the endpoint — `malt watch`, not the SDK

**Decision**: `malt-bin` gains a `malt watch <SESSION_ID>` subcommand that consumes the SSE stream with real reconnect, `Last-Event-ID` resume, and gap reporting. The `malt-gateway-sdk` crate is **not** built here.

**Rationale**: As originally planned, this feature would have shipped an event stream with no first-party client — `curl` in the quickstart, and tests driving the backend directly. That leaves the reconnect-with-resume loop, which is where all the fiddly behavior lives, written by nobody. This project's recurring failure mode is exactly that: mechanisms that are built, unit-tested, and never actually driven (`should_shed`, the rate limiter, `TokenStore`, `CommandBlock`). Feature 003 is the counter-example — its most important behavior only became real when driven through an actual client.

A consumer also validates the contract in the one direction tests cannot: it proves the endpoint is *consumable*, not merely *servable*.

**Why not the full SDK**: `docs/BACKLOG.md` already sequences it — "a typed SDK wrapping **already-correct shapes** … once the Gateway contract stabilizes," explicitly *after*, not instead of, the fixes it depends on. This feature is part of what stabilizes that contract. The decisive test was whether designing for an SDK would change the contract already written, and it does not: the resume token is standard SSE `Last-Event-ID` rather than a bespoke handshake, loss is an explicit `gap` event, errors are HTTP statuses returned before the stream opens, and field names already mirror the schema messages. Nothing here would be redesigned by an SDK, so building one now buys no design safety and would convert a delivery-path feature into a client-library feature mid-flight (Constitution IX).

**Seeding, not deferring**: the reconnect/resume/gap loop in `malt watch` is written to be the nucleus the SDK later extracts — kept in a self-contained module in `malt-bin`, with no CLI-specific assumptions in the stream-handling logic. That is stated in tasks.md so the extraction is a move, not a rewrite.

**Alternatives considered**: *`curl` and tests only* — rejected per above; it is how the resume path ships unexercised. *Build the SDK crate now* — rejected: larger surface (auth, every endpoint, error types, retries, versioning), no design safety gained, and it contradicts the backlog's own sequencing. *Put the consumer in `malt-tui`* — rejected: `malt-tui`'s HTTP mode is already recommended for freeze, and its VNP mode is the deferred path (R1); adding an HTTP consumer there would invest in a surface the audit says to stop investing in.

**Consequence for `malt-bin`**: no new dependency. `reqwest`'s blocking `Response` implements `std::io::Read`, so SSE frame parsing is a `BufReader` line loop over the existing client.
