# Phase 3D: Session Store + Persistence

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-daemon/src/store/` module — Value converters for persist types, atomic file I/O, SessionStore API
**Depends on:** Phase 3A (malt-daemon crate), vexil-store (Value API), persist schemas

---

## Context

Persistence uses Vexil Store's `.vx` text format. The vexil-store crate provides a dynamic `Value` API for encoding/decoding. We bridge the gap between `Value` and typed Rust structs with manual converters for each persist type.

---

## Value Converters

For each persist type, two functions:
```
fn to_value(T) -> Value
fn from_value(Value) -> Result<T, StoreError>
```

### Types to convert (from schemas/persist/):

- **PersistedSession** — schema_version, id, name, layout, focus, panes, theme, group, isolation
- **PersistedPane** — cwd, title, pane_type
- **PersistedPaneType** — union: Shell { shell_path }, App { app_id, config }, Compat { program, args }
- **DaemonState** — schema_version, sessions, active_groups, next_session_id, next_pane_id
- **GroupState** — id, name, policy, session_ids
- **GroupPolicy** — max_memory_mb, max_cpu_cores, max_sessions, ttl_secs, idle_timeout_secs, on_empty, on_oom

---

## SessionStore API

```
SessionStore::new(base_dir: PathBuf) -> Self
SessionStore::save(id: SessionId, session: &PersistedSession) -> Result<(), StoreError>
SessionStore::load(id: SessionId) -> Result<PersistedSession, StoreError>
SessionStore::list() -> Result<Vec<SessionId>, StoreError>
SessionStore::delete(id: SessionId) -> Result<(), StoreError>
SessionStore::save_daemon_state(state: &DaemonState) -> Result<(), StoreError>
SessionStore::load_daemon_state() -> Result<DaemonState, StoreError>
```

### Atomic Writes

Write to `{id}.vx.tmp`, then rename to `{id}.vx`. On failure, temp file is cleaned up.

### File Layout

```
$base_dir/
  sessions/
    1.vx
    2.vx
  daemon.vx
```

---

## Module Structure

```
malt-daemon/
  src/
    store/
      mod.rs            — SessionStore public API, file I/O
      convert.rs        — Value ↔ persist type converters
      error.rs          — StoreError enum
```

### Dependencies (added to malt-daemon)

- `vexil-store` (from vexil-lang git dep)

---

## Testing

### convert.rs (8 tests)
- persisted_session_roundtrip
- persisted_pane_shell_roundtrip
- persisted_pane_app_roundtrip
- persisted_pane_compat_roundtrip
- daemon_state_roundtrip
- group_state_roundtrip
- empty_optional_fields
- layout_tree_roundtrip

### store.rs (6 tests)
- save_and_load_session
- list_sessions
- delete_session
- atomic_write_creates_file
- save_and_load_daemon_state
- load_nonexistent_returns_error

All store tests use tempdir for isolation.

### Not in Scope
- `.vxb` binary format (optional fast-restore, Phase 5)
- Scrollback storage / mmap (separate concern)
- Debounce timer (wired when daemon event loop exists)
- Crash recovery beyond load-last-saved
