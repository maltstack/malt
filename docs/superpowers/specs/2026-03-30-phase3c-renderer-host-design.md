# Phase 3C: Renderer Host

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-renderer` crate — FrameElement tree walking, capability-based degradation, dirty tracking, RenderCommand emission, per-client frame sequencing and backpressure
**Depends on:** Phase 3A (malt-daemon bus), malt-protocol (all render/frame_element types)

---

## Context

Phase 3 is decomposed into 6 sub-projects. This is 3C — the Renderer Host that transforms semantic FrameElement trees into RenderCommand deltas for clients:

| Sub-project | Status |
|---|---|
| 3A: Message Bus + Executor | Complete |
| 3B: Compat Translator | Not started |
| **3C: Renderer Host** | **This spec** |
| 3D: Session Store + Persistence | Not started |
| 3E: Process Supervisor + Plugin Host | Not started |
| 3F: API Gateway | Blocked on 3C |

---

## Core Pipeline

The Renderer Host is a bus subscriber that transforms FrameElement trees into RenderCommand deltas.

### Pipeline Stages

1. Receive FrameElement tree + ResolvedPane layout from bus
2. Walk the tree (depth-limited to 64 levels, node-limited to 10,000)
3. Resolve theme tokens to concrete RGB values
4. Check client capabilities and degrade unsupported elements
5. Diff against previous frame (dirty tracking)
6. Emit minimal RenderCommand delta as RenderBatch

### Complexity Bounds (Enforced)

| Bound | Limit | Behavior |
|---|---|---|
| Max tree depth | 64 levels | Deeper nodes truncated |
| Max nodes per frame | 10,000 | Excess partially rendered |
| Max RenderCommand output | 1 MiB per frame | Excess deferred to next tick |

---

## Client Tracking & Backpressure

### Per-Client Frame Sequencing

- Every RenderBatch carries a monotonic `frame_seq`
- Clients ack via `FrameAck { frame_seq }`
- RendererHost tracks per-client: last acked seq, unacked count

### Slow Client Detection

- Unacked frames > 30 (~500ms at 60fps) → stop producing for that client, mark "lagging"
- Client catches up (ack received) → resume producing

### Slow Client Shedding

- No FrameAck for 10 seconds → emit `SlowClientDisconnect`, remove client state
- Client can reconnect and re-sync via `SyncRequest`

### Attach Sync Protocol

- New client attaches → RendererHost snapshots full FrameElement tree + layout as `InitialState`
- Deltas begin from `frame_seq + 1`
- `SyncRequest` from client → fresh `InitialState` snapshot

### Capability-Based Degradation

- Each client declares `ClientCapabilities` (color depth, unicode, image protocol, VT passthrough, overlay)
- RendererHost produces separate RenderCommand streams per client when capabilities differ
- If all clients have identical capabilities, single stream (optimization)

---

## Module Structure

```
malt-renderer/
  src/
    lib.rs              — crate root, re-exports
    host.rs             — RendererHost: owns client states, orchestrates pipeline
    walker.rs           — FrameWalker: tree traversal, capability checks, command generation
    dirty.rs            — DirtyTracker: diff previous vs current frame
    theme.rs            — ThemeResolver: token → RGB (stub, extensible later)
    client_state.rs     — ClientState: per-client frame_seq, ack tracking, capabilities
    error.rs            — RendererError enum
  tests/
    walker.rs           — tree walking, depth/node limits, capability degradation
    dirty.rs            — dirty tracking diff tests
    client_state.rs     — frame sequencing, slow client detection/shedding
    host.rs             — integration: FrameElement → RenderBatch end-to-end
```

### Key Types

```
RendererHost          — owns per-client state, orchestrates pipeline
ClientState           — tracks capabilities, last frame, frame_seq, ack state
FrameWalker           — walks FrameElement tree, applies capability checks, produces draw commands
DirtyTracker          — diffs current vs previous frame, emits only changes
ThemeResolver         — maps theme tokens to concrete RGB (stubbed for now)
PaneFrame             — ties a PaneId to its FrameElement tree
ClientRenderBatch     — RenderBatch targeted to a specific client
```

### Dependencies

- `malt-protocol` — FrameElement, RenderCommand, RenderBatch, InitialState, FrameAck, SlowClientDisconnect, SyncRequest, ResolvedPane, ClientCapabilities, ResolvedStyle
- `thiserror`, `tracing`

### Bus Integration

- RendererHost exposes `fn process_frame(elements: &[PaneFrame], layout: &[ResolvedPane]) -> Vec<ClientRenderBatch>`
- Pure function approach: takes input, produces output, no bus dependency in the crate itself
- Actual bus subscription wiring happens when integrated into malt-daemon (separate task)

### Not In Scope

- Theme file loading (stub returns default colors)
- Scrollback rendering (Phase 3D)
- Plugin-contributed frame elements (Phase 3E)
- Bus wiring into malt-daemon (integration task after both 3B and 3C land)

---

## Testing Strategy

### Unit Tests — walker.rs

- Single Text element → DrawText command
- Paragraph → multiple DrawText commands
- Split element → SetClip + recursive children + ClearClip
- Depth limit: tree deeper than 64 levels truncates
- Node limit: tree with >10,000 nodes partially renders
- VtPassthrough → WriteRaw command
- Capability degradation: TrueColor client gets RGB, Basic256 client gets nearest color
- Unknown FrameElement variant → skipped gracefully

### Unit Tests — dirty.rs

- Identical frames → empty delta
- New text added → DrawText in delta
- Text changed → DrawText with new content
- Element removed → Clear region in delta
- First frame (no previous) → full render

### Unit Tests — client_state.rs

- Frame ack advances state
- 30 unacked frames → lagging flag set
- Ack received while lagging → lagging cleared
- 10 seconds no ack → shedding triggered
- New client → InitialState snapshot generated

### Integration Tests — host.rs

- End-to-end: FrameElement tree + layout → RenderBatch with correct frame_seq
- Two clients with different capabilities → different RenderCommand streams
- Client attach → InitialState, then deltas
- Output size limit: large tree → output capped at 1 MiB, remainder deferred
