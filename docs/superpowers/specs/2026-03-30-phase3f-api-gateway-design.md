# Phase 3F: API Gateway

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-gateway` crate — HTTP REST API, local auth, rate limiting, shadow FrameElement tree
**Depends on:** malt-protocol (all types)

---

## Context

The API Gateway exposes MALT's capabilities via HTTP REST for automation, agents, and tooling. It's designed as an extractable subsystem — communicates through a `GatewayBackend` trait, no direct dependency on malt-daemon internals.

---

## HTTP Endpoints

| Method | Path | Description | Auth Scope |
|---|---|---|---|
| GET | `/health` | Health check, daemon uptime | `monitor` |
| GET | `/sessions` | List all sessions | `read` |
| POST | `/sessions` | Create new session | `interact` |
| GET | `/sessions/:id` | Get session info | `read` |
| DELETE | `/sessions/:id` | Destroy session | `admin` |
| POST | `/sessions/:id/exec` | Execute command (blocking) | `interact` |
| POST | `/sessions/:id/send` | Send input (non-blocking) | `interact` |
| GET | `/sessions/:id/output` | Get structured output (shadow tree JSON) | `read` |
| GET | `/sessions/:id/panes` | List panes | `read` |
| POST | `/sessions/:id/panes/split` | Split a pane | `interact` |
| DELETE | `/sessions/:id/panes/:pane_id` | Close a pane | `interact` |

### Response Format

```json
{ "ok": true, "data": { ... } }
{ "ok": false, "error": { "code": "session_not_found", "message": "..." } }
```

---

## Authentication & Authorization

### Local Auth (This Phase)

- Localhost connections trusted, authenticated via bearer token from `~/.config/malt/api-token`
- Local connections default to `admin` scope

### Authorization Scopes

- `monitor` — health, diagnostics
- `read` — monitor + session list, output, pane info
- `interact` — read + send input, exec, create/split
- `admin` — interact + destroy, daemon control

### Rate Limiting

- Per-client identity, in-memory token bucket
- Default: 10 req/s per session, 100 req/s globally per client
- 429 Too Many Requests when exceeded

---

## Shadow FrameElement Tree

- Serializes FrameElement trees to semantic JSON for `/output` endpoint
- Text → `{"type": "Text", "text": "...", "style": {...}}`
- Paragraph → `{"type": "Paragraph", "lines": [...]}`
- Split → `{"type": "Split", "direction": "...", "children": [...]}`
- VtPassthrough → `{"type": "VtPassthrough", "raw_bytes": <length>}`
- On-demand rendering: gateway queries current state per request

---

## GatewayBackend Trait

```rust
pub trait GatewayBackend: Send + Sync {
    fn list_sessions(&self) -> Vec<SessionInfo>;
    fn create_session(&self, name: Option<String>, isolation: IsolationTier) -> Result<SessionId, GatewayError>;
    fn destroy_session(&self, id: SessionId) -> Result<(), GatewayError>;
    fn get_session(&self, id: SessionId) -> Result<SessionInfo, GatewayError>;
    fn exec_command(&self, id: SessionId, command: String) -> Result<ExecResult, GatewayError>;
    fn send_input(&self, id: SessionId, input: String) -> Result<(), GatewayError>;
    fn get_output(&self, id: SessionId) -> Result<serde_json::Value, GatewayError>;
    fn list_panes(&self, id: SessionId) -> Result<Vec<PaneInfo>, GatewayError>;
    fn split_pane(&self, id: SessionId, target: PaneId, direction: Direction) -> Result<PaneId, GatewayError>;
    fn close_pane(&self, id: SessionId, pane_id: PaneId) -> Result<(), GatewayError>;
}
```

---

## Module Structure

```
malt-gateway/
  src/
    lib.rs              — crate root, re-exports
    error.rs            — GatewayError enum
    server.rs           — GatewayServer: axum router, start/stop
    routes/
      mod.rs            — route registration
      health.rs         — GET /health
      sessions.rs       — session CRUD + exec/send/output
      panes.rs          — pane list/split/close
    auth.rs             — AuthContext, scope checking
    rate_limit.rs       — token bucket rate limiter
    shadow.rs           — FrameElement → JSON serialization
```

---

## Testing

- **auth.rs** (4 tests): scope hierarchy, rejection, default scope, unknown token
- **rate_limit.rs** (4 tests): under limit, at limit, refill, per-client isolation
- **shadow.rs** (4 tests): Text/Paragraph/Split/VtPassthrough → JSON
- **routes.rs** (6 tests): health, list/create/get/destroy sessions, 404

---

## Deferred to Phase 5 (Ecosystem)

The following capabilities are architecturally planned but NOT implemented in this phase:

- **MCP protocol transport** — stdio/SSE for AI agents speaking Model Context Protocol. Requires MCP SDK integration and tool mapping for session/exec/output operations.
- **Remote authentication** — TLS (mandatory for remote), self-signed cert + TOFU pinning (personal), CA-signed + API key/OAuth 2.0 (shared infrastructure). Includes certificate management, key rotation, and OAuth token validation.
- **Raw VNP transport** — Direct bitpack-over-socket access for high-performance consumers. Would bypass HTTP/JSON overhead entirely.
- **WebSocket transport** — Persistent connections for real-time streaming of RenderCommand deltas to browser clients.
- **OS peer credential auth** — `SO_PEERCRED` (Unix) / `GetNamedPipeClientProcessId` (Windows) for zero-config auth over Unix sockets / named pipes. Requires non-TCP transport.
- **Prompt-aware execution** — `POST /exec` blocking until `PromptReady` with configurable timeout, rate limited at 10 req/s per session. Initial implementation is non-blocking send + poll.
- **Per-session rate limiting** — Current implementation does global per-client limiting. Per-session limits (10 req/s per session) deferred.
- **Live shadow tree** — Maintaining a continuously-updated shadow FrameElement tree via bus subscription. Initial implementation queries on-demand per request.
