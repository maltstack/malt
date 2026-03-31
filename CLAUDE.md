# CLAUDE.md

## What is MALT?

MALT (structured terminal platform) inverts the traditional terminal model: the daemon is the authority, not the renderer. The daemon owns session state, layout, pane identity, and structured output. Clients are interchangeable consumers of a typed RenderCommand stream. All inter-component communication uses VNP (Vexil Native Protocol) — typed, schema-defined, bitpack-encoded messages.

**Current state:** All phases implemented. 18 Rust crates + 1 SvelteKit web client. ~830 tests across 88 test suites. The `malt` command starts a daemon with an in-process mash (POSIX shell), serves HTTP + VNP socket APIs, and opens an interactive TUI. MCP server available for AI agents. WASM plugin host ready. GPU terminal (maltty) scaffolded.

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
  specs/                       # Sub-project design specs (3A-3F, 4.1-4.2, integration)
  plans/                       # Implementation plans
crates/
  malt-protocol/               # L0: VNP types, framing, envelope (34 tests)
  malt-platform/               # L0: PTY, process, signals, sockets (25 tests)
  malt-config/                 # L0: Config loading (5 tests)
  mash/                        # L1: POSIX shell — lexer, parser, expander, executor (330 tests)
  malt-term/                   # L1: Line editor — vi/emacs, completion, history (37 tests)
  malt-tools/                  # L1: In-process POSIX utilities (40 tests)
  malt-layout/                 # L1: Layout engine — n-ary tree, resolution, focus (48 tests)
  malt-session/                # L1: Session lifecycle, pane runtime, groups (23 tests)
  malt-elevate/                # Elevated helper binary (17 tests)
  malt-daemon/                 # L2: Daemon core (75 tests)
  malt-compat/                 # L2: VT emulator — vte 0.15 parser + grid (22 tests)
  malt-renderer/               # L2: Renderer host — walker, dirty, client state (29 tests)
  malt-gateway/                # L2: HTTP API — axum, auth, rate limiting (22 tests)
  malt-plugin-sdk/             # L3: WASM plugin host — wasmtime, fuel budgets (5 tests)
  malt-mcp/                    # L3: MCP server for AI agents (6 tests)
  malt-bin/                    # L3: CLI entry point — `malt` command (9 tests)
  malt-tui/                    # Client: TUI terminal — ratatui + crossterm (12 tests)
  maltty/                      # Client: GPU terminal — wgpu + winit (scaffold)
clients/
  malt-web/                    # Client: Browser terminal — SvelteKit MVP
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
                      process supervisor, VNP listener, gateway backend, mash integration
L2  malt-gateway      HTTP REST API: axum routes, auth, rate limiting, shadow tree

L3  malt-plugin-sdk   WASM plugin host: wasmtime, fuel budgets, manifests (zero internal deps)
L3  malt-mcp          MCP server: AI agent tools over stdio JSON-RPC
L3  malt-bin          CLI entry point (`malt` command): clap + reqwest

    malt-elevate      Elevated helper (standalone binary, outside layer system)

Clients:
    malt-tui           TUI terminal: ratatui 0.30 + crossterm 0.29
    maltty             GPU terminal: wgpu 24 + winit 0.30 + cosmic-text 0.12
    malt-web           Browser terminal: SvelteKit 2 + Svelte 5
```

## Hard Invariants

These are non-negotiable. Violating any is a bug.

1. **VT codes in `malt-compat` only.** No other crate may import `vte` or handle escape sequences. ✅ Clean.
2. **OS calls in `malt-platform` only.** No `nix`, `windows-sys`, `libc`, `std::os::unix` elsewhere.
   - ⚠️ Known violations: mash (`isatty`), malt-tools (`PermissionsExt`), malt-daemon (`FromRawHandle`). Fix pending.
3. **`malt-protocol` is dependency-free within workspace.** Only external deps. ✅ Clean.
4. **`malt-plugin-sdk` has zero internal deps.** ✅ Clean — only external deps (wasmtime, serde, thiserror).
5. **All `unsafe` blocks require `// SAFETY:` comments.** ✅ Clean (one minor gap in mash).
6. **No `unwrap()` or `expect()` in non-test code.** ⚠️ Known violations in mash (parser, env). Fix pending.
7. **VNP is the only inter-component protocol.** ⚠️ Post-handshake uses JSON-in-frames as interim. Full VNP bitpack pending.
8. **Shell ships when POSIX conformance suite passes.** Smoosh (186 tests) + Modernish. Not yet wired to CI.
9. **Layer violations are compile errors.** No upward dependencies in the crate graph. ✅ Clean.

## CLI Commands (all working)

```
malt                          # Auto: start daemon → create session → attach TUI
malt daemon [--port N]        # Run daemon foreground (HTTP on port, VNP on port+1)
malt start                    # Start daemon in background
malt stop                     # Graceful shutdown via /shutdown endpoint
malt status                   # Show daemon health + session list
malt new [--name N]           # Create session (spawns in-process mash shell)
malt list                     # List sessions
malt attach [ID]              # Open TUI connected to session (VNP + HTTP fallback)
malt exec ID "command"        # Run command via mash, return output
malt send ID "input"          # Send raw input to session
malt kill ID                  # Destroy session
```

## What's Implemented

### Daemon (malt-daemon)
- **Message bus:** Per-session synchronous dispatcher, bounded ring buffers, priority eviction (Low→Normal→High), Reliable never evicted, Critical separate inbox, flow control (25% per-producer, 60% global)
- **Session-sharded executor:** Per-session thread with own bus, authority tracker, compat translator, mash shell environment
- **Mash integration:** In-process POSIX shell — parse → execute_list → capture stdout/stderr → feed compat translator
- **Coordinator:** Session lifecycle, message routing, output retrieval via reply channels
- **Process supervisor:** spawn_with_pty, kill, check_exited, resize (for future compat pane processes)
- **Session store:** Bitpack persistence (Pack/Unpack), atomic writes (temp+rename), save/load/list/delete
- **VNP listener:** TCP socket on port+1, VNP handshake, JSON-in-frames message loop, output streaming
- **Gateway backend:** GatewayBackend impl wrapping Coordinator, exec via RunCommand with reply channel

### VT Emulator (malt-compat)
- Cell/Row grid, TerminalGrid (cursor, scroll, erase, resize)
- VT parser via vte 0.15 (SGR colors/attributes, cursor movement, erase, scroll)
- CompatTranslator: feed bytes → VtPassthrough FrameElement

### Renderer (malt-renderer)
- FrameWalker: tree traversal, depth limit (64), node limit (10K), capability degradation (TrueColor/Basic256/None)
- DirtyTracker: frame diffing for delta-only emission
- ClientState: frame_seq, lagging detection (30 frames), shedding (10s timeout)
- RendererHost: pipeline orchestration, per-client state, InitialState snapshots

### API Gateway (malt-gateway)
- HTTP REST: axum 0.8, 11 endpoints + /shutdown
- GatewayBackend trait (extractable subsystem)
- Auth: scopes (Monitor < Read < Interact < Admin), TokenStore with bearer tokens, token file persistence
- Token bucket rate limiter
- Shadow tree: FrameElement → semantic JSON (styled spans with RGB)

### MCP Server (malt-mcp)
- JSON-RPC 2.0 over stdio
- 6 tools: list_sessions, create_session, run_command, get_output, send_input, destroy_session
- Delegates to daemon HTTP API

### WASM Plugin Host (malt-plugin-sdk)
- wasmtime 29 integration with fuel metering
- PluginManifest (JSON): name, version, permissions, hooks, fuel_budget
- PluginHost: load/unload/call with fuel budgets
- Zero internal workspace dependencies

### Clients
- **malt-bin:** clap CLI with auto-workflow (start daemon → create session → attach TUI)
- **malt-tui:** ratatui rendering (DrawText, DrawRect, Clear, clip), crossterm input, HttpConnection + VnpConnection, styled output with RGB colors
- **maltty:** wgpu window, surface setup, dark background clear, daemon HTTP connection, input mapping (scaffold — text rendering pending)
- **malt-web:** SvelteKit MVP, styled span grid rendering, keyboard input, session polling

## What's NOT Implemented (remaining work)

### High Priority
- **cosmic-text glyph rendering in maltty** — GPU terminal opens but shows black screen. Needs glyph atlas, quad instancing, shader pipeline.
- **Full VNP bitpack messages** — Post-handshake communication uses JSON-in-frames. Should use real VNP envelope + bitpack encode/decode for all message types.
- **OS-call invariant fixes** — Move `isatty`, `PermissionsExt`, `FromRawHandle` from mash/malt-tools/malt-daemon into malt-platform.
- **unwrap/expect cleanup** — Fix remaining violations in mash parser and env modules.

### Medium Priority
- **Structured output parser** — Recognize command output patterns (cargo build, git status, etc.) and emit StructuredOutput messages for semantic rendering.
- **Isolation enforcement** — IsolationTier is defined but not enforced. Need to actually apply namespaces/cgroups/Job Objects/Seatbelt at process spawn.
- **malt-app-sdk** — App trait with dual-mode runner (VNP connected + standalone ratatui fallback). Needed for third-party terminal apps.
- **Observability** — Metrics registry, Prometheus `/metrics` endpoint, structured logging with tracing fields.
- **POSIX conformance CI** — Wire smoosh (186 tests) + Modernish test suite into CI pipeline.

### Lower Priority
- **Out-of-process compat worker** — malt-compat-worker binary for Restricted+ isolation tiers. Heartbeat, crash recovery, memory limits.
- **Named layout presets** — Save/load layout configurations from .vx files.
- **OutputChunk batching** — Adaptive batching with 8-chunk threshold and 16ms tick.
- **malt-elevate integration** — Wire elevated helper into daemon spawn flow for privileged operations.
- **Remote TLS** — rustls integration, self-signed certs with TOFU, CA-signed for shared infra.
- **Config file loading** — malt-config currently returns defaults. Wire actual .vx file parsing when vexil-store typed API matures.

## Code Standards

- `thiserror` for library errors; `anyhow` in binary crates only.
- `tracing` for all logging. No `eprintln!`/`println!` in library crates.
- `#[non_exhaustive]` on public enums that will grow.
- `#[derive(Debug, Clone, PartialEq)]` on data types.
- Explicit re-exports only — no `pub use foo::*`.
- Rust edition 2021.
- Key deps: `vte` 0.15, `axum` 0.8, `ratatui` 0.30, `crossterm` 0.29, `clap` 4, `reqwest` 0.13, `wasmtime` 29, `wgpu` 24, `winit` 0.30.

## Type Notes (for implementers)

- FrameElement/RenderCommand **union variants** have NO `_unknown` field
- Message structs (RenderBatch, InitialState, etc.) DO have `_unknown: Vec<u8>`
- `style` in FrameElement::Text/Paragraph is `Box<ResolvedStyle>`
- `direction` in FrameElement::Split is `Box<Direction>`
- `child` fields are `Box<FrameElement>`, `fallback` is `Option<Box<FrameElement>>`
- `rgb` type = `(u8, u8, u8)` tuple in generated code
- `PaneId(u32)`, `SessionId(u32)` — NOT Copy, use `.clone()`
- `encode_message`/`encode_envelope` return `Result` (not infallible)
- mash `Env` created per session thread via `Env::from_os()` + `set_interactive(true)`
- mash `execute_list` is synchronous — perfect for session executor's thread model

## Testing

- 88 test suites, ~830 tests, zero failures
- Tests use `tempfile::tempdir()` for file I/O isolation
- VNP listener tests use random ports (`bind("127.0.0.1:0")`)
- Supervisor tests spawn real processes (echo, sleep/ping)
- Gateway route tests use mock backend with `tower::ServiceExt::oneshot`
- Plugin SDK tests use `wat` crate for minimal WASM modules
- No tests requiring GPU (maltty build-only verification)
