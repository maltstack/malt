use std::fmt;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use malt_platform::process::Child;
use malt_platform::pty::Pty;
use malt_protocol::common::{IsolationTier, PaneId};

/// Request to spawn a new process attached to a pane.
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
#[non_exhaustive]
pub enum ProcessState {
    /// Process is currently running.
    Running,
    /// Process exited with the given exit code.
    Exited(i32),
    /// Process failed with the given error description.
    Failed(String),
}

/// A process managed by the supervisor, associated with a pane.
pub struct ManagedProcess {
    pane_id: PaneId,
    pid: u32,
    state: ProcessState,
    restart_count: u32,
    child: Option<Child>,
    pty_handle: Option<Arc<dyn Pty>>,
    reader: Option<File>,
    writer: Option<File>,
}

impl ManagedProcess {
    /// Create a new managed process with real child and PTY handles.
    pub fn new(
        pane_id: PaneId,
        pid: u32,
        child: Child,
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

    /// Create a managed process for testing without real OS handles.
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

    /// Returns the pane ID this process is associated with.
    pub fn pane_id(&self) -> &PaneId {
        &self.pane_id
    }

    /// Returns the OS process ID.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Returns the current process state.
    pub fn state(&self) -> &ProcessState {
        &self.state
    }

    /// Sets the process state.
    pub fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    /// Returns the number of times this process has been restarted.
    pub fn restart_count(&self) -> u32 {
        self.restart_count
    }

    /// Increments the restart counter.
    pub fn increment_restart(&mut self) {
        self.restart_count += 1;
    }

    /// Returns true if the restart count has reached or exceeded the given max.
    pub fn restart_limit_exceeded(&self, max: u32) -> bool {
        self.restart_count >= max
    }

    /// Returns a mutable reference to the child process, if present.
    pub fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// Returns a reference to the PTY handle, if present.
    pub fn pty_handle(&self) -> Option<&Arc<dyn Pty>> {
        self.pty_handle.as_ref()
    }

    /// Takes the reader and writer IO handles, leaving None in their place.
    pub fn take_io(&mut self) -> Option<(File, File)> {
        let reader = self.reader.take()?;
        let writer = self.writer.take()?;
        Some((reader, writer))
    }

    /// Returns true if both reader and writer IO handles are present.
    pub fn has_io(&self) -> bool {
        self.reader.is_some() && self.writer.is_some()
    }
}

impl fmt::Debug for ManagedProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedProcess")
            .field("pane_id", &self.pane_id)
            .field("pid", &self.pid)
            .field("state", &self.state)
            .field("restart_count", &self.restart_count)
            .field("has_child", &self.child.is_some())
            .field("has_pty", &self.pty_handle.is_some())
            .field("has_io", &self.has_io())
            .finish()
    }
}
