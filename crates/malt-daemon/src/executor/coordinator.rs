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
    /// The session's one pane, under today's single-pane model. Stable
    /// across Active<->Dormant transitions since it lives on the outer
    /// struct, not inside `SessionLifecycle`.
    first_pane: PaneId,
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
        let mut initial_sessions: HashMap<u32, SessionHandle> = HashMap::new();

        match store.load_daemon_state() {
            Ok(state) => {
                next_session_id = state.next_session_id;
                next_pane_id = state.next_pane_id;
                info!(
                    next_session_id,
                    next_pane_id, "coordinator: restored counters from daemon state"
                );

                for sid in &state.sessions {
                    match store.load_session(sid) {
                        Ok(persisted) => {
                            let first_pane = persisted
                                .panes
                                .keys()
                                .next()
                                .copied()
                                .map(PaneId)
                                .unwrap_or(PaneId(0));
                            initial_sessions.insert(
                                sid.0,
                                SessionHandle {
                                    id: persisted.id.clone(),
                                    name: persisted.name.clone(),
                                    isolation: persisted.isolation,
                                    first_pane,
                                    lifecycle: SessionLifecycle::Dormant { persisted },
                                },
                            );
                        }
                        Err(StoreError::SessionNotFound(_)) => {
                            warn!(
                                ?sid,
                                "coordinator startup: persisted session not found; skipping"
                            );
                        }
                        Err(e) => {
                            warn!(?sid, %e, "coordinator startup: failed to load persisted session; skipping");
                        }
                    }
                }
            }
            Err(StoreError::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
                // First run — defaults are fine.
            }
            Err(e) => {
                warn!(%e, "coordinator: daemon state unreadable; starting with defaults");
            }
        }

        Self {
            sessions: initial_sessions,
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
                first_pane: pane_id,
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

    /// Get the current output text for a session, as styled-grid JSON.
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

    /// Get the current output text for a session, as plain text with no
    /// styling — for programmatic/agent consumption.
    pub fn get_session_output_text(&self, session_id: SessionId) -> Result<String, DaemonError> {
        let handle = self
            .sessions
            .get(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &handle.lifecycle {
            SessionLifecycle::Active { cmd_tx, .. } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                cmd_tx
                    .send(SessionCommand::GetOutputText { reply: reply_tx })
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
                    cmd_tx, mut thread, ..
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
        // Check whether the session is Dormant — if so, restore it first.
        // We do this with a short immutable borrow so that the subsequent
        // `restore_session(&mut self)` call does not fight the borrow checker.
        let is_dormant = match self.sessions.get(&session_id.0) {
            None => return Err(DaemonError::SessionNotFound(session_id)),
            Some(h) => matches!(h.lifecycle, SessionLifecycle::Dormant { .. }),
        };
        if is_dormant {
            self.restore_session(session_id.clone())?;
        }

        // At this point the session is Active.
        let handle = self
            .sessions
            .get_mut(&session_id.0)
            .ok_or(DaemonError::SessionNotFound(session_id.clone()))?;
        match &mut handle.lifecycle {
            SessionLifecycle::Active {
                cmd_tx,
                client_count,
                ..
            } => {
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
            // restore_session succeeded but lifecycle is still Dormant — should
            // never happen, but defend against it rather than panic.
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
                SessionLifecycle::Active {
                    cmd_tx,
                    client_count,
                    ..
                } => {
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
    pub fn send_key_input(&self, session_id: SessionId, key: KeyEvent) -> Result<(), DaemonError> {
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
                .send(SessionCommand::AckFrame {
                    client_id,
                    frame_seq,
                })
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

    /// The session's one real pane id, under today's single-pane model.
    pub fn session_first_pane(&self, id: SessionId) -> Option<PaneId> {
        self.sessions.get(&id.0).map(|h| h.first_pane.clone())
    }

    /// Snapshot all Active sessions, persist them, join threads.
    ///
    /// After this call, all sessions are either Dormant (snapshot succeeded) or
    /// their threads are joined (snapshot timed out). The store is flushed.
    ///
    /// Called explicitly from daemon.rs before process exit. Also called from
    /// `Drop` as a safety net (idempotent — second call finds no Active sessions).
    pub fn shutdown_graceful(&mut self) {
        let ids: Vec<u32> = self.sessions.keys().copied().collect();
        for id in ids {
            let session_id = SessionId(id);
            let (cmd_tx_clone, session_name, session_isolation) = {
                let handle = match self.sessions.get(&session_id.0) {
                    Some(h) => h,
                    None => continue,
                };
                match &handle.lifecycle {
                    SessionLifecycle::Active { cmd_tx, .. } => {
                        (cmd_tx.clone(), handle.name.clone(), handle.isolation)
                    }
                    SessionLifecycle::Dormant { .. } => continue,
                }
            };

            // Snapshot.
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = cmd_tx_clone.send(SessionCommand::Snapshot {
                reply: reply_tx,
                name: session_name,
                isolation: session_isolation,
            });
            match reply_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(persisted) => {
                    self.store.mark_dirty(session_id.clone(), persisted.clone());
                    // Shut down thread and transition to Dormant.
                    if let Some(handle) = self.sessions.get_mut(&session_id.0) {
                        if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
                            let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
                            if let Some(t) = thread.take() {
                                let _ = t.join();
                            }
                        }
                        handle.lifecycle = SessionLifecycle::Dormant { persisted };
                    }
                }
                Err(_) => {
                    warn!(
                        ?session_id,
                        "shutdown_graceful: Snapshot timeout; skipping save for this session"
                    );
                    // Shut down thread anyway so it doesn't linger.
                    let _ = cmd_tx_clone.send(SessionCommand::Shutdown);
                    if let Some(handle) = self.sessions.get_mut(&session_id.0) {
                        if let SessionLifecycle::Active { thread, .. } = &mut handle.lifecycle {
                            if let Some(t) = thread.take() {
                                let _ = t.join();
                            }
                        }
                    }
                }
            }
        }

        self.persist_daemon_state();
        self.store.flush_all();
    }

    /// Shutdown all sessions gracefully.
    ///
    /// Active sessions are sent `Shutdown` and their threads are joined.
    /// Dormant sessions are left intact in the store so they can be restored
    /// on the next daemon startup.
    pub fn shutdown_all(&mut self) {
        let ids: Vec<u32> = self.sessions.keys().copied().collect();
        for id in ids {
            let is_active = matches!(
                self.sessions.get(&id).map(|h| &h.lifecycle),
                Some(SessionLifecycle::Active { .. })
            );
            if is_active {
                self.destroy_session(SessionId(id));
            }
            // Dormant handles are intentionally left in place — their data is
            // already on disk and should survive daemon restart.
        }
    }

    // --- Private ---

    /// Restore a Dormant Shell session to Active by re-spawning its mash thread.
    ///
    /// For App or Compat pane types this returns a specific error — those restore
    /// paths are not yet implemented.  On success the lifecycle transitions from
    /// `Dormant` to `Active` and `persist_daemon_state` is called.
    fn restore_session(&mut self, id: SessionId) -> Result<(), DaemonError> {
        let (persisted, _session_name, session_isolation) = {
            let handle = self
                .sessions
                .get(&id.0)
                .ok_or(DaemonError::SessionNotFound(id.clone()))?;
            match &handle.lifecycle {
                SessionLifecycle::Dormant { persisted } => {
                    (persisted.clone(), handle.name.clone(), handle.isolation)
                }
                SessionLifecycle::Active { .. } => return Ok(()),
            }
        };

        let (pane_id_raw, pane) = persisted.panes.iter().next().ok_or_else(|| {
            DaemonError::RestoreFailed(id.clone(), "no panes in persisted session".to_string())
        })?;

        let pane_id = PaneId(*pane_id_raw);
        let cwd = std::path::PathBuf::from(&pane.cwd);

        let (cmd_tx, thread) = match &pane.pane_type {
            malt_protocol::persist::session::PersistedPaneType::Shell {
                shell_path,
                env_snapshot,
            } => {
                let env_snapshot = env_snapshot
                    .as_ref()
                    .map(crate::executor::session_thread::from_persisted_env_snapshot);
                SessionExecutor::spawn_with_cwd(
                    id.clone(),
                    pane_id,
                    session_isolation,
                    cwd,
                    Some(shell_path.clone()),
                    env_snapshot,
                )
                .map_err(|e| DaemonError::RestoreFailed(id.clone(), e.to_string()))?
            }
            malt_protocol::persist::session::PersistedPaneType::App { .. } => {
                return Err(DaemonError::AppRestoreNotSupported);
            }
            malt_protocol::persist::session::PersistedPaneType::Compat { program, args } => {
                // Re-launch (not re-attach) -- matches the documented restore
                // policy for every pane type: process memory is not
                // captured, so a fresh CompatTranslator (blank grid) is
                // correct, not a shortcut. A SessionExecutor is still needed
                // even for a Compat-typed pane (it owns the renderer,
                // editor, and CompatTranslator regardless of pane kind) --
                // spawn one exactly like the Shell path, then separately
                // launch the real external process and forward its output
                // into it via PtyOutput.
                let (cmd_tx, thread) = SessionExecutor::spawn_with_cwd(
                    id.clone(),
                    pane_id.clone(),
                    session_isolation,
                    cwd.clone(),
                    None,
                    None,
                )
                .map_err(|e| DaemonError::RestoreFailed(id.clone(), e.to_string()))?;

                let req = crate::supervisor::process::SpawnRequest {
                    program: std::path::PathBuf::from(program),
                    args: args.clone(),
                    cwd,
                    pane_id: pane_id.clone(),
                    isolation: session_isolation,
                    cols: 80,
                    rows: 24,
                };
                if let Err(e) = self.supervisor.spawn(req) {
                    // Roll back the SessionExecutor thread we already
                    // started -- don't leave an Active session with no
                    // process behind it.
                    let _ = cmd_tx.send(SessionCommand::Shutdown);
                    let _ = thread.join();
                    return Err(DaemonError::RestoreFailed(id.clone(), e.to_string()));
                }

                // NOTE: the restored process is not assigned to the
                // session's isolation Job Object -- ProcessSupervisor has
                // no access to it (that lives inside the SessionExecutor
                // thread's mash::Env, not the Coordinator). A
                // Restricted/Capped/Contained session's restored compat
                // process runs uncontained. Real, tracked gap, not a
                // silent one -- see docs/BACKLOG.md.
                if let Some((reader, _writer)) = self.supervisor.take_io(&pane_id) {
                    spawn_pty_reader(pane_id.clone(), reader, cmd_tx.clone());
                }

                (cmd_tx, thread)
            }
            _ => {
                return Err(DaemonError::RestoreFailed(
                    id.clone(),
                    "unknown pane type".to_string(),
                ));
            }
        };

        if let Some(handle) = self.sessions.get_mut(&id.0) {
            handle.lifecycle = SessionLifecycle::Active {
                cmd_tx,
                thread: Some(thread),
                client_count: 0,
            };
        }

        self.persist_daemon_state();
        info!(?id, "session restored from Dormant to Active");
        Ok(())
    }

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
                warn!(
                    ?id,
                    "go_dormant: Snapshot timed out; leaving session Active"
                );
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
        self.shutdown_graceful();
    }
}

/// Read a spawned process's PTY output in a loop and forward each chunk as
/// a `SessionCommand::PtyOutput` to the owning session's command channel.
/// Exits cleanly on EOF (process closed its output), a real read error, or
/// the session channel being gone (session shut down before the process
/// did). Not a restart/supervision loop — that's `ProcessSupervisor::check_exited`'s
/// job, and nothing currently polls it (see `docs/BACKLOG.md`); this
/// thread's only responsibility is getting bytes from the process into the
/// session's renderer pipeline.
fn spawn_pty_reader(
    pane_id: PaneId,
    mut reader: std::fs::File,
    cmd_tx: mpsc::Sender<SessionCommand>,
) {
    use std::io::Read;
    let result = std::thread::Builder::new()
        .name(format!("pty-reader-{}", pane_id.0))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF: process closed its output
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        if cmd_tx
                            .send(SessionCommand::PtyOutput {
                                pane_id: pane_id.clone(),
                                data,
                            })
                            .is_err()
                        {
                            break; // session gone
                        }
                    }
                    Err(e) => {
                        warn!(pane_id = pane_id.0, error = %e, "pty reader: read error, stopping");
                        break;
                    }
                }
            }
        });
    if let Err(e) = result {
        warn!(error = %e, "failed to spawn pty reader thread");
    }
}
