use crate::bus::BusMessage;
use crate::executor::pools::PoolConfig;
use crate::executor::session_thread::{SessionCommand, SessionExecutor};
use crate::DaemonError;
use malt_protocol::common::{
    GroupId, IsolationTier, PaneId, SessionId, SessionInfo, SessionState,
};
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread::JoinHandle;
use tracing::info;

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    cmd_tx: mpsc::Sender<SessionCommand>,
    thread: Option<JoinHandle<()>>,
}

/// Coordinator manages session lifecycle and routes messages to session threads.
///
/// Monotonically increasing session IDs — never recycled within daemon lifetime.
pub struct Coordinator {
    sessions: HashMap<u32, SessionHandle>,
    next_session_id: u32,
    next_pane_id: u32,
    #[allow(dead_code)]
    pool_config: PoolConfig,
}

impl Coordinator {
    pub fn new(pool_config: PoolConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            next_session_id: 1,
            next_pane_id: 1,
            pool_config,
        }
    }

    /// Create a new session. Returns the assigned SessionId.
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

        let (cmd_tx, thread) = SessionExecutor::spawn(session_id.clone(), pane_id, isolation)?;

        info!(?session_id, ?name, ?isolation, "session created");

        self.sessions.insert(
            session_id.0,
            SessionHandle {
                id: session_id.clone(),
                name,
                isolation,
                cmd_tx,
                thread: Some(thread),
            },
        );

        Ok(session_id)
    }

    /// Destroy a session. Sends shutdown and joins the thread.
    pub fn destroy_session(&mut self, id: SessionId) {
        if let Some(mut handle) = self.sessions.remove(&id.0) {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown);
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
            info!(?id, "session destroyed");
        }
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
