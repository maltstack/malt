# Phase 4.1: malt-bin — CLI Entry Point

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-bin` crate — `malt` CLI command for daemon lifecycle and session management
**Depends on:** malt-gateway (API types)

---

## CLI Commands

```
malt                          # Default: check daemon, show status
malt start                    # Start daemon (stub — daemon binary is Phase 4.2)
malt stop                     # Stop daemon (stub)
malt status                   # GET /health — show daemon status

malt list                     # GET /sessions — list sessions
malt new [--name NAME]        # POST /sessions — create session
malt attach [SESSION_ID]      # GET /sessions/:id — show info (TUI attach is Phase 4.2)
malt kill SESSION_ID          # DELETE /sessions/:id — destroy session

malt exec SESSION_ID COMMAND  # POST /sessions/:id/exec — run command, print output
malt send SESSION_ID INPUT    # POST /sessions/:id/send — send input

malt version                  # Print version
```

### Default Behavior

`malt` with no args: connect to daemon, print status and session list. If daemon not running, print message.

---

## Communication

- Connects to daemon via HTTP REST (gateway API)
- Default: `http://127.0.0.1:7700` (configurable via `MALT_API_ADDR`)
- Uses `reqwest` blocking client

---

## Module Structure

```
malt-bin/
  Cargo.toml
  src/
    main.rs           — entry point, clap parse, dispatch
    cli.rs            — clap command definitions
    client.rs         — MaltClient: HTTP wrapper for gateway API
    output.rs         — formatted terminal output
    error.rs          — CliError (uses anyhow)
```

### Dependencies

- `clap` (derive) — argument parsing
- `reqwest` (blocking) — HTTP client
- `serde`, `serde_json` — JSON
- `anyhow` — error handling (anyhow allowed in malt-bin only per spec)

---

## Testing

### client.rs (4 tests)
- parse_session_list_response
- parse_health_response
- parse_exec_response
- api_url_construction

### cli.rs (4 tests)
- parse_no_args
- parse_list
- parse_new_with_name
- parse_exec

---

## Deferred

- Daemon process spawning (`malt start`) — needs daemon binary
- TUI attach (`malt attach`) — needs malt-tui
- Interactive session — needs full daemon + TUI integration
