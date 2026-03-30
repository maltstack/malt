# Integration Layer 1: CLI ↔ Daemon via HTTP

**Date:** 2026-03-30
**Status:** Approved
**Scope:** Wire malt-bin CLI to real daemon via gateway HTTP server

---

## Components

### gateway_backend.rs (malt-daemon)
- Implements `GatewayBackend` trait for real Coordinator
- Wraps `Arc<Mutex<Coordinator>>`
- Maps trait methods to Coordinator calls

### daemon.rs (malt-bin)
- `malt daemon` command: starts Coordinator + HTTP server on 127.0.0.1:7700
- `malt start`: spawns `malt daemon` as detached background process
- `malt stop`: sends shutdown signal

### Startup Sequence
1. Create Coordinator
2. Wrap in GatewayBackend impl
3. Build axum router
4. Bind and serve on 127.0.0.1:7700
5. Ctrl-C → graceful shutdown

---

## Testing

### gateway_backend.rs (4 tests)
- list_sessions_empty, create_and_list, create_and_get, destroy_session

### Integration (2 tests)
- daemon_health_endpoint — HTTP round-trip
- daemon_session_lifecycle — full CRUD over HTTP

---

## Deferred
- Shell process spawning (Layer 2)
- TUI live connection (Layer 3)
- malt stop implementation (needs shutdown endpoint)
