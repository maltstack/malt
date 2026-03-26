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

1. **Thin coordinator, specialized subsystems** — The daemon core is a
   scheduler, priority manager, and message router. It delegates all
   domain-specific work (rendering, parsing, process supervision, shell
   execution) to independent subsystems. No subsystem accumulates unrelated
   responsibilities.

2. **VNP is the universal contract** — All components communicate through
   Vexil Native Protocol messages — typed, schema-defined, deterministic.
   In-process subsystems use the same message types as remote clients; the
   only difference is whether bytes cross a process boundary.

3. **Vexil is the schema authority** — Every VNP message type is a Vexil
   schema. Wire encoding is bitpack (deterministic, LSB-first). JSON exists
   only for debugging. Schema identity via BLAKE3 hash. No hand-written wire
   types, no encoding ambiguity.

4. **The protocol is the product** — Any process that speaks VNP is a valid
   participant. Official clients define the quality bar, but community
   clients, IDE plugins, mobile apps, and automation tools consume the same
   protocol. Extensibility comes from the wire format being public and
   documented, not from internal APIs.

5. **Subsystems are extractable** — Every subsystem is designed so it can run
   in-process (zero-copy, direct calls) or out-of-process (VNP over
   transport) without changing its interface. The deployment topology is a
   runtime decision, not an architectural one.

---

## 2. System Architecture

The daemon core runs a central message bus. Subsystems subscribe to message
types they handle. The daemon dispatches messages based on type, routing
metadata, and priority class. Subsystems are independent — they don't know
about each other, they know about message types.

**MASH has a direct input path.** Keystrokes flow from client → daemon core →
MASH with no bus intermediary. This is the latency-critical hot path. MASH's
output (CommandStarted, OutputChunk, PromptReady, etc.) flows through the bus
normally, where other subsystems subscribe.

```
Client → InputEvent → daemon core ──direct──→ MASH
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

### Subsystems

| Subsystem | Responsibility | Process Model |
|-----------|---------------|---------------|
| **MASH** | POSIX shell, line editor, prompt state, command lifecycle. Direct input path from daemon core. | In-process, privileged |
| **Renderer Host** | FrameElement → RenderCommand translation, dirty tracking, capability degradation, theme resolution | In-process |
| **Structured Output Parser** | Recognizes command output, dispatches to parsers (built-in + WASM plugins), produces StructuredOutput | In-process |
| **Process Supervisor** | Spawns/monitors/restarts app and compat processes, resource tracking, cleanup | In-process |
| **Compat Translator** | PTY output → VNP messages. Only component that touches VT escape codes | In-process |
| **Layout Engine** | Resolves abstract LayoutNode tree → concrete ResolvedPane positions. Pure computation. | In-process |
| **Plugin Host** | WASM runtime, sandbox, permission enforcement, plugin lifecycle | In-process |
| **API Gateway** | HTTP/MCP external surface for agents, CI, scripts, automation | In-process, extractable |
| **Session Store** | Persistence, session lifecycle, state serialization | In-process |

### Priority Classes

| Priority | Message types | Rationale |
|----------|--------------|-----------|
| Critical | Resize, Signal | Immediate system response |
| High | RenderCommand, FrameElement | Visual responsiveness |
| Normal | OutputChunk, CommandStarted/Finished, StructuredOutput | Data flow |
| Low | SessionPersist, PluginEvent, Telemetry | Background work |

Note: InputEvent is not on the bus — it flows directly to MASH.

### Message Flow Example — User Types `cargo build`

```
1. Client sends InputEvent → daemon core → MASH (direct)
2. MASH processes keystrokes via line editor
3. User hits Enter → MASH emits CommandStarted{cmd: "cargo build"} → bus
4. Process Supervisor spawns cargo (subscribed to process lifecycle messages)
5. cargo output → MASH emits OutputChunk → bus
6. Structured Output Parser picks up OutputChunk, recognizes cargo format
7. Parser emits StructuredOutput{kind: CargoBuild} → bus
8. Renderer Host picks up StructuredOutput, produces FrameElement tree
9. Renderer Host walks tree → emits RenderCommand delta to connected clients
10. Clients rasterize and display
```

---

## 3. Rendering Pipeline

The rendering pipeline has three stages, owned by three different layers.

### Stage 1 — Authoring (content sources)

Content sources produce FrameElement trees. These are semantic UI descriptions
— "a table with these columns", "a diff with these hunks", "styled text".
Content sources include:

- MASH (prompt, command output, structured output widgets)
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
traversal. Always produces intuitive results regardless of layout complexity.

**Layout operations** are messages on the bus: SplitPane, ClosePane,
FloatPane, SwapPanes, ResizeSplit, FocusDirection, SaveLayout, LoadLayout,
etc. The daemon core routes them to the layout engine, which recomputes and
emits updated ResolvedPane[] back to the bus. Renderer Host subscribes and
redraws.

**Persistence:** LayoutNode trees are serializable. Named layout presets
stored as TOML files. Sessions persist their layout tree — pixel positions are
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
- Output emission (OutputChunk messages to the bus)

### What MASH Does NOT Own

- Output parsing — the Structured Output Parser subsystem handles this
- Process spawning for apps/compat — the Process Supervisor handles this
- Rendering — MASH emits text and metadata, never FrameElements directly

### Privileged Status Means

- Direct input path, no bus latency on keystrokes
- Daemon core understands prompt state (can answer "is the shell idle?"
  without a bus round-trip)
- In-process, zero-copy message passing

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

---

## 6. VNP — Vexil Native Protocol

VNP is the typed message protocol that replaces escape codes. Every
inter-component message is a Vexil schema, encoded with bitpack, framed for
transport.

### Three Layers

1. **Framing** — 4-byte LE length prefix + 1-byte flags + payload. Max 16 MiB
   per frame. Flags encode: compression (zstd), encoding format
   (bitpack/JSON), priority bit.

2. **Envelope** — Bit-packed header (8-12 bytes): protocol version (4 bits),
   domain (3 bits, routing hint), message type (7 bits), timestamp (48-bit
   microsecond), optional message ID (32-bit). Compact, deterministic,
   sufficient for 128 message types across 8 domains.

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
2. Client sends Hello (protocol version, client type, capabilities)
3. Daemon responds HelloAck (negotiated version, session list)
4. Client sends AttachSession or CreateSession
5. Bidirectional message flow begins

### Version Negotiation

Both sides declare max supported wire version. They operate at the lower of
the two. Wire versions are additive — v2 is a superset of v1. Same message,
same field values → bit-identical bytes across all versions that include it.

### Transport Agnostic

VNP defines framing and messages, not transport. Any reliable ordered byte
stream works:

- Named pipes (Windows)
- Unix sockets (Linux/macOS)
- TCP (remote/Tailscale)
- WebSocket (browser)
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
FrameElement (can change any release)
        │
   Renderer Host
        │
RenderCommand (wire-stable, versioned, breaking changes require wire version bump)
```

This is the contract that makes community clients viable. They implement
against RenderCommand, not FrameElement. Internal rendering improvements — new
widget types, better diffing, smarter degradation — never break external
clients.

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
- Native apps run with OS-level process isolation
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
- Output retrieval (raw text, structured output)
- Pane management (create, close, list, read content)
- Layout control (split, resize, focus)
- Status queries (is shell idle, running command, session state)

### Prompt-Aware Execution

Agents don't poll or scrape. The API Gateway understands prompt state because
MASH emits PromptReady to the bus. The gateway exposes this as:

- `POST /exec` — blocking: sends command, waits for PromptReady, returns
  output + exit code
- `POST /send` + `GET /await` — interactive: send input, then poll or
  long-poll for prompt readiness

### Authentication

Local connections (named pipe, Unix socket) authenticate via OS-level peer
credentials — same user, no token needed. Remote connections (TCP, WebSocket)
require token-based authentication. The specific auth scheme is defined in the
API Gateway spec, not the architecture.

### Extractability

The API Gateway is designed as an extractable subsystem. It communicates with
the daemon exclusively through bus messages. If load or isolation requirements
demand it, it can move to a separate process that connects to the daemon via
VNP transport — no interface changes.

---

## 10. Compat Translator

The Compat Translator is the boundary where legacy terminal programs enter the
MALT world. It is the only component in the entire system that handles VT
escape codes.

### What It Does

1. Reads raw byte output from a PTY (spawned by the Process Supervisor)
2. Parses VT escape sequences (cursor movement, color, screen manipulation)
3. Maintains a virtual terminal grid state
4. Emits VNP messages: VtPassthrough FrameElements for Renderer Host, or
   translated equivalents where possible

### What It Does NOT Do

- Spawn processes (Process Supervisor does that)
- Render anything (Renderer Host and clients do that)
- Interact with the shell (MASH handles all shell concerns)

**One instance per legacy pane.** Each PTY-backed pane gets its own Compat
Translator instance. They're lightweight — a VT state machine and a grid
buffer.

**Enforcement:** The VT parsing library is a dependency of the compat crate
only. A workspace-level `deny.toml` rule bans VT parsing dependencies in all
other crates. This is a hard invariant, not a convention.

**Client support for WriteRaw:** Renderer Host emits
`WriteRaw { data, rect }` RenderCommands for VtPassthrough content. Clients
that support VT (maltty, malt-tui) render it natively. Clients that don't
skip it. Legacy program support gracefully degrades per client capability —
the architecture never forces VT knowledge on any client.

---

## 11. Crate Architecture

Strict layering — no upward dependencies. Each layer depends only on layers
below it.

```
L0  malt-platform        OS abstractions: PTY, process, signals, fs, sockets,
                         isolation (4 tiers: Bare → Restricted → Capped → Contained),
                         HCS bindings (Windows, feature-gated), cgroup v2 (Linux),
                         seatbelt (macOS), seccomp BPF, network (veth/bridge)

L0  malt-config          TOML config: typed structs, validated at load time

L0  malt-protocol        VNP: all message types (Vexil-generated), framing, envelope

L1  mash                 POSIX shell: lexer, parser, expander, executor, builtins,
                         job control, VNP integration for structured output

L1  malt-term            Line editor: vi/emacs modes, multiline, completion (fs/history/
                         command-specific), syntax highlighting, prompt renderer, history

L1  malt-tools           In-process POSIX utilities: echo, cat, ls, grep, sed, awk,
                         find, kill, ps, ln, which, env, pwd + uutils-backed (head,
                         tail, sort, uniq, cut, etc.)

L1  malt-layout          Layout engine: n-ary LayoutNode tree, resolution, layout ops,
                         directional focus, persistence

L1  malt-session         Core session/pane types: PaneState, SessionState, PaneKind,
                         command block ring buffer. Shared between daemon and protocol.

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
L3  malt-bin             CLI entry point (`malt` command, daemon management)
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

### Dependency Rules

- `malt-protocol` has zero workspace dependencies — only external deps (serde,
  bytes, vexil-runtime)
- `mash` depends on `malt-platform` traits only — never on daemon or protocol
  internals
- `malt-compat` is the only crate permitted to depend on VT parsing libraries
  (enforced by `deny.toml`)
- `malt-plugin-sdk` and `malt-app-sdk` are the public surface for third-party
  developers
- Clients depend on `malt-protocol` and `malt-platform` — never on daemon
  internals

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
network modes, and seccomp support.

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
(SIGSTOP/SuspendThread), resume, kill, checkpoint. This enables:

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
    cwd:       PathBuf
    title:     Option<String>
    pane_type: Shell { shell_path } | App { app_id, config } | Compat { program, args }
}
```

### What's NOT Persisted

Terminal dimensions, resolved positions, texture atlases, scroll offsets,
active connections, client identity, animation state, process handles.

### Session Lifecycle

```
CreateSession → Active → Detach → Dormant → Attach → Active
                  │                   │
                  ├── Destroy ────────┤
                  │                   │
                  └── Checkpoint ─────┘ (if isolation tier supports it)
```

- **Active** — at least one client attached, panes running
- **Dormant** — no clients attached, processes may be suspended (depends on
  policy), state persisted
- **Checkpoint** — full state snapshot including isolation environment
  (Contained tier only, where supported)
- **Destroy** — cleanup, processes killed, state removed

**Named layout presets** are stored as TOML files. Users save and load
workspace configurations independently of session lifecycle. A preset defines
structure (splits, tabs, pane types) but not runtime state.

**The daemon is the truth.** Any client can attach to any session and get a
fully restored workspace. Client crashes lose nothing. Daemon restart restores
from persisted state.

---

## 14. Security Model

Security properties emerge from architectural decisions, not bolted-on checks.

### Input Isolation

Keystrokes flow directly from client → daemon core → MASH. They never touch
the message bus. WASM plugins and other subsystems cannot observe raw input. A
malicious plugin can subscribe to shell output events (CommandStarted,
OutputChunk) but never intercept keystrokes, passwords, or interactive input.
Input interception would require explicit daemon-level permission granted at a
layer above the bus.

### Plugin Sandboxing

WASM plugins run in a sandbox with no implicit capabilities. No filesystem, no
network, no process spawning unless explicitly declared in the manifest and
approved by the user. Execution budgets enforce latency limits — a
misbehaving plugin is killed, not allowed to stall the system. Manifest
declarations are the permission boundary; runtime enforcement is the Plugin
Host's responsibility.

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

### Wire Format Integrity

Vexil bitpack encoding is deterministic. Same message, same field values →
bit-identical bytes. This enables content-addressed message identity (BLAKE3
hash), replay detection, and cross-platform verification.

---

## 15. Spec Inventory & Build Sequence

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

### Build Sequence

```
Phase 1: Foundation
  ├── VNP Protocol spec (everything communicates through it)
  ├── Vexil schemas for all message types
  └── malt-protocol crate (generated from schemas)

Phase 2: Core
  ├── malt-platform (port OS abstractions)
  ├── mash (port shell)
  ├── malt-term (port line editor)
  ├── malt-tools (port utilities)
  └── malt-layout (port layout engine)

Phase 3: Daemon
  ├── Session Model spec
  ├── malt-daemon (bus, session store, process supervisor)
  ├── malt-compat (port VT translator)
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

---

## 16. Hard Invariants

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
   to MASH. No bus subscription can observe raw input.

10. **Subsystems communicate through VNP message types.** No ad-hoc
    inter-subsystem channels. Whether in-process or out-of-process, the
    contract is the same message vocabulary.
