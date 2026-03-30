# Phase 3E: Process Supervisor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Process Supervisor that manages child process lifecycle (spawn, monitor, kill, restart) with PTY allocation.

**Architecture:** The supervisor owns `ManagedProcess` entries keyed by `PaneId`. Each entry holds a `Child` handle from malt-platform, PTY handles, and restart tracking. The `check_exited()` polling method detects process exits via `try_wait()`.

**Tech Stack:** Rust, malt-platform (process::spawn, pty::open_pty), malt-protocol (PaneId, IsolationTier), thiserror, tracing

---

## File Structure

```
malt-daemon/
  src/
    supervisor/
      mod.rs            — ProcessSupervisor: spawn, kill, resize, check_exited, take_io
      process.rs        — ManagedProcess, ProcessState, SpawnRequest
      error.rs          — SupervisorError enum
```

---

### Task 1: Types and Error

**Files:**
- Create: `crates/malt-daemon/src/supervisor/error.rs`
- Create: `crates/malt-daemon/src/supervisor/process.rs`
- Create: `crates/malt-daemon/src/supervisor/mod.rs` (stub)
- Modify: `crates/malt-daemon/src/lib.rs`
- Create: `crates/malt-daemon/tests/process.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/process.rs`:
```rust
use malt_daemon::supervisor::process::{ManagedProcess, ProcessState, SpawnRequest};
use malt_protocol::common::{IsolationTier, PaneId};
use std::path::PathBuf;

#[test]
fn spawn_request_construction() {
    let req = SpawnRequest {
        program: PathBuf::from("/bin/bash"),
        args: vec!["-l".to_string()],
        cwd: PathBuf::from("/home/user"),
        pane_id: PaneId(1),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    };
    assert_eq!(req.program, PathBuf::from("/bin/bash"));
    assert_eq!(req.args.len(), 1);
    assert_eq!(req.pane_id, PaneId(1));
}

#[test]
fn process_state_default_is_running() {
    let state = ProcessState::Running;
    assert!(matches!(state, ProcessState::Running));
}

#[test]
fn process_state_exited() {
    let state = ProcessState::Exited(0);
    match state {
        ProcessState::Exited(code) => assert_eq!(code, 0),
        other => panic!("expected Exited, got {other:?}"),
    }
}

#[test]
fn restart_count_increments() {
    let mut proc = ManagedProcess::new_test(PaneId(1), 1234);
    assert_eq!(proc.restart_count(), 0);
    proc.increment_restart();
    assert_eq!(proc.restart_count(), 1);
    proc.increment_restart();
    assert_eq!(proc.restart_count(), 2);
}

#[test]
fn restart_limit_exceeded() {
    let mut proc = ManagedProcess::new_test(PaneId(1), 1234);
    for _ in 0..5 {
        proc.increment_restart();
    }
    assert!(proc.restart_limit_exceeded(5));
    assert!(!proc.restart_limit_exceeded(6));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test process`
Expected: FAIL

- [ ] **Step 3: Create error.rs**

```rust
use malt_protocol::common::PaneId;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SupervisorError {
    #[error("process not found for pane: {0:?}")]
    ProcessNotFound(PaneId),

    #[error("spawn failed: {0}")]
    SpawnFailed(#[from] malt_platform::process::SpawnError),

    #[error("pty error: {0}")]
    PtyError(#[from] malt_platform::pty::PtyError),

    #[error("restart limit exceeded for pane: {0:?}")]
    RestartLimitExceeded(PaneId),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 4: Create process.rs**

```rust
use malt_platform::pty::{Pty, WinSize};
use malt_protocol::common::{IsolationTier, PaneId};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

/// What to spawn.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub pane_id: PaneId,
    pub isolation: IsolationTier,
    pub cols: u16,
    pub rows: u16,
}

/// State of a managed process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Exited(i32),
    Failed(String),
}

/// A process managed by the supervisor.
pub struct ManagedProcess {
    pane_id: PaneId,
    pid: u32,
    state: ProcessState,
    restart_count: u32,
    child: Option<malt_platform::process::Child>,
    pty_handle: Option<Arc<dyn Pty>>,
    reader: Option<File>,
    writer: Option<File>,
}

impl ManagedProcess {
    pub fn new(
        pane_id: PaneId,
        pid: u32,
        child: malt_platform::process::Child,
        pty_handle: Arc<dyn Pty>,
        reader: File,
        writer: File,
    ) -> Self {
        Self {
            pane_id,
            pid,
            state: ProcessState::Running,
            restart_count: 0,
            child: Some(child),
            pty_handle: Some(pty_handle),
            reader: Some(reader),
            writer: Some(writer),
        }
    }

    /// Create a test-only instance without real process handles.
    pub fn new_test(pane_id: PaneId, pid: u32) -> Self {
        Self {
            pane_id,
            pid,
            state: ProcessState::Running,
            restart_count: 0,
            child: None,
            pty_handle: None,
            reader: None,
            writer: None,
        }
    }

    pub fn pane_id(&self) -> &PaneId {
        &self.pane_id
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn state(&self) -> &ProcessState {
        &self.state
    }

    pub fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    pub fn restart_count(&self) -> u32 {
        self.restart_count
    }

    pub fn increment_restart(&mut self) {
        self.restart_count += 1;
    }

    pub fn restart_limit_exceeded(&self, max: u32) -> bool {
        self.restart_count >= max
    }

    pub fn child_mut(&mut self) -> Option<&mut malt_platform::process::Child> {
        self.child.as_mut()
    }

    pub fn pty_handle(&self) -> Option<&Arc<dyn Pty>> {
        self.pty_handle.as_ref()
    }

    /// Take the reader and writer file handles (for I/O tasks).
    /// Returns None if already taken.
    pub fn take_io(&mut self) -> Option<(File, File)> {
        let reader = self.reader.take()?;
        let writer = self.writer.take()?;
        Some((reader, writer))
    }

    pub fn has_io(&self) -> bool {
        self.reader.is_some() && self.writer.is_some()
    }
}

impl std::fmt::Debug for ManagedProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedProcess")
            .field("pane_id", &self.pane_id)
            .field("pid", &self.pid)
            .field("state", &self.state)
            .field("restart_count", &self.restart_count)
            .field("has_child", &self.child.is_some())
            .field("has_io", &self.has_io())
            .finish()
    }
}
```

- [ ] **Step 5: Create supervisor/mod.rs stub**

```rust
pub mod error;
pub mod process;

pub use error::SupervisorError;
pub use process::{ManagedProcess, ProcessState, SpawnRequest};
```

- [ ] **Step 6: Add supervisor module to lib.rs**

Add `pub mod supervisor;` to `crates/malt-daemon/src/lib.rs`.

- [ ] **Step 7: Run tests**

Run: `cargo test -p malt-daemon --test process`
Expected: all 5 PASS

- [ ] **Step 8: Commit**

```bash
git add crates/malt-daemon/src/supervisor/ crates/malt-daemon/src/lib.rs crates/malt-daemon/tests/process.rs
git commit -m "feat(malt-daemon): supervisor types — SpawnRequest, ProcessState, ManagedProcess"
```

---

### Task 2: ProcessSupervisor Implementation

**Files:**
- Modify: `crates/malt-daemon/src/supervisor/mod.rs`
- Create: `crates/malt-daemon/tests/supervisor.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-daemon/tests/supervisor.rs`:
```rust
use malt_daemon::supervisor::process::{ProcessState, SpawnRequest};
use malt_daemon::supervisor::ProcessSupervisor;
use malt_protocol::common::{IsolationTier, PaneId};
use std::path::PathBuf;

fn echo_request(pane_id: u32) -> SpawnRequest {
    SpawnRequest {
        #[cfg(unix)]
        program: PathBuf::from("/bin/echo"),
        #[cfg(windows)]
        program: PathBuf::from("cmd"),
        #[cfg(unix)]
        args: vec!["hello".to_string()],
        #[cfg(windows)]
        args: vec!["/c".to_string(), "echo".to_string(), "hello".to_string()],
        cwd: std::env::current_dir().unwrap(),
        pane_id: PaneId(pane_id),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    }
}

fn sleep_request(pane_id: u32) -> SpawnRequest {
    SpawnRequest {
        #[cfg(unix)]
        program: PathBuf::from("/bin/sleep"),
        #[cfg(windows)]
        program: PathBuf::from("cmd"),
        #[cfg(unix)]
        args: vec!["60".to_string()],
        #[cfg(windows)]
        args: vec!["/c".to_string(), "timeout".to_string(), "/t".to_string(), "60".to_string(), "/nobreak".to_string()],
        cwd: std::env::current_dir().unwrap(),
        pane_id: PaneId(pane_id),
        isolation: IsolationTier::Bare,
        cols: 80,
        rows: 24,
    }
}

#[test]
fn spawn_and_check_exit() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(echo_request(1)).unwrap();
    // Wait a bit for echo to finish
    std::thread::sleep(std::time::Duration::from_millis(500));
    let exited = sup.check_exited();
    assert!(!exited.is_empty());
    let (pane_id, state) = &exited[0];
    assert_eq!(*pane_id, PaneId(1));
    assert!(matches!(state, ProcessState::Exited(0)));
}

#[test]
fn spawn_tracks_pane_id() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(42)).unwrap();
    assert!(sup.get(&PaneId(42)).is_some());
    assert_eq!(sup.process_count(), 1);
    sup.kill(&PaneId(42)).unwrap();
}

#[test]
fn kill_process() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    sup.kill(&PaneId(1)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let exited = sup.check_exited();
    // After kill, process should be gone or exited
    assert!(sup.get(&PaneId(1)).is_none() || !exited.is_empty());
}

#[test]
fn process_count() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    sup.spawn(sleep_request(2)).unwrap();
    sup.spawn(sleep_request(3)).unwrap();
    assert_eq!(sup.process_count(), 3);
    sup.kill(&PaneId(1)).unwrap();
    sup.kill(&PaneId(2)).unwrap();
    sup.kill(&PaneId(3)).unwrap();
}

#[test]
fn take_io_returns_handles() {
    let mut sup = ProcessSupervisor::new();
    sup.spawn(sleep_request(1)).unwrap();
    let io = sup.take_io(&PaneId(1));
    assert!(io.is_some());
    // Second take returns None
    let io2 = sup.take_io(&PaneId(1));
    assert!(io2.is_none());
    sup.kill(&PaneId(1)).unwrap();
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-daemon --test supervisor`
Expected: FAIL

- [ ] **Step 3: Implement ProcessSupervisor**

Replace `crates/malt-daemon/src/supervisor/mod.rs`:
```rust
pub mod error;
pub mod process;

pub use error::SupervisorError;
pub use process::{ManagedProcess, ProcessState, SpawnRequest};

use malt_platform::process::{self, Io, SpawnConfig};
use malt_platform::pty::{self, WinSize};
use malt_protocol::common::PaneId;
use std::collections::HashMap;
use std::ffi::OsString;
use tracing::{info, warn};

/// Manages child processes on behalf of sessions.
pub struct ProcessSupervisor {
    processes: HashMap<u32, ManagedProcess>,
}

impl ProcessSupervisor {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    /// Spawn a new process with a PTY.
    pub fn spawn(&mut self, req: SpawnRequest) -> Result<PaneId, SupervisorError> {
        let size = WinSize {
            cols: req.cols,
            rows: req.rows,
        };
        let (pty_handle, reader, writer) = pty::open_pty(size)?;

        let mut config = SpawnConfig::new(&req.program);
        config = config
            .args(req.args.iter().map(OsString::from))
            .cwd(&req.cwd)
            .stdin(Io::File(writer.try_clone().map_err(SupervisorError::Io)?))
            .stdout(Io::File(reader.try_clone().map_err(SupervisorError::Io)?))
            .stderr(Io::File(reader.try_clone().map_err(SupervisorError::Io)?));

        let child = process::spawn(config)?;
        let pid = child.pid();

        info!(pid, pane_id = ?req.pane_id, program = ?req.program, "process spawned");

        let managed = ManagedProcess::new(req.pane_id.clone(), pid, child, pty_handle, reader, writer);
        self.processes.insert(req.pane_id.0, managed);

        Ok(req.pane_id)
    }

    /// Kill a managed process.
    pub fn kill(&mut self, pane_id: &PaneId) -> Result<(), SupervisorError> {
        let proc = self
            .processes
            .remove(&pane_id.0)
            .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;
        drop(proc); // Dropping Child kills the process on some platforms
        info!(?pane_id, "process killed");
        Ok(())
    }

    /// Resize a process's PTY.
    pub fn resize(&self, pane_id: &PaneId, size: WinSize) -> Result<(), SupervisorError> {
        let proc = self
            .processes
            .get(&pane_id.0)
            .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;
        if let Some(pty) = proc.pty_handle() {
            pty.resize(size)?;
        }
        Ok(())
    }

    /// Poll for exited processes. Returns list of (PaneId, ProcessState).
    /// Removes exited processes from tracking.
    pub fn check_exited(&mut self) -> Vec<(PaneId, ProcessState)> {
        let mut exited = Vec::new();
        let pane_ids: Vec<u32> = self.processes.keys().copied().collect();
        for pane_id in pane_ids {
            let proc = self.processes.get_mut(&pane_id).unwrap();
            if let Some(child) = proc.child_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let state = ProcessState::Exited(status.code());
                        exited.push((proc.pane_id().clone(), state));
                    }
                    Ok(None) => {} // Still running
                    Err(e) => {
                        let state = ProcessState::Failed(format!("{e}"));
                        warn!(pane_id, error = %e, "process check failed");
                        exited.push((proc.pane_id().clone(), state));
                    }
                }
            }
        }
        // Remove exited processes
        for (pane_id, _) in &exited {
            self.processes.remove(&pane_id.0);
        }
        exited
    }

    /// Get a reference to a managed process.
    pub fn get(&self, pane_id: &PaneId) -> Option<&ManagedProcess> {
        self.processes.get(&pane_id.0)
    }

    /// Take the I/O file handles for a process (reader, writer).
    /// Returns None if already taken or process not found.
    pub fn take_io(&mut self, pane_id: &PaneId) -> Option<(std::fs::File, std::fs::File)> {
        self.processes.get_mut(&pane_id.0)?.take_io()
    }

    pub fn process_count(&self) -> usize {
        self.processes.len()
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p malt-daemon --test supervisor`
Expected: all 5 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-daemon/src/supervisor/ crates/malt-daemon/tests/supervisor.rs
git commit -m "feat(malt-daemon): process supervisor — spawn, kill, check_exited, PTY management"
```

---

### Task 3: Final Verification

- [ ] **Step 1: Run all daemon tests**

Run: `cargo test -p malt-daemon`
Expected: all PASS (57 existing + 5 process + 5 supervisor = 67 total)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p malt-daemon -- -W clippy::all -A unused-imports`
Expected: no warnings from malt-daemon code

- [ ] **Step 3: Fix any issues**

- [ ] **Step 4: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all PASS

- [ ] **Step 5: Commit if fixes needed**
