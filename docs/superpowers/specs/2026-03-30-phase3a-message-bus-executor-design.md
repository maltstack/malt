# Phase 3A: Message Bus + Session-Sharded Executor

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-daemon` core — message bus, session-sharded executor, coordinator, client connection, input routing
**Depends on:** All L0/L1 crates (malt-protocol, malt-platform, malt-config, mash, malt-term, malt-tools, malt-layout, malt-session)

---

## Context

Phase 3 is decomposed into 6 sub-projects. This is 3A — the foundation that everything else plugs into:

| Sub-project | What | Depends on |
|---|---|---|
| **3A: Message Bus + Executor** | Bus, executor, coordinator, connections | L0/L1 crates |
| 3B: Compat Translator | VT parser, virtual grid, worker modes | 3A |
| 3C: Renderer Host | FrameElement walker, dirty tracking, RenderCommand | 3A |
| 3D: Session Store + Persistence | vexil-store, crash recovery, scrollback | 3A |
| 3E: Process Supervisor + Plugin Host | App lifecycle, WASM runtime, fuel budgets | 3A |
| 3F: API Gateway | HTTP/MCP, auth, rate limiting, shadow tree | 3A, 3C |

---

## Message Bus

Per-session synchronous in-process message dispatcher. Each subscriber gets a bounded ring buffer.

### Types

```
Bus                    — owns subscriber slots, dispatches messages
Subscriber<T>         — typed receiver with bounded ring buffer
SubscriberId          — opaque handle for unsubscribe
BusMessage            — envelope + payload (any VNP domain message)
EvictionPolicy        — derived from Priority level
```

### Priority-Based Eviction

| Priority | Examples | Eviction | Flow Control |
|---|---|---|---|
| Critical | Resize, Signal | Separate unbounded inbox, delivered inline before ring drain | None needed |
| Reliable | CommandStarted/Finished, StructuredOutput | Never evicted | 25% per-producer cap + 60% global saturation guard |
| High | RenderCommand, FrameElement | Evictable only by newer High messages | — |
| Normal | OutputChunk | Evictable, streaming/loss-tolerant | — |
| Low | Persist, Plugin, Telemetry | First to evict | — |

### Subscriber Buffer Sizes (Configurable)

| Subscriber | Default Buffer |
|---|---|
| Renderer Host | 4,096 |
| Structured Output Parser | 2,048 |
| Session Store | 512 |
| Plugin Host | 1,024 |
| API Gateway | 2,048 |

### Invariant

Bus never blocks producers. Overflow → evict according to priority policy.

---

## Session-Sharded Executor

### Threading Model

```
Coordinator (own current_thread runtime)
  ├── Session 1 (own current_thread runtime, own thread)
  │     ├── MASH instance
  │     ├── Bus + subscribers
  │     ├── Renderer Host (subscriber)
  │     └── Plugin Host (subscriber)
  ├── Session 2 (own current_thread runtime, own thread)
  │     └── ...
  └── Session N

Shared thread pools:
  ├── WASM pool: max(2, num_cpus / 2) threads
  ├── PTY I/O pool: max(4, num_cpus) threads
  └── Disk I/O pool: 4 threads
```

### Coordinator Responsibilities

- Accept new connections (handshake: Hello/HelloAck)
- Route messages to correct session thread via bounded async channels (default 256)
- Critical/Reliable messages use separate unbounded channel (never dropped)
- Normal/Low messages dropped on channel overflow
- Cross-session routing (e.g., session list queries)
- Session lifecycle (create, destroy, checkpoint)
- Daemon-global concerns (shutdown, health, metrics)

### Per-Session Executor Responsibilities

- Owns its Bus instance
- Runs MASH, processes input, dispatches to subscribers
- Manages pane state via `SessionRuntime` (from `malt-session`)
- Layout recomputation via `malt-layout`
- VT parsing budget: 128 KiB per executor tick, round-robin across compat panes

### Session Lifecycle States

Active → Dormant → Checkpoint → Destroyed (from `malt-session`)

### Latency SLA

Input-to-MASH < 1ms p99 with 10 panes per session.

---

## Client Connection & Input Routing

### Connection Lifecycle

1. Client opens transport (named pipe / Unix socket)
2. Coordinator accepts, spawns connection task
3. Client sends `Hello` → Coordinator responds `HelloAck` (negotiated version, session list)
4. Client sends `AttachSession` or `CreateSession`
5. Coordinator routes to session thread, bidirectional flow begins

### Input Routing (Direct Path, No Bus)

```
Client → InputEvent → Coordinator → Session thread → focus check →
  Shell pane  → MASH
  App pane    → App (via VNP)
  Compat pane → PTY write fd
```

Input bypasses the bus — direct delivery to focused pane for latency.

### Multi-Client Input Authority

- One client holds authority at a time
- Most recent attach gets authority (unless `--observe`)
- `InputClaim` message transfers authority
- On authoritative client detach → next attached (FIFO)
- No clients remaining → session goes Dormant

### Resize Flow (Ordering Guarantee)

1. Client sends `Resize` (Critical priority)
2. Coordinator delivers inline to session
3. Layout engine recomputes `ResolvedPane[]`
4. PTY `ioctl(TIOCSWINSZ)` / ConPTY resize **before** RenderCommand
5. Renderer Host emits delta
6. Clients receive and redraw

### Signal Routing

| Signal | Source | Path |
|---|---|---|
| Ctrl-C / Ctrl-Z | Client KeyEvent | Daemon recognizes → `SignalInput` to MASH |
| SIGCHLD | OS | `malt-platform` catches → notifies MASH/supervisor |
| SIGTERM | OS | Graceful shutdown sequence |
| SIGWINCH | Client Resize | Handled by resize flow above |

---

## Module Structure

```
malt-daemon/
  src/
    lib.rs
    bus/
      mod.rs          — Bus, BusMessage, SubscriberId
      ring_buffer.rs  — bounded ring buffer with priority eviction
      priority.rs     — eviction policy per priority level
      flow_control.rs — per-producer 25% cap, 60% global saturation guard
    executor/
      mod.rs          — SessionExecutor, coordinator loop
      coordinator.rs  — connection accept, session routing, lifecycle
      session_thread.rs — per-session current_thread runtime, tick loop
      pools.rs        — shared thread pool configuration (WASM, PTY, disk)
    connection/
      mod.rs          — ClientConnection, handshake, framing I/O
      transport.rs    — named pipe / Unix socket abstraction (delegates to malt-platform)
      authority.rs    — input authority tracking per session
    error.rs          — DaemonError (thiserror)
```

### Public API

```rust
// Bus
Bus::new(config: BusConfig) -> Bus
Bus::subscribe<T>(buffer_size: usize) -> Subscriber<T>
Bus::publish(msg: BusMessage)  // never blocks

// Daemon
Daemon::new(config: DaemonConfig) -> Result<Daemon>
Daemon::run(self) -> Result<()>  // blocking, runs coordinator
Daemon::shutdown(&self)          // graceful

// Coordinator (internal)
Coordinator::create_session(opts: CreateSession) -> Result<SessionId>
Coordinator::destroy_session(id: SessionId)
Coordinator::route_input(session: SessionId, input: InputEvent)
```

### Dependencies

- `malt-protocol` — all VNP types
- `malt-platform` — sockets, PTY, process, signals
- `malt-session` — SessionRuntime, pane state
- `malt-layout` — layout recomputation
- `malt-config` — DaemonConfig loading
- `tokio` — async runtime (current_thread per session)
- `thiserror`, `tracing`

### Deferred to Later Sub-Projects

- Persistence module (3D)
- Process supervisor module (3E)
- Plugin host module (3E)
- API gateway module (3F)
- Compat subscriber integration (3B)
- Renderer subscriber integration (3C)

---

## Testing Strategy

### Unit Tests

- Bus: publish/subscribe, priority eviction order, flow control (25% producer cap, 60% saturation), Critical inbox bypass, buffer overflow behavior
- Ring buffer: insert, drain, eviction by priority, capacity bounds
- Coordinator: session create/destroy lifecycle, message routing to correct session
- Authority: claim/release, FIFO fallback on detach, observe mode

### Integration Tests

- Full daemon startup → client connect → handshake → create session → send input → receive bus messages → shutdown
- Multi-client: two clients attach, authority transfer, observer mode, detach fallback
- Resize ordering: verify PTY resize completes before RenderCommand emission
- Session isolation: one session's heavy load doesn't starve another (timing test)
- Graceful shutdown: all sessions checkpointed, connections closed cleanly

### Performance Tests (CI-Gated)

- Input-to-MASH latency < 1ms p99 with 10 panes at 100 KiB/s
- Bus throughput: 10 subscribers, sustained publish rate, no producer blocking
