pub mod error;
pub mod process;

use std::collections::HashMap;

use malt_platform::process::SpawnConfig;
use malt_platform::pty::{self, WinSize};
use malt_protocol::common::PaneId;

pub use error::SupervisorError;
pub use process::{ManagedProcess, ProcessState, SpawnRequest};

/// Manages child processes on behalf of the daemon, one per pane.
///
/// Each managed process is associated with a `PaneId`. The supervisor
/// handles spawning, killing, polling for exit, and PTY resize.
pub struct ProcessSupervisor {
    processes: HashMap<u32, ManagedProcess>,
}

impl ProcessSupervisor {
    /// Create a new, empty process supervisor.
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    /// Spawn a new child process for the given pane.
    ///
    /// Opens a PTY, spawns the child with inherited stdio (PTY slave
    /// attachment will be wired in a later phase), and stores the
    /// managed process keyed by pane ID.
    pub fn spawn(&mut self, req: SpawnRequest) -> Result<(), SupervisorError> {
        let size = WinSize {
            cols: req.cols,
            rows: req.rows,
        };
        let (pty_handle, reader, writer) = pty::open_pty(size)?;

        let mut config = SpawnConfig::new(&req.program);
        for arg in &req.args {
            config = config.arg(arg);
        }
        config = config.cwd(&req.cwd);
        // Leave stdin/stdout/stderr as Inherit for now.
        // Full PTY slave attachment will be wired in Phase 4.

        let child = malt_platform::process::spawn(config)?;
        let pid = child.pid();

        let key = req.pane_id.0;
        let managed = ManagedProcess::new(req.pane_id, pid, child, pty_handle, reader, writer);
        self.processes.insert(key, managed);

        Ok(())
    }

    /// Kill and remove the process associated with the given pane.
    ///
    /// Dropping the `ManagedProcess` (and its `Child`) will clean up
    /// OS resources.
    pub fn kill(&mut self, pane_id: &PaneId) -> Result<(), SupervisorError> {
        self.processes
            .remove(&pane_id.0)
            .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;
        Ok(())
    }

    /// Resize the PTY associated with the given pane.
    pub fn resize(&mut self, pane_id: &PaneId, size: WinSize) -> Result<(), SupervisorError> {
        let proc = self
            .processes
            .get(&pane_id.0)
            .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;
        if let Some(pty) = proc.pty_handle() {
            pty.resize(size)?;
        }
        Ok(())
    }

    /// Poll all managed processes for exit, returning those that have exited.
    ///
    /// Exited processes are removed from the supervisor.
    pub fn check_exited(&mut self) -> Vec<(PaneId, ProcessState)> {
        let mut exited = Vec::new();

        // Collect pane IDs of exited processes first, then remove.
        let mut to_remove = Vec::new();
        for (&key, proc) in self.processes.iter_mut() {
            if let Some(child) = proc.child_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let state = ProcessState::Exited(status.code());
                        proc.set_state(state.clone());
                        exited.push((proc.pane_id().clone(), state));
                        to_remove.push(key);
                    }
                    Ok(None) => {
                        // Still running.
                    }
                    Err(e) => {
                        let state = ProcessState::Failed(e.to_string());
                        proc.set_state(state.clone());
                        exited.push((proc.pane_id().clone(), state));
                        to_remove.push(key);
                    }
                }
            }
        }

        for key in to_remove {
            self.processes.remove(&key);
        }

        exited
    }

    /// Look up a managed process by pane ID.
    pub fn get(&self, pane_id: &PaneId) -> Option<&ManagedProcess> {
        self.processes.get(&pane_id.0)
    }

    /// Take the I/O handles (PTY reader/writer) from the managed process.
    ///
    /// Returns `None` if the pane is not found or I/O was already taken.
    pub fn take_io(&mut self, pane_id: &PaneId) -> Option<(std::fs::File, std::fs::File)> {
        self.processes.get_mut(&pane_id.0)?.take_io()
    }

    /// Returns the number of currently managed processes.
    pub fn process_count(&self) -> usize {
        self.processes.len()
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}
