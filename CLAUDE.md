# CLAUDE.md

## What is MALT?

MALT (structured terminal platform) inverts the traditional terminal model: the daemon is the authority, not the renderer. The daemon owns session state, layout, pane identity, and structured output. Clients are interchangeable consumers of a typed RenderCommand stream. All inter-component communication uses VNP (Vexil Native Protocol) — typed, schema-defined, bitpack-encoded messages.

**Current state:** Functional end-to-end. Phases 1-4 implemented. 15 crates, ~815 tests. The `malt` command starts a daemon, spawns a shell, and opens an interactive TUI.

## Project Relationships

```
orix/vexil-lang/    Vexil schema language compiler + runtime (v0.1.0)
orix/malt/          This repo — MALT terminal platform
orix/vexil-v2/      Legacy prototype — source for porting
```

MALT depends on `vexil-lang` for schema compilation and `vexil-runtime` for encode/decode. The `vexil-store` crate (in vexil-lang workspace) provides `.vx`/`.vxb` persistence formats.

## Repo Structure

```
specs/
  architecture.md              # Single source of truth (~2,380 lines)
docs/superpowers/
  specs/                       # Sub-project design specs
  plans/                       # Implementation plans
crates/
  malt-protocol/               # VNP types, framing, envelope (34 tests)
  malt-platform/               # PTY, process, signals, sockets (25 tests)
  malt-config/                 # Config loading (5 tests)
  mash/                        # POSIX shell (330 tests)
  malt-term/                   # Line editor (37 tests)
  malt-tools/                  # In-process utilities (40 tests)
  malt-layout/                 # Layout engine (48 tests)
  malt-session/                # Session lifecycle (23 tests)
  malt-elevate/                # Elevated helper (17 tests)
  malt-daemon/                 # Daemon core (73 tests)
  malt-compat/                 # VT emulator (22 tests)
  malt-renderer/               # Renderer host (29 tests)
  malt-gateway/                # HTTP API gateway (18 tests)
  malt-bin/                    # CLI entry point (9 tests)
  malt-tui/                    # TUI terminal client (12 tests)
schemas/
  *.vexil                      # VNP message schemas (15 files)
  persist/                     # Persistence schemas
```

## Crate Architecture (strict layering — no upward deps)

```
L0  malt-platform     OS abstractions: PTY, process, signals, sockets, spawn_with_pty
L0  malt-config       Vexil Store config: typed structs
L0  malt-protocol     VNP spine: all shared types, framing, envelope, encode/decode

L1  mash              POSIX shell: lexer, parser, expander, executor, builtins
L1  malt-term         Line editor: vi/emacs, completion, history, multiline
L1  malt-tools        In-process POSIX utilities (cat, env, which, grep, wc)
L1  malt-layout       Layout engine: n-ary LayoutNode tree, resolution, directional focus
L1  malt-session      Session lifecycle, command block ring buffer, group management

L2  malt-compat       VT emulator (vte 0.15) — ONLY VT code in entire system
L2  malt-renderer     Renderer Host: FrameElement walker → RenderCommand emission
L2  malt-daemon       Daemon core: message bus, session-sharded executor, session store,
                      process supervisor, VNP listener, gateway backend
L2  malt-gateway      HTTP REST API: axum routes, auth, rate limiting, shadow tree

L3  malt-bin          CLI entry point (`malt` command): clap + reqwest

    malt-elevate      Elevated helper (standalone binary, outside layer system)

Clients:
    malt-tui           TUI terminal: ratatui 0.30 + crossterm 0.29
```

## Hard Invariants

These are non-negotiable. Violating any is a bug.

1. **VT codes in `malt-compat` only.** No other crate may import `vte` or handle escape sequences.
2. **OS calls in `malt-platform` only.** No `nix`, `windows-sys`, `libc`, `std::os::unix` elsewhere.
   - Known violations: mash (isatty), malt-tools (PermissionsExt), malt-daemon (FromRawHandle). To be fixed.
3. **`malt-protocol` is dependency-free within workspace.** Only external deps.
4. **`malt-plugin-sdk` and `malt-app-sdk` have zero internal deps.** Published standalone. (Not yet implemented.)
5. **All `unsafe` blocks require `// SAFETY:` comments.**
6. **No `unwrap()` or `expect()` in non-test code.** Use `?`, match, or error types.
   - Known violations in mash (parser, env). To be fixed.
7. **VNP is the only inter-component protocol.** Post-handshake uses JSON-in-frames as interim.
8. **Shell ships when POSIX conformance suite passes.** Smoosh (186 tests) + Modernish.
9. **Layer violations are compile errors.** No upward dependencies in the crate graph.

## What's Implemented

### Daemon (malt-daemon)
- **Message bus:** Per-session synchronous dispatcher, bounded ring buffers, priority eviction (Low→Normal→High), Reliable never evicted, Critical separate inbox, flow control (25% per-producer, 60% global)
- **Session-sharded executor:** Per-session thread with own bus, authority tracker, compat translator, layout recomputation
- **Coordinator:** Session lifecycle (create/destroy), message routing, PTY writer management, output retrieval
- **Process supervisor:** spawn_with_pty, kill, check_exited, resize, take_io
- **Session store:** Bitpack persistence (Pack/Unpack), atomic writes (temp+rename), save/load/list/delete
- **VNP listener:** TCP socket listener, VNP handshake, JSON-in-frames message loop, output streaming
- **Gateway backend:** GatewayBackend impl wrapping Coordinator, exec/send/output wired to real PTY
- **Connection handshake:** Hello/HelloAck, version negotiation, VersionSkew rejection

### VT Emulator (malt-compat)
- Cell/Row grid, TerminalGrid (cursor, scroll, erase, resize)
- VT parser via vte 0.15 (SGR colors/attributes, cursor movement, erase sequences)
- CompatTranslator: feed bytes → VtPassthrough FrameElement

### Renderer (malt-renderer)
- FrameWalker: tree traversal with depth limit (64), node limit (10K), capability degradation
- DirtyTracker: frame diffing for delta-only emission
- ClientState: frame_seq, lagging detection (30 frames), shedding (10s timeout)
- RendererHost: pipeline orchestration, per-client state, InitialState snapshots

### API Gateway (malt-gateway)
- HTTP REST: axum 0.8, 11 endpoints (health, sessions CRUD, exec/send/output, panes)
- GatewayBackend trait (extractable subsystem)
- Auth scopes (Monitor < Read < Interact < Admin)
- Token bucket rate limiter
- Shadow tree: FrameElement → semantic JSON

### CLI (malt-bin)
- `malt` — auto: start daemon → create session → attach TUI
- `malt daemon` — run daemon foreground (HTTP on port, VNP on port+1)
- `malt start/stop` — background daemon lifecycle
- `malt status/list/new/kill` — session management
- `malt exec/send` — command execution
- `malt attach` — launch malt-tui with VNP connection

### TUI Client (malt-tui)
- Style converter: ResolvedStyle → ratatui::Style (RGB colors + 7 modifiers)
- TuiRenderer: RenderCommand → ratatui Buffer (DrawText, DrawRect, Clear, clip)
- Input handler: crossterm → VNP input events
- HttpConnection: polls /output, sends /send (fallback)
- VnpConnection: VNP framed socket, styled output parsing
- App event loop with Ctrl+Q quit

## What's NOT Implemented (vs architecture spec)

### High Priority
- **MASH integration:** System shell used, not mash. Need to instantiate mash per session.
- **Full VNP message routing:** Post-handshake uses JSON-in-frames, not bitpack VNP messages.
- **ConPTY attachment:** Windows uses piped stdio workaround. Need PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.

### Medium Priority
- **MCP protocol:** No Model Context Protocol transport for AI agents.
- **WASM plugin host:** No plugin loading, manifests, fuel budgets.
- **Remote auth:** Local only. No TLS, TOFU, OAuth, API keys.
- **Isolation enforcement:** Tiers defined but not enforced at spawn time.
- **Structured output parser:** No command output recognition or parsing.
- **Out-of-process compat worker:** In-process only (no worker binary for Restricted+).

### Lower Priority
- **OutputChunk batching:** No adaptive batching or 16ms tick.
- **Observability:** Minimal tracing, no metrics/Prometheus/health diagnostics.
- **Named layout presets:** Layout engine works but no preset save/load.
- **maltty GPU terminal:** Not started (wgpu + cosmic-text + winit).
- **malt-web browser client:** Not started (SvelteKit).

## Persistence Format

Session persistence uses **vexil-runtime bitpack** (Pack/Unpack traits):
- `.vxb` — binary bitpack format (currently used)
- `.vx` — human-readable text format (deferred pending vexil-store typed API)
- Persistence schemas in `schemas/persist/` (PersistedSession, PersistedPane, DaemonState)
- Atomic writes: temp file + rename

## Platform Support

- **Windows:** ConPTY for PTY (piped stdio workaround for child attachment). Named pipes defined but TCP used for VNP.
- **Linux:** openpty + nix for PTY. Unix sockets defined but TCP used for VNP.
- **macOS:** Same as Linux.

## Code Standards

- `thiserror` for library errors; `anyhow` in `malt-bin` and `malt-tui` only.
- `tracing` for all logging. No `eprintln!`/`println!` in library crates.
- `#[non_exhaustive]` on public enums that will grow.
- `#[derive(Debug, Clone, PartialEq)]` on data types.
- Explicit re-exports only — no `pub use foo::*`.
- Rust edition 2021.
- `vte` 0.15, `axum` 0.8, `ratatui` 0.30, `crossterm` 0.29, `clap` 4, `reqwest` 0.13.

## Key Architecture Decisions

- **Daemon-as-authority:** daemon owns all state, clients are display consumers
- **Session-sharded executor:** each session gets its own thread; sessions cannot block each other
- **Bus flow control:** per-producer 25% cap + global 60% saturation guard
- **Input direct path:** input bypasses bus for latency (routed directly to focused pane)
- **Styled grid output:** compat translator maintains styled cells, serialized as spans with RGB
- **GatewayBackend trait:** gateway is extractable subsystem, no direct daemon dependency
- **spawn_with_pty:** platform-level API combining PTY allocation + child process attachment

## Type Notes (for implementers)

- FrameElement/RenderCommand **union variants** have NO `_unknown` field
- Message structs (RenderBatch, InitialState, etc.) DO have `_unknown: Vec<u8>`
- `style` in FrameElement::Text/Paragraph is `Box<ResolvedStyle>`
- `direction` in FrameElement::Split is `Box<Direction>`
- `child` fields are `Box<FrameElement>`, `fallback` is `Option<Box<FrameElement>>`
- `rgb` type = `(u8, u8, u8)` tuple in generated code
- `PaneId(u32)`, `SessionId(u32)` — NOT Copy, use `.clone()`
- `encode_message`/`encode_envelope` return `Result` (not infallible)

## Testing

- 84 test suites, ~815 tests, zero failures
- Tests use `tempfile::tempdir()` for file I/O isolation
- VNP listener tests use random ports (`bind("127.0.0.1:0")`)
- Supervisor tests spawn real processes (echo, sleep/ping)
- Gateway route tests use mock backend with `tower::ServiceExt::oneshot`
