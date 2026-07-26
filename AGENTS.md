# AGENTS.md

Instructions for AI agents (and anyone else) working in this repo. Claude
Code loads this via the `@AGENTS.md` import in `CLAUDE.md` — see that file
if you're looking for why there are two files; there aren't, really.

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

## Session Ritual (added 2026-07-24, see ADR-0001)

This project has been abandoned-via-rewrite three times (vexil-v2 → malt →
malt-stack) after multi-day uncommitted sprints. Two habits meant to prevent
a fourth:

1. **Rebuild + retest before resuming work after any gap.** Don't trust a
   stale status doc — `cargo build --workspace && cargo test --workspace`
   (see `docs/adr/` for `vexilc` PATH setup) and update the numbers below if
   they've drifted.
2. **Commit at real checkpoints, not multi-day piles.** If a change is big
   enough to feel risky to commit, that's a signal to commit sooner, not
   later. If something starts looking like it needs a bigger architectural
   rethink mid-task, write that down as a proposal (an ADR draft is fine)
   instead of silently pivoting into it.

## What is MALT?

MALT (structured terminal platform) inverts the traditional terminal model: the daemon is the authority, not the renderer. The daemon owns session state, layout, pane identity, and structured output. Clients are interchangeable consumers of a typed RenderCommand stream. All inter-component communication uses VNP (Vexil Native Protocol) — typed, schema-defined, bitpack-encoded messages.

**Current state:** Foundation + Phase A + Phase B1 implemented. 18 Rust crates + 1 SvelteKit web client. 1,200+ tests, all green (verified 2026-07-24 after a 3.5-month dormancy — see `docs/findings/2026-07-24-live-daemon-session.md` for what "green" did and didn't turn out to mean in practice). The `malt` command starts a daemon with an in-process mash (POSIX shell), serves HTTP + VNP socket APIs, and opens an interactive TUI. Full VNP bitpack encoding used end-to-end (no JSON post-handshake). Real `.vx` config file loading wired. Session store hardened (.bak backup, corruption quarantine, debounced 1s flush, XDG-compliant data dir, counter restore on startup) — confirmed surviving a real 3.5-month restart, not just a test fixture. MCP server available for AI agents. WASM plugin host ready. GPU terminal (maltty) scaffolded. Many architecture spec features remain unimplemented — see Implementation Roadmap below, and `docs/BACKLOG.md` for concrete near-term items.

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
                               # /speckit-specify (Claude) or $speckit-specify (Codex).
                               # Empty until the first feature uses it.
.specify/                     # Spec Kit config, templates, constitution (.specify/memory/constitution.md)
.claude/skills/speckit-*/     # Spec Kit's Claude Code skills — /speckit-constitution,
                               # /speckit-specify, /speckit-plan, /speckit-tasks, /speckit-implement,
                               # /speckit-converge, plus optional clarify/analyze/checklist/taskstoissues
.agents/skills/speckit-*/     # The same Spec Kit workflows for Codex — invoke as
                               # $speckit-constitution, $speckit-specify, $speckit-plan,
                               # $speckit-tasks, $speckit-implement, and $speckit-converge.
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
plans/                          # RETIRED 2026-07-24 — original Phase 0-2 implementation plans,
                                # historical record. Audited against current code, see
                                # docs/findings/2026-07-24-plan-implementation-audit.md.
crates/
  malt-protocol/               # L0: VNP types, framing, envelope, codec (60 tests)
  malt-platform/               # L0: PTY, process, signals, sockets, isolation (78 tests; session
                                # isolation now wired to real Job Object containment as of 2026-07-24
                                # — see ADR-0001 and docs/findings/)
  malt-config/                 # L0: Config loading, VxDecoder, real .vx parsing (17 tests)
  mash/                        # L1: POSIX shell — lexer, parser, expander, executor (600+ tests, plus 183/183 Smoosh POSIX conformance on native Windows)
  malt-term/                   # L1: Line editor — vi/emacs, completion, history (41 tests)
  malt-tools/                  # L1: In-process POSIX utilities (80 tests)
  malt-layout/                 # L1: Layout engine — n-ary tree, resolution, focus (48 tests)
  malt-session/                # L1: Session lifecycle, pane runtime, groups (23 tests)
  malt-elevate/                # Elevated helper binary (17 tests; only CreateSymlink is real, other ops stub)
  malt-daemon/                 # L2: Daemon core (126 tests)
  malt-compat/                 # L2: VT emulator — vte 0.15 parser + grid (26 tests)
  malt-renderer/               # L2: Renderer host — walker, dirty, client state (29 tests;
                                # known cursor-position "staircase" render bug, see docs/BACKLOG.md P0)
  malt-gateway/                # L2: HTTP API — axum, auth, rate limiting (22 tests)
  malt-plugin-sdk/             # L3: WASM plugin host — wasmtime, fuel budgets (5 tests)
  malt-mcp/                    # L3: MCP server for AI agents (6 tests)
  malt-bin/                    # L3: CLI entry point — `malt` command (9 tests; no --isolation flag yet, see docs/BACKLOG.md)
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
L0  malt-platform     OS abstractions: PTY, process, signals, sockets, spawn_with_pty, isolation
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

These are non-negotiable. Violating any is a bug. (Also captured, with the
day's process lessons added, in `.specify/memory/constitution.md`.)

1. **VT codes in `malt-compat` only.** No other crate may import `vte` or handle escape sequences. ✅ Clean.
2. **OS calls in `malt-platform` only.** No `nix`, `windows-sys`, `libc`, `std::os::unix` elsewhere. ✅ Clean — `PermissionsExt` violation in `mash/src/executor.rs` fixed in Phase A.
3. **`malt-protocol` is dependency-free within workspace.** Only external deps. ✅ Clean.
4. **`malt-plugin-sdk` has zero internal deps.** ✅ Clean — only external deps (wasmtime, serde, thiserror).
5. **All `unsafe` blocks require `// SAFETY:` comments.** ✅ Clean — enforced throughout, including the 2026-07-24 HCS port (27 unsafe blocks, all documented).
6. **No `unwrap()` or `expect()` in non-test code.** ✅ Clean — all found uses are in `#[cfg(test)]` blocks.
7. **VNP is the only inter-component protocol.** ✅ Clean. Full bitpack envelope used post-handshake for all message types.
8. **Shell ships when POSIX conformance suite passes.** Smoosh (183/183 Windows, 186/186 WSL) + Modernish. Not yet wired to CI. Fix in Phase C.
9. **Layer violations are compile errors.** No upward dependencies in the crate graph. ✅ Clean.
10. **Invariants are CI-enforced via `deny.toml`.** ✅ Clean — `deny.toml` added in Phase A.
11. **Vendor, never depend on unstable siblings.** Added 2026-07-24 (ADR-0001). If something from malt-stack or another sibling project is useful, port it in as owned source — never a live path/git dependency.
12. **No silent scope-jumps; commit at real checkpoints.** Added 2026-07-24. If a task starts looking like it needs a bigger rethink, write that down (an ADR draft, a backlog item) instead of pivoting into it mid-task.

## CLI Commands (all working)

```
malt                          # Auto: start daemon → create session → attach TUI
malt daemon [--port N]        # Run daemon foreground (HTTP on port, VNP on port+1)
malt start                    # Start daemon in background
malt stop                     # Graceful shutdown via /shutdown endpoint
malt status                   # Show daemon health + session list
malt new [--name N] [--isolation <bare|restricted|capped|contained>]
                               # Create session (spawns in-process mash shell)
malt list                     # List sessions
malt attach [ID]              # Open TUI connected to session (VNP + HTTP fallback)
malt exec ID "command"        # Run command via mash, return output (reports truncation if the
                               # reply exceeds the 1 MiB cap)
malt output ID                # Print session's current output as plain text
malt history ID               # List the session's command execution history
malt watch ID [--output]      # Stream the session's lifecycle events live (SSE); --output
                               # streams the command's raw stdout/stderr chunks instead
malt send ID "input"          # Send raw bytes to whatever is reading (NOT a command)
malt eof ID                   # Signal end-of-input to the current reader (Ctrl-D)
malt kill ID                  # Destroy session
```

The Gateway now enforces real auth (2026-07-25) — every HTTP route requires
a bearer token. `malt-bin` and `malt-mcp` read it automatically from
`~/.config/malt/api-token` (the same file the daemon's `TokenStore` writes on
first start); nothing to configure for normal CLI/agent use on the same
machine. See `docs/BACKLOG.md`'s Gateway-auth entry for what does and
doesn't have a token mechanism yet.

## What's Implemented

### Daemon (malt-daemon)
- **Message bus:** Per-session synchronous dispatcher, bounded ring buffers, priority eviction (Low→Normal→High), Reliable never evicted, Critical separate inbox, flow control (25% per-producer, 60% global)
- **Session-sharded executor:** Per-session thread with own bus, authority tracker, compat translator, mash shell environment
- **Mash integration:** In-process POSIX shell — parse → execute_list → capture stdout/stderr → feed compat translator
- **Coordinator:** Session lifecycle, message routing, output retrieval via reply channels; `DebouncedStore` field; counter restore from `DaemonState` on startup; session name uniqueness (auto-suffix `-2`…`-100`); `persist_daemon_state` after every create/destroy
- **Process supervisor:** spawn_with_pty, kill, check_exited, resize (for future compat pane processes) — not yet wired to session isolation tiers, unlike mash's own external-command spawn path (see `docs/BACKLOG.md`)
- **Session store:** Bitpack persistence (Pack/Unpack), atomic writes (temp+rename → `.bak` backup), corruption quarantine (`.corrupt.{ts}.vxb`), save/load/list/delete; `DebouncedStore` wrapper with background 1s flush thread and `flush_all()`
- **Command lifecycle events (2026-07-25, spec 004):** `GET /sessions/{id}/events` streams `command_started`/`command_finished` as Server-Sent Events, with `Last-Event-ID` resume from a bounded per-session event log (1024) and a defined slow-consumer policy — a subscriber that stops reading is told it lagged and dropped, never accommodated by growing its 256-event channel. Published from the same two control-actor handlers that drive command history, so the two cannot disagree. `malt watch` is the first-party consumer. **Note: this does not use the `Bus`** — its `Reliable` tier grows without bound, which is unsafe for an untrusted subscriber; the Bus still has zero consumers, see `docs/BACKLOG.md`. See `specs/004-command-lifecycle-events/`.
- **Command execution history (2026-07-25, spec 003):** `SessionExecutor` owns a `PaneRuntime`; `run_mash_command` pushes an *open* `CommandBlock` before executing and finalizes it after, so a daemon that stops mid-command persists an honestly-unfinished record. Persisted via `PersistedPane.command_blocks`, restored on `spawn_with_cwd` (which also resumes `next_command_id` from the restored max). Retrieved via `GET /sessions/{id}/history`, `malt history`, or the MCP `get_command_history` tool — a dormant session answers from its snapshot rather than being woken. See `specs/003-command-execution-history/`.
- **VNP listener:** TCP socket on port+1, **authenticated** VNP handshake, typed bitpack envelope dispatch post-handshake — KeyEvent/Resize/FrameAck/InputClaim inbound, RenderBatch/InitialState/InputAuthorityChanged outbound. No JSON in the message loop.
- **VNP authentication (2026-07-25, spec 005 US1):** the transport used to accept every connection and send the session inventory during the handshake, before establishing any identity — audit A-01. It now validates the same bearer token as the HTTP surface *before* disclosing anything; the inventory is passed to the handshake as a `FnOnce`, so "authenticate first" is structural rather than a convention someone has to remember. A 10 s deadline applies before the first blocking read (set on the stream, not the write-side clone — a receive timeout is per-descriptor on Windows), and `MAX_PENDING_HANDSHAKES` bounds unidentified connections.
- **Raw input + input authority (2026-07-25, spec 005):** `send` writes raw bytes to whatever is reading — it no longer submits them as a command to execute, which had made it impossible to answer a prompt and possible to run text the caller never intended. Input carries an `InputOrigin`; at most one attached client holds authority, non-holders are refused with a reason naming the holder rather than silently dropped, and authority releases on disconnect (clean or abrupt) with no timeout. `GET /sessions/{id}/authority` reports the holder at Read scope. `POST /sessions/{id}/eof` (`malt eof`) ends the current read so a command consuming to end-of-input can finish. See `specs/005-raw-input-authority/` and `docs/findings/2026-07-25-spec-005-quickstart-verification.md`.
- **Streaming command output (2026-07-26, spec 006):** a running command's output used to reach anyone only once at completion. `mash`'s `Env` now carries an `OutputSink`; the session worker installs one per command, and every leaf dispatch point (builtins, in-process tools, external processes, pipelines) forwards output to it as produced rather than after the fact — including `cat`/`grep`/`sed`/`head`, whose `ToolFn` signature grew a writer alongside its reader so a tool emits as it works (`wc` still emits once: it has exactly one summary line, known only once input is exhausted). The control actor feeds the compat grid and dispatches a `RenderBatch` per chunk, and a bounded per-session `OutputLog` (4 MiB, byte- not count-bounded, with a reserved subscriber slot so a lagged consumer can always be told so rather than silently dropped) lets both VNP clients and `GET /sessions/{id}/output/stream` (SSE, `Last-Event-ID` resume) resume from a position instead of only seeing what arrives after they attach. `malt watch --output` is the first-party CLI consumer. Command substitution and pipeline stages explicitly clear the sink on their `env.clone()` (a captured value or an internal pipe must never leak to a live viewer), and redirected output still lands only in the file — the writer is an additional destination, never a replacement for the buffer `apply_output_redirects` acts on. See `specs/006-streaming-command-output/` and `docs/BACKLOG.md`'s now-closed "Gateway/agent-driven execution never notified" entry.
- **Input bridge:** `input_bridge` module — `vnp_key_to_input_event` converts VNP `KeyEvent` → mash `InputEvent`
- **RendererHost + Editor wiring:** Session executor owns a `RendererHost` and `Editor`; `RegisterVnpClient` returns `InitialState`, `KeyInput` drives the line editor, `dispatch_render` pushes `RenderBatch` to per-client `SyncSender<RenderBatch>(4)` channels
- **Gateway backend:** GatewayBackend impl wrapping Coordinator, exec via RunCommand with reply channel
- **Config loading:** `VxDecoder` trait + `DaemonConfig`/`UserConfig` impls; `load_or_default_daemon/user` parse real `.vx` files via `vexil_lang::compile` + `vexil_store::parse`; missing fields use `Default`, wrong-type fields log `warn` and use `Default`
- **XDG storage:** daemon startup uses `malt_config::paths::data_dir()` for all persistence; `std::fs::create_dir_all` ensures path exists before `SessionStore` construction
- **Session isolation (2026-07-24):** `IsolationTier` is applied for real on the mash external-command spawn path via Windows Job Objects — see ADR-0001 and `docs/findings/2026-07-24-live-daemon-session.md`. Backgrounded commands (`&`) through the daemon's `/exec` route don't appear to survive — not yet root-caused, see `docs/BACKLOG.md`.

### VT Emulator (malt-compat)
- Cell/Row grid, TerminalGrid (cursor, scroll, erase, resize)
- VT parser via vte 0.15 (SGR colors/attributes, cursor movement, erase, scroll)
- CompatTranslator: feed bytes → VtPassthrough FrameElement

### Renderer (malt-renderer)
- FrameWalker: tree traversal, depth limit (64), node limit (10K), capability degradation (TrueColor/Basic256/None)
- DirtyTracker: frame diffing for delta-only emission
- ClientState: frame_seq, lagging detection (30 frames), shedding (10s timeout)
- RendererHost: pipeline orchestration, per-client state, InitialState snapshots
- **Known bug (2026-07-24):** output grid rendering has a cursor-position "staircase" bug — successive lines render with growing left-padding instead of resetting to column 0. P0 in `docs/BACKLOG.md`; the single biggest known gap to daily-driver usability.

### API Gateway (malt-gateway)
- HTTP REST: axum 0.8, 17 endpoints + /shutdown
- GatewayBackend trait (extractable subsystem)
- Auth: scopes (Monitor < Read < Interact < Admin), TokenStore with bearer tokens, token file persistence
- SSE event stream: `GET /sessions/{id}/events` (Read scope), `Last-Event-ID` resume, bounded per-subscriber delivery
- SSE output stream (2026-07-26, spec 006): `GET /sessions/{id}/output/stream` (Read scope) — same `Last-Event-ID`/bounded-delivery shape as the event stream, but carrying base64-encoded stdout/stderr chunks instead of lifecycle events
- Token bucket rate limiter
- Shadow tree: FrameElement → semantic JSON (styled spans with RGB)

### MCP Server (malt-mcp)
- JSON-RPC 2.0 over stdio
- 7 tools: list_sessions, create_session, run_command, get_output, get_command_history, send_input, destroy_session
- Delegates to daemon HTTP API — confirmed working by calling the underlying gateway routes directly (`/sessions/{id}/send` etc.), not just by reading the code

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

## Priorities (correctness-first, see ADR-0003)

The phased feature roadmap that used to live here (Phase A–I) is retired
as of 2026-07-24 — see `docs/adr/ADR-0003-correctness-first-strategic-pivot.md`
for the full reasoning. Phase A and Phase B1 are historical fact and stay
recorded below; Phase B2 onward is replaced by a flat, priority-ordered
correctness/hardening list, informed by a five-agent audit
(`docs/findings/2026-07-24-audit-*.md`) run specifically to find what's
missing, buggy, or on shaky ground — not to plan new features.

**Guiding lens, not a task:** a 9-step demo — an agent starts `cargo test`
in a persistent session, MALT reports structured progress, a human attaches
to the same session, both see the same authoritative state, the command
requests input or fails, temporary input authority changes hands, the
session survives client disconnection, the daemon restarts and restores
session history, and the agent resumes from the last execution event
instead of scraping the terminal — is used only to judge whether a given
gap actually matters for genuine human/agent coexistence. It is not a
feature to build toward directly; no work item should be justified solely
by "the demo needs it."

**Priority order** (see `docs/BACKLOG.md` for the concrete, evidence-based
items behind each — this list is the ordering, not the detail):

- **0a. Gateway auth actually enforced.** `TokenStore`/`AuthContext`/
  `RateLimiter` are fully built and tested in isolation but never wired
  into `build_router` — every route is Admin-equivalent open to anyone who
  can reach the port today. Audit-discovered prerequisite, not on the
  original list, sequenced first because it gates safely exposing the
  Gateway to any agent at all.
- **0b. Decouple command execution from the session's single
  command-dispatch thread.** One thread, one blocking `mpsc::Receiver` per
  session; a long-running command blocks attach/output/input entirely for
  its whole duration. Audit-discovered prerequisite for genuine raw input,
  for gateway-driven execution to ever notify an attached human, and for
  attach to degrade gracefully instead of hard-timing-out.
- 1. Correct plain stdout and stderr
- 2. Real exit codes and execution IDs
- 3. Command lifecycle events
- 4. Persistent execution history
- 5. Genuine raw input
- 6. Human and agent coexistence
- 7. Fail-closed requested isolation (`required`/`preferred`/`disabled`
  policy, plus the underlying per-tier enforcement the audit found
  missing on every platform)
- 8. A correct TUI rendering path
- 9. Session restoration
- 10. One excellent agent client or Gateway SDK

**Explicitly paused**, with equal weight to the list above — a deliberate
decision per ADR-0003, not silent deferral: the GPU client (`maltty`)
beyond basic usability, plugin marketplace infrastructure, broad plugin
lifecycle features, remote shared deployments, exotic isolation tiers
beyond what's already built, large collections of new FrameElement/UI
variants, MCP-specific expansion, elaborate observability systems, and
maintaining multiple competing client experiences. `malt-tui` (VNP mode)
is the single reference human client; `maltty` and `malt-web` are frozen
as-is.

### Historical: Phase A — Foundation Hardening ✅ COMPLETE
- ✅ Fix Invariant 2: moved `PermissionsExt` usage into malt-platform
- ✅ Fix Invariant 5: added `// SAFETY:` to `malt-platform/src/env.rs:25,27`
- ✅ Add `deny.toml` to workspace root
- ✅ Wire-format golden file tests for envelope encode/decode stability

### Historical: Phase B1 — Config + Persistence ✅ COMPLETE
(specs: `docs/superpowers/specs/2026-03-31-phase-b1-persistence-hardening-design.md`)
- ✅ malt-config: `VxDecoder` trait + real `.vx` file parsing via `vexil_lang::compile`
- ✅ Corruption handler: `.corrupt.{ts}.vxb` quarantine, `.bak` backup on atomic overwrite
- ✅ Session name uniqueness + numeric suffix (`-2`…`-100`) on conflict
- ✅ Persistence debouncing: `DebouncedStore` with 1s timer + `flush_all()`
- ✅ XDG storage conventions: `malt_config::paths::data_dir()` wired at daemon startup
- ✅ Counter restore: `Coordinator` loads `DaemonState` on construction

## Code Standards

- `thiserror` for library errors; `anyhow` in binary crates only.
- `tracing` for all logging. No `eprintln!`/`println!` in library crates.
- `#[non_exhaustive]` on public enums that will grow.
- `#[derive(Debug, Clone, PartialEq)]` on data types.
- Explicit re-exports only — no `pub use foo::*`.
- Rust edition 2021.
- Key deps: `vte` 0.15, `axum` 0.8, `ratatui` 0.30, `crossterm` 0.29, `clap` 4, `reqwest` 0.13, `wasmtime` 29, `wgpu` 24, `winit` 0.30, `windows-sys` 0.61.

## Type Notes (for implementers)

- FrameElement/RenderCommand **union variants** have NO `_unknown` field
- Message structs (RenderBatch, InitialState, etc.) DO have `_unknown: Vec<u8>`
- `style` in FrameElement::Text/Paragraph is `Box<ResolvedStyle>` — `ResolvedStyle` has a `token_name: Option<String>` field (added after the initial schema; six test fixtures missed it and broke the build until fixed 2026-07-24 — if you hand-construct a `ResolvedStyle` in a test, don't forget it)
- `direction` in FrameElement::Split is `Box<Direction>`
- `child` fields are `Box<FrameElement>`, `fallback` is `Option<Box<FrameElement>>`
- `rgb` type = `(u8, u8, u8)` tuple in generated code
- `PaneId(u32)`, `SessionId(u32)` — NOT Copy, use `.clone()`
- `encode_message`/`encode_envelope` return `Result` (not infallible)
- `ToolFn` is `fn(&[String], &mut dyn Read)` — a *reader*, not a pre-read
  `&[u8]` (changed 2026-07-25). Tools that consume to the end call
  `malt_tools::read_all`; tools that stop early (`head -n`) must read
  incrementally, or they wait for an end a live session never reaches. There
  are three tool-dispatch sites in `executor.rs` and all call `tool_stdin` —
  keep it that way; guarding them individually is how two were once missed.
- mash `Env` created per session thread via `Env::from_os()` + `set_interactive(true)`
- mash `execute_list` is synchronous — perfect for session executor's thread model
- `Env` carries an `Option<Arc<isolation::job_objects::JobObject>>` on Windows (added 2026-07-24) — shared across `Env::clone()` (subshells) so a session's whole process tree lives in one job

## Testing

- 1,200+ tests across the workspace, all green as of 2026-07-24 (see the
  Session Ritual above — re-verify after any gap, don't trust this number
  blindly).
- Tests use `tempfile::tempdir()` for file I/O isolation.
- VNP listener tests use random ports (`bind("127.0.0.1:0")`).
- Supervisor tests spawn real processes (echo, sleep/ping).
- Gateway route tests use mock backend with `tower::ServiceExt::oneshot`.
- Plugin SDK tests use `wat` crate for minimal WASM modules.
- No tests requiring GPU (maltty build-only verification).
- **Shared-process-state races are a real, recurring test bug class in this
  repo.** A third was found 2026-07-25 in `mash/tests/executor.rs`:
  `function_prefix_redirect_expansion_precedes_assignment_word_expansion`
  called `set_current_dir` without holding `CWD_LOCK`. It never failed
  itself — it yanked the CWD out from under whichever lock-holding test was
  running, so the failures surfaced in `heredoc_redirect_feeds_stdin_to_cat`
  and `exec_input_redirect_registers_readable_shell_fd`. That is why this
  looked like three independent flaky tests for months. **When a test flakes,
  suspect a different test**, and grep the whole binary for the shared-state
  mutation rather than debugging the test that reported the failure.

- **`thread::sleep` cannot establish a precondition**, only make a race
  likely to resolve one way. Fixed 2026-07-25 across four daemon tests that
  used `sleep(50ms)` to mean "the first command has started by now"; under
  parallel load it hadn't, and `gateway_backend.rs` failed 3 runs in 5. Wait
  on observable state instead — `Coordinator::execution_queue_state` exists
  for exactly this. Note that `active` is set by the worker *before* the
  control actor records history, so waiting on `active` is not sufficient
  when the assertion is about history; wait on the record itself.

- Two earlier instances of the same class were found and fixed 2026-07-24: a `CWD_LOCK` mutex
  protecting tests that call `cd` (mutating the process-wide working
  directory) wasn't held by two tests doing relative-path file I/O, and 9
  of its 19 call sites used poison-fragile `.lock().unwrap()` instead of
  `.lock().unwrap_or_else(|e| e.into_inner())`, so one panic cascaded into
  unrelated test failures. Same pattern separately in
  `malt-platform/src/fs.rs`'s `MALT_SESSION_ID` env-var tests. If a test
  mutates shared process state (CWD, env vars, global statics) and doesn't
  hold a lock for its whole duration, treat that as a bug, not a
  coincidence, if it ever flakes.
- **Two real bugs were found in `job_objects.rs` in 2026-07 specifically
  because new tests exercised the real Win32 calls instead of constructing
  structs directly** (undersized `IO_COUNTERS` struct silently failing
  every job creation; a hardcoded active-process-limit of 1 silently
  rejecting the second process in any job). If a file's tests only check
  `Send`/`Debug` bounds or construct types directly without calling the
  real functions, that's a signal to add a test that actually does, not
  evidence the code works.

### Windows PowerShell quick reference
```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 183, skipped unsupported: 3

cargo test -p mash --test executor
# Expected: 228 passed, 0 failed
```

### NTFS Caching (Critical)
On WSL with NTFS-backed repo paths, cargo caching is unreliable due to NTFS mtime granularity.
Use `CARGO_TARGET_DIR=/tmp/malt-build` for builds on WSL, or `cargo clean -p mash` followed by rebuild on Windows.
Stale binary symptoms: test failures that don't match expected behavior from source changes.

### Binary Paths
- **Repo build (default):** `target/debug/mash`
- **WSL build:** `/tmp/malt-build/debug/mash`

## Known Issues (evidence-based)

- **Process substitution (`<(...)`, `>()`) unimplemented.** Lexer tokenizes these as `Word`, executor has no support. No executor code exists for process substitution.
- **`{`/`}` brace tokenization context sensitivity.** Current `is_word_break` treats `{` and `}` contextually, which may need further analysis for edge cases.
- **Terminal grid rendering "staircase" bug** and **backgrounded commands not surviving through `/exec`** — see `docs/BACKLOG.md` P0/P1, both found 2026-07-24 by actually running the daemon, not by reading code.

## Historical Changes (2026-04-10, pre-dormancy)

Real fixes at the time, kept here as a reference for the specific
before/after behavior — not a current status list. See git log and
`docs/adr/`/`docs/BACKLOG.md` for anything after 2026-07-24.

### 1. `#` removed from `is_word_break` (`lexer.rs`)
- **Before**: `#` was in the word-break character set, causing `echo hello#world` to tokenize as `echo`, `hello` (with `#world` silently dropped as a comment).
- **After**: `#` only starts a comment at token-start position (handled at lexer.rs:949). Inside a word, `#` is a literal character. POSIX-compliant behavior verified with Smoosh 183/183.

### 2. `${#@}` and `${#*}` return parameter count (`expander.rs`)
- **Before**: `${#@}` and `${#*}` fell through to `env.get_str(name).chars().count()`, returning the character length of the joined string (e.g., 5 for "a b c") instead of the count of positional parameters.
- **After**: Added explicit handling for `name == "@"` and `name == "*"` in the `${#VAR}` expansion code path (expander.rs:498-530), returning `env.get_str("#")` (the count of positional parameters).

### 3. Multi-pass alias expansion (`parser.rs`)
- **Before**: `preparse_expanded` ran a single pass over alias expansion, so `alias LOOP='while'; alias DO='do'; alias DONE='done'` wouldn't fully expand (LOOP wouldn't see that DO is an alias).
- **After**: Refactored into `preparse_expanded` (public, iterates up to 100 passes until output is stable) and `preparse_expanded_pass` (single-pass logic). Also expanded `ends_with_sep` to include `;|&` characters (not just whitespace) so aliases ending with command-separator tokens reset `in_command_position`.

### 4. `collect_grammar_aliases_from_script` preserves all aliases (`parser.rs`)
- **Before**: Only kept LOOP/DO/DONE aliases from the script, filtering out all others. User-defined aliases in scripts were unavailable for preparse expansion.
- **After**: Changed to call `collect_aliases_from_script` directly, preserving all aliases.

### 5. Heredoc expansion error message prefix (`executor.rs:5188`)
- **Before**: `format!("{e}\n")` — no prefix, producing stderr like `x: z`.
- **After**: `format!("mash: heredoc expansion: {e}\n")` — matches the pattern used by here-string (`mash: here-string expansion: {e}\n`) and regular redirect (`mash: redirect: {e}\n`). The test `heredoc_expansion_error_aborts_noninteractive_script` and the `redirect_error_aborts_noninteractive_shell` check both expect `"heredoc expansion:"` in stderr.

### 6. Windows env var case normalization (`env.rs:410-429`)
- **Problem**: On Windows, `std::env::vars()` returns keys with their original casing (e.g., `Path` not `PATH`). POSIX shell variable names are case-sensitive and the codebase uses uppercase names like `"PATH"`, `"HOME"`, etc. for lookups. The HashMap lookup `env.get("PATH")` wouldn't find `Path`, causing `find_in_path` to fail and pipelines like `echo hello | findstr hello` to produce empty output.
- **Fix**: In `Env::from_os()`, selected well-known environment variable names are uppercased via `key.to_ascii_uppercase()` before insertion. Only variables that POSIX shells reference by uppercase name are normalized (PATH, HOME, TEMP, TMP, COMSPEC, SYSTEMROOT, WINDIR, USERPROFILE, HOMEDRIVE, HOMEPATH, PSMODULEPATH, PATHEXT). Other variable names are preserved in their original case to maintain POSIX case-sensitivity for user-defined variables.
- **Evidence**: `echo hello | findstr hello` now outputs `hello`. `echo hello world | findstr world` outputs `hello world`. Pipeline tests `pipeline_echo_findstr` and `pipeline_filters` now pass.

## Last Known Green Checkpoints
- `dd7ad26` — preparse alias isolation + PSREPLACE fix (Smoosh 186/186 WSL, 183/183 Windows)
- `1296955` — mash: revert pre-tokenized lexer, preserve artifacts (last commit before the 3.5-month dormancy)
- `009711b` — docs: reorganize specs/ into docs/design/, adopt GitHub Spec Kit (2026-07-24, full workspace green — see git log for the 10 commits between these two)
