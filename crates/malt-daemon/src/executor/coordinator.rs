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
use malt_protocol::persist::session::PersistedSession;
use malt_protocol::render::{InitialState, RenderBatch};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Duration;
use tracing::{info, warn};

enum SessionLifecycle {
    Active {
        cmd_tx: mpsc::Sender<SessionCommand>,
        thread: Option<std::thread::JoinHandle<()>>,
        client_count: u32,
    },
    Dormant {
        persisted: PersistedSession,
    },
}

struct SessionHandle {
    id: SessionId,
    name: Option<String>,
    isolation: IsolationTier,
    lifecycle: SessionLifecycle,
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
                lifecycle: SessionLifecycle::Active {
                    cmd_tx,
                    thread: Some(thread),
                    client_count: 0,
                },
            },
        );

        self.persist_daemon_state();
        Ok(session_id)
    }

    /// Get the current output text for a session.
    pub fn get_session_output(&self, session_id: SessionId) -> Result<String, DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                cmd_tx
                    .send(SessionCommand::GetOutput { reply: reply_tx })
                    .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))?;
                reply_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone()))
            }
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
    }

    /// Destroy a session. Sends shutdown and joins the thread.
    pub fn destroy_session(&mut self, id: SessionId) {
        if let Some(mut handle) = self.sessions.remove(&id.0) {
            match handle.lifecycle {
                SessionLifecycle::Active {
                    cmd_tx,
                    mut thread,
                    ..
                } => {
                    let _ = cmd_tx.send(SessionCommand::Shutdown);
                    if let Some(t) = thread.take() {
                        let _ = t.join();
                    }
                }
                SessionLifecycle::Dormant { .. } => {
                    // No thread to shut down.
                }
            }
            let _ = self.store.delete_session(&id);
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
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => cmd_tx
                .send(SessionCommand::Deliver(msg))
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone())),
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
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
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => cmd_tx
                .send(cmd)
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone())),
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
    }

    /// List all active sessions.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .values()
            .map(|h| SessionInfo {
                session_id: h.id.clone(),
                name: h.name.clone(),
                pane_count: match &h.lifecycle {
                    SessionLifecycle::Active { .. } => 1,
                    SessionLifecycle::Dormant { persisted } => persisted.panes.len() as u16,
                },
                isolation: h.isolation,
                state: match &h.lifecycle {
                    SessionLifecycle::Active { .. } => SessionState::Active,
                    SessionLifecycle::Dormant { .. } => SessionState::Dormant,
                },
                _unknown: Vec::new(),
            })
            .collect()
    }

    /// Register a VNP client with a session's renderer.
    ///
    /// Increments the session's `client_count`. When the session is Active,
    /// forwards `RegisterVnpClient` to the session thread and returns the
    /// `InitialState` snapshot. Returns `DaemonError::SessionDormant` if the
    /// session is not currently running.
    pub fn register_vnp_client(
        &mut self,
        session_id: SessionId,
        client_id: u64,
        capabilities: ClientCapabilities,
        render_tx: mpsc::SyncSender<RenderBatch>,
    ) -> Result<InitialState, DaemonError> {
        let handle = self
            .sessions
            .get_mut(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &mut handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, client_count, .. } => {
                let cmd_tx = cmd_tx.clone();
                // Clone the id upfront to avoid a second borrow of `handle` inside
                // the error closures below.
                let session_id_for_err = handle.id.clone();
                let (initial_tx, initial_rx) = mpsc::channel();
                cmd_tx
                    .send(SessionCommand::RegisterVnpClient {
                        client_id,
                        capabilities,
                        render_tx,
                        initial_reply: initial_tx,
                    })
                    .map_err(|_| DaemonError::SessionUnreachable(session_id_for_err.clone()))?;
                let result = initial_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| DaemonError::SessionUnreachable(session_id_for_err));
                if result.is_ok() {
                    *client_count += 1;
                }
                result
            }
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
    }

    /// Unregister a VNP client from a session's renderer.
    ///
    /// Decrements the session's `client_count`. If `client_count` reaches zero
    /// the session is transitioned to `Dormant` via `go_dormant`.
    pub fn unregister_vnp_client(
        &mut self,
        session_id: SessionId,
        client_id: u64,
    ) -> Result<(), DaemonError> {
        {
            let handle = self
                .sessions
                .get_mut(&session_id.0)
                .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
            match &mut handle.lifecycle {
                SessionLifecycle::Active { cmd_tx, client_count, .. } => {
                    let _ = cmd_tx.send(SessionCommand::UnregisterVnpClient { client_id });
                    if *client_count > 0 {
                        *client_count -= 1;
                    }
                }
                SessionLifecycle::Dormant { .. } => return Ok(()), // nothing to unregister
            }
        }
        // Transition to Dormant if the last client just detached.
        let should_go_dormant = match self.sessions.get(&session_id.0) {
            Some(h) => matches!(
                &h.lifecycle,
                SessionLifecycle::Active { client_count, .. } if *client_count == 0
            ),
            None => false,
        };
        if should_go_dormant {
            self.go_dormant(session_id);
        }
        Ok(())
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
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => cmd_tx
                .send(SessionCommand::KeyInput { key })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone())),
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
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
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => cmd_tx
                .send(SessionCommand::AckFrame { client_id, frame_seq })
                .map_err(|_| DaemonError::SessionUnreachable(handle.id.clone())),
            SessionLifecycle::Dormant { .. } => Err(DaemonError::SessionDormant(session_id)),
        }
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

    /// Transition an Active session to Dormant.
    ///
    /// Sends a `Snapshot` command to the session thread and waits up to 5 s
    /// for the reply. On success:
    ///   1. Persists the snapshot synchronously via `flush_all`.
    ///   2. Sends `Shutdown` to the session thread and joins it.
    ///   3. Replaces the session lifecycle with `Dormant { persisted }`.
    ///   4. Updates the persisted daemon state.
    ///
    /// If the snapshot times out, the session is left Active and a warning is
    /// logged — this is a best-effort degradation path, not a hard error.
    fn go_dormant(&mut self, id: SessionId) {
        let (cmd_tx_clone, session_name, session_isolation) = {
            let handle = match self.sessions.get(&id.0) {
                Some(h) => h,
                None => return,
            };
            match &handle.lifecycle {
                SessionLifecycle::Active { cmd_tx, .. } => {
                    (cmd_tx.clone(), handle.name.clone(), handle.isolation)
                }
                SessionLifecycle::Dormant { .. } => return,
            }
        };

        // Request a snapshot from the session thread.
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = cmd_tx_clone.send(SessionCommand::Snapshot {
            reply: reply_tx,
            name: session_name,
            isolation: session_isolation,
        });

        let persisted = match reply_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(p) => p,
            Err(_) => {
                warn!(?id, "go_dormant: Snapshot timed out; leaving session Active");
                return;
            }
        };

        // Persist synchronously so state is on disk before killing the thread.
        self.store.mark_dirty(id.clone(), persisted.clone());
        self.store.flush_all();

        // Shut down the session thread.
        let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
        if let Some(handle) = self.sessions.get_mut(&id.0) {
            if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
                if let Some(t) = thread.take() {
                    let _ = t.join();
                }
            }
            handle.lifecycle = SessionLifecycle::Dormant { persisted };
        }

        self.persist_daemon_state();
        info!(?id, "session transitioned to Dormant");
    }

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
