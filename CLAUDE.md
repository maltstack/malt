# CLAUDE.md

## Engineering Standards

**No time constraints. No deadlines. Every line of code is production-ready and mission-critical.**

- No stubs, no `todo!()`, no `unimplemented!()`, no placeholder logic
- No hand-waving ("this could be improved later", "for now we just...")
- No deferring — if a component is needed, implement it fully
- No shortcuts that would need to be revisited
- No half-measures: if the architecture specifies X, implement X completely
- All error paths handled — no silent ignores, no bare `unwrap()` outside tests
- All tests pass before any task is marked complete

This is the standard for every change, regardless of scope.


## What is MALT?

MALT (structured terminal platform) inverts the traditional terminal model: the daemon is the authority, not the renderer. The daemon owns session state, layout, pane identity, and structured output. Clients are interchangeable consumers of a typed RenderCommand stream. All inter-component communication uses VNP (Vexil Native Protocol) — typed, schema-defined, bitpack-encoded messages.

**Current state:** Foundation + Phase A + Phase B1 implemented. 18 Rust crates + 1 SvelteKit web client. 1,200+ tests, all green (verified 2026-07-24 after a 3.5-month dormancy). The `malt` command starts a daemon with an in-process mash (POSIX shell), serves HTTP + VNP socket APIs, and opens an interactive TUI. Full VNP bitpack encoding used end-to-end (no JSON post-handshake). Real `.vx` config file loading wired. Session store hardened (.bak backup, corruption quarantine, debounced 1s flush, XDG-compliant data dir, counter restore on startup). MCP server available for AI agents. WASM plugin host ready. GPU terminal (maltty) scaffolded. Many architecture spec features remain unimplemented — see Implementation Roadmap below.

## Project Relationships

```
orix/vexil-lang/    Vexil schema language compiler + runtime — actively developed, MALT's build dependency
orix/malt/          This repo — MALT terminal platform
orix/vexil-v2/      Legacy prototype — reference only, deprecated donor for the carboy extraction
orix/malt-stack/    Sibling substrate project (Carboy/Keg/Kettle/Hops/Cask/Tap) — do NOT depend on.
                    MALT briefly depended on carboy/keg (2026-04-11, uncommitted); reverted 2026-07-24.
                    Full reverted state preserved on branch checkpoint/pre-carboy-revert-2026-07-24.
                    If porting anything from carboy-isolation (it has broader OS coverage than
                    malt-platform::isolation — AppContainer/HCS/CRIU/capability-probing), vendor the
                    code in as owned source. Never add a path/git dependency on malt-stack.
```

MALT depends on `vexil-lang` for schema compilation and `vexil-runtime` for encode/decode. The `vexil-store` crate (in vexil-lang workspace) provides `.vx`/`.vxb` persistence formats.

`~/projects/vexil-v2/` is a legacy prototype. Code quality and engineering standards are poor — do not port code verbatim. It does contain working implementations of systems not yet built in MALT (isolation, scrollback, plugin lifecycle, etc.) that can be used as **reference for logic and algorithms**. When implementing a Phase B–I feature, check vexil-v2 for a working equivalent first.

## Repo Structure

```
specs/                        # GitHub Spec Kit territory (adopted 2026-07-24) — per-feature
                               # specs/NNN-feature-name/{spec,plan,tasks}.md, created by
                               # /speckit-specify. Empty until the first feature uses it.
.specify/                     # Spec Kit config, templates, constitution (.specify/memory/constitution.md)
.claude/skills/speckit-*/     # Spec Kit's Claude Code skills — /speckit-constitution,
                               # /speckit-specify, /speckit-plan, /speckit-tasks, /speckit-implement,
                               # /speckit-converge, plus optional clarify/analyze/checklist/taskstoissues
docs/
  design/
    architecture.md            # Single source of truth (~2,380 lines) — moved from specs/
                                # 2026-07-24 to free that path for Spec Kit
    legacy-specs/               # The old specs/ directory's phase0-2 build specs — historical,
                                # point-in-time, not living docs. Reference only.
  adr/                         # Architecture decisions and their reasoning — check before
                                # re-deciding something already settled (e.g. ADR-0001: malt-stack)
  findings/                    # Dated evidence from actually running/testing the product —
                                # not conclusions, the "how do we know this" record
  BACKLOG.md                   # Living, prioritized "what's next and why" — check before
                                # picking up new work
docs/superpowers/              # DEPRECATED 2026-07-24 — no longer used, no new content goes here.
                                # Historical specs/plans left as point-in-time record.
crates/
  malt-protocol/               # L0: VNP types, framing, envelope, codec (60 tests)
  malt-platform/               # L0: PTY, process, signals, sockets, isolation (77 tests; isolation/ not yet wired into malt-daemon)
  malt-config/                 # L0: Config loading, VxDecoder, real .vx parsing (17 tests)
  mash/                        # L1: POSIX shell — lexer, parser, expander, executor (600+ tests, plus 183/183 Smoosh POSIX conformance on native Windows)
  malt-term/                   # L1: Line editor — vi/emacs, completion, history (41 tests)
  malt-tools/                  # L1: In-process POSIX utilities (80 tests)
  malt-layout/                 # L1: Layout engine — n-ary tree, resolution, focus (48 tests)
  malt-session/                # L1: Session lifecycle, pane runtime, groups (23 tests)
  malt-elevate/                # Elevated helper binary (17 tests; only CreateSymlink is real, other ops stub)
  malt-daemon/                 # L2: Daemon core (126 tests)
  malt-compat/                 # L2: VT emulator — vte 0.15 parser + grid (26 tests)
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
L0  malt-protocol     VNP spine: all shared types, framing, envelope, codec (domain/type constants, make_envelope), encode/decode

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
2. **OS calls in `malt-platform` only.** No `nix`, `windows-sys`, `libc`, `std::os::unix` elsewhere. ✅ Clean — `PermissionsExt` violation in `mash/src/executor.rs` fixed in Phase A.
3. **`malt-protocol` is dependency-free within workspace.** Only external deps. ✅ Clean.
4. **`malt-plugin-sdk` has zero internal deps.** ✅ Clean — only external deps (wasmtime, serde, thiserror).
5. **All `unsafe` blocks require `// SAFETY:` comments.** ✅ Clean — `malt-platform/src/env.rs:25,27` fixed in Phase A.
6. **No `unwrap()` or `expect()` in non-test code.** ✅ Clean — all found uses are in `#[cfg(test)]` blocks.
7. **VNP is the only inter-component protocol.** ✅ Clean. Full bitpack envelope used post-handshake for all message types.
8. **Shell ships when POSIX conformance suite passes.** Smoosh (186 tests) + Modernish. Not yet wired to CI. Fix in Phase C.
9. **Layer violations are compile errors.** No upward dependencies in the crate graph. ✅ Clean.
10. **Invariants are CI-enforced via `deny.toml`.** ✅ Clean — `deny.toml` added in Phase A.

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
- **Coordinator:** Session lifecycle, message routing, output retrieval via reply channels; `DebouncedStore` field; counter restore from `DaemonState` on startup; session name uniqueness (auto-suffix `-2`…`-100`); `persist_daemon_state` after every create/destroy
- **Process supervisor:** spawn_with_pty, kill, check_exited, resize (for future compat pane processes)
- **Session store:** Bitpack persistence (Pack/Unpack), atomic writes (temp+rename → `.bak` backup), corruption quarantine (`.corrupt.{ts}.vxb`), save/load/list/delete; `DebouncedStore` wrapper with background 1s flush thread and `flush_all()`
- **VNP listener:** TCP socket on port+1, VNP handshake, typed bitpack envelope dispatch post-handshake — KeyEvent/Resize/FrameAck inbound, RenderBatch/InitialState outbound. No JSON in the message loop.
- **Input bridge:** `input_bridge` module — `vnp_key_to_input_event` converts VNP `KeyEvent` → mash `InputEvent`
- **RendererHost + Editor wiring:** Session executor owns a `RendererHost` and `Editor`; `RegisterVnpClient` returns `InitialState`, `KeyInput` drives the line editor, `dispatch_render` pushes `RenderBatch` to per-client `SyncSender<RenderBatch>(4)` channels
- **Gateway backend:** GatewayBackend impl wrapping Coordinator, exec via RunCommand with reply channel
- **Config loading:** `VxDecoder` trait + `DaemonConfig`/`UserConfig` impls; `load_or_default_daemon/user` parse real `.vx` files via `vexil_lang::compile` + `vexil_store::parse`; missing fields use `Default`, wrong-type fields log `warn` and use `Default`
- **XDG storage:** daemon startup uses `malt_config::paths::data_dir()` for all persistence; `std::fs::create_dir_all` ensures path exists before `SessionStore` construction

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
- **malt-tui:** ratatui rendering (DrawText, DrawRect, Clear, clip), crossterm input, `VnpConnection` uses full bitpack handshake + typed envelope for all messages (send_key_event, send_resize, poll_commands), styled output with RGB colors
- **maltty:** wgpu window, surface setup, dark background clear, daemon HTTP connection, input mapping (scaffold — text rendering pending)
- **malt-web:** SvelteKit MVP, styled span grid rendering, keyboard input, session polling

## Implementation Roadmap

Goal: fully realize `docs/design/architecture.md`. Every phase is production-ready before starting the next.
Audit baseline: `docs/baseline-audit-2026-03-31.md`.

### Phase A — Foundation Hardening ✅ COMPLETE
- ✅ Fix Invariant 2: moved `PermissionsExt` usage into malt-platform
- ✅ Fix Invariant 5: added `// SAFETY:` to `malt-platform/src/env.rs:25,27`
- ✅ Add `deny.toml` to workspace root
- ✅ Wire-format golden file tests for envelope encode/decode stability

### Phase B — Config + Persistence
**B1 ✅ COMPLETE** (specs: `docs/superpowers/specs/2026-03-31-phase-b1-persistence-hardening-design.md`)
- ✅ malt-config: `VxDecoder` trait + real `.vx` file parsing via `vexil_lang::compile`
- ✅ Corruption handler: `.corrupt.{ts}.vxb` quarantine, `.bak` backup on atomic overwrite
- ✅ Session name uniqueness + numeric suffix (`-2`…`-100`) on conflict
- ✅ Persistence debouncing: `DebouncedStore` with 1s timer + `flush_all()`
- ✅ XDG storage conventions: `malt_config::paths::data_dir()` wired at daemon startup
- ✅ Counter restore: `Coordinator` loads `DaemonState` on construction

**B2 — in design** (graceful shutdown + session restore on attach)
- Graceful shutdown: `flush_all()` → session threads stop cleanly
- Session lifecycle: Active → Dormant state transition on last-client-detach
- Session restore on attach: re-launch processes (mash + compat) in persisted cwd/args
- `DetachSession` VNP message handler

**B3 — pending**
- Scrollback: mmap append-only log per pane, ring buffer header, disk budget (default 10K lines), per-client scroll offsets

### Phase C — Shell Completeness
- MASH FrameElement emission: MASH composes shell output into FrameElement trees and emits them directly (currently absent)
- Add 11 missing FrameElement schema variants: Table, List, Tree, Diff, ProgressBar, Sparkline, Badge, KeyValue, Tabs, Modal, StatusBar
- `catch_unwind` at every MASH poll point (keystroke, execution, expansion)
- Alias expansion depth limit (1024) and subshell recursion limit (256)
- Per-session watchdog: heartbeat pings, 500ms SLA enforcement
- IsolationContext token injection from daemon into each MASH instance
- Wire Smoosh (186 tests) + Modernish diagnostic suite into CI

### Phase D — Gateway Hardening
- Wire rate limiter into all route handlers (currently exists but not integrated)
- Global rate limit (100 req/s across all clients)
- X-RateLimit response headers on 429 responses
- Payload size limits: reject POST /exec and POST /send > 64 KiB
- Per-endpoint scope enforcement (Monitor/Read/Interact/Admin)
- Make gateway bus-extractable: replace direct Coordinator calls with bus messages only

### Phase E — Observability
- Metrics registry: counters, gauges, histograms per subsystem (message bus, MASH, compat translator, plugin host, session store, gateway, process supervisor)
- Prometheus `/metrics` scrape endpoint in malt-gateway
- Real `/health`: uptime, per-subsystem status (running/degraded/failed), bus queue pressure, active session count, severity mapping
- Diagnostics bus channel: System domain Diagnostic type (Low priority), `/diagnostics` endpoint
- JSON structured logging with session_id, pane_id, subsystem, msg_type fields; 50 MiB rotation

### Phase F — Layout + Theme Completeness
- Focus layer segmentation: tiled base layer vs float overlay layers; directional navigation stays within layer; Escape returns to base layer
- Theme token resolution: replace stub in malt-renderer with actual token → resolved color mapping

### Phase G — Plugin System Completion
- Plugin startup lifecycle: `[startup]` manifest section (daemon/session hooks, priority 0–100, lazy flag)
- User overrides: `~/.config/malt/startup.vx`
- Plugin output filtering: declare command patterns (e.g. `cargo *`), Plugin Host routes OutputChunks to matching plugins only
- `malt-app-sdk` crate: App trait with VNP-connected mode + standalone ratatui fallback.
  Not the same thing as `malt-tui` (already built, malt's own first-party client) —
  this is the still-unbuilt third-party app-authoring SDK per architecture.md §7,
  re-exporting only the stable FrameElement core-primitive subset for external
  developers building their own malt apps. Easy to conflate with malt-tui because
  the "dual-mode runner" wording is similar; they are distinct crates.
- `malt plugin audit` command
- Specific latency SLO enforcement: < 5ms keystroke, < 10ms prompt, < 50ms output parser

### Phase H — Isolation Enforcement
*Most complex phase. Platform-specific per OS.*
- `tier_available()`: runtime capability probing
- Linux: namespaces (mount, pid, net, uts, ipc), cgroup v2 (memory + cpu limits), overlayfs, seccomp BPF, veth/bridge for network isolation
- macOS: sandbox-exec / Seatbelt profile, setrlimit
- Windows: Job Objects, restricted tokens, HCS bindings (feature-gated), AppContainer
- Group atomic lifecycle ops: pause, resume, kill, checkpoint
- malt-compat out-of-process worker (Restricted+ sessions): heartbeat (2s), restart limit (5 in 60s), memory limits, output stall detection
- IsolationContext enforcement at MASH spawn

### Phase I — Security Hardening
- Local transport: OS peer credential verification
- BLAKE3 content-addressed wire identity
- Token enforcement: verify bearer tokens on every authenticated route in malt-gateway
- malt-elevate: wire elevated helper into daemon spawn flow for privileged operations
- Remote TLS: rustls, self-signed certs with TOFU, CA-signed for shared infra

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

- 63 non-empty test suites, ~885 tests, zero failures
- Tests use `tempfile::tempdir()` for file I/O isolation
- VNP listener tests use random ports (`bind("127.0.0.1:0")`)
- Supervisor tests spawn real processes (echo, sleep/ping)
- Gateway route tests use mock backend with `tower::ServiceExt::oneshot`
- Plugin SDK tests use `wat` crate for minimal WASM modules
- No tests requiring GPU (maltty build-only verification)
