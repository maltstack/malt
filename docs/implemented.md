# Subsystem inventory

Moved out of `AGENTS.md` on 2026-07-28: it is genuinely useful when orienting,
but it is long, it duplicates `specs/NNN-*/` and `docs/BACKLOG.md`, and it goes
stale silently — which is the worst property a document loaded into every
session can have.

**Treat every claim here as dated, not current.** The authority for "does this
work" is the test suite and `docs/findings/`; the authority for "what was
built and why" is `specs/`. Re-verify before relying on anything below.

---

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
