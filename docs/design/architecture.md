# MALT Architecture

> **Status:** Draft v0.1. This document defines the architecture for MALT, a
> structured terminal platform. It is the entry point for all subsequent specs
> and implementation work.

---

## 1. Philosophy & Core Principles

MALT is a structured terminal platform. It inverts the traditional terminal
model: the daemon is the authority, not the renderer. The daemon owns session
state, layout, pane identity, and structured output. Clients are
interchangeable consumers of a typed command stream.

```
Traditional architecture:
  Shell → escape codes → terminal emulator → you see text
  The emulator owns everything. The shell is a byte pipe.

MALT architecture:
  Shell → VNP events → daemon → RenderCommand stream → client
  The daemon owns everything. Clients are display consumers.
```

**The three consequences of this inversion:**

1. Multiple clients can attach to the same session simultaneously
2. The UI authoring model is declarative — apps emit FrameElement trees,
   never escape codes
3. Structured output is a first-class type, not a parsing afterthought

### Core Principles

1. **Coordinator with specialized subsystems** — The daemon core is a
   scheduler, priority manager, and message router. It delegates all
   domain-specific work (rendering, parsing, process supervision, shell
   execution) to independent subsystems. No subsystem accumulates unrelated
   responsibilities. The daemon is a monolith at the process level — all
   subsystems share one address space for performance — but a modular one:
   each subsystem has a clear boundary and can be extracted to a separate
   process if scaling or isolation demands it (see Principle 5).

2. **VNP is the universal contract** — All components communicate through
   Vexil Native Protocol messages — typed, schema-defined, deterministic.
   In-process subsystems use the same message types as remote clients; the
   only difference is whether bytes cross a process boundary. One exception:
   `malt-elevate` uses VNP with a **restricted schema** (`elevate.vexil`) —
   a single, small Vexil file defining the complete helper protocol (~6
   message types). This gives the helper schema validation, versioning, and
   codegen for free while keeping the audit surface to one readable file.
   The helper binary depends only on `malt-platform` + generated types from
   `vexil-runtime` — it does not import the full VNP domain system.

3. **Vexil is the schema authority** — Every VNP message type is a Vexil
   schema. Wire encoding is bitpack (deterministic, LSB-first). JSON exists
   only for debugging. Schema identity via BLAKE3 hash. No hand-written wire
   types, no encoding ambiguity.

4. **The protocol is the product** — Any process that speaks VNP is a valid
   participant. Official clients define the quality bar, but community
   clients, IDE plugins, mobile apps, and automation tools consume the same
   protocol. Extensibility comes from the wire format being public and
   documented, not from internal APIs.

5. **Subsystems are designed for extractability** — Every subsystem
   communicates through VNP message types, making out-of-process deployment
   structurally possible. However, in-process subsystems currently benefit
   from zero-copy message passing and shared heap references. Extracting a
   subsystem to a separate process requires serializing all bus messages it
   consumes, adding a transport layer, and handling connection failures — work
   that is straightforward but non-trivial. The Compat Translator is the
   **first validated extraction**: it runs out-of-process for Restricted+
   sessions (see §10), proving the pattern. The API Gateway is designed as
   the next extraction candidate. Other subsystems remain in-process until
   profiling or isolation requirements justify the overhead.

---

## 2. System Architecture

The daemon core runs a central message bus. Subsystems subscribe to message
types they handle. The daemon dispatches messages based on type, routing
metadata, and priority class. Subsystems are independent — they don't know
about each other, they know about message types.

**Input routing is focus-aware.** The daemon core tracks which pane is focused
and routes InputEvent directly to that pane's owner — MASH for shell panes,
the App process via VNP for native app panes, or the PTY write fd for legacy
panes (the Compat Translator handles only the output direction — reading PTY
output and emitting VNP messages). This is the latency-critical hot path — no
bus intermediary. Output from all pane types flows through the bus normally.

```
Client → InputEvent → daemon core ──focus check──→ MASH (shell pane)
                                         │         App (app pane, via VNP)
                                         │         PTY write fd (legacy pane)
                                         │
                                    OutputChunk, CommandStarted, PromptReady
                                         │
                                         ▼
                                   message bus
                       ┌──────────┬──────────┼──────────┬──────────┐
                       ▼          ▼          ▼          ▼          ▼
                    Parser   Renderer    Session    Plugin     API GW
                    (SO)     Host        Store      Host
```

### Multi-Client Input Authority

When multiple clients attach to the same session, one client at a time holds
**input authority** — its keystrokes reach the focused pane. Other attached
clients are observers: they receive RenderCommand streams (seeing the same
output) but their input is silently discarded.

- **Default:** The most recently attached client gets input authority.
- **Transfer:** `malt attach --observe` attaches without claiming authority.
  `malt input-claim` transfers authority to the requesting client.
- **Authority release:** When the authoritative client detaches, authority
  transfers to the next attached client (by attachment order). If no clients
  remain, the session enters Dormant state.
- **Cooperative sharing:** For pair programming or demos, a future extension
  can allow explicit authority passing between clients. The architecture
  supports this — the daemon core holds the authority token and can reassign
  it on any bus message.

The API Gateway respects input authority: `POST /exec` and `POST /send`
always have authority on the session they target (agents are not observers).

### Subsystems

| Subsystem | Responsibility | Process Model |
|-----------|---------------|---------------|
| **MASH** | POSIX shell, line editor, prompt state, command lifecycle, FrameElement construction. Spawns and reads output for shell commands. Direct input path from daemon core. Emits FrameElement trees directly to the bus — no intermediate Shell Presenter. | In-process, privileged |
| **Renderer Host** | FrameElement → RenderCommand translation, dirty tracking, capability degradation, theme resolution | In-process |
| **Structured Output Parser** | Recognizes command output, dispatches to parsers (built-in + WASM plugins), produces StructuredOutput. Fully async — drains OutputChunk from its ring buffer, never blocks the bus. WASM parsers run on a budget (50ms) and are killed on timeout. | In-process |
| **Process Supervisor** | Spawns/monitors/restarts **app and compat** processes (not shell commands — MASH owns those). Applies isolation tier at spawn time via `malt-platform`. Resource tracking, cleanup. | In-process |
| **Compat Translator** | PTY output → VNP messages (output direction only). Only component that touches VT escape codes. Input to legacy PTYs is written directly by daemon core. | In-process (Bare sessions) / out-of-process via `malt-compat-worker` (Restricted+ sessions) |
| **Layout Engine** | Resolves abstract LayoutNode tree → concrete ResolvedPane positions. Pure computation. | In-process |
| **Plugin Host** | WASM runtime, sandbox, permission enforcement, plugin lifecycle | In-process |
| **API Gateway** | HTTP/MCP external surface for agents, CI, scripts, automation | In-process, extractable |
| **Session Store** | Persistence, session lifecycle, state serialization, isolation tier tracking, session group policy enforcement | In-process |

### Priority Classes

| Priority | Message types | Drop policy | Rationale |
|----------|--------------|-------------|-----------|
| Critical | Resize, Signal | Never dropped (inline delivery) | Immediate system response |
| Reliable | CommandStarted/Finished, StructuredOutput | Never evicted from ring buffer (see flow control below) | State machine integrity — parsers and session store depend on seeing every lifecycle event |
| High | RenderCommand, FrameElement | Oldest overwritten | Visual responsiveness |
| Normal | OutputChunk | Oldest dropped per subscriber | Data flow — inherently streaming, loss tolerable |
| Low | SessionPersist, PluginEvent, Telemetry | Oldest dropped per subscriber | Background work |

Note: InputEvent is not on the bus — it flows directly to the focused pane's
owner (MASH, App, or Compat Translator).

### Message Flow Example — User Types `cargo build`

```
1. Client sends InputEvent → daemon core → MASH (direct)
2. MASH processes keystrokes via line editor
3. User hits Enter → MASH emits CommandStarted{cmd: "cargo build"} → bus
4. MASH spawns cargo via platform traits (isolation applied at spawn time)
5. MASH reads cargo's PTY output, emits OutputChunk → bus
6. Structured Output Parser picks up OutputChunk, recognizes cargo format
7. Parser emits StructuredOutput{kind: CargoBuild} → bus
8. MASH builds FrameElement tree from output + structured output, emits to bus
9. Renderer Host receives FrameElement tree, walks it → emits RenderCommand delta
10. Clients rasterize and display
```

Note: MASH spawns and reads output for shell commands (pipelines, builtins,
scripts). The Process Supervisor handles app and compat process lifecycle —
it does not manage MASH's child processes. This keeps the shell's process
model (job control, pipelines, subshells) self-contained.

### Bus Flow Control

Each subscriber has a single bounded ring buffer. All priorities share the
buffer, preserving insertion order. The priority determines eviction policy,
not delivery path:

- **Critical (Resize, Signal)** — delivered via a separate per-subscriber
  inbox, not the ring buffer. The daemon dispatches Critical messages by
  posting to each subscriber's inbox asynchronously (non-blocking). Subscribers
  poll their Critical inbox before draining the ring buffer. Critical messages
  are rare and small — the inbox is a small unbounded queue.
- **Reliable (CommandStarted/Finished, StructuredOutput)** — inserted into
  the ring buffer like any other message, but marked never-evict. When the
  buffer is full and a new message arrives, the oldest evictable message
  (Normal or Low) is removed to make room. If only Reliable messages remain
  (buffer entirely full of never-evict entries), new Normal/Low messages are
  dropped. **Reliable messages never block the producer** — this avoids
  deadlock in the in-process architecture where producer and consumer share
  the same thread pool. Ordering between Reliable and Normal messages is
  naturally preserved because they share the same buffer.
  **Reliable saturation guard (two-level):** (a) Per-producer cap: each
  producer may hold at most 25% of buffer capacity as Reliable entries. If
  exceeded, the oldest Reliable message from that producer is demoted to
  Normal (becoming evictable). (b) Global cap: total Reliable entries across
  all producers may not exceed 60% of buffer capacity. When the global cap
  is reached, the oldest Reliable entry regardless of producer is demoted.
  Without the global cap, 4 producers each at 25% could collectively fill
  100% of the buffer with never-evict entries, starving all Normal/Low
  delivery. The global cap ensures this cannot happen. The bus
  tracks `reliable_pressure` per subscriber — ratio of Reliable entries to
  total capacity. Diagnostic emitted at 50%.
- **High (RenderCommand, FrameElement)** — evictable, but only by newer High
  messages. Renderer Host can always reconstruct from the latest FrameElement
  tree.
- **Normal (OutputChunk) / Low (SessionPersist, PluginEvent, Telemetry)** —
  first to be evicted when the buffer is full. Dropped message count is
  tracked per subscriber for diagnostics.

**The bus never blocks producers.** No priority level causes producer blocking.
This is a hard constraint of the in-process architecture — blocking an
in-process producer risks deadlock when producer and consumer share threads.

Buffer sizes are configurable per subscriber class. Defaults:

| Subscriber | Default Buffer Size | Rationale |
|------------|-------------------|-----------|
| Renderer Host | 4,096 entries | High-frequency FrameElement updates; large to absorb bursts |
| Structured Output Parser | 2,048 entries | Trails behind OutputChunk; similar volume |
| Session Store | 512 entries | Low-frequency lifecycle events only |
| Plugin Host | 1,024 entries | Medium; plugins have varied consumption rates |
| API Gateway | 2,048 entries | Shadows FrameElement tree; needs burst tolerance |

Memory footprint: with 10 sessions × 6 subscribers × ~2K entries × ~256
bytes/entry average ≈ 30 MiB. Acceptable for a daemon process.

**Session routing:** Every bus message carries a SessionId in its envelope
metadata. The bus routes messages to subscribers scoped to that session. A
high-output session cannot starve subscribers in other sessions — each
session's message flow is independent.

### Client Backpressure & Sync Protocol

**Problem:** If a client falls behind (slow network, GPU stall, backgrounded
browser tab), RenderCommands accumulate. The bus ring buffer evicts old High
messages, but the Renderer Host keeps producing — without backpressure, the
daemon does wasted work generating RenderCommands that will be evicted before
the client sees them.

**Frame sequencing:** Every RenderCommand batch (one per frame tick) carries a
monotonic `frame_seq` counter. Clients acknowledge the last processed frame
via `FrameAck { frame_seq }` messages.

**Slow client detection:** If a client's unacknowledged frame count exceeds a
threshold (default: 30 frames, ~500ms at 60fps), the Renderer Host stops
producing RenderCommands for that client. The client is marked "lagging." When
the client catches up (FrameAck within 5 of the current frame), production
resumes.

**Slow client shedding:** If a client has not sent a FrameAck for 10 seconds,
the daemon disconnects it with a `SlowClientDisconnect` message. The client
can reconnect and re-sync.

**Attach sync protocol:** When a client attaches to an active session:

1. Daemon snapshots the current full FrameElement tree + resolved layout as
   an atomic `InitialState` message (includes `frame_seq` baseline)
2. Daemon begins sending RenderCommand deltas starting from `frame_seq + 1`
3. No gap exists between snapshot and delta stream — the snapshot is taken
   under the executor's single-threaded guarantee, so no FrameElement updates
   can occur between snapshot and first delta
4. Client can request a full re-sync at any time via `SyncRequest` — the
   daemon responds with a fresh `InitialState` snapshot and resets the delta
   stream. This is the recovery path after a slow-client disconnect.

**API Gateway:** The shadow FrameElement tree is always up-to-date (the
gateway is an in-process subscriber, not a remote client). HTTP/MCP consumers
poll the shadow tree directly — no frame sequencing needed.

### Terminal Resize Flow

Resize must be sequenced carefully to avoid visual corruption in legacy
programs. The end-to-end flow:

```
1. Client detects window resize → sends Resize{cols, rows} to daemon
2. Daemon core delivers Resize as Critical priority (inline, no queue)
3. Layout Engine recomputes ResolvedPane[] with new dimensions → bus
4. For each PTY-backed pane: daemon calls ioctl(TIOCSWINSZ) / ConPTY resize
   BEFORE emitting any RenderCommands at new dimensions
5. Renderer Host receives new ResolvedPane[], produces RenderCommand delta
6. Clients receive RenderCommands and redraw at new dimensions
```

**Ordering guarantee:** PTY resize (step 4) happens before RenderCommand
emission (step 5). This ensures legacy programs see SIGWINCH and can redraw
before the client renders the new frame. The daemon enforces this ordering
— it is not a convention, it is a sequencing contract.

### Signal Routing

Signals are delivered through VNP messages, not raw OS signals. The daemon
translates platform-specific signal mechanisms into typed SignalInput messages.

| Signal | Source | Path |
|--------|--------|------|
| SIGINT (Ctrl-C) | Client sends KeyEvent → daemon recognizes Ctrl-C | Daemon delivers SignalInput{SIGINT} to MASH → MASH forwards to foreground job via platform traits |
| SIGWINCH | Client sends Resize message | Handled by resize flow above, not MASH |
| SIGCHLD | OS delivers to daemon process | `malt-platform` catches, notifies MASH (for shell children) or Process Supervisor (for app/compat children) |
| SIGTERM | OS delivers to daemon process | Daemon initiates graceful shutdown sequence |
| SIGTSTP (Ctrl-Z) | Client sends KeyEvent → daemon recognizes Ctrl-Z | Daemon delivers SignalInput{SIGTSTP} to MASH → MASH suspends foreground job |

**Windows equivalents:** Ctrl-C → `GenerateConsoleCtrlEvent(CTRL_C_EVENT)`.
Ctrl-Break → `CTRL_BREAK_EVENT`. No SIGTSTP equivalent — job suspension uses
Job Object `JOBOBJECT_MSG_ACTIVE_PROCESS_ZERO` + `TerminateJobObject` for
group control, or `NtSuspendProcess` for individual process suspension.
`SuspendThread` is avoided for individual thread suspension — it can deadlock
if the target thread holds a heap lock, loader lock, or critical section.
`NtSuspendProcess` suspends all threads atomically (preferred for whole-
process suspension), but note that the deadlock risk is not eliminated — it
is deferred to resume/interaction time. If the daemon needs to allocate
memory in a suspended process or inspect its state, threads holding heap or
loader locks will deadlock. The daemon should treat a suspended process as
opaque until resume. `SIGCHLD` equivalent uses job object completion port
notifications.

**Isolation context:** Signals to processes in PID namespaces are delivered
by `malt-platform`, which understands the namespace mapping. The daemon never
sends raw `kill(2)` — it calls platform traits that handle namespace-aware
signal delivery.

### Concurrency Model

The daemon uses a **session-sharded executor model**: each session runs on
its own dedicated `current_thread` tokio runtime thread. The daemon core
runs on a separate coordinator thread that routes messages between sessions
and manages global state.

```
┌─────────────────────────────────────────────────────┐
│  Daemon Core Thread (coordinator)                    │
│  - Client connection management                      │
│  - SessionId → session executor routing              │
│  - Global state: session store, group policies       │
│  - Cross-session messages (System domain)            │
└──────┬──────────┬──────────┬──────────┬─────────────┘
       │ channel  │ channel  │ channel  │
  ┌────▼────┐┌────▼────┐┌────▼────┐┌────▼────┐
  │Session 1││Session 2││Session 3││Session N│
  │executor ││executor ││executor ││executor │  (one thread each)
  │ MASH    ││ MASH    ││ MASH    ││ MASH    │
  │ bus     ││ bus     ││ bus     ││ bus     │
  │ subs    ││ subs    ││ subs    ││ subs    │
  └─────────┘└─────────┘└─────────┘└─────────┘
```

**Why session-sharded, not single-threaded:** A single-threaded executor
creates a scaling cliff — VT parsing, WASM plugin budgets, and heavy output
in one session stall input delivery in all other sessions. Per-session threads
eliminate cross-session interference by construction. Within a session,
subsystems are still single-threaded (no data races, `!Send` state is fine).
The sharding is done from day one because retrofitting it later requires
redesigning the bus, subsystem traits, and state management — a cost that
grows with every line of code built on the single-threaded assumption.

**Thread affinity:**

- **Daemon core (coordinator):** its own `current_thread` runtime. Handles
  client connections, session creation/destruction, cross-session routing,
  and global state. Lightweight — no heavy computation.
- **Per-session executor:** its own `current_thread` runtime. Runs all of
  that session's subsystems: MASH, Renderer Host, Plugin
  Host, Structured Output Parser, Compat Translator instances. Each session's
  subsystems are cooperative async tasks on that session's thread.
- **WASM plugin execution:** runs on a **dedicated WASM thread pool** (shared
  across sessions, capped at `max(2, num_cpus / 2)` threads). The Plugin Host
  dispatches WASM invocations to this pool and receives results via oneshot
  channels back to the session executor. Fuel-based budgeting (wasmtime fuel)
  still applies — budget expiry kills the invocation. A plugin consuming its
  full 50ms budget stalls only the WASM thread, not the session executor —
  input processing, rendering, and bus dispatch continue uninterrupted. This
  eliminates the tradeoff where a technically-compliant plugin (49ms budget
  use) degrades the session's frame rate.
- **Blocking I/O:** dispatched to two separate thread pools via
  `spawn_blocking`: one for PTY reads (capped at `max(4, num_cpus)` threads),
  one for disk I/O (capped at 4 threads). Separate pools prevent PTY-heavy
  workloads from starving session persistence writes.
- **Compat Translator VT parsing:** runs on the session's executor thread.
  A global VT parsing budget caps total VT bytes processed per executor tick
  across all compat panes in that session (default: 128 KiB/tick, distributed
  round-robin). This bounds worst-case executor stall from multiple heavy-
  output compat panes. Individual pane budgets are derived from the global
  budget divided by active compat pane count.

**Latency SLA:** Input-to-MASH delivery < 1ms p99 with 10 active panes
per session. This is a design constraint, not a tuning target. The session-
sharded model makes this achievable: input routing touches only the target
session's executor thread, which handles at most that session's subsystems.
Load profile ceiling: 20 sessions × 10 panes × 100 KiB/s output per pane.

**Coordinator ↔ Session communication:** The coordinator and session
executors communicate via bounded async channels (tokio `mpsc`):

- **Coordinator → session:** InputEvent routing (direct path for latency),
  Resize, Signal, cross-session messages, session lifecycle commands
- **Session → coordinator:** RenderCommand batches (for client delivery),
  session state updates, diagnostic events

The coordinator never accesses session-internal state directly. Channel
capacity is bounded (default: 256 messages) — if a session's inbound channel
is full, the coordinator drops Normal/Low messages (same eviction policy as
the ring buffer). Critical and Reliable messages use a separate unbounded
channel to guarantee delivery.

**Per-session bus dispatch:** Within a session, the bus is a synchronous
in-process dispatcher on that session's executor thread — identical to the
ring buffer design described in Bus Flow Control. The "bus never blocks
producers" invariant holds within each session. Cross-session messages are
routed through the coordinator's channels.

**OutputChunk batching:** The per-session bus batches OutputChunks using
adaptive flushing: flush immediately when fewer than 8 chunks are pending
(interactive responsiveness); batch and flush at tick boundary (default:
16ms) when throughput exceeds the threshold (high-output programs). This
avoids the fixed 16ms latency penalty for interactive programs (editors,
REPLs) while still amortizing dispatch cost under load.

**Ordering guarantees:**

- **Within a session:** identical to single-threaded — all subsystems on the
  same thread, FIFO ring buffer, no interleaving within `.await` boundaries.
  InputEvent and bus messages are processed in arrival order at poll points.
- **Cross-session causality:** not guaranteed. Sessions are independent
  threads. A message emitted in session A and routed to session B via the
  coordinator has channel latency. This is acceptable — sessions are
  logically independent; cross-session ordering is not a correctness
  requirement.
- **Client delivery order:** The coordinator serializes RenderCommand batches
  per client. A client attached to multiple sessions sees frames in the order
  the coordinator receives them — no interleaving within a single frame batch.

**Subsystem trait requirements:** Per-session subsystem state is `!Send` and
`!Sync` — enforced by the `current_thread` runtime per session. Cross-session
message types (`OutputChunk`, `RenderCommand`, etc.) are `Clone + Send`
because they cross thread boundaries via channels. Global state in the
coordinator (session store metadata, group policies) is behind `Arc<Mutex>`
or partitioned by SessionId.

### Error Handling

Subsystem errors follow a consistent pattern:

- **Recoverable errors** — the subsystem emits a diagnostic message
  (System domain, Error type) to the bus and continues. Other subsystems
  can subscribe to error events for monitoring.
- **Fatal subsystem errors** — the subsystem notifies the daemon core, which
  logs the failure, attempts restart (for restartable subsystems like Plugin
  Host), and notifies connected clients.
- **Message processing errors** — if a subsystem cannot process a message
  (malformed payload, unknown type), it drops the message and emits a
  diagnostic. No error propagation back to the producer.

The Plugin Host has additional error handling: WASM plugin crashes are
isolated per-plugin (the host survives), budget violations are immediate
kills, and manifest validation failures prevent loading.

---

## 3. Rendering Pipeline

The rendering pipeline has three stages, owned by three different layers.

### Stage 1 — Authoring (content sources)

Content sources produce FrameElement trees. These are semantic UI descriptions
— "a table with these columns", "a diff with these hunks", "styled text".
Content sources include:

- MASH (emits FrameElement trees directly — shell output, prompt, structured
  output are all composed into FrameElements within MASH itself, eliminating
  the extra bus hop of a separate Shell Presenter subsystem)
- Apps (native processes implementing the App trait)
- Compat Translator (VtPassthrough elements for legacy programs)
- Plugins (status widgets, prompt segments)

### Stage 2 — Translation (Renderer Host subsystem)

The Renderer Host receives FrameElement trees and resolved layout positions
from the daemon bus. It:

1. Walks the FrameElement tree
2. Checks client capabilities (color support, unicode, overlays, images)
3. Degrades gracefully for unsupported elements
4. Resolves theme tokens to concrete RGB values
5. Diffs against previous frame state (dirty tracking)
6. Emits a minimal RenderCommand delta

All style resolution, capability adaptation, and diffing happens here. Clients
never see FrameElement trees or theme tokens.

### Stage 3 — Rasterization (clients)

Clients receive RenderCommand streams and draw. Each client maps commands to
its native rendering surface:

- GPU terminal (maltty): wgpu draw calls, glyph atlas, GPU text shaping
- TUI client: ratatui widgets, crossterm
- Browser client: HTML/CSS/canvas DOM manipulation

Clients are deliberately simple — they execute concrete drawing instructions.
No layout math, no style resolution, no diffing.

### The Wire Boundary

```
FrameElement (internal, unstable)  →  Renderer Host  →  RenderCommand (wire protocol, versioned, stable)
```

FrameElement is an internal type that can evolve freely. RenderCommand is the
public protocol — versioned, documented, the contract that community clients
implement. Internal rendering improvements never break external clients.

### Official Clients

| Client | Tech | Role |
|--------|------|------|
| **maltty** | wgpu + cosmic-text + winit | Flagship. GPU-accelerated. Defines the visual identity. |
| **malt-tui** | ratatui + crossterm | Universal. Works over SSH, CI, low-resource environments. |
| **malt-web** | SvelteKit, served by daemon | Zero-install. Browser access via Tailscale/localhost. |

---

## 4. Layout Engine

The layout engine is a pure computation subsystem. It takes an abstract
LayoutNode tree and terminal dimensions, and produces concrete ResolvedPane
positions. No side effects, no state beyond the tree itself.

**N-ary tree, not binary.** Each split node holds multiple children with
proportional, fixed, or constrained sizes. Resize operations are local —
changing one child redistributes among siblings at the same level, no
tree-recursive ratio propagation.

### Node Types

- **Split** — N children arranged horizontally or vertically, each with a
  SplitSize (Ratio, Fixed, Min, Max)
- **Tabbed** — N panes sharing the same screen area, one active at a time
- **Float** — A subtree anchored relative to a position, with z-ordering

Every non-leaf node has a stable SplitId for resize and structural operations.
PaneId addresses content, SplitId addresses structure — orthogonal identities.

### Resolution Output

```
ResolvedPane {
    pane_id:      PaneId
    x, y:         u16          // absolute position
    width, height: u16         // concrete dimensions
    focused:      bool
    visible:      bool         // false for inactive tab members
    z_order:      u32
    tab_context:  Option<TabContext>
}
```

**Directional focus** uses geometric distance on resolved positions, not tree
traversal. Focus navigation operates within **focus layers**: tiled panes form
the base layer, floating panes form overlay layers (one per z-level). Arrow
key navigation stays within the current layer. A floating pane captures focus
when shown; Escape or an explicit dismiss action returns focus to the base
layer. Mouse click targets the topmost pane at the click position regardless
of layer. This prevents floating modals (completion menus, dialogs) from
intercepting directional navigation meant for the tiled layout beneath.

**Layout operations** are messages on the bus: SplitPane, ClosePane,
FloatPane, SwapPanes, ResizeSplit, FocusDirection, SaveLayout, LoadLayout,
etc. The daemon core routes them to the layout engine, which recomputes and
emits updated ResolvedPane[] back to the bus. Renderer Host subscribes and
redraws.

**Persistence:** LayoutNode trees are serializable. Named layout presets
stored as .vx files. Sessions persist their layout tree — pixel positions are
never serialized, only the abstract structure.

**Layout is optional for consumers.** The layout engine serves full terminal
clients. Single-pane embeddings (IDE plugins, automation), headless consumers
(agents, CI), and custom layout implementations can subscribe to per-pane
messages directly, bypassing layout entirely. The protocol imposes no coupling
between layout participation and content consumption.

---

## 5. MASH — The Shell

MASH is a POSIX-conformant shell and the primary user interaction surface. It
runs as a privileged in-process subsystem with a direct input path from the
daemon core.

### What MASH Owns

- POSIX sh execution (lexer, parser, expander, executor, builtins)
- Line editor (completion, highlighting, history, multiline)
- Prompt state and rendering
- Command lifecycle events (CommandStarted, CommandFinished, PromptReady)
- Shell child process spawning and PTY reading (pipelines, subshells, builtins)
- Output emission (OutputChunk messages to the bus — each OutputChunk carries
  the originating command string in its metadata, tagged by MASH at emission
  time. This enables the Plugin Host to filter OutputChunks by command pattern
  without stateful correlation with CommandStarted events.)
- **FrameElement construction** — MASH composes shell output, prompt state,
  and structured output (received from the Structured Output Parser via the
  bus) into FrameElement trees and emits them directly. This eliminates the
  extra bus hop of a separate Shell Presenter subsystem. MASH is already a
  MALT-specific component, not a generic POSIX shell — it knows about
  FrameElements by design.

### What MASH Does NOT Own

- Output parsing — the Structured Output Parser subsystem handles recognition
  and classification; MASH consumes the results
- App/compat process spawning — the Process Supervisor handles this

### Privileged Status Means

- Direct input path, no bus latency on keystrokes
- Daemon core understands prompt state (can answer "is the shell idle?"
  without a bus round-trip)
- In-process, zero-copy message passing

### POSIX Conformance Scope

MASH targets **POSIX.1-2017 Shell Command Language** (IEEE 1003.1-2017,
§2). Conformance is validated against two test suites:

- **Smoosh test corpus** (186 tests) — primary CI gate, covers core POSIX sh
  semantics (variable expansion, pipelines, redirections, traps, subshells,
  arithmetic, here-documents, case statements, parameter expansion)
- **Modernish diagnostic suite** — secondary validation for implementation-
  defined behaviors and edge cases

**In scope for v1.0:** All POSIX.1-2017 §2 mandatory features. This includes:
field splitting, pathname expansion, tilde expansion, parameter expansion
(all forms), command substitution, arithmetic expansion, here-documents,
pipelines, lists (AND-OR, sequential, async), compound commands (if, for,
while, until, case, subshell, brace group), function definitions, redirections
(all forms), special builtins, and trap handling.

**Implementation-defined behaviors:** Where POSIX leaves behavior
implementation-defined, MASH follows dash precedent unless there is a
compelling reason to diverge. Divergences are documented in the codebase.

**Explicitly deferred:** Process substitution (`<()`), arrays, associative
arrays, extended globbing (`extglob`), `[[` test syntax, `select` loops.
These are bash/zsh extensions, not POSIX. They may be added as MASH
extensions post-v1.0 but will never change POSIX-mandated behavior.

### MASH Extensions Beyond POSIX

MASH is a conformant POSIX shell first. Extensions are additive and don't
break POSIX semantics:

- Structured output annotations (commands can tag their output type)
- Session-aware builtins (pane management, layout control)
- Rich prompt protocol (prompt segments from plugins via the bus)

### Shell ↔ Daemon Boundary

MASH depends on platform abstractions (PTY, process, signals, filesystem)
through traits, not on the daemon directly. This preserves extractability —
the trait boundary is the seam if MASH is ever published standalone.

### MASH Crash Isolation

MASH processes arbitrary user-provided code paths (expansion, arithmetic,
traps). A panic or corruption in MASH must not take down other sessions.

**The session-sharded executor limits blast radius by construction.** A MASH
crash (panic, stack overflow, infinite loop) kills only that session's
executor thread. Other sessions continue unaffected on their own threads.
The coordinator detects the dead session thread and can restart it (restoring
from persisted state) or mark the session as failed.

**Within a session, defense is layered:**

1. **`catch_unwind` boundaries:** Every MASH poll point (keystroke processing,
   command execution, expansion) is wrapped in `catch_unwind`. A panic in MASH
   kills that shell pane, not the session. The pane displays
   `[shell crashed — press Enter to restart]` and a fresh MASH instance is
   spawned in the same cwd.

2. **`catch_unwind` limitations (acknowledged):** Stack overflows, abort-on-
   panic, double panics, and `extern "C"` boundary crossings bypass
   `catch_unwind`. A panic during a `.await` point may leave the session
   executor's internal state (wakers, task queues) inconsistent. Mitigation:
   the coordinator runs a **per-session watchdog** — if a session executor
   stops responding to heartbeat pings for 500ms, the coordinator assumes
   executor corruption, kills the thread, persists what it can from the last
   known state, and offers to restart the session.

3. **Recursion/stack limits:** Alias expansion, arithmetic evaluation, and
   nested subshells have explicit depth limits (configurable, default: 1024
   for alias/expansion, 256 for subshells). Stack overflow from recursive
   constructs is prevented by counting, not by relying on OS stack guards.

4. **System-service mode: MASH runs out-of-process.** This is a **hard
   requirement**, not a mitigation. In system-service deployment (multi-user),
   each session's MASH instances run as separate processes communicating via
   VNP over Unix socket or named pipe. A MASH crash cannot corrupt the session
   executor's state. The per-user worker model provides cross-user isolation;
   out-of-process MASH provides within-user, within-session isolation. The
   latency cost of IPC is measured during Phase 2 — if it exceeds the 1ms
   SLA, the input path uses a dedicated Unix socket with `MSG_DONTWAIT` to
   minimize overhead.

5. **Fuzz testing:** `cargo-fuzz` targets MASH expansion, arithmetic, and
   trap handling paths **in the context of a session executor** (not
   standalone). Post-`catch_unwind`, the harness verifies the executor
   continues processing other panes' messages correctly. Executor corruption
   after `catch_unwind` is a release blocker.

**Infinite loops:** MASH execution is cooperative on the session executor. A
`while true; do :; done` loop yields at each iteration's command dispatch
point. The session executor detects a non-yielding MASH instance (no yield
for 100ms) and force-cancels it.

### Shell and Isolation

MASH itself runs in the daemon process — it is never sandboxed. The isolation
tier applies to the child processes that MASH spawns (commands, pipelines,
subshells). MASH runs as multiple instances (one per shell pane), and each
instance is bound to a session with its own isolation tier. The daemon injects
an `IsolationContext` token into each MASH instance at creation time. When
MASH calls `malt-platform` spawn traits, it passes this token — the platform
layer reads the token and applies the appropriate sandbox. MASH never
interprets the token or contains isolation logic; it is an opaque handle that
flows from daemon → MASH → platform. This keeps MASH clean of isolation
logic while ensuring all user commands execute within the session's security
boundary.

---

## 6. VNP — Vexil Native Protocol

VNP is the typed message protocol that replaces escape codes. Every
inter-component message is a Vexil schema, encoded with bitpack, framed for
transport.

### Three Layers

1. **Framing** — 4-byte LE length prefix + 1-byte flags + payload. Max 16 MiB
   per frame (configurable per transport — e.g., 64 KiB for named pipes,
   1 MiB for TCP; the 16 MiB limit is the protocol maximum, not the default).
   Flags encode: compression (zstd), encoding format (bitpack/JSON), priority
   bit. **Chunking:** When a logical message exceeds
   the frame size limit (e.g., a large OutputChunk from a command dumping
   binary data), the sender splits it into multiple frames with a
   continuation flag in the flags byte. The receiver reassembles before
   dispatching. Producers that generate unbounded output (e.g., `cat
   /dev/urandom`) are chunked at the frame boundary — no single command can
   produce a message larger than memory. Individual OutputChunk message bodies
   are capped at the per-transport frame size (not the 16 MiB protocol max)
   — MASH emits OutputChunks at PTY read boundaries, so no single OutputChunk
   approaches 16 MiB in practice.

2. **Envelope** — Bit-packed header (12-20 bytes): protocol version (4 bits),
   domain (4 bits, 16 domains — 8 defined, 8 reserved for future use),
   message type (7 bits, per-domain), session
   ID (32-bit, for bus routing), timestamp (48-bit microsecond, epoch =
   daemon start time, wraps at ~8.9 years — acceptable for session lifetime;
   long-running system services should document that timestamp comparison
   across daemon restarts is invalid; diagnostic output converts to wall-clock
   by adding the daemon's start-time offset from HelloAck for external log
   correlation), optional message ID (32-bit). The
   7-bit type field is scoped per domain — each of the 16 domains has its own
   128-type namespace, giving 2048 total message types (8 domains defined, 8
   reserved). Session ID 0 =
   daemon-global messages (not scoped to a session). 32-bit SessionId is
   monotonically increasing and never recycled within a daemon lifetime.
   At 1 million sessions/day, the space exhausts in ~11.7 years. Daemon
   restart resets the counter (acceptable — persistent sessions are identified
   by their persisted file, not in-memory ID). Stale references from
   disconnected API clients return `SessionNotFound` — no risk of accidentally
   targeting a recycled session.

3. **Payload** — The Vexil-schema-defined message body. Bitpack-encoded. Every
   message type has a corresponding `.vexil` schema file — codegen produces
   Rust types, encode/decode, and documentation.

### Message Domains

| Domain | Scope |
|--------|-------|
| Handshake | Connection setup, version negotiation, capability exchange |
| Shell | CommandStarted, CommandFinished, PromptReady, OutputChunk |
| Input | KeyEvent, MouseEvent, SignalInput, Resize |
| Mux | PaneCreated, PaneDestroyed, LayoutChanged |
| Session | CreateSession, AttachSession, DetachSession, ListSessions |
| Task | Task lifecycle (create, status, complete) |
| Render | FrameElement, RenderCommand, RenderBatch |
| System | Plugin events, structured output, theme, telemetry |

### Connection Lifecycle

1. Client opens transport (named pipe, Unix socket, TCP, WebSocket)
2. Client sends Hello (protocol version, client type, capabilities struct)
3. Daemon responds HelloAck (negotiated version, session list, daemon
   capabilities)
4. Client sends AttachSession or CreateSession
5. Bidirectional message flow begins

**Capability exchange:** The Hello message includes a structured capability
declaration (Vexil schema):

```
ClientCapabilities {
    color_depth:    None | Basic256 | TrueColor
    unicode:        None | Basic | Full
    image_protocol: None | Sixel | KittyGraphics | ItermInline
    overlay:        bool        // supports PushLayer/PopLayer
    vt_passthrough: bool        // can render WriteRaw
    max_fps:        u16         // client's preferred frame rate cap
}
```

The Renderer Host uses these negotiated capabilities to produce a per-client
RenderCommand stream. Clients that don't support images never receive
DrawImage commands — the Renderer Host substitutes a text fallback.
Capabilities are per-connection, not per-session, so two clients attached to
the same session can have different rendering fidelity.

### Version Negotiation

Both sides declare max supported wire version. They operate at the lower of
the two. Wire versions are additive — v2 is a superset of v1.

**Bitpack forward compatibility:** New fields are always appended at the end
of the message schema. Bitpack reads fields in declaration order; a decoder
at wire version N reads the first N fields and ignores trailing bytes it
doesn't recognize. This preserves the "same fields → bit-identical prefix"
property: the shared field prefix between v1 and v2 encodes identically.
New fields have default values that preserve v1 semantics when absent.

**Formal schema evolution rules:**

| Operation | Compatibility | Mechanism |
|-----------|--------------|-----------|
| Append new field to message | Forward + backward compatible | Decoder ignores trailing bytes; encoder uses default for missing fields |
| Add variant to union/enum type | Forward compatible only | Old decoders see unknown variant → use declared fallback or reject |
| Change field from required to optional | Breaking | Wire version bump required |
| Remove or reorder existing fields | Breaking | Wire version bump required |
| Expand a bit-width field (e.g., 3-bit domain → 4-bit) | Breaking | Wire version bump; envelope layout changes |
| Add new domain to domain field | Breaking (field too narrow) | Use reserved domain values or bump wire version |

These rules are formally specified in the Vexil encoding spec (Phase 0
deliverable) with golden-byte test vectors for each evolution scenario.

**Schema hash mismatch at runtime:** If a received message's BLAKE3 schema
hash doesn't match the expected hash for that message type at the negotiated
wire version, the message is rejected with a diagnostic error (not silently
dropped). On sustained mismatch (>10 rejected messages in 5 seconds), the
daemon proactively disconnects the client with a typed `VersionSkew` error
that includes the expected version range, enabling client-side "please
update" UX. **Compatibility window:** daemon version N must support clients
at version N and N-1. Clients at version N-2 or older are disconnected with
a `VersionSkew` error immediately after Hello/HelloAck.

### Transport Agnostic

VNP defines framing and messages, not transport. Any reliable ordered byte
stream works:

- Named pipes (Windows)
- Unix sockets (Linux/macOS)
- TCP with TLS (remote/Tailscale — TLS is mandatory for TCP transport; VNP
  payloads contain keystrokes and command output, which must not be exposed
  to network observers. Self-signed certs with TOFU pinning for personal use;
  CA-signed for shared infrastructure.)
- WebSocket with TLS (browser — same TLS requirement as TCP)
- In-process channel (embedded subsystems — no serialization, messages passed
  by reference)

---

## 7. Rendering Types

Two key types define the rendering contract. FrameElement is internal — the
semantic UI description that content sources produce and Renderer Host
consumes. RenderCommand is the public wire protocol that clients implement.

### FrameElement (internal, unstable)

Semantic UI primitives that describe *what* to display, not *how*:

- **Primitives** — Text, Paragraph, Empty
- **Layout** — Split (n-ary, with SplitSize), Stack, Padded, Centered,
  Scrollable
- **Data structures** — Table, List, Tree
- **Rich widgets** — Diff, ProgressBar, Sparkline, Badge, KeyValue
- **Structural** — Tabs, Modal, StatusBar
- **Terminal passthrough** — VtPassthrough (only from Compat Translator)
- **Extension** — Custom { type_id, data, fallback } for plugin-provided
  widgets

FrameElement can evolve freely. Adding a new variant doesn't break anything —
Renderer Host handles the walk, clients never see it.

**Complexity bounds:** The Renderer Host enforces limits on FrameElement trees
to prevent a misbehaving app or plugin from stalling the session executor:

- **Maximum tree depth:** 64 levels. Deeper nesting is truncated with a
  `[content truncated: nesting too deep]` placeholder.
- **Maximum node count per frame:** 10,000 nodes. Trees exceeding this are
  partially rendered up to the limit.
- **Maximum RenderCommand output size per frame:** 1 MiB. If the walk
  produces more, remaining commands are deferred to the next frame tick.

These limits are documented in the App SDK spec. Well-behaved apps will never
approach them — they exist to bound the worst case for adversarial input.

### RenderCommand (public, versioned, stable)

Concrete drawing instructions. All positions absolute, all colors resolved to
RGB, all theme tokens eliminated:

- **Positioning** — SetCursor, SetClip, ClearClip
- **Drawing** — DrawText, DrawRect, DrawBorder, DrawLine, DrawImage
- **Scrolling** — ScrollRegion
- **Layering** — PushLayer, PopLayer
- **Passthrough** — WriteRaw (for VT compat, clients that don't support VT
  ignore it)
- **Frame control** — Clear, Flush

ResolvedStyle accompanies drawing commands — fg/bg as RGB, bold, italic,
underline, dim. No token references, no theme indirection.

### The Stability Boundary

```
FrameElement (two tiers — see below)
        │
   Renderer Host
        │
RenderCommand (wire-stable, versioned, breaking changes require wire version bump)
```

**FrameElement has two stability tiers:**

- **Core primitives** (Text, Paragraph, Table, List, Tree, Split, Stack,
  Tabs, Diff, ProgressBar, VtPassthrough, Custom) — these are the authoring
  API that third-party apps use via `malt-app-sdk`. They are versioned and
  follow semver. Breaking changes to core primitives require an App SDK major
  version bump.
- **Internal variants** (layout hints, optimization nodes, Renderer Host
  intermediates) — these can change any release. They are not exposed through
  `malt-app-sdk`.

This resolves the tension between "FrameElement evolves freely" (true for
internal variants) and "apps author FrameElement trees" (true, using the
stable core subset). `malt-app-sdk` re-exports only the core primitives.

**RenderCommand** is the public client contract — versioned, documented,
wire-stable. Internal rendering improvements — new widget types, better
diffing, smarter degradation — never break external clients.

---

## 8. Plugin System

Two-tier extension model: WASM plugins for low-latency in-process hooks,
native apps for interactive pane content.

### Tier 1: WASM Plugins

Run inside the Plugin Host subsystem. Sandboxed, permission-gated,
budget-constrained. These handle hot-path work where latency matters:

- **Prompt segments** — contribute sections to the shell prompt
- **Completion providers** — suggest completions during line editing
- **Output parsers** — recognize command output and produce StructuredOutput
- **Shell hooks** — react to command lifecycle events
- **Status widgets** — contribute to the status bar
- **Background tasks** — periodic or event-driven background work

Execution budgets per hook class:

| Hook | Latency budget | Rationale |
|------|---------------|-----------|
| Keystroke/completion | < 5ms | Must not degrade typing feel |
| Prompt segment | < 10ms | Prompt must appear instantly |
| Output parser | < 50ms | Can trail slightly behind output |
| Background task | Unbounded | Not on the critical path |

Plugins declare capabilities and permissions in a manifest. The Plugin Host
enforces budgets and permissions at runtime. A plugin that exceeds its budget
is killed, not stalled.

### Tier 2: Native Apps

Separate processes that implement the App trait. They connect to the daemon
via VNP, receive input events, return FrameElement trees. The Process
Supervisor manages their lifecycle.

Apps are first-class panes — the daemon treats them identically to shell panes
in terms of layout, focus, and rendering. Apps can run in two modes:

- **VNP mode** — connected to daemon, full platform features
- **Standalone mode** — ratatui fallback, works without a daemon (SSH, CI,
  plain terminal)

### Startup Scheduling

Plugins and apps can register to launch automatically at two lifecycle points:

**Daemon startup hooks** — activate when the daemon starts, before any session
exists. These are global background services:

- Sync agents (dotfile sync, cloud backup)
- Notification listeners (webhook receivers, watch services)
- System monitors (resource tracking, health checks)
- Integration bridges (IDE connectors, clipboard managers)

**Session startup hooks** — activate when a session is created (or restored
from persistence). These are per-session:

- Prompt plugins (git status, environment indicator, time display)
- Status bar widgets (CPU/memory, active jobs, weather)
- Auto-launched apps (file browser sidebar, log tail pane)
- Session-specific integrations (project-scoped tools)

**Registration:** Plugins and apps declare their startup behavior in their
manifest:

```
[startup]
daemon = true           # launch at daemon start
session = true          # launch at session create
session_filter = "..."  # optional: only for sessions matching a pattern
priority = 50           # ordering (0 = first, 100 = last)
lazy = false            # true = activate on first relevant event, not immediately
```

**User control:** Users manage startup entries through config and CLI:

- `malt startup list` — show registered startup plugins/apps
- `malt startup enable/disable <name>` — toggle without uninstalling
- `~/.config/malt/startup.vx` — user overrides (enable, disable, reorder,
  add session filters)
- Per-session profile overrides — a "dev" session profile can have different
  startup hooks than a "prod" profile

**Startup order:** Daemon hooks run in priority order. Session hooks run after
the session is initialized (layout resolved, MASH ready). A slow startup hook
doesn't block session creation — hooks with `lazy = false` launch in parallel,
and the session becomes interactive immediately. Hooks that fail to start are
logged and disabled for the current lifecycle (not permanently).

### Marketplace

Plugins and apps are published to a marketplace. The architecture supports
this from the start — manifest-based metadata, declared permissions, versioned
interfaces. The package manager and registry infrastructure is a separate
concern, built on top of this foundation.

### Security Model

- WASM plugins are sandboxed — no filesystem, no network, no raw process
  access unless explicitly granted in manifest
- Plugins on the bus observe shell output events but never raw keystrokes
  (architectural property of the direct input path)
- **Output sensitivity:** Plugins that subscribe to `OutputChunk` can observe
  command output, which may contain secrets (env vars, API keys, file contents
  of sensitive files). This is a real exfiltration vector. Mitigations:
  - Plugins with both `output` subscription and `network` permission are
    flagged as **high-risk** in the marketplace and at install time. Users see
    an explicit warning: "This plugin can observe command output AND make
    network requests."
  - The Plugin Host provides an output filtering API: plugins declare which
    command patterns they need output from (e.g., `cargo *`, `git *`). The
    Plugin Host drops OutputChunks from commands that don't match the declared
    patterns. **This is defense-in-depth, not a security boundary** — command
    patterns are bypassable if the user aliases or wraps commands. The real
    security controls are: (a) Plugins declaring `output: *` (all commands)
    with `network` permission are **rejected from the marketplace** unless
    they pass manual security review, and (b) sideloaded plugins with this
    combination require explicit `--allow-exfiltration-risk` flag at install.
  - Per-session plugin output policy: users can restrict which plugins observe
    output in sensitive sessions (e.g., disable output plugins in a session
    used for secrets management).
  - `malt plugin audit` lists all installed plugins with their output access
    and network permissions for user review.
- Native apps run with OS-level process isolation. When running in a session
  with Restricted/Capped/Contained isolation, the Process Supervisor spawns
  apps within the session's isolation context — apps inherit the session's
  security boundary automatically
- Permission escalation requires explicit user consent, not just manifest
  declaration

---

## 9. API Gateway

The API Gateway is a subsystem that exposes MALT's capabilities to external
consumers over HTTP and MCP. It subscribes to bus messages and translates
between VNP and external protocols.

### Consumers

- AI agents (Claude, Copilot, custom)
- CI/CD pipelines
- Automation scripts
- Monitoring and observability tools
- Webhooks and integrations
- IDE extensions

### Integration Modes

| Mode | Transport | Audience |
|------|-----------|----------|
| HTTP REST | JSON over HTTP | Scripts, CI, webhooks, general automation (JSON is the external API format, not the VNP wire encoding) |
| MCP | stdio/SSE | AI agents that speak Model Context Protocol |
| Raw VNP | bitpack over socket | High-performance consumers that want full protocol access |

### Core Capabilities

- Session management (create, attach, detach, list, destroy)
- Command execution (blocking: run and wait; interactive: send input, await
  prompt)
- Output retrieval (raw text, structured output, semantic UI tree)
- Pane management (create, close, list, read content)
- Layout control (split, resize, focus)
- Status queries (is shell idle, running command, session state)

### Structured Output for Agents

The API Gateway maintains a **shadow FrameElement tree** per session by
subscribing to FrameElement messages on the bus. For HTTP/MCP consumers, it
serializes this tree as semantic JSON:

```json
{"type": "Table", "columns": ["Status", "Crate"], "rows": [
  {"Status": "Compiling", "Crate": "malt-protocol"},
  {"Status": "Finished", "Crate": "malt-platform"}
]}
```

Agents receive structured UI semantics, not raw terminal text. This is the
key advantage over traditional terminal scraping — an agent knows it's
looking at a table, not guessing from character positions. The shadow tree
updates on every FrameElement bus message; HTTP clients poll or long-poll,
MCP clients receive push updates.

### Prompt-Aware Execution

Agents don't poll or scrape. The API Gateway understands prompt state because
MASH emits PromptReady to the bus. The gateway exposes this as:

- `POST /exec` — blocking: sends command, waits for PromptReady, returns
  output + exit code. **Rate limited:** configurable per-client rate limit
  (default: 10 requests/second). An agent bug or runaway script cannot
  fire unbounded commands. Rate limit headers (X-RateLimit-Remaining,
  Retry-After) are included in responses. Rate limits are per-session AND
  per-client-identity. A per-client global rate limit (default: 100 req/s
  across all sessions) prevents an attacker from opening N sessions to bypass
  per-session limits. **Payload size limits:** `POST /exec` and `POST /send`
  reject request bodies exceeding 64 KiB (configurable). Command strings
  larger than this are not legitimate shell input.
- `POST /send` + `GET /await` — interactive: send input, then poll or
  long-poll for prompt readiness

### Authentication & Authorization

Authentication is category-based, with minimum requirements per transport:

| Category | Transport | Mechanism | Setup |
|----------|-----------|-----------|-------|
| **Local** | Named pipe, Unix socket | OS peer credentials (`SO_PEERCRED` / `GetNamedPipeClientProcessId`) | Zero-config. Same user = authenticated. |

> **As-built note (2026-07-25).** The local transport is TCP on `port+1` and
> authenticates with the same shared bearer token as the HTTP surface, read
> from `~/.config/malt/api-token` — *not* OS peer credentials, and not a named
> pipe or Unix socket. This closed the disclosure and injection hole (audit
> A-01) but does not make "same user" structural: any local process able to
> read the token file is indistinguishable from the owner. The peer-credential
> design above remains the intended end state and is tracked in
> `docs/BACKLOG.md` under A-01. Recorded here so this table is not read as a
> description of what exists.
| **Remote-personal** | TCP+TLS, WebSocket+TLS | Self-signed TLS certificate + TOFU pinning + bearer token | `malt remote setup` generates cert + token. First connection pins the cert. |
| **Remote-shared** | TCP+TLS, WebSocket+TLS | CA-signed TLS + API key (per-client, revocable) or OAuth 2.0 | For shared infrastructure, CI, team environments. |

TLS is mandatory for all remote transports (TCP, WebSocket). Unencrypted
remote connections are rejected. Self-signed certs with TOFU are the default
for personal use; CA-signed certs are required for shared deployments.

**Authorization scopes:** Not all consumers need all capabilities. Access is
scoped:

| Scope | Capabilities | Typical consumer |
|-------|-------------|------------------|
| **monitor** | `GET /health`, `GET /diagnostics`, `GET /sessions` (list only) | Monitoring tools, alerting |
| **read** | monitor + `GET /pane/:id/scrollback`, `GET /pane/:id/output`, session status | Log collectors, dashboards |
| **interact** | read + `POST /send`, `POST /exec`, `POST /session/create`, session attach | AI agents, automation scripts |
| **admin** | interact + session destroy, plugin management, daemon control, group policy | Admin CLI, CI orchestrators |

Local connections default to `admin` scope (same user = full trust). Remote
connections must specify scope at token creation time. API keys are scoped —
a monitoring tool's key cannot call `POST /exec`. MCP connections default to
`interact` scope.

**Client identity for rate limiting:** Local connections are identified by
PID (via peer credentials). Remote connections are identified by API key or
OAuth client ID. Rate limits are enforced per identity, not per connection —
opening multiple connections from the same client does not bypass limits.

### Extractability

> **Amended 2026-07-25 (ADR-0004).** "Exclusively through bus messages" no
> longer holds for client-facing delivery. The Bus's `Reliable` tier grows
> its ring beyond capacity rather than evict, which is unsafe when the drain
> rate is controlled by an external client rather than the daemon — a
> subscriber that stops reading would grow daemon memory without bound.
> Feature 004 therefore delivers lifecycle events over bounded
> per-subscriber channels with an explicit loss policy. In-daemon delivery
> is unchanged. See
> `docs/adr/ADR-0004-bounded-direct-channels-for-client-delivery.md`.

The API Gateway is designed as an extractable subsystem. It communicates with
the daemon exclusively through bus messages. If load or isolation requirements
demand it, it can move to a separate process that connects to the daemon via
VNP transport — no interface changes.

---

## 10. Compat Translator

The Compat Translator is the boundary where legacy terminal programs enter the
MALT world. It is the only component in the entire system that handles VT
escape codes.

### What It Does (output direction only)

1. Reads raw byte output from a PTY's read fd (PTY spawned by Process Supervisor)
2. Parses VT escape sequences (cursor movement, color, screen manipulation)
3. Maintains a virtual terminal grid state
4. Maps VT absolute cursor positions to pane-relative coordinates using the
   pane's ResolvedPane dimensions
5. Emits VNP messages: VtPassthrough FrameElements for Renderer Host, or
   translated equivalents where possible

### What It Does NOT Do

- Handle input — the daemon core writes keystrokes directly to the PTY's
  write fd for legacy panes (line discipline, ECHO mode, and character/line
  buffering are handled by the PTY kernel driver, not by MALT)
- Spawn processes (Process Supervisor does that)
- Render anything (Renderer Host and clients do that)
- Interact with the shell (MASH handles all shell concerns)

**One instance per legacy pane.** Each PTY-backed pane gets its own Compat
Translator instance. They're lightweight — a VT state machine and a grid
buffer.

**Process isolation for Restricted+ sessions:** Compat Translator instances
in sessions with Restricted, Capped, or Contained isolation tiers run
out-of-process. The VT parser processes attacker-controlled byte streams
(command output from untrusted programs) and is historically a rich source of
memory safety bugs. In Bare sessions, Compat Translator runs in-process for
simplicity. In Restricted+, the daemon spawns a separate `malt-compat-worker`
process per legacy pane, communicating via VNP over a Unix socket or named
pipe. The architecture already supports this — Compat Translator communicates
exclusively through VNP messages, and the subsystem is extractable (Principle
5). The overhead is one extra process per legacy pane in higher isolation
tiers — negligible compared to the PTY process itself.

**Worker failure handling contract:** The out-of-process compat worker has
explicit failure modes and recovery:

- **Heartbeat:** The worker sends a liveness message to the daemon every 2
  seconds. If the daemon receives no heartbeat for 6 seconds (3 missed), it
  declares the worker dead.
- **Worker crash:** The daemon displays `[compat: restarting...]` in the
  pane, spawns a new worker, and re-attaches it to the same PTY fd. The PTY
  fd is held by the daemon, not the worker — child processes survive worker
  crashes. The new worker starts with a fresh VT state machine; some screen
  content may be lost but the underlying process is unaffected.
- **Worker hang (output stall):** If the worker produces no VNP messages for
  10 seconds while the PTY has readable data (detected via poll on the PTY
  fd), the daemon kills the worker (SIGKILL) and restarts it.
- **Worker memory exhaustion:** The worker is spawned within the session's
  isolation context. In Capped/Contained tiers, cgroup memory limits apply
  and the OOM killer handles runaway workers. In Restricted tier, the daemon
  monitors worker RSS via `/proc/pid/statm` (Linux) or `GetProcessMemoryInfo`
  (Windows) and kills workers exceeding a configurable limit (default: 64 MiB).
- **VNP connection loss:** Treated as worker crash — restart and re-attach.
- **Maximum restart count:** After 5 consecutive crashes within 60 seconds,
  the pane is marked as failed and displays `[compat worker failed — close
  and reopen pane]`. The PTY child is killed.

**Enforcement:** The VT parsing library is a dependency of the compat crate
only. A workspace-level `deny.toml` rule bans VT parsing dependencies in all
other crates. This is a hard invariant, not a convention.

**Client support for WriteRaw:** Renderer Host emits
`WriteRaw { data, rect }` RenderCommands for VtPassthrough content. The
`rect` specifies the pane's absolute screen position and dimensions. The
`data` contains VT bytes with cursor positions already translated to
pane-relative coordinates by the Compat Translator (which maintains a virtual
grid and knows the pane's dimensions from ResolvedPane). Clients that support
VT (maltty, malt-tui) render `data` within `rect` — the coordinate system is
consistent. Clients that don't support VT skip WriteRaw. Legacy program
support gracefully degrades per client capability — the architecture never
forces VT knowledge on any client.

---

## 11. Crate Architecture

Strict layering — no upward dependencies. Each layer depends only on layers
below it.

```
L0  malt-platform        OS abstractions: PTY, process, signals, fs, sockets,
                         isolation (4 tiers: Bare → Restricted → Capped → Contained),
                         HCS bindings (Windows, feature-gated), cgroup v2 (Linux),
                         seatbelt (macOS), seccomp BPF, network (veth/bridge),
                         elevated helper client (IPC to malt-elevate)

L0  malt-config          Vexil Store config: schema-validated .vx files, loaded at startup

L0  malt-protocol        VNP: all message types (Vexil-generated), framing, envelope,
                         shared domain types (PaneState, SessionState, PaneKind,
                         PaneId, SessionId, IsolationTier)

L1  mash                 POSIX shell: lexer, parser, expander, executor, builtins,
                         job control. Depends on malt-platform (process/PTY/signal
                         traits) and malt-protocol (VNP message types for OutputChunk,
                         CommandStarted, etc.)

L1  malt-term            Line editor: vi/emacs modes, multiline, completion (fs/history/
                         command-specific), syntax highlighting, prompt renderer, history

L1  malt-tools           In-process POSIX utilities: echo, cat, ls, grep, sed, awk,
                         find, kill, ps, ln, which, env, pwd + uutils-backed (head,
                         tail, sort, uniq, cut, etc.)

L1  malt-layout          Layout engine: n-ary LayoutNode tree, resolution, layout ops,
                         directional focus, persistence

L1  malt-session         Session lifecycle logic, command block ring buffer,
                         session group management. Depends on malt-protocol for
                         shared types (PaneState, SessionState, PaneKind live in
                         malt-protocol, not here — L0 types cannot import L1).

L2  malt-compat          VT emulator: VtParser, VtTranslator (VT → VNP), PtyShim,
                         pluggable output parsers. Only VT code in system.

L2  malt-renderer        Renderer Host: FrameElement walker, RenderCommand emission,
                         RenderTransport trait, widget registry, theme resolver,
                         dirty tracker, capability adaptation. No rendering code.

L2  malt-daemon          Daemon core: message bus, scheduler, session store,
                         process supervisor, structured output parser, plugin host,
                         API gateway (HTTP/MCP), session groups (policy, lifecycle)

L3  malt-plugin-sdk      WASM plugin SDK, #[plugin] macro
L3  malt-app-sdk         App trait, dual-mode runner (VNP + standalone fallback)
                         [status 2026-07-24: NOT built. Distinct from `malt-tui`
                         (malt's own first-party client, which does exist) —
                         malt-app-sdk is the still-unbuilt third-party app-authoring
                         SDK described in §7 (re-exports only the stable FrameElement
                         core-primitive subset, own semver line). See CLAUDE.md
                         Phase G roadmap.]
L3  malt-bin             CLI entry point (`malt` command, daemon lifecycle,
                         service install/uninstall/status, privilege mode dispatch)

    malt-elevate          Elevated helper service (standalone binary, outside the
                          layer system — no workspace deps except malt-platform for
                          shared types). Thin, auditable. Runs as admin/root.
                          IPC: VNP over named pipe (Windows) or Unix socket
                          (Linux/macOS) using a restricted schema (`elevate.vexil`
                          — ~6 message types). Full VNP codegen and validation,
                          but the audit surface is one small file. Operations: symlink
                          creation, namespace setup, cgroup configuration, HCS
                          container management, mount operations, restricted token
                          creation.
```

### First-Party Clients (separate binaries, same repo)

```
    maltty                GPU terminal: wgpu + cosmic-text + winit
    malt-tui              TUI terminal: ratatui + crossterm
    malt-web              Browser client: SvelteKit, served by daemon
```

### Isolation

Isolation is a platform capability, not a separate crate. The 4-tier model
(Bare, Restricted, Capped, Contained) lives in `malt-platform`:

- Linux: PID/mount namespaces, cgroup v2, overlayfs (native + FUSE), seccomp,
  veth networking
- Windows: Job Objects, restricted tokens, HCS containers (feature-gated),
  AppContainer sandbox
- macOS: Seatbelt sandbox (Contained tier)

Session groups in the daemon apply isolation policy across multiple sessions —
minimum tier, resource caps, TTL, idle timeout, OOM handling.

### Privilege Modes

MALT supports two privilege modes, chosen at daemon startup:

**Unprivileged mode (default):** The daemon runs as the current user.
Operations requiring elevation are delegated to `malt-elevate`, a separate
helper service. This is the recommended mode for interactive use — it
minimizes the attack surface and follows least-privilege principles.

**Root mode (`malt --privileged`):** The daemon itself runs elevated
(admin/root). All privileged operations execute directly — no helper needed,
no IPC overhead, no capability probing. The daemon performs its own privilege
operations through `malt-platform` directly.

Root mode is appropriate for:
- CI/CD environments (already running as root in containers)
- Personal dev machines where privilege separation is unnecessary overhead
- Server-side automation (headless sessions, agent workloads)
- Environments where installing a system service is impractical

Root mode removes all elevation barriers — no `malt-elevate` needed, no
"elevation required" errors. OS capability probing still applies (root on
Linux doesn't enable Windows-only features). The tradeoff is that a daemon
compromise has full system access.

**The mode is a startup decision, not a runtime toggle.** Switching requires
restarting the daemon. `malt-platform` abstracts this: callers use the same
trait methods regardless of mode; the platform layer routes to either the
elevated helper IPC or direct syscalls based on how the daemon was started.

### Elevated Helper Service (`malt-elevate`)

In unprivileged mode, operations that require elevation are delegated to
`malt-elevate`, a separate small binary that runs with admin/root privileges.
The daemon communicates with it over a local IPC channel (named pipe on
Windows, Unix socket on Linux/macOS).

**Operations requiring elevation:**

| Operation | Platform | Why elevation is needed |
|-----------|----------|------------------------|
| Symlink creation | Windows | Requires `SeCreateSymbolicLinkPrivilege` (unless Developer Mode) |
| PID/mount namespace setup | Linux | `unshare(2)` for non-user namespaces requires `CAP_SYS_ADMIN` |
| cgroup v2 configuration | Linux | Writing to cgroup hierarchy requires root or delegated subtree |
| overlayfs mount | Linux | `mount(2)` requires `CAP_SYS_ADMIN` (FUSE fallback is unprivileged) |
| HCS container management | Windows | HCS APIs require elevation |
| Restricted token creation | Windows | `CreateRestrictedToken` is callable by any process on its own token, but *using* the resulting token with `CreateProcessAsUser` or `NtFilterToken` with certain flag combinations requires `SeTcbPrivilege` |
| Seatbelt profile loading | macOS | `sandbox_init` may need entitlements |
| Network namespace + veth | Linux | Creating network interfaces requires `CAP_NET_ADMIN` |

**Design constraints:**

1. **Minimal surface area** — `malt-elevate` exposes a fixed set of operations,
   not a general-purpose privileged shell. Each operation has explicit parameter
   validation.
2. **Auditable** — The binary is small enough to audit manually. No plugin
   loading, no dynamic dispatch, no network access.
3. **Optional** — The daemon operates without `malt-elevate` for Bare tier and
   any operation that doesn't need elevation. If the helper isn't running,
   operations that require it fail with a clear error explaining what's needed.
4. **Authenticated** — Trust is established by the helper, not the daemon.
   `malt-elevate` generates a random session nonce at startup and writes it
   to a file in a directory it owns (permission 0700 on the directory, 0600
   on the file / ACL restricted to the user SID). The primary symlink attack
   protection is the 0700 directory ownership — an attacker cannot create
   symlinks in a directory they don't control. The file is additionally
   created with `O_NOFOLLOW | O_EXCL` as defense-in-depth (prevents opening
   over an existing symlink at the final path component, though it does not
   protect against path component traversal attacks — the directory
   permissions handle that). The daemon reads this nonce file and includes it in every request. This reverses the trust direction — a malicious
   process cannot race the daemon to establish the nonce because the helper
   controls nonce generation. Additionally: (1) OS-level peer credential
   check (same user via `SO_PEERCRED` / `GetNamedPipeClientProcessId`), (2)
   nonce validation on every request (the nonce rotates every hour — the
   helper generates a new nonce, writes it to the nonce file, and the daemon
   re-reads it on next request; during rotation, the helper accepts both old
   and new nonces for a 30-second overlap window to prevent race conditions;
   this limits the window of a compromised nonce), AND (3) on
   Windows, the helper verifies the client process creation time via
   `GetProcessTimes` to prevent PID reuse attacks. On Linux, the helper
   verifies the connecting process's executable path via `/proc/pid/exe` to
   ensure it is the expected daemon binary. (Note: `/proc/pid/exe` is raceable
   in theory — the process could be replaced between check and request
   processing. The combination of peer credentials + nonce + exe check is
   strong enough in practice; the exe check is a defense-in-depth measure,
   not a standalone guarantee.)
5. **Installable as a system service** — On Windows, registered as a Windows
   Service. On Linux, a systemd unit. On macOS, a launchd plist. Users can also
   run it manually for ad-hoc elevation.

### Dependency Rules

- `malt-protocol` has zero workspace dependencies — only external deps (serde,
  bytes, vexil-runtime). `vexil-runtime` is the Vexil bitpack runtime library,
  published as a standalone crate.
- `mash` depends on `malt-platform` (process/PTY/signal traits) and
  `malt-protocol` (VNP message type definitions). It never depends on daemon
  internals — the trait boundary is the seam for standalone extraction.
- `malt-compat` is the only crate permitted to depend on VT parsing libraries
  (enforced by `deny.toml`)
- `malt-plugin-sdk` and `malt-app-sdk` are the public surface for third-party
  developers
- Rust clients (maltty, malt-tui) depend on `malt-protocol` and
  `malt-platform` — never on daemon internals. `malt-web` (TypeScript)
  implements the VNP/RenderCommand wire protocol independently — it does not
  consume Rust crates.
- `malt-daemon` depends on `malt-compat` and `malt-renderer` (same-layer L2
  dependencies). This is permitted — the layering rule forbids upward deps,
  not horizontal ones. `malt-compat` and `malt-renderer` do not depend on
  `malt-daemon`.
- `malt-elevate` is outside the layer system. It shares type definitions with
  `malt-platform` and uses VNP with a restricted schema (`elevate.vexil`).

### Daemon Internal Structure

`malt-daemon` is one crate but internally organized by subsystem. Each
subsystem is a module with a clear boundary — if extraction is needed later,
the module becomes a crate.

```
malt-daemon/
  src/
    core/           message bus, scheduler, priority dispatch
    session/        session store, lifecycle, persistence
    supervisor/     process spawning, monitoring, restart
    parser/         structured output recognition, plugin dispatch
    plugin/         WASM runtime, sandbox, permission enforcement
    gateway/        HTTP/MCP server, auth, external API
    groups/         session groups, policy enforcement
```

---

## 12. Isolation & Session Groups

### Four Isolation Tiers

| Tier | What it provides | Linux | Windows | macOS |
|------|-----------------|-------|---------|-------|
| **Bare** | No isolation. Standard user-level execution. | ✓ | ✓ | ✓ |
| **Restricted** | Bounded filesystem, limited process access, no privilege escalation | PID/mount namespaces, `PR_SET_NO_NEW_PRIVS` | Job Objects, restricted tokens | — (deferred) |
| **Capped** | Restricted + resource enforcement (memory, CPU, I/O, PID limits) | cgroup v2 | Extended Job Object limits | — (deferred) |
| **Contained** | Full rootfs isolation, image-backed environments, network isolation | overlayfs (native + FUSE), seccomp BPF, veth/bridge | HCS containers (feature-gated) or AppContainer | Seatbelt sandbox |

**Runtime capability probing:** `tier_available(tier)` checks the current
platform at runtime. Not all tiers are available everywhere — the daemon
exposes what the host supports, never pretends. Clients can query
`contained_capability()` to discover available backends, checkpoint modes,
network modes, and seccomp support. In root mode, the daemon skips elevation
checks but still probes OS capability — root mode on Linux cannot enable HCS
(Windows-only), and root mode on Windows cannot enable cgroup v2 (Linux-only).
"Root mode unlocks all tiers" means "no elevation barriers," not "all OS
features magically exist."

**macOS isolation strategy:**

- **Restricted tier (v1.0):** Implemented via `sandbox-exec` profiles.
  **Risk acknowledgment:** Apple has formally deprecated `sandbox-exec` and
  the `sandbox_init` family of functions. While macOS system daemons
  (mDNSResponder, ntpd) still use it internally, it is not a public API and
  could be removed or broken in any macOS release. App Store apps cannot use
  it. MALT uses it as the best available option for non-App Store CLI tools,
  with the explicit understanding that a future macOS release may require
  migration to Apple's Endpoint Security framework or App Sandbox entitlements.
  This is tracked as a platform risk, not treated as stable.
- **Capped tier (v1.0):** macOS lacks cgroup equivalents. Capped on macOS
  uses `setrlimit` for memory/CPU limits (coarser than cgroup v2 but
  functional) plus `sandbox-exec` for the Restricted properties. Resource
  enforcement is best-effort — `setrlimit` limits are per-process, not per-
  group. This is documented in the platform parity matrix as a known
  limitation.
- **Contained tier:** Seatbelt sandbox only (no overlayfs equivalent on
  macOS). Filesystem isolation via `sandbox-exec` + bind mounts where
  possible; no rootfs overlay support.
- **malt-elevate verification:** macOS lacks `/proc/pid/exe`. Substitute:
  code signature verification via `SecCodeCopySigningInformation` — the
  helper verifies the connecting process is signed by the expected identity.
  This requires MALT binaries to be code-signed (standard for macOS
  distribution).

Isolation tier is a per-platform explicit capability. Clients check
`tier_available(tier)` — if a requested tier isn't supported, the session
creation fails with a clear error explaining what's available, not a silent
fallback to Bare. Users explicitly opt into the lower tier if they accept
the tradeoff.

### Session Groups

Session groups bundle multiple sessions under a shared policy:

```
GroupPolicy {
    min_tier:          minimum isolation tier for all sessions in the group
    max_memory:        aggregate memory cap
    max_cpu_cores:     aggregate CPU limit
    max_sessions:      capacity limit
    ttl_secs:          group auto-destroy after duration
    idle_timeout_secs: auto-destroy after inactivity
    on_empty:          Destroy | Keep | Checkpoint
    on_oom:            KillOffender | CheckpointThenKill | PauseAndNotify
}
```

Groups support atomic lifecycle operations: pause all sessions
(SIGSTOP on Linux, NtSuspendProcess on Windows), resume, kill, checkpoint.
This enables:

- Agent workloads with bounded resource usage
- Development environments with isolation guarantees
- Multi-pane task execution under shared policy
- Classroom/demo environments with time-limited sessions

Isolation lives in `malt-platform`. The daemon's session store and group
manager apply policy; the platform crate executes the OS-level primitives. No
other crate touches namespace, cgroup, or sandbox APIs directly.

---

## 13. Persistence & Session Lifecycle

Sessions are the unit of persistence. A session captures everything needed to
restore the user's workspace: layout tree, pane configuration, focus state.
Pixel positions, GPU state, client connections, and animation state are never
persisted — they're derived at attach time.

### What's Persisted

```
PersistedSession {
    id:        SessionId
    name:      Option<String>
    layout:    LayoutNode          // abstract tree, no pixel positions
    focus:     PaneId
    panes:     Map<PaneId, PersistedPane>
    theme:     Option<ThemeOverride>
    group:     Option<GroupId>
    isolation: IsolationTier
}

PersistedPane {
    cwd:            PathBuf
    title:          Option<String>
    pane_type:      Shell { shell_path, env_snapshot } | App { app_id, config }
                    | Compat { program, args }
    command_blocks: Array<PersistedCommandBlock>
}

PersistedCommandBlock {
    command_id:  u32
    cmd:         String
    started_at:  u64             // epoch ms
    finished_at: Option<u64>     // absent while running, and permanently
    exit_code:   Option<i32>     // absent if the daemon stopped mid-command
}
```

`env_snapshot` (shell variables, aliases, functions, options, directory
stack, traps) and `command_blocks` both ship as of feature 003 — see
`specs/003-command-execution-history/` and `schemas/persist/session.vexil`,
which is the authority on the exact shape.

**Schema versioning:** Every persisted file includes a `schema_version` field
(integer, starting at 1). Deserialization is version-gated:

- Unknown fields are ignored (forward-compatible — a v2 file read by v1 code
  silently drops new fields, preserving layout and pane config).
- Missing fields use documented defaults (a v1 file read by v2 code fills new
  fields with defaults that preserve v1 behavior).
- Reordering or removing existing fields is a breaking change that bumps the
  major schema version. A reader encountering a major version it doesn't
  understand refuses to load and emits a clear error ("session file requires
  MALT ≥ X.Y").
- The corruption handler distinguishes parse failures (genuinely corrupt →
  moved to `corrupted/`) from version mismatches (schema too new → left in
  place with an error, never moved to `corrupted/`).

```
PersistedSession {
    schema_version: u32             // starts at 1, incremented on field changes
    ...
}
```

### What's NOT Persisted

Terminal dimensions, resolved positions, texture atlases, scroll offsets,
active connections, client identity, animation state, process handles.

### Session Lifecycle

```
CreateSession → Active → Detach → Dormant → Attach → Active
                  │                   │
                  ├── Destroy ────────┤──→ Destroyed (terminal)
                  │                   │
                  └── Checkpoint ─────┘ (if isolation tier supports it)
```

- **Active** — at least one client attached, panes running
- **Dormant** — no clients attached, processes may be suspended (depends on
  policy), state persisted
- **Checkpoint** — state snapshot including isolation environment (Contained
  tier only, where supported). Checkpoint fidelity scales with isolation tier:
  Bare preserves layout + metadata only; Restricted/Capped add process tree
  metadata (PIDs, cwds, argv — not process memory); Contained preserves the
  full rootfs overlay, enabling filesystem-level restore including file
  changes made during the session. **Process memory is not captured** —
  running processes are terminated on checkpoint and re-launched on restore.
  CRIU-based live process migration is a potential future enhancement but is
  not part of this architecture.
- **Destroy** — cleanup, processes killed, isolation environment torn down
  (cgroups removed, overlayfs unmounted, Job Objects closed), state removed

**Restore behavior:** When a Dormant session is re-attached, the daemon
restores the layout tree and creates fresh processes for each pane:
- Shell panes: new MASH instance in the persisted `cwd`, fresh prompt
- App panes: re-launch with persisted `app_id` and `config`
- Compat panes: re-launch `program` with persisted `args` in persisted `cwd`

The user sees their previous workspace layout with fresh processes. Scrollback
from the previous session is gone (ephemeral). This is the same model as tmux
session restore — layout persists, process state does not.

### Scrollback

Each pane maintains a scrollback buffer backed by a **memory-mapped
append-only log file**. Scrollback stores the OutputChunk history for that
pane — text that has scrolled off the visible area.

- **Size:** configurable per session or globally (default: 10,000 lines per
  pane, ~800 KiB per pane at ~80 chars average)
- **Storage:** one memory-mapped file per pane
  (`data/scrollback/{session_id}/{pane_id}.log`). New output is appended to
  the file; the OS page cache handles write-back to disk automatically. No
  explicit flush timers, no bulk-rewrite-on-lifecycle-event pattern, no crash
  window — writes are visible to the filesystem immediately via the mmap.
  On unclean daemon crash, scrollback is intact up to the last appended line.
- **Ring behavior:** when the file reaches the per-pane line limit, it wraps.
  The file header tracks the current write position and line count. On read,
  the consumer calculates the start position from the wrap offset.
- **Memory footprint:** the mmap is backed by the filesystem, not daemon heap.
  The OS pages in only the actively-accessed portion. 20 sessions × 10 panes
  = 200 mmap files, but resident memory is determined by access patterns, not
  file count. This eliminates the ~320 MiB heap overhead of in-memory
  scrollback for large deployments.
- **Scrollback files are deleted atomically with pane/session destruction.**
- **Client access:** newly attached clients receive the current FrameElement
  tree (current visible state) plus up to N lines of scrollback via a
  ScrollbackRequest VNP message. Clients render scrollback as they see fit.
- **API access:** `GET /pane/:id/scrollback?lines=N` returns recent scrollback
  as text or structured output (if the Structured Output Parser has annotated
  it).
- **Aggregate disk budget:** Total scrollback disk usage is capped
  (configurable, default: 1 GiB). When the budget is exceeded, the oldest
  scrollback files (by last-write time) are deleted via LRU eviction.
  Scrollback file cleanup is atomic with session destruction — when a session
  is destroyed, its scrollback files are deleted in the same operation, not
  by a background task that can race or fail silently.
- **Scroll offsets are per-client, not per-pane.** Each attached client tracks
  its own scroll position independently. The daemon doesn't store scroll
  offsets — they are client-local state.

**Named layout presets** are stored as .vx files. Users save and load
workspace configurations independently of session lifecycle. A preset defines
structure (splits, tabs, pane types) but not runtime state.

**The daemon is the truth.** Any client can attach to any session and get a
fully restored workspace. Client crashes lose nothing. Daemon restart restores
from persisted state.

### Crash Recovery & Persistence Storage

**Storage locations** follow platform conventions and separate config
(user-editable, version-controllable) from data (daemon-managed, ephemeral):

Config (`~/.config/malt/` on Linux, `%APPDATA%\malt\` on Windows,
`~/Library/Preferences/malt/` on macOS):
- `config.vx` — daemon configuration (connection, isolation defaults, log level)
- `startup.vx` — user startup overrides
- `presets/` — named layout presets (user-created, dotfile-friendly)
- `themes/` — custom themes

All structured configuration uses Vexil Store .vx format, validated against
Vexil config schemas at load time.

Data (`$XDG_DATA_HOME/malt/` on Linux, `%LOCALAPPDATA%\malt\` on Windows,
`~/Library/Application Support/malt/` on macOS):
- `sessions/{session_id}.vx` — persisted session state (Vexil Store text format)
- `sessions/{session_id}.vxb` — binary snapshot (Vexil Store bitpack, optional fast-restore)
- `daemon.vx` — daemon runtime state (active session list, group policies)
- `corrupted/` — moved here on corruption detection

Config is intended to be checked into dotfiles; data is not.

**Session data format: Vexil Store.** Session persistence uses Vexil Store —
the human-readable text format (`.vx`) and binary bitpack format (`.vxb`) from
the `vexil-store` crate. This gives schema evolution for free (Vexil's
append-only field rules apply), handles recursive LayoutNode trees natively
(Vexil structs nest arbitrarily), and keeps the schema authority unified — the
same `.vexil` schemas that define wire types also define persistence types.

The `.vx` text format is the primary persistence format: human-inspectable,
diffable, and debuggable. The daemon writes `.vx` on every persist cycle. An
optional `.vxb` binary snapshot can be written alongside for fast restore —
the daemon reads `.vxb` if present and falls back to `.vx`. The binary format
is ~10x faster to parse than text for large session trees.

Persistence schemas (`PersistedSession`, `PersistedPane`, `PersistedLayout`,
`DaemonState`) are defined in `.vexil` schema files under `schemas/persist/`.
The `vexil-store` crate's schema-driven encoder/decoder walks the `Value` tree
in parallel with the schema's `TypeDef`, so no hand-written serialization code
is needed — adding a field to the schema automatically extends persistence.

**Persistence debouncing:** Session state is marked dirty on every change but
written at most once per second (configurable via `persist_interval_ms`). A
dirty flag plus a timer ensures rapid changes (interactive resizing, fast
layout operations) don't create I/O pressure. Persistence is also triggered
immediately on detach, shutdown, and checkpoint — the timer is a floor, not
a ceiling.

**Session name uniqueness:** Session names (`Option<String>`) must be unique
within a daemon instance, including persisted sessions loaded at startup.
On daemon restart, if persisted session files contain duplicate names, the
first loaded wins and subsequent duplicates are renamed with a numeric suffix
(`dev` → `dev-2`). Creating a new session with a name that already exists
returns an error. Unnamed sessions (name = None) are always allowed. This
prevents ambiguity in CLI commands (`malt attach dev`) and API endpoints.

**Atomic writes with backup:** All persistence writes use write-to-temp +
atomic rename. A crash mid-write leaves the previous valid file intact. The
temp file is written to the same directory (same filesystem) to guarantee
rename atomicity. Before each atomic rename, the daemon keeps the previous
version as `{session_id}.vx.bak` — a single-generation backup that enables
manual recovery if a bug writes valid-but-wrong state.

**Corruption detection:** On startup, the daemon validates each session file.
Files that fail Vexil Store parsing or schema validation are moved to a `corrupted/`
subdirectory with a timestamp suffix. The daemon logs the corruption and
continues — a single corrupted session does not prevent other sessions from
restoring. The `corrupted/` directory is capped at 50 files; when the cap is
reached, the oldest entries are deleted. This prevents unbounded growth from
recurring serialization bugs.

**What survives a daemon crash:**
- Session layout, pane configuration, focus state (persisted on every change)
- Named layout presets (separate files, unaffected)
- Session group policies (persisted)

**What does NOT survive:**
- In-flight OutputChunks not yet delivered to subscribers
- Unsaved scrollback content (scrollback is ephemeral by design)
- Process handles (shell commands and apps must be re-launched on restore)

### Graceful Shutdown

When the daemon receives SIGTERM (or equivalent service stop):

1. Stop accepting new connections
2. Persist all active sessions (atomic writes)
3. Send DetachSession to all connected clients
4. Send SIGTERM to all child processes (via platform traits)
5. Wait up to N seconds for child processes to exit. The timeout is
   configurable globally (`shutdown_timeout_secs`, default: 5) and per-session
   (`session.shutdown_timeout_secs`). Per-session overrides allow sessions
   running long builds or migrations to request more time. If neither is set,
   the global default applies. After the timeout, daemon sends SIGKILL.
6. Force-kill remaining children
7. Tear down isolation environments (cgroups, overlayfs, Job Objects)
8. Exit

### Daemon Upgrade Strategy

Upgrading a running daemon to a new version without losing active sessions:

**Standard upgrade (service modes):** The daemon performs a graceful shutdown
(persisting all sessions), the service manager restarts with the new binary,
and sessions restore from persisted state. Running processes (shell commands,
apps) are terminated and re-launched from scratch. This is the default
upgrade path — simple, reliable, and consistent with the persistence model.

**PTY-preserving upgrade (`malt upgrade`):** For long-running sessions where
process continuity matters, the daemon supports fd-handoff restart:

1. New daemon binary is installed alongside the running daemon
2. `malt upgrade` signals the running daemon to enter upgrade mode
3. Running daemon persists all session state (layout, config, group policies)
4. Running daemon serializes PTY file descriptors and child PIDs into a
   handoff file (fd numbers + child PID + PTY path + pane mapping)
5. Running daemon `exec`s the new binary, passing the handoff file path
6. New daemon reads the handoff file, re-acquires PTY fds (inherited via
   exec), re-attaches to still-running child processes, and restores session
   state from persisted `.vx` files
7. Connected clients see a brief disconnect/reconnect (< 1 second)

**Constraints:** PTY-preserving upgrade requires that the new daemon version
can read the old daemon's persistence format (guaranteed by schema versioning
— see above). PTY fds are inherited via exec, so child processes are never
signaled. This only works on Unix (exec replaces the process image); on
Windows, the daemon uses a different mechanism: the old daemon sends PTY
handles to the new daemon process via `DuplicateHandle` before exiting.

**Version compatibility:** The handoff file includes the daemon's wire
version. If the new daemon cannot read the handoff format (major version
mismatch), it falls back to standard upgrade (graceful shutdown + restore).
The user is warned that running processes will be lost.

### Daemon Service Model

The daemon can run in three deployment modes, configurable per platform:

**Foreground process (`malt daemon`):** Launched manually or by a client on
first use. Persists sessions to disk on exit. Dies when the last client
disconnects (or when explicitly stopped). **Weaker durability than service
modes:** sessions are persisted but running processes are lost — on next
daemon start, sessions restore layout and cwd but re-launch shells/apps from
scratch. Suitable for development and testing where process continuity across
daemon restarts is not needed.

**User service (default for interactive use):** Starts at user login, runs in
the background, survives terminal closes and client crashes. Sessions persist
across logoff/login cycles. Platform registration:

- Windows: Task Scheduler task (triggered at logon) or user-mode service
- Linux: `systemd --user` unit (`malt-daemon.service`)
- macOS: `~/Library/LaunchAgents/com.malt.daemon.plist`

**System service (pre-logon):** Starts at boot, before any user logs in.
Required for server-side automation, headless agent workloads, and CI
environments where sessions must be available immediately. Platform
registration:

- Windows: Windows Service (registered via `malt service install`,
  runs as `LocalService` or a configured service account)
- Linux: system-level systemd unit (`/etc/systemd/system/malt-daemon.service`)
- macOS: `/Library/LaunchDaemons/com.malt.daemon.plist`

**System service considerations:**

- When running as a system service, the daemon runs as a service account, not
  any particular user. Session access is gated by authentication — clients
  connecting via named pipe or Unix socket are identified by OS peer
  credentials; only sessions owned by the connecting user are visible.
- Multi-user: a system-service daemon **spawns a separate worker process per
  user**. The system service acts as a process manager — it accepts
  connections, authenticates via peer credentials, and routes each user to
  their dedicated worker process. Workers are isolated by OS-level process
  boundaries: different users' sessions never share an address space. This
  eliminates the shared-memory threat model where a vulnerability in one
  user's session (WASM escape, VT parser bug) could access another user's
  data. The per-user worker model adds process management complexity but
  provides real security isolation — the system service is a thin dispatcher,
  not a shared runtime.
- The service mode is orthogonal to the privilege mode — a system service can
  run in either unprivileged mode (delegating to `malt-elevate`) or root mode
  (performing privileged operations directly). Root mode system service is the
  simplest high-capability deployment.
- `malt service install` / `malt service uninstall` manages registration.
  `malt service status` reports the current mode.

**Mode selection is a deployment decision.** The daemon code is identical in
all three modes — the only difference is how it's started and what user
identity it runs under. `malt-bin` handles mode dispatch: if invoked by a
service manager, it enters service mode; if invoked interactively, it runs
in foreground or registers as a user service.

---

## 14. Security Model

Security properties emerge from architectural decisions, not bolted-on checks.

### Input Isolation

Keystrokes flow directly from client → daemon core → focused pane owner
(MASH, App via VNP, or PTY write fd for legacy panes). They never touch the
message bus. WASM
plugins and other subsystems cannot observe raw input through the bus
subscription mechanism. A malicious plugin can subscribe to shell output
events (CommandStarted, OutputChunk) but never intercept keystrokes,
passwords, or interactive input via the bus. Input interception would require
explicit daemon-level permission granted at a layer above the bus.

**Threat model limitation:** All in-process subsystems share the daemon's
address space. The direct input path prevents bus-level snooping, but it does
not constitute a memory isolation boundary. A memory corruption vulnerability
in any in-process subsystem (e.g., a VT parser exploit in Compat Translator
processing attacker-controlled bytes) could theoretically read MASH's input
buffer. WASM plugins are memory-isolated by the WASM sandbox; native code
subsystems are not. This is an accepted tradeoff of the in-process
architecture — the alternative (separate processes for each subsystem) would
add significant latency and complexity.

### Plugin Sandboxing

WASM plugins run in a sandbox with no implicit capabilities. No filesystem, no
network, no process spawning unless explicitly declared in the manifest and
approved by the user. Execution budgets enforce latency limits — a
misbehaving plugin is killed, not allowed to stall the system. Manifest
declarations are the permission boundary; runtime enforcement is the Plugin
Host's responsibility.

**Plugin-to-plugin isolation:** Plugins cannot communicate with each other
directly. Each plugin interacts only with the Plugin Host, which mediates all
input and output. A plugin's bus subscriptions are filtered by the Plugin Host
— it receives only the message types declared in its manifest. A malicious
plugin cannot read data emitted by other plugins unless it has declared (and
been granted) access to those message types.

### Process Isolation

- Clients run as separate processes. A client crash (GPU driver failure, OOM)
  never affects the daemon or other clients. Sessions survive any client
  failure.
- Apps run as separate processes managed by the Process Supervisor. An app
  crash affects only its pane.
- Isolation tiers provide OS-level sandboxing for sessions that need it —
  namespace isolation, resource caps, restricted filesystem access, seccomp
  syscall filtering.

### Transport Authentication

- Local connections (named pipe, Unix socket) authenticate via OS peer
  credentials — same user, zero configuration.
- Remote connections (TCP, WebSocket) require token-based authentication.
  Specific scheme defined in the API Gateway spec.
- The API Gateway is the only external-facing surface. Internal subsystems are
  not network-accessible.

### Privilege Separation

In unprivileged mode (default), all privileged operations are delegated to
`malt-elevate`, a separate binary with a fixed operation set. This creates a
clear privilege boundary: even if the daemon is compromised (e.g., via a
malicious plugin or a bug in VNP parsing), the attacker has no elevated
access. The elevated helper validates every request independently — it does
not trust the daemon.

In root mode (`malt --privileged`), the daemon has full privileges. This
trades isolation for simplicity — appropriate for CI, automation, and
personal machines where the threat model doesn't justify the separation
overhead. The `malt-platform` abstraction ensures code doesn't branch on
mode — callers use the same API regardless.

**Root mode risk:** In root mode, a WASM JIT escape or a bug in any in-process
subsystem results in root-level code execution. Marketplace plugins are
particularly risky in this context. The daemon logs a warning at startup when
root mode is active and marketplace plugins are installed. Future work may
include stricter WASM sandboxing in root mode (seccomp on the daemon process
itself, reduced plugin permissions).

### Wire Format Integrity

Vexil bitpack encoding is deterministic. Same message, same field values →
bit-identical bytes. This enables content-addressed message identity (BLAKE3
hash), replay detection, and cross-platform verification.

---

## 15. Observability

The daemon exposes structured diagnostics for debugging, monitoring, and
incident response. Observability is built into the architecture, not bolted
on after deployment issues.

### Metrics

Each subsystem exposes key metrics via a shared metrics registry. Metrics are
counters, gauges, and histograms — no string formatting, no ad-hoc logging.

| Subsystem | Key Metrics |
|-----------|------------|
| **Message bus** | queue depth per subscriber, message drop count (per priority), reliable_pressure ratio, dispatch latency (p50/p99) |
| **MASH** | command count, command latency histogram, prompt-to-command time |
| **Compat Translator** | VT parse latency per chunk, bytes processed/sec, error count |
| **Plugin Host** | budget utilization per plugin (%), kill count, load time |
| **Structured Output Parser** | parse latency histogram, recognition rate, timeout count |
| **Session Store** | persistence write latency, corruption count, active session count |
| **API Gateway** | request rate, error rate, latency histogram per endpoint |
| **Process Supervisor** | spawn count, restart count, OOM kill count |

### Structured Logging

All daemon logging uses `tracing` with structured fields. Every log event
includes:

- `session_id` (if session-scoped)
- `pane_id` (if pane-scoped)
- `subsystem` (originating subsystem name)
- `msg_type` (if related to a bus message)

Format: JSON lines to stderr or file (configurable). **Log rotation:** when
logging to file, logs rotate at 50 MiB (configurable via `log_max_size_mb`)
with 5 rotated files kept (configurable via `log_max_files`). Per-subsystem
log verbosity is configurable at runtime via `malt log-level <subsystem>
<level>` or `MALT_LOG` env var (same syntax as `RUST_LOG`). Correlation
across subsystems uses SessionId + PaneId — no synthetic trace IDs needed
for single-daemon deployments. For system services with per-user worker
processes, log events include `worker_id` for cross-process correlation.
Future multi-daemon deployments can adopt W3C Trace Context propagation in
the VNP envelope (a reserved extension field) without changing the logging
format.

### Metric Export

Metrics are exposed via a **Prometheus scrape endpoint** at
`GET /metrics` (served by the API Gateway alongside `/health`). This is the
natural fit — Prometheus scraping integrates with existing monitoring
infrastructure (Grafana, Alertmanager) without requiring a push agent. The
endpoint returns metrics in Prometheus text exposition format. Metrics are
per-session-executor (tagged with `session_id`) and per-daemon (global
counters). In system-service mode with per-user workers, each worker exposes
its own `/metrics` endpoint; the system service aggregates or proxies them.

### Health Check

The API Gateway exposes `GET /health` — returns daemon status, uptime,
per-subsystem health (running/degraded/failed), bus queue pressure, and
active session count. In user-service mode, this endpoint is unauthenticated
(only the user's own daemon is accessible). In system-service mode, the
health check returns only aggregate status (no session counts or per-user
data) to unauthenticated callers; detailed health requires the same auth as
other API endpoints. This prevents leaking operational data to arbitrary
local processes.

### Alertable Conditions

The health check endpoint reports degraded status (not just healthy/unhealthy)
for conditions that warrant attention but don't require immediate action:

| Condition | Severity | Trigger |
|-----------|----------|---------|
| `reliable_pressure` > 50% on any subscriber | Warning | Reliable messages accumulating |
| Compat worker restart count > 3 in 60s | Warning | Possible malicious output stream |
| Plugin killed for budget violation | Info | Single occurrence is normal; repeated kills on same plugin = Warning |
| Session persistence write latency > 1s | Warning | Disk I/O degradation |
| Bus dispatch latency p99 > 500µs | Warning | Approaching input latency SLA |
| API Gateway error rate > 10% | Warning | Possible misconfigured agent |
| Worker process OOM killed | Error | Resource limits too tight or memory leak |

Severity levels map to operational responses: Info = log only, Warning =
surface in `malt status` and health check, Error = surface + emit diagnostic
to all connected clients. External alerting integration (PagerDuty, webhook)
is out of scope for the architecture — the health check endpoint is the
integration point.

### Diagnostics Bus Channel

Subsystems emit diagnostic events to the bus (System domain, Diagnostic
message type). These are Low priority — they don't interfere with normal
operation. Diagnostic events include: dropped message counts, budget
violations, persistence failures, isolation setup errors. Connected clients
can subscribe to diagnostics for real-time visibility; the API Gateway
exposes them via `GET /diagnostics`.

---

## 16. Spec Inventory & Build Sequence

Not every component needs a formal spec. Components with external consumers,
wire contracts, or multi-implementation requirements get specs. Internal
implementation details don't.

### Needs a Spec

| Spec | Why | Depends on |
|------|-----|------------|
| **VNP Protocol** | Wire contract. Multiple implementations (Rust daemon, TS browser client, community clients). Message types, framing, envelope, versioning. | Vexil schemas |
| **RenderCommand Protocol** | Public client contract. Community clients implement this. Versioned, stable. | VNP |
| **Session Model** | State machine with defined transitions. Persistence format. Client-visible behavior. | VNP |
| **Plugin ABI** | Third-party developers build against this. WASM host calls, manifest format, hook types, budgets. | VNP, RenderCommand |
| **App SDK** | Third-party developers build against this. App trait, dual-mode runner, FrameElement authoring. | VNP, RenderCommand |
| **API Gateway** | External HTTP/MCP surface. Auth scheme, rate limiting, endpoint contracts, scope rules. **Must include MCP tool/resource mapping** — which MALT concepts map to MCP tools (exec, send), resources (session state, pane content, scrollback), and prompts (structured output summaries). MCP is a primary agent integration surface and must be specced early, not deferred to Phase 5. | VNP, Session Model |

### Does NOT Need a Spec

| Component | Why |
|-----------|-----|
| MASH | POSIX sh is the spec (IEEE 1003.1). Extensions documented in codebase. |
| Layout Engine | Internal subsystem. One implementation. |
| Renderer Host | Internal translation layer. One implementation. |
| Structured Output Parser | Internal. Plugin interface defined in Plugin ABI spec. |
| Process Supervisor | Internal. One implementation. |
| Platform abstractions | Internal traits. One implementation per OS. |
| Daemon core / message bus | Internal architecture. |
| `malt-elevate` IPC | VNP with restricted `elevate.vexil` schema. Internal to malt-platform. |

### Build Sequence

```
Phase 0: Protocol Validation (**PROJECT RISK #1** — blocks everything)
  ├── Vexil + bitpack standalone spec: formal encoding rules, edge cases
  │   (empty optionals, nested schemas, recursive types, max depth,
  │   zero-length payloads, union variant addition, required→optional)
  ├── Schema evolution stress test: add field, add union variant, deprecate
  │   field, cross-version decode (v1 encoder → v2 decoder and reverse)
  ├── Compliance test suite: golden byte vectors for every VNP message type
  ├── Two independent implementations passing the suite (Rust + TypeScript)
  ├── Performance benchmarks: encode/decode throughput vs Cap'n Proto,
  │   FlatBuffers, protobuf on representative VNP messages
  └── Written justification: exactly which property (deterministic encoding
      for BLAKE3 content addressing, bit-identical output) existing formats
      fail, with concrete examples showing the failure. If the justification
      is not compelling, adopt an existing format and layer MALT framing on
      top. A custom format must clear a higher bar than "we prefer it."

Phase 1: Foundation
  ├── VNP Protocol spec (everything communicates through it)
  ├── Vexil schemas for all message types
  └── malt-protocol crate (generated from schemas)

Phase 2: Core
  ├── malt-platform (OS abstractions: PTY, process, signals, fs, sockets)
  ├── malt-platform isolation (4-tier model, platform-specific backends)
  ├── malt-elevate (elevated helper service for privileged operations)
  ├── mash (shell)
  ├── malt-term (line editor)
  ├── malt-tools (utilities)
  └── malt-layout (layout engine)

Phase 3: Daemon
  ├── Session Model spec
  ├── API Gateway spec (auth, rate limiting, endpoint contracts)
  ├── malt-daemon core (bus, scheduler, session store)
  ├── malt-daemon supervisor (process spawning with isolation context)
  ├── malt-daemon groups (session groups, policy enforcement)
  ├── malt-compat (VT translator)
  └── malt-renderer (Renderer Host, RenderTransport trait)

Phase 4: Clients
  ├── RenderCommand Protocol spec
  ├── maltty (GPU terminal)
  ├── malt-tui (TUI terminal)
  └── malt-bin (CLI entry point)

Phase 5: Ecosystem
  ├── Plugin ABI spec
  ├── App SDK spec
  ├── malt-plugin-sdk
  ├── malt-app-sdk
  └── malt-web (browser client)
```

Phase 1 is the critical path. VNP defines the vocabulary. Everything else
implements against it. Phases 2 and 3 can partially overlap — porting shell
and platform abstractions doesn't depend on the daemon being complete.
Isolation is a Phase 2 deliverable (platform primitives) with Phase 3
integration (supervisor applies isolation at spawn, session groups enforce
policy).

---

## 17. Hard Invariants

Non-negotiable rules. Violating any is a bug, not a tradeoff.

1. **VT codes in `malt-compat` only.** No other crate may depend on VT
   parsing libraries. Enforced by `deny.toml`.

2. **OS calls in `malt-platform` only.** No direct use of `nix`,
   `windows-sys`, `libc`, `std::os::unix`, or platform-specific APIs in any
   other crate.

3. **`malt-protocol` has zero workspace dependencies.** Only external deps
   (serde, bytes, vexil-runtime).

4. **`malt-plugin-sdk` and `malt-app-sdk` have minimal workspace deps.** They
   are the public surface for third-party developers. Breaking changes require
   a wire version bump.

5. **All `unsafe` blocks require `// SAFETY:` comments.**

6. **No `unwrap()` or `expect()` in non-test code.** Use `?`, match, or error
   types.

7. **Vexil schemas are the single source of truth for wire types.** No
   hand-written message types on the wire. All VNP message types are generated
   from `.vexil` schema files.

8. **Wire format stability.** Once a wire version is released, encoding is
   immutable. Same message, same field values → bit-identical bytes on every
   platform. Breaking wire changes require a version bump with migration
   protocol.

9. **Input never touches the bus.** Keystrokes flow directly from daemon core
   to the focused pane's owner (MASH, App via VNP, or PTY write fd for
   legacy panes). No bus subscription can observe raw input.

10. **Subsystems communicate through VNP message types.** Whether in-process
    or out-of-process, the contract is the same message vocabulary. One
    exception: the MASH direct input path (Invariant 9) bypasses the bus
    for latency, though it still uses VNP InputEvent types. `malt-elevate`
    also uses VNP, but with a restricted schema (`elevate.vexil`) — the
    full domain system is not required.

11. **Privilege operations are routed, never scattered.** In unprivileged
    mode, all privileged OS operations go through `malt-elevate`. In root
    mode, they go through `malt-platform` directly. No crate outside
    `malt-platform` and `malt-elevate` may perform privileged operations.

### Invariant Enforcement in CI

Invariants are not conventions — they have mechanical enforcement:

| Invariant | CI Enforcement |
|-----------|---------------|
| 1 (VT in malt-compat only) | `deny.toml` bans `vte`, `termwiz`, VT parsing deps in all other crates |
| 2 (OS calls in malt-platform only) | `deny.toml` bans `nix`, `windows-sys`, `libc` in non-platform crates; clippy lint for `std::os::unix` / `std::os::windows` |
| 3 (malt-protocol zero workspace deps) | `cargo metadata` check in CI — script verifies protocol's workspace dep list is empty |
| 4 (SDK minimal deps) | Same `cargo metadata` check for plugin-sdk and app-sdk |
| 5 (unsafe SAFETY comments) | `clippy::undocumented_unsafe_blocks` as deny |
| 6 (no unwrap in non-test) | Custom clippy lint or `#[cfg_attr(not(test), deny(clippy::unwrap_used))]` |
| 7 (Vexil schemas = truth) | Build script: generated types have `#[doc = "GENERATED"]`; CI grep rejects hand-written VNP message structs |
| 8 (wire stability) | Golden file tests: known messages → known bytes, checked into repo |
| 9 (input off bus) | Integration test: subscribe to all bus message types, send keystrokes, assert zero InputEvent messages received |
| 10 (VNP communication) | Architecture review (not automatable — convention enforced by code review) |
| 11 (privilege routing) | `deny.toml` bans privilege-related syscall crates outside malt-platform and malt-elevate |

---

## Appendix A. Platform Parity Matrix

Feature status per platform at v1.0 target. "Full" = complete implementation.
"Partial" = implemented with known limitations. "None" = not available.

| Feature | Linux | Windows | macOS | Notes |
|---------|-------|---------|-------|-------|
| **Bare isolation** | Full | Full | Full | Default on all platforms |
| **Restricted isolation** | Full (PID/mount ns) | Full (Job Objects + restricted tokens) | Full (sandbox-exec profiles) | macOS sandbox-exec is deprecated — tracked as platform risk (see §12) |
| **Capped isolation** | Full (cgroup v2) | Full (extended Job Object limits) | Partial (setrlimit + sandbox-exec) | macOS: per-process limits only, not per-group |
| **Contained isolation** | Full (overlayfs + seccomp + veth) | Partial (HCS, feature-gated, requires Hyper-V) | Partial (Seatbelt only) | Windows HCS: most dev machines lack Hyper-V; AppContainer as fallback |
| **PTY** | Full (native PTY) | Partial (ConPTY) | Full (native PTY) | ConPTY limitations: incomplete VT support, resize races, perf overhead |
| **ConPTY mitigations** | N/A | Test against Windows Terminal's ConPTY; WinPTY fallback for known-broken scenarios | N/A | Tracked as named risk |
| **PTY-preserving upgrade** | Full (exec + fd inheritance) | Partial (DuplicateHandle) | Full (exec + fd inheritance) | Windows: handle duplication failure → fallback to standard restart |
| **Signal handling** | Full (POSIX signals) | Partial (Ctrl-C/Break via ConsoleCtrl; no SIGTSTP; NtSuspendProcess) | Full (POSIX signals) | Windows job suspension uses NtSuspendProcess (not SuspendThread) |
| **Service model: foreground** | Full | Full | Full | |
| **Service model: user service** | Full (systemd --user) | Full (Task Scheduler / user-mode service) | Full (LaunchAgents) | |
| **Service model: system service** | Full (systemd) | Full (Windows Service) | Full (LaunchDaemons) | |
| **Compat Translator out-of-process** | Full (Unix socket IPC) | Full (named pipe IPC) | Full (Unix socket IPC) | |
| **malt-elevate** [1] | Full (Unix socket, SO_PEERCRED, /proc/pid/exe) | Full (named pipe, GetNamedPipeClientProcessId, GetProcessTimes) | Full (Unix socket, SO_PEERCRED, code signature verification via SecCodeCopySigningInformation) | macOS uses code signing instead of /proc/pid/exe |
| **Network isolation (veth/bridge)** | Full | None (Windows networking is different) | None | Windows: network isolation via HCS only (Contained tier) |

[1] This row rates the IPC transport and nonce-auth layer only (`protocol.rs`/`auth.rs`),
which is genuinely complete on all three platforms. It does not mean the privileged
*operations* dispatched over that transport are implemented: as of 2026-07-24, only
`CreateSymlink` does real work in `dispatch.rs` — CreateNamespace, MountOverlay,
SetCgroup, SetupNetns, ApplySeccomp, CreateRestrictedToken, ManageHcsContainer,
ApplySeatbelt, and BindPort all return stub success without performing the operation.

**v1.0 cross-platform launch requirement:** All three platforms must support
Bare + Restricted + foreground + user service + Compat Translator +
malt-elevate. Capped isolation is required on Linux and Windows; best-effort
on macOS. Contained isolation is Linux-only at v1.0 (Windows HCS is
feature-gated experimental). Isolation tier requests that exceed platform
capability fail with an explicit error, not a silent fallback.

---

## Appendix B. Testing Strategy

### Latency Benchmark Suite (CI gate)

A benchmark harness validates the 1ms input latency SLA under load:

- **Configuration:** N active panes (5, 10, 20), M output rate per pane
  (1 KiB/s, 100 KiB/s, 1 MiB/s), K compat panes (0, 3, 5)
- **Measurement:** p50, p95, p99 input-to-MASH delivery latency
- **Gate:** p99 < 1ms with 10 panes at 100 KiB/s output rate
- **Runs in CI** on every commit that touches daemon core, bus, or concurrency
  code

### Compat Worker Chaos Tests

Integration tests simulating each failure mode in the compat worker contract:

- Worker process killed mid-frame → verify pane shows restart message,
  PTY child survives, new worker re-attaches
- Worker hang (no output for 10s with readable PTY) → verify daemon kills
  and restarts
- Worker memory exhaustion → verify cgroup OOM kill and restart
- 5 crashes in 60s → verify pane marked failed, PTY child killed
- VNP connection loss → verify treated as crash, restart + re-attach

### POSIX Conformance Regression Suite

- Smoosh (186 tests) and Modernish suites run in CI on every commit touching
  `mash` crate
- Test count must be monotonically non-decreasing — a commit that causes a
  previously-passing test to fail is rejected
- Baseline score and current score tracked in CI artifacts

### Multi-Client Input Authority Tests

Integration tests for the multi-client state machine:

- Two clients attach simultaneously → verify only one has authority
- Authority holder detaches → verify authority transfers to next client
- Two clients send `input-claim` simultaneously → verify exactly one wins
- Authority holder disconnects uncleanly → verify timeout-based transfer
- API Gateway `POST /exec` while client has authority → verify API always
  gets execution (agents are not observers)

### Daemon Upgrade Chaos Tests

- PTY-preserving upgrade with active sessions → verify fd handoff
- Upgrade with corrupt handoff file → verify fallback to standard restart
- New binary fails to start during exec → verify old daemon state is
  recoverable (sessions persisted before exec attempt)
- Upgrade during active `POST /exec` → verify in-flight request completes
  or fails cleanly (no hang)

### Security Testing

- **MASH fuzz harness:** `cargo-fuzz` targeting expansion, arithmetic, and
  trap handling in session executor context. Verify executor integrity
  post-`catch_unwind`.
- **Compat Translator fuzz:** `cargo-fuzz` targeting VT parser with
  attacker-crafted byte streams. Run both in-process and out-of-process modes.
- **API Gateway:** fuzz `POST /exec` and `POST /send` with malformed payloads,
  oversized bodies, and auth bypass attempts.
- **Plugin Host:** fuzz WASM module loading with malformed manifests and
  adversarial WASM binaries.
- **Threat model document** (separate artifact): covers WASM sandbox escape
  in root mode, malicious VNP messages, compat translator attack surface,
  API Gateway as RCE vector, `malt-elevate` impersonation, multi-user
  system-service isolation.
