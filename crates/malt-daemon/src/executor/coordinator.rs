use crate::bus::BusMessage;
use crate::executor::pools::PoolConfig;
use crate::executor::session_thread::{SessionCommand, SessionExecutor};
use crate::store::{DebouncedStore, StoreError};
use crate::supervisor::ProcessSupervisor;
use crate::DaemonError;
use malt_protocol::common::{
    ClientCapabilities, GroupId, IsolationTier, PaneId, SessionId, SessionInfo, SessionState,
};
use malt_protocol::input::KeyEvent;
use malt_protocol::persist::daemon::DaemonState;
use malt_protocol::render::{InitialState, RenderBatch};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Duration;
use tracing::{info, warn};

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    cmd_tx: mpsc::Sender<SessionCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Coordinator manages session lifecycle and routes messages to session threads.
///
/// Monotonically increasing session IDs — never recycled within daemon lifetime.
/// Counter state is persisted to the store and restored on construction.
pub struct Coordinator {
    sessions: HashMap<u32, SessionHandle>,
    next_session_id: u32,
    next_pane_id: u32,
    store: DebouncedStore,
    #[allow(dead_code)]
    supervisor: ProcessSupervisor,
    #[allow(dead_code)]
    pool_config: PoolConfig,
}

impl Coordinator {
    pub fn new(pool_config: PoolConfig, store: DebouncedStore) -> Self {
        let mut next_session_id = 1u32;
        let mut next_pane_id = 1u32;

        match store.load_daemon_state() {
            Ok(state) => {
                next_session_id = state.next_session_id;
                next_pane_id = state.next_pane_id;
                info!(
                    next_session_id,
                    next_pane_id,
                    "coordinator: restored counters from daemon state"
                );
            }
            Err(StoreError::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
                // First run — defaults are fine.
            }
            Err(e) => {
                warn!(%e, "coordinator: daemon state unreadable; starting with defaults");
            }
        }

        Self {
            sessions: HashMap::new(),
            next_session_id,
            next_pane_id,
            store,
            supervisor: ProcessSupervisor::new(),
            pool_config,
        }
    }

    /// Create a new session with an in-process mash shell.
    ///
    /// If `name` is `None`, the base `"session"` is used. If the name (or base)
    /// already exists, numeric suffixes `-2`, `-3` … `-100` are tried in order.
    /// Returns `DaemonError::NameConflict` if all 100 suffixes are taken.
    pub fn create_session(
        &mut self,
        name: Option<String>,
        isolation: IsolationTier,
        _group: Option<GroupId>,
    ) -> Result<SessionId, DaemonError> {
        // --- Name uniqueness ---
        let base = name.unwrap_or_else(|| "session".to_string());
        let existing: HashSet<String> = self
            .sessions
            .values()
            .filter_map(|h| h.name.clone())
            .collect();

        let final_name = if !existing.contains(&base) {
            base.clone()
        } else {
            let mut candidate = String::new();
            let mut found = false;
            for i in 2u32..=100 {
                candidate = format!("{base}-{i}");
                if !existing.contains(&candidate) {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(DaemonError::NameConflict(base));
            }
            candidate
        };

        // --- Session creation ---
        let session_id = SessionId(self.next_session_id);
        self.next_session_id += 1;

        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;

        let (cmd_tx, thread) =
            SessionExecutor::spawn(session_id.clone(), pane_id.clone(), isolation)?;

        info!(?session_id, name = %final_name, "session created with in-process mash shell");

        self.sessions.insert(
            session_id.0,
            SessionHandle {
                id: session_id.clone(),
                name: Some(final_name),
                isolation,
                cmd_tx,
                thread: Some(thread),
            },
        );

        self.persist_daemon_state();
        Ok(session_id)
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
            info!(?id, "session destroyed");
            self.persist_daemon_state();
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

    /// Register a VNP client with a session's renderer.
    pub fn register_vnp_client(
        &self,
        session_id: SessionId,
        client_id: u64,
        capabilities: ClientCapabilities,
        render_tx: mpsc::SyncSender<RenderBatch>,
    ) -> Result<InitialState, DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        let (initial_tx, initial_rx) = mpsc::channel();
        handle
            .cmd_tx
            .send(SessionCommand::RegisterVnpClient {
                client_id,
                capabilities,
                render_tx,
                initial_reply: initial_tx,
            })
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
        initial_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
    }

    /// Unregister a VNP client from a session's renderer (fire-and-forget).
    pub fn unregister_vnp_client(
        &self,
        session_id: SessionId,
        client_id: u64,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        handle
            .cmd_tx
            .send(SessionCommand::UnregisterVnpClient { client_id })
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
    }

    /// Route a typed keyboard event to a session's line editor.
    pub fn send_key_input(
        &self,
        session_id: SessionId,
        key: KeyEvent,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        handle
            .cmd_tx
            .send(SessionCommand::KeyInput { key })
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
    }

    /// Forward a frame acknowledgement to a session's renderer host.
    pub fn ack_frame(
        &self,
        session_id: SessionId,
        client_id: u64,
        frame_seq: u64,
    ) -> Result<(), DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id))?;
        handle
            .cmd_tx
            .send(SessionCommand::AckFrame { client_id, frame_seq })
            .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
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

    // --- Private ---

    fn persist_daemon_state(&self) {
        let state = DaemonState {
            schema_version: 1,
            sessions: self.sessions.values().map(|h| h.id.clone()).collect(),
            active_groups: vec![],
            next_session_id: self.next_session_id,
            next_pane_id: self.next_pane_id,
            _unknown: vec![],
        };
        self.store.mark_dirty_daemon(state);
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
