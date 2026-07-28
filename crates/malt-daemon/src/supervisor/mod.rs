pub mod error;
pub mod process;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use malt_platform::pty::{spawn_with_pty, WinSize};
use malt_platform::signals::{send_signal, terminate_process, SignalError, SignalKind};
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

    /// Spawn a new child process attached to a PTY.
    pub fn spawn(&mut self, req: SpawnRequest) -> Result<(), SupervisorError> {
        let size = WinSize {
            cols: req.cols,
            rows: req.rows,
        };

        let pty_proc =
            spawn_with_pty(&req.program, &req.args, &req.cwd, size).map_err(|e| match e {
                malt_platform::pty::PtySpawnError::Pty(e) => SupervisorError::PtyError(e),
                malt_platform::pty::PtySpawnError::Spawn(e) => SupervisorError::SpawnFailed(e),
                malt_platform::pty::PtySpawnError::Io(e) => SupervisorError::Io(e),
            })?;

        #[cfg(windows)]
        let mut pty_proc = pty_proc;

        let pid = pty_proc.child.pid();

        // On Windows, child has piped stdio — take those handles for I/O.
        // On Unix, I/O goes through the PTY master (reader/writer).
        #[cfg(windows)]
        let (reader, writer) = {
            let stdout = pty_proc
                .child
                .take_stdout()
                .map(malt_platform::io::into_file);
            let stdin = pty_proc
                .child
                .take_stdin()
                .map(malt_platform::io::into_file);
            (
                stdout.unwrap_or(pty_proc.reader),
                stdin.unwrap_or(pty_proc.writer),
            )
        };
        #[cfg(unix)]
        let (reader, writer) = (pty_proc.reader, pty_proc.writer);

        let key = req.pane_id.0;
        let managed = ManagedProcess::new(
            req.pane_id,
            pid,
            pty_proc.child,
            pty_proc.pty,
            reader,
            writer,
            pty_proc.slave,
        );
        self.processes.insert(key, managed);

        Ok(())
    }

    /// Terminate, reap, and remove the process associated with the given pane.
    ///
    /// A graceful `TERM` gets a short bounded wait. If it is ignored, the
    /// process is forcibly terminated and reaped. The bookkeeping entry stays
    /// present on failure so a caller can retry without dropping PTY ownership.
    pub fn kill(&mut self, pane_id: &PaneId) -> Result<(), SupervisorError> {
        const GRACEFUL_WAIT: Duration = Duration::from_millis(500);
        const FORCED_WAIT: Duration = Duration::from_secs(2);

        let terminated = {
            let proc = self
                .processes
                .get_mut(&pane_id.0)
                .ok_or_else(|| SupervisorError::ProcessNotFound(pane_id.clone()))?;

            // An exit that won the race with this request is already the
            // requested outcome. Reap it before dropping PTY handles.
            if process_exited(proc)? {
                true
            } else {
                let pid = proc.pid();
                let exited_after_term_signal = match send_signal(pid, SignalKind::Term) {
                    Ok(()) => false,
                    Err(SignalError::NoSuchProcess { .. }) if process_exited(proc)? => true,
                    Err(source) => {
                        return Err(SupervisorError::Signal {
                            pane_id: pane_id.clone(),
                            source,
                        });
                    }
                };

                if exited_after_term_signal || wait_for_exit(proc, GRACEFUL_WAIT)? {
                    true
                } else {
                    let exited_after_force_signal = match terminate_process(pid) {
                        Ok(()) => false,
                        Err(SignalError::NoSuchProcess { .. }) if process_exited(proc)? => true,
                        Err(source) => {
                            return Err(SupervisorError::Signal {
                                pane_id: pane_id.clone(),
                                source,
                            });
                        }
                    };

                    exited_after_force_signal || wait_for_exit(proc, FORCED_WAIT)?
                }
            }
        };

        if terminated {
            self.processes.remove(&pane_id.0);
            return Ok(());
        }

        Err(SupervisorError::TerminationTimedOut(pane_id.clone()))
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

    /// Poll all managed processes for exit.
    pub fn check_exited(&mut self) -> Vec<(PaneId, ProcessState)> {
        let mut exited = Vec::new();
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
                    Ok(None) => {}
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

    /// Take the I/O handles from the managed process.
    pub fn take_io(&mut self, pane_id: &PaneId) -> Option<(std::fs::File, std::fs::File)> {
        self.processes.get_mut(&pane_id.0)?.take_io()
    }

    pub fn process_count(&self) -> usize {
        self.processes.len()
    }
}

fn process_exited(proc: &mut ManagedProcess) -> Result<bool, SupervisorError> {
    let Some(child) = proc.child_mut() else {
        return Ok(false);
    };
    match child.try_wait()? {
        Some(status) => {
            proc.set_state(ProcessState::Exited(status.code()));
            Ok(true)
        }
        None => Ok(false),
    }
}

fn wait_for_exit(proc: &mut ManagedProcess, timeout: Duration) -> Result<bool, SupervisorError> {
    let deadline = Instant::now() + timeout;
    loop {
        if process_exited(proc)? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}
