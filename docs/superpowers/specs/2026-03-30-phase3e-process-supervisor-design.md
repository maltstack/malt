# Phase 3E: Process Supervisor

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-daemon/src/supervisor/` module — child process lifecycle, PTY allocation, restart logic
**Depends on:** malt-platform (process spawning, PTY), malt-protocol (PaneId)

---

## Context

The Process Supervisor manages child processes (shell, app, compat worker) on behalf of sessions. WASM Plugin Host is deferred to Phase 5.

---

## Process Supervisor Core

### Types

- **ProcessSupervisor** — owns all managed processes
- **ManagedProcess** — tracks child: pid, pane, state, PTY handle, restart count
- **ProcessState** — Running, Exited(i32), Failed(String)
- **SpawnRequest** — program, args, cwd, pane_id, isolation tier

### Lifecycle

1. Session sends SpawnRequest
2. Supervisor opens PTY, spawns process via malt-platform
3. Tracks Child handle, associates with PaneId
4. On exit: notifies via check_exited() polling
5. On crash: can restart (max 5 in 60s)

### PTY Ownership

- Supervisor opens PTY via open_pty(size)
- Reader/writer stored in ManagedProcess
- Pty handle stored for resize
- On restart: new PTY allocated

---

## Public API

```
ProcessSupervisor::new() -> Self
ProcessSupervisor::spawn(req: SpawnRequest) -> Result<PaneId>
ProcessSupervisor::kill(pane_id) -> Result<()>
ProcessSupervisor::resize(pane_id, size: WinSize) -> Result<()>
ProcessSupervisor::check_exited() -> Vec<(PaneId, ProcessState)>
ProcessSupervisor::get(pane_id) -> Option<&ManagedProcess>
ProcessSupervisor::take_io(pane_id) -> Option<(File, File)>
ProcessSupervisor::process_count() -> usize
```

---

## Module Structure

```
malt-daemon/src/supervisor/
  mod.rs            — ProcessSupervisor
  process.rs        — ManagedProcess, ProcessState, SpawnRequest
  error.rs          — SupervisorError
```

---

## Testing

### process.rs (5 tests)
- spawn_request_construction
- process_state_transitions
- restart_count_increments
- restart_limit_exceeded
- managed_process_tracks_pid

### supervisor.rs (5 tests)
- spawn_and_check_exit
- spawn_tracks_pane_id
- kill_process
- process_count
- take_io_returns_handles

---

## Not In Scope

- WASM Plugin Host (Phase 5)
- Out-of-process compat worker heartbeat/restart (needs supervisor + compat integration)
- Isolation context injection (stubbed — full implementation needs malt-platform isolation tiers)
