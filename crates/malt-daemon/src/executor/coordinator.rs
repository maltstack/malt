use crate::bus::BusMessage;
use crate::executor::pools::PoolConfig;
use crate::executor::session_thread::{SessionCommand, SessionExecutor};
use crate::supervisor::process::SpawnRequest;
use crate::supervisor::ProcessSupervisor;
use crate::DaemonError;
use malt_protocol::common::{
    GroupId, IsolationTier, PaneId, SessionId, SessionInfo, SessionState,
};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use tracing::{info, warn};

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    cmd_tx: mpsc::Sender<SessionCommand>,
    thread: Option<JoinHandle<()>>,
    #[allow(dead_code)]
    pty_reader_thread: Option<JoinHandle<()>>,
}

/// Coordinator manages session lifecycle and routes messages to session threads.
///
/// Monotonically increasing session IDs — never recycled within daemon lifetime.
pub struct Coordinator {
    sessions: HashMap<u32, SessionHandle>,
    pty_writers: HashMap<u32, std::fs::File>,
    next_session_id: u32,
    next_pane_id: u32,
    supervisor: ProcessSupervisor,
    #[allow(dead_code)]
    pool_config: PoolConfig,
}

impl Coordinator {
    pub fn new(pool_config: PoolConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            pty_writers: HashMap::new(),
            next_session_id: 1,
            next_pane_id: 1,
            supervisor: ProcessSupervisor::new(),
            pool_config,
        }
    }

    /// Detect the default shell for the current platform.
    fn default_shell() -> PathBuf {
        #[cfg(unix)]
        {
            // Try SHELL env var, fall back to /bin/sh
            std::env::var("SHELL")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/bin/sh"))
        }
        #[cfg(windows)]
        {
            std::env::var("COMSPEC")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("cmd.exe"))
        }
    }

    /// Create a new session with a shell process.
    pub fn create_session(
        &mut self,
        name: Option<String>,
        isolation: IsolationTier,
        _group: Option<GroupId>,
    ) -> Result<SessionId, DaemonError> {
        let session_id = SessionId(self.next_session_id);
        self.next_session_id += 1;

        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;

        let (cmd_tx, thread) =
            SessionExecutor::spawn(session_id.clone(), pane_id.clone(), isolation)?;

        // Spawn a shell process with PTY
        let shell = Self::default_shell();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        let spawn_result = self.supervisor.spawn(SpawnRequest {
            program: shell.clone(),
            args: Vec::new(),
            cwd,
            pane_id: pane_id.clone(),
            isolation,
            cols: 80,
            rows: 24,
        });

        let pty_reader_thread = match spawn_result {
            Ok(_) => {
                // Take I/O handles from supervisor
                if let Some((reader, writer)) = self.supervisor.take_io(&pane_id) {
                    // PTY writer stored in coordinator for input routing via write_to_session().
                    // PTY reader fed to a background thread that sends PtyOutput commands.

                    // Start PTY reader thread
                    let reader_tx = cmd_tx.clone();
                    let reader_pane_id = pane_id.clone();
                    let reader_handle = thread::Builder::new()
                        .name(format!("pty-reader-{}", session_id.0))
                        .spawn(move || {
                            pty_reader_loop(reader, reader_tx, reader_pane_id);
                        })
                        .ok();

                    // Store the writer in the session handle for input routing
                    // We'll use a separate storage since SessionHandle needs it
                    self.store_pty_writer(session_id.0, writer);

                    info!(?session_id, ?shell, "shell process spawned with PTY");
                    reader_handle
                } else {
                    warn!(?session_id, "failed to get PTY I/O handles");
                    None
                }
            }
            Err(e) => {
                warn!(?session_id, error = %e, "failed to spawn shell, session created without process");
                None
            }
        };

        self.sessions.insert(
            session_id.0,
            SessionHandle {
                id: session_id.clone(),
                name,
                isolation,
                cmd_tx,
                thread: Some(thread),
                pty_reader_thread,
            },
        );

        Ok(session_id)
    }

    /// Store a PTY writer handle for a session.
    fn store_pty_writer(&mut self, session_id: u32, writer: std::fs::File) {
        self.pty_writers.insert(session_id, writer);
    }

    /// Write data to a session's PTY.
    pub fn write_to_session(&mut self, session_id: u32, data: &[u8]) -> Result<(), DaemonError> {
        use std::io::Write;
        let writer = self
            .pty_writers
            .get_mut(&session_id)
            .ok_or(DaemonError::SessionNotFound(SessionId(session_id)))?;
        writer
            .write_all(data)
            .map_err(DaemonError::Io)?;
        Ok(())
    }

    /// Get the current output text for a session.
    pub fn get_session_output(&self, session_id: u32) -> Result<String, DaemonError> {
        let handle = self
            .sessions
            .get(&session_id)
            .ok_or(DaemonError::SessionNotFound(SessionId(session_id)))?;
        let (reply_tx, reply_rx) = mpsc::channel();
        handle
            .cmd_tx
            .send(SessionCommand::GetOutput { reply: reply_tx })
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
        reply_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
    }

    /// Destroy a session. Sends shutdown and joins the thread.
    pub fn destroy_session(&mut self, id: SessionId) {
        if let Some(mut handle) = self.sessions.remove(&id.0) {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown);
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
            // PTY reader thread will exit when the channel closes
            info!(?id, "session destroyed");
        }
        self.pty_writers.remove(&id.0);
        // Kill the process if still running
        self.supervisor.kill(&PaneId(id.0)).ok();
    }

    /// Route a message to a specific session.
    pub fn route_to_session(
        &self,
        session_id: SessionId,
        msg: BusMessage,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        handle
            .cmd_tx
            .send(SessionCommand::Deliver(msg))
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
        Ok(())
    }

    /// Route a command to a specific session.
    pub fn send_command(
        &self,
        session_id: SessionId,
        cmd: SessionCommand,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        handle
            .cmd_tx
            .send(cmd)
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
        Ok(())
    }

    /// List all active sessions.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .values()
            .map(|h| SessionInfo {
                session_id: h.id.clone(),
                name: h.name.clone(),
                pane_count: 1,
                isolation: h.isolation,
                state: SessionState::Active,
                _unknown: Vec::new(),
            })
            .collect()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn has_session(&self, id: SessionId) -> bool {
        self.sessions.contains_key(&id.0)
    }

    /// Shutdown all sessions gracefully.
    pub fn shutdown_all(&mut self) {
        let ids: Vec<u32> = self.sessions.keys().copied().collect();
        for id in ids {
            self.destroy_session(SessionId(id));
        }
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

/// Background thread that reads from a PTY and sends output to the session executor.
fn pty_reader_loop(
    mut reader: std::fs::File,
    tx: mpsc::Sender<SessionCommand>,
    pane_id: PaneId,
) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF — process exited
                info!(?pane_id, "PTY reader: EOF");
                break;
            }
            Ok(n) => {
                let data = buf[..n].to_vec();
                if tx
                    .send(SessionCommand::PtyOutput {
                        pane_id: pane_id.clone(),
                        data,
                    })
                    .is_err()
                {
                    // Session executor has been dropped
                    break;
                }
            }
            Err(e) => {
                warn!(?pane_id, error = %e, "PTY reader error");
                break;
            }
        }
    }
}
