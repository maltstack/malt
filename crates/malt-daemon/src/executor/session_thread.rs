use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use crate::executor::command_worker::{
    spawn_command_worker, ExecutionCompletion, ExecutionIngress,
};
use crate::DaemonError;
use malt_compat::CompatTranslator;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{ClientCapabilities, IsolationTier, PaneId, ResolvedPane, SessionId};
use malt_protocol::input::KeyEvent;
use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
use malt_protocol::priority::Priority;
use malt_protocol::render::{InitialState, RenderBatch};
use malt_renderer::host::{PaneFrame, RendererHost};
use malt_session::session::SessionRuntime;
use malt_term::{EditMode, EditResult, Editor};
use mash::env::{Env, EnvSnapshot, Variable};
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};

/// Placeholder Job Object resource caps for the Capped/Contained tiers,
/// pending a real per-session/group configuration surface (see
/// `docs/BACKLOG.md`'s isolation-policy item). Deliberately conservative-but-
/// generous rather than tuned: the point of this pass is that Capped and
/// Contained sessions actually get *some* real, different-from-Restricted
/// resource limit, not that these specific numbers are load-bearing.
#[cfg(windows)]
const CAPPED_MEMORY_LIMIT_MB: u64 = 2048;
#[cfg(windows)]
const CAPPED_CPU_RATE_PERCENT: u32 = 200;

/// Job Object `(memory_limit_mb, cpu_rate)` for a given tier. `Bare` is
/// never passed in (callers return before reaching this); `Restricted` gets
/// an uncapped Job Object (group-kill only); `Capped`/`Contained` get real
/// limits. Pulled out as a pure function so it's unit-testable without
/// creating a real Windows Job Object.
#[cfg(windows)]
fn job_object_limits_for_tier(isolation: IsolationTier) -> (u64, u32) {
    match isolation {
        IsolationTier::Bare | IsolationTier::Restricted => (0, 0),
        IsolationTier::Capped | IsolationTier::Contained => {
            (CAPPED_MEMORY_LIMIT_MB, CAPPED_CPU_RATE_PERCENT)
        }
    }
}

/// Apply a session's isolation tier to its MASH environment: sets the opaque
/// isolation context token, and on Windows, creates a Job Object every
/// externally-spawned command in this session gets assigned to (see
/// `mash::executor`'s spawn call site). Best-effort — if job object creation
/// fails, the session still runs, just without process containment.
///
/// Bare tier does nothing (no job object needed). Restricted gets an
/// uncapped Job Object (group-kill only). Capped and Contained get the same
/// Job Object with real memory/CPU limits — previously all three non-Bare
/// tiers got an identical, uncapped Job Object, so Capped's "resource
/// enforcement" promise and Contained's went unfulfilled even on the
/// success path (see `docs/BACKLOG.md`).
///
/// This does **not** yet give Contained anything beyond Capped-level Job
/// Object containment. Real HCS container isolation for Contained requires
/// launching processes *inside* the compute system
/// (`malt_platform::isolation::hcs::create_process`), not just creating one
/// — that needs the actual process spawn path
/// (`malt_platform::process::spawn`, `mash`'s external-command call sites)
/// to become HCS-aware, which is real design work, not a parameter change.
/// Creating an HCS compute system here that no process ever actually runs
/// inside would be exactly the "looks done but isn't" pattern this project
/// is trying to stop repeating — tracked as a separate, larger item in
/// `docs/BACKLOG.md` rather than half-wired here.
fn apply_session_isolation(env: &mut Env, session_id: SessionId, isolation: IsolationTier) {
    env.set_isolation_context(malt_platform::isolation::IsolationContext::from(isolation));

    #[cfg(windows)]
    {
        if isolation == IsolationTier::Bare {
            return;
        }
        let (memory_limit_mb, cpu_rate) = job_object_limits_for_tier(isolation);
        let job_name = format!("malt-session-{}", session_id.0);
        match malt_platform::isolation::job_objects::create_job_object(
            &job_name,
            memory_limit_mb,
            cpu_rate,
        ) {
            Ok(job) => env.set_job_object(std::sync::Arc::new(job)),
            Err(error) => warn!(
                ?session_id,
                %error,
                "failed to create job object for session isolation; session will run without process containment"
            ),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = session_id;
    }
}

/// Result of running a command line through mash: the captured stdout/stderr
/// text (what callers display) plus the command's exit code (what `$?`
/// would be).
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub command_id: u32,
    pub output: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// The control and worker handles created for an active session.  Keeping
/// these separate makes ownership explicit: the control thread owns UI,
/// persistence, and lifecycle state; the worker alone owns MASH state.
pub struct SessionSpawn {
    pub control_tx: mpsc::Sender<SessionCommand>,
    pub ingress: ExecutionIngress,
    pub control_thread: JoinHandle<()>,
    pub worker_thread: JoinHandle<()>,
}

const FINALIZATION_SLICE_BYTES: usize = 128 * 1024;

struct Finalization {
    sequence: u64,
    output: CommandOutput,
    snapshot: EnvSnapshot,
    reply: mpsc::Sender<Result<CommandOutput, DaemonError>>,
    finalized: mpsc::Sender<()>,
    staged_compat: Option<CompatTranslator>,
    stdout_offset: usize,
    stderr_offset: usize,
}

/// Commands sent from the coordinator to a session executor.
pub enum SessionCommand {
    /// Deliver a message to the session's bus.
    Deliver(BusMessage),
    /// Attach a client to this session.
    AttachClient {
        client_id: u64,
        authority: malt_protocol::common::InputAuthority,
    },
    /// Detach a client from this session.
    DetachClient { client_id: u64 },
    /// Resize the terminal.
    Resize { cols: u16, rows: u16 },
    /// Raw bytes from PTY output (from reader thread).
    PtyOutput { pane_id: PaneId, data: Vec<u8> },
    /// Execute a command via mash (from exec_command API).
    RunCommand {
        command: String,
        reply: mpsc::Sender<CommandOutput>,
    },
    /// A result sent by the sole MASH owner. It is committed on this actor
    /// before the worker may take another request.
    ExecutionCompleted(ExecutionCompletion),
    /// Write input to PTY stdin.
    WriteInput { data: Vec<u8> },
    /// Get the current output snapshot as styled-grid JSON (requester sends
    /// back via channel). Built for human rendering clients
    /// (`malt-tui`/`maltty`) — for a program-readable variant see
    /// `GetOutputText`.
    GetOutput { reply: mpsc::Sender<String> },
    /// Get the current output snapshot as plain text, no styling — built
    /// for programmatic/agent consumption. Same underlying grid as
    /// `GetOutput`, different rendering.
    GetOutputText { reply: mpsc::Sender<String> },
    /// Register a VNP client with this session's renderer.
    RegisterVnpClient {
        client_id: u64,
        capabilities: ClientCapabilities,
        render_tx: SyncSender<RenderBatch>,
        initial_reply: mpsc::Sender<InitialState>,
    },
    /// Remove a VNP client from this session's renderer.
    UnregisterVnpClient { client_id: u64 },
    /// A typed keyboard event from a VNP client.
    KeyInput { key: KeyEvent },
    /// A frame acknowledgement from a VNP client.
    AckFrame { client_id: u64, frame_seq: u64 },
    /// Take a snapshot of the current session state for persistence.
    /// The reply channel receives a `PersistedSession` built from current env + layout.
    Snapshot {
        reply: mpsc::Sender<PersistedSession>,
        name: Option<String>,
        isolation: IsolationTier,
    },
    /// Graceful shutdown.
    Shutdown,
}

// Manual Debug impl since mpsc::Sender doesn't implement Debug
impl std::fmt::Debug for SessionCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deliver(m) => f.debug_tuple("Deliver").field(m).finish(),
            Self::AttachClient {
                client_id,
                authority,
            } => f
                .debug_struct("AttachClient")
                .field("client_id", client_id)
                .field("authority", authority)
                .finish(),
            Self::DetachClient { client_id } => f
                .debug_struct("DetachClient")
                .field("client_id", client_id)
                .finish(),
            Self::Resize { cols, rows } => f
                .debug_struct("Resize")
                .field("cols", cols)
                .field("rows", rows)
                .finish(),
            Self::PtyOutput { pane_id, data } => f
                .debug_struct("PtyOutput")
                .field("pane_id", pane_id)
                .field("len", &data.len())
                .finish(),
            Self::RunCommand { command, .. } => f
                .debug_struct("RunCommand")
                .field("command", command)
                .finish(),
            Self::ExecutionCompleted(_) => f.debug_struct("ExecutionCompleted").finish(),
            Self::WriteInput { data } => f
                .debug_struct("WriteInput")
                .field("len", &data.len())
                .finish(),
            Self::GetOutput { .. } => f.debug_struct("GetOutput").finish(),
            Self::GetOutputText { .. } => f.debug_struct("GetOutputText").finish(),
            Self::RegisterVnpClient { client_id, .. } => f
                .debug_struct("RegisterVnpClient")
                .field("client_id", client_id)
                .finish(),
            Self::UnregisterVnpClient { client_id } => f
                .debug_struct("UnregisterVnpClient")
                .field("client_id", client_id)
                .finish(),
            Self::KeyInput { .. } => f.debug_struct("KeyInput").finish(),
            Self::AckFrame {
                client_id,
                frame_seq,
            } => f
                .debug_struct("AckFrame")
                .field("client_id", client_id)
                .field("frame_seq", frame_seq)
                .finish(),
            Self::Snapshot { .. } => write!(f, "Snapshot"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// Per-session executor running on a dedicated thread.
pub struct SessionExecutor {
    session: SessionRuntime,
    bus: Bus,
    authority: AuthorityTracker,
    terminal_size: Rect,
    layout_config: LayoutConfig,
    compat: Option<CompatTranslator>,
    /// Full shell state as of the last finalized command. The worker's live
    /// Env is never shared or read by this actor.
    env_snapshot: EnvSnapshot,
    ingress: ExecutionIngress,
    renderer: RendererHost,
    editor: Editor,
    render_pushers: HashMap<u64, SyncSender<RenderBatch>>,
    resolved_panes: Vec<ResolvedPane>,
    finalization: Option<Finalization>,
    expected_completion_sequence: u64,
}

impl SessionExecutor {
    /// Spawn a new session executor on a dedicated thread.
    /// Returns the command sender and thread handle.
    pub fn spawn(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
    ) -> Result<(mpsc::Sender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let spawned = Self::spawn_with_capacity(session_id, first_pane, isolation, 256)?;
        Ok((spawned.control_tx, spawned.control_thread))
    }

    /// Start the split control/worker architecture with an explicit bounded
    /// pending-execution capacity.
    pub fn spawn_with_capacity(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        capacity: usize,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        apply_session_isolation(&mut env, session_id.clone(), isolation);
        Self::spawn_with_env(session_id, first_pane, isolation, capacity, env)
    }

    /// Spawn a new session executor on a dedicated thread, setting the working
    /// directory and restoring shell state from a prior session.
    ///
    /// If `initial_cwd` is not a directory at spawn time, a warning is logged
    /// and the OS-inherited cwd is used instead. `shell_path`, if present, is
    /// restored into the `SHELL` env var (previously computed and persisted
    /// but never read back — see `docs/BACKLOG.md`). `env_snapshot`, if
    /// present, restores exported/non-exported variables, options, aliases,
    /// functions, directory stack, and traps via `Env::apply_snapshot` —
    /// this is the actual fix for "session state silently dropped on
    /// restart," not just the two scalars (`SHELL`/`PWD`) this function
    /// already handled.
    pub fn spawn_with_cwd(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
        shell_path: Option<String>,
        env_snapshot: Option<mash::env::EnvSnapshot>,
    ) -> Result<(mpsc::Sender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let spawned = Self::spawn_with_cwd_and_capacity(
            session_id,
            first_pane,
            isolation,
            initial_cwd,
            shell_path,
            env_snapshot,
            256,
        )?;
        Ok((spawned.control_tx, spawned.control_thread))
    }

    pub fn spawn_with_cwd_and_capacity(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
        shell_path: Option<String>,
        env_snapshot: Option<EnvSnapshot>,
        capacity: usize,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        apply_session_isolation(&mut env, session_id.clone(), isolation);
        if let Some(snapshot) = &env_snapshot {
            env.apply_snapshot(snapshot);
        }
        if let Some(shell_path) = shell_path {
            if let Err(error) = env.set_global("SHELL", Variable::exported_string(shell_path)) {
                warn!(%error, "spawn_with_cwd: failed to restore SHELL in mash env");
            }
        }
        if initial_cwd.is_dir() {
            let cwd = initial_cwd.to_string_lossy().to_string();
            if let Err(error) = env.set_global("PWD", Variable::exported_string(cwd)) {
                warn!(%error, "spawn_with_cwd: failed to restore PWD in mash env");
            }
        } else {
            warn!(?initial_cwd, "spawn_with_cwd: directory no longer exists; falling back to OS cwd");
        }
        Self::spawn_with_env(session_id, first_pane, isolation, capacity, env)
    }

    fn spawn_with_env(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        capacity: usize,
        env: Env,
    ) -> Result<SessionSpawn, DaemonError> {
        let snapshot = env.to_snapshot();
        let (control_tx, control_rx) = mpsc::channel();
        let (ingress, requests) = ExecutionIngress::new(session_id.clone(), capacity)?;
        let worker_thread = spawn_command_worker(
            session_id.clone(),
            env,
            requests,
            control_tx.clone(),
            ingress.clone(),
        )?;
        let control_ingress = ingress.clone();
        let control_thread = thread::Builder::new()
            .name(format!("session-control-{}", session_id.0))
            .spawn(move || {
                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                    compat: None,
                    env_snapshot: snapshot,
                    ingress: control_ingress,
                    renderer: RendererHost::new(),
                    editor: Editor::new(EditMode::Emacs),
                    render_pushers: HashMap::new(),
                    resolved_panes: Vec::new(),
                    finalization: None,
                    expected_completion_sequence: 1,
                };
                executor.run(control_rx);
            })
            .map_err(DaemonError::Io)?;
        Ok(SessionSpawn {
            control_tx,
            ingress,
            control_thread,
            worker_thread,
        })
    }

    /// Initialize the compat translator for this session.
    pub fn init_compat(&mut self, cols: u16, rows: u16) {
        self.compat = Some(CompatTranslator::new(cols, rows));
    }

    fn run(&mut self, rx: mpsc::Receiver<SessionCommand>) {
        info!(session = ?self.session.id(), "session executor started");
        // Initialize compat translator with default terminal size
        self.init_compat(self.terminal_size.w, self.terminal_size.h);
        // Compute initial layout so resolved_panes is populated before any render.
        self.recompute_layout();

        loop {
            let next = if self.finalization.is_some() {
                rx.recv_timeout(Duration::from_millis(1))
            } else {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            match next {
                Ok(SessionCommand::Shutdown) => {
                    info!(session = ?self.session.id(), "session executor shutting down");
                    self.ingress.close();
                    break;
                }
                Ok(SessionCommand::Deliver(msg)) => {
                    self.bus.publish(msg);
                }
                Ok(SessionCommand::AttachClient {
                    client_id,
                    authority,
                }) => {
                    self.authority.attach(client_id, authority);
                    let _ = self.session.attach(client_id, authority);
                }
                Ok(SessionCommand::DetachClient { client_id }) => {
                    self.authority.detach(client_id);
                    let _ = self.session.detach(client_id);
                }
                Ok(SessionCommand::Resize { cols, rows }) => {
                    self.terminal_size = Rect::new(0, 0, cols, rows);
                    if let Some(compat) = &mut self.compat {
                        compat.resize(cols, rows);
                    }
                    self.recompute_layout();
                }
                Ok(SessionCommand::RunCommand { command, reply }) => {
                    self.submit_legacy_command(command, reply);
                }
                Ok(SessionCommand::ExecutionCompleted(completion)) => self.begin_finalization(completion),
                Ok(SessionCommand::PtyOutput { data, .. }) => {
                    if let Some(compat) = &mut self.compat {
                        compat.feed(&data);
                    }
                    // Publish output to bus for subscribers
                    self.bus.publish(BusMessage {
                        domain: 1,   // Shell
                        msg_type: 4, // OutputChunk
                        priority: Priority::Normal,
                        producer_id: 0,
                        payload: data,
                    });
                    self.dispatch_render();
                }
                Ok(SessionCommand::WriteInput { data }) => {
                    let input = String::from_utf8_lossy(&data);
                    let input = input.trim();
                    if !input.is_empty() {
                        self.submit_discarded_command(input.to_string());
                    }
                }
                Ok(SessionCommand::GetOutput { reply }) => {
                    let output = self.get_grid_output();
                    let _ = reply.send(output);
                }
                Ok(SessionCommand::GetOutputText { reply }) => {
                    let output = self.get_plain_text_output();
                    let _ = reply.send(output);
                }
                Ok(SessionCommand::RegisterVnpClient {
                    client_id,
                    capabilities,
                    render_tx,
                    initial_reply,
                }) => {
                    self.renderer.register_client(client_id, capabilities);
                    self.render_pushers.insert(client_id, render_tx);
                    let element = match &self.compat {
                        Some(c) => c.frame_element(),
                        None => malt_protocol::frame_element::FrameElement::VtPassthrough {
                            data: Vec::new(),
                        },
                    };
                    let pane_id = self.session.focused_pane().clone();
                    let panes = vec![PaneFrame { pane_id, element }];
                    let layout = self.resolved_panes.clone();
                    let initial = self
                        .renderer
                        .snapshot_initial_state(&panes, &layout, client_id);
                    if initial_reply.send(initial).is_err() {
                        warn!(
                            client_id,
                            "RegisterVnpClient: initial_reply receiver dropped before send"
                        );
                    }
                }
                Ok(SessionCommand::UnregisterVnpClient { client_id }) => {
                    self.renderer.remove_client(client_id);
                    self.render_pushers.remove(&client_id);
                }
                Ok(SessionCommand::KeyInput { key }) => {
                    if let Some(input_event) = crate::input_bridge::vnp_key_to_input_event(&key) {
                        match self.editor.feed(input_event) {
                            EditResult::Accept(line) => {
                                self.submit_discarded_command(line);
                                self.editor.reset();
                                self.dispatch_render();
                            }
                            EditResult::Interrupt => {
                                self.editor.reset();
                                if let Some(compat) = &mut self.compat {
                                    compat.feed(b"^C\r\n");
                                }
                                self.dispatch_render();
                            }
                            EditResult::Eof | EditResult::Suspend => {
                                self.editor.reset();
                                self.dispatch_render();
                            }
                            EditResult::Continue => {}
                            // EditResult is #[non_exhaustive]; handle future variants gracefully.
                            _ => {}
                        }
                    }
                }
                Ok(SessionCommand::AckFrame {
                    client_id,
                    frame_seq,
                }) => {
                    self.renderer.ack_frame(client_id, frame_seq);
                }
                Ok(SessionCommand::Snapshot {
                    reply,
                    name,
                    isolation,
                }) => {
                    let persisted = build_persisted_session(
                        self.session.id(),
                        self.session.focused_pane(),
                        name.as_deref(),
                        isolation,
                        &self.env_snapshot,
                    );
                    let _ = reply.send(persisted);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    warn!(session = ?self.session.id(), "command channel closed");
                    break;
                }
            }
            self.advance_finalization();
        }
    }

    /// Dispatch a render frame to all registered VNP clients.
    fn dispatch_render(&mut self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        for stale_id in self.renderer.shed_stale_clients(now_ms) {
            // Dropping the sender here closes the VNP listener's render_rx,
            // which its main loop already treats as a disconnect signal
            // (see vnp_listener.rs's TryRecvError::Disconnected branch).
            warn!(
                client_id = stale_id,
                "shedding VNP client: no FrameAck in 10s"
            );
            self.render_pushers.remove(&stale_id);
        }

        if self.render_pushers.is_empty() {
            return;
        }
        let element = match &self.compat {
            Some(c) => c.frame_element(),
            // When compat is None, no content exists yet — skip the render dispatch.
            // Note: RegisterVnpClient sends an empty VtPassthrough frame in this case,
            // because the initial snapshot is required to complete the handshake regardless.
            None => return,
        };
        let pane_id = self.session.focused_pane().clone();
        let panes = vec![PaneFrame { pane_id, element }];
        let layout = self.resolved_panes.clone();
        let batches = self.renderer.process_frame(&panes, &layout);
        for crb in batches {
            if let Some(tx) = self.render_pushers.get(&crb.client_id) {
                // Non-blocking: drop if channel full (client lagging)
                let _ = tx.try_send(crb.batch);
            }
        }
    }

    fn submit_legacy_command(&mut self, command: String, reply: mpsc::Sender<CommandOutput>) {
        let (worker_reply, worker_result) = mpsc::channel();
        match self.ingress.submit(command, worker_reply) {
            Ok(()) => {
                // A legacy SessionCommand caller expects a plain CommandOutput.
                // Keep that compatibility without making the control actor wait.
                let _ = thread::Builder::new()
                    .name("session-command-reply".to_string())
                    .spawn(move || match worker_result.recv() {
                        Ok(Ok(output)) => {
                            let _ = reply.send(output);
                        }
                        Ok(Err(error)) => {
                            let _ = reply.send(command_error_output(error));
                        }
                        Err(_) => {
                            let _ = reply.send(command_error_output(DaemonError::ExecutionUnavailable(
                                malt_protocol::common::SessionId(0),
                            )));
                        }
                    });
            }
            Err(error) => {
                self.append_execution_diagnostic(&error);
                let _ = reply.send(command_error_output(error));
            }
        }
    }

    fn submit_discarded_command(&mut self, command: String) {
        let (reply, _result) = mpsc::channel::<Result<CommandOutput, DaemonError>>();
        if let Err(error) = self.ingress.submit(command, reply) {
            self.append_execution_diagnostic(&error);
        }
    }

    fn begin_finalization(&mut self, completion: ExecutionCompletion) {
        match completion.result {
            Ok(worker_output) => {
                if worker_output.sequence != self.expected_completion_sequence {
                    let error = DaemonError::ExecutionUnavailable(self.session.id().clone());
                    self.ingress.mark_unavailable();
                    let _ = completion.reply.send(Err(error));
                    let _ = completion.finalized.send(());
                    return;
                }
                let staged_compat = self
                    .compat
                    .as_ref()
                    .map(CompatTranslator::staging_clone);
                self.finalization = Some(Finalization {
                    sequence: worker_output.sequence,
                    output: worker_output.result,
                    snapshot: worker_output.env_snapshot,
                    reply: completion.reply,
                    finalized: completion.finalized,
                    staged_compat,
                    stdout_offset: 0,
                    stderr_offset: 0,
                });
            }
            Err(error) => {
                self.append_execution_diagnostic(&error);
                let _ = completion.reply.send(Err(error));
                let _ = completion.finalized.send(());
            }
        }
    }

    /// Advance a completed result by at most 128 KiB. While this work is in
    /// progress, the live compat grid and snapshot remain unchanged, so every
    /// control request observes one previous finalized boundary rather than a
    /// partial command. `run` services one mailbox event between turns.
    fn advance_finalization(&mut self) {
        let mut complete = false;
        if let Some(finalization) = &mut self.finalization {
            let mut remaining = FINALIZATION_SLICE_BYTES;
            if let Some(compat) = &mut finalization.staged_compat {
                let stdout = finalization.output.output.as_bytes();
                let stdout_end = (finalization.stdout_offset + remaining).min(stdout.len());
                if stdout_end > finalization.stdout_offset {
                    compat.feed(&stdout[finalization.stdout_offset..stdout_end]);
                    remaining -= stdout_end - finalization.stdout_offset;
                    finalization.stdout_offset = stdout_end;
                }
                let stderr = finalization.output.stderr.as_bytes();
                let stderr_end = (finalization.stderr_offset + remaining).min(stderr.len());
                if stderr_end > finalization.stderr_offset {
                    compat.feed(&stderr[finalization.stderr_offset..stderr_end]);
                    finalization.stderr_offset = stderr_end;
                }
            }
            complete = finalization.stdout_offset == finalization.output.output.len()
                && finalization.stderr_offset == finalization.output.stderr.len();
        }
        if !complete {
            return;
        }
        let Some(finalization) = self.finalization.take() else {
            return;
        };
        if finalization.staged_compat.is_some() {
            self.compat = finalization.staged_compat;
        }
        if !finalization.output.output.is_empty() {
            self.bus.publish(BusMessage {
                domain: 1,
                msg_type: 4,
                priority: Priority::Normal,
                producer_id: 0,
                payload: finalization.output.output.as_bytes().to_vec(),
            });
        }
        self.env_snapshot = finalization.snapshot;
        self.expected_completion_sequence = finalization.sequence.saturating_add(1);
        self.dispatch_render();
        let _ = finalization.reply.send(Ok(finalization.output));
        // The acknowledgement is deliberately last: only a completely
        // materialized view and reply let the worker begin its next request.
        let _ = finalization.finalized.send(());
    }

    fn append_execution_diagnostic(&mut self, error: &DaemonError) {
        let text = format!("malt: {error}\n");
        if let Some(compat) = &mut self.compat {
            compat.feed(text.as_bytes());
        }
        self.bus.publish(BusMessage {
            domain: 1,
            msg_type: 4,
            priority: Priority::Normal,
            producer_id: 0,
            payload: text.into_bytes(),
        });
        self.dispatch_render();
    }

    /// Extract the grid as plain text, no styling — for programmatic/agent
    /// consumption. Same underlying `TerminalGrid` as `get_grid_output`,
    /// just characters instead of styled spans.
    fn get_plain_text_output(&self) -> String {
        let Some(compat) = &self.compat else {
            return String::new();
        };
        let grid = compat.grid();
        let mut lines: Vec<String> = grid
            .rows_data()
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.ch).collect::<String>())
            .collect();

        // Trim trailing blank lines, matching get_grid_output's equivalent
        // trim of trailing empty rows.
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }

        lines.join("\n")
    }

    fn recompute_layout(&mut self) {
        self.resolved_panes = compute_resolved_panes(
            self.session.layout(),
            self.terminal_size,
            self.session.focused_pane().clone(),
            &self.layout_config,
        );
    }

    /// Extract the grid as styled JSON for the API.
    fn get_grid_output(&self) -> String {
        let Some(compat) = &self.compat else {
            return "[]".to_string();
        };
        let grid = compat.grid();
        let mut rows_json = Vec::new();

        for row in grid.rows_data() {
            let mut spans = Vec::new();
            let mut current_text = String::new();
            let mut current_fg = (0u8, 0u8, 0u8);
            let mut current_bg = (0u8, 0u8, 0u8);
            let mut current_bold = false;
            let mut first = true;

            for cell in &row.cells {
                let fg = cell.style.fg;
                let bg = cell.style.bg;
                let bold = cell.style.bold;

                if first {
                    current_fg = fg;
                    current_bg = bg;
                    current_bold = bold;
                    first = false;
                }

                if fg != current_fg || bg != current_bg || bold != current_bold {
                    // Flush current span
                    if !current_text.is_empty() {
                        spans.push(serde_json::json!({
                            "t": current_text,
                            "fg": [current_fg.0, current_fg.1, current_fg.2],
                            "bg": [current_bg.0, current_bg.1, current_bg.2],
                            "b": current_bold,
                        }));
                    }
                    current_text = String::new();
                    current_fg = fg;
                    current_bg = bg;
                    current_bold = bold;
                }
                current_text.push(cell.ch);
            }
            // Flush last span
            if !current_text.is_empty() {
                spans.push(serde_json::json!({
                    "t": current_text,
                    "fg": [current_fg.0, current_fg.1, current_fg.2],
                    "bg": [current_bg.0, current_bg.1, current_bg.2],
                    "b": current_bold,
                }));
            }
            rows_json.push(serde_json::Value::Array(spans));
        }

        // Trim trailing empty rows
        while rows_json.last().map_or(false, |r| {
            r.as_array().map_or(true, |a| {
                a.len() == 1
                    && a[0]
                        .get("t")
                        .and_then(|t| t.as_str())
                        .map_or(false, |s| s.trim().is_empty())
            })
        }) {
            rows_json.pop();
        }

        serde_json::to_string(&rows_json).unwrap_or_else(|_| "[]".to_string())
    }
}

/// Convert mash's live `EnvSnapshot` to the schema-generated persisted
/// shape. See `from_persisted_env_snapshot` for the reverse direction.
fn to_persisted_env_snapshot(
    snapshot: &mash::env::EnvSnapshot,
) -> malt_protocol::persist::session::EnvSnapshot {
    use malt_protocol::persist::session::{
        EnvSnapshot as PEnvSnapshot, PersistedShellOptions, PersistedVarValue, PersistedVariable,
    };

    let variables = snapshot
        .variables
        .iter()
        .map(|(name, var)| {
            let value = match &var.value {
                mash::env::VarValue::String(s) => PersistedVarValue::Str { value: s.clone() },
                mash::env::VarValue::Integer(i, formatted) => PersistedVarValue::Int {
                    value: *i,
                    formatted: formatted.clone(),
                },
                mash::env::VarValue::Array(items) => {
                    PersistedVarValue::Arr { values: items.clone() }
                }
                mash::env::VarValue::AssocArray(map) => PersistedVarValue::AssocArr {
                    entries: map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                },
                // VarValue is #[non_exhaustive] in mash -- a future variant
                // this daemon build doesn't know about yet. Best-effort:
                // persist it as an empty string rather than failing the
                // whole snapshot over one unrecognized variable.
                _ => {
                    warn!(
                        variable = %name,
                        "unrecognized VarValue variant while snapshotting env; persisting as empty string"
                    );
                    PersistedVarValue::Str {
                        value: String::new(),
                    }
                }
            };
            (
                name.clone(),
                PersistedVariable {
                    value,
                    exported: var.exported,
                    readonly: var.readonly,
                    integer: var.integer,
                    _unknown: vec![],
                },
            )
        })
        .collect();

    let options = PersistedShellOptions {
        allexport: snapshot.options.allexport,
        errexit: snapshot.options.errexit,
        nounset: snapshot.options.nounset,
        pipefail: snapshot.options.pipefail,
        xtrace: snapshot.options.xtrace,
        verbose: snapshot.options.verbose,
        noglob: snapshot.options.noglob,
        notify: snapshot.options.notify,
        monitor: snapshot.options.monitor,
        noclobber: snapshot.options.noclobber,
        noexec: snapshot.options.noexec,
        nonlexicalctrl: snapshot.options.nonlexicalctrl,
        hash_cmds: snapshot.options.hash_cmds,
        nolog: snapshot.options.nolog,
        sourcepath: snapshot.options.sourcepath,
        _unknown: vec![],
    };

    PEnvSnapshot {
        variables,
        options,
        aliases: snapshot.aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        functions: snapshot
            .functions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        dir_stack: snapshot.dir_stack.clone(),
        cwd: snapshot.cwd.clone(),
        traps: snapshot.traps.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _unknown: vec![],
    }
}

/// Convert a schema-generated persisted `EnvSnapshot` back to mash's live
/// shape, for `Env::apply_snapshot`. Reverse of `to_persisted_env_snapshot`.
pub(crate) fn from_persisted_env_snapshot(
    persisted: &malt_protocol::persist::session::EnvSnapshot,
) -> mash::env::EnvSnapshot {
    use malt_protocol::persist::session::PersistedVarValue;

    let variables = persisted
        .variables
        .iter()
        .map(|(name, pv)| {
            let value = match &pv.value {
                PersistedVarValue::Str { value } => mash::env::VarValue::String(value.clone()),
                PersistedVarValue::Int { value, formatted } => {
                    mash::env::VarValue::Integer(*value, formatted.clone())
                }
                PersistedVarValue::Arr { values } => mash::env::VarValue::Array(values.clone()),
                PersistedVarValue::AssocArr { entries } => mash::env::VarValue::AssocArray(
                    entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                ),
                // PersistedVarValue is #[non_exhaustive] too (includes the
                // schema's own forward-compat `Unknown` variant) -- treat
                // anything unrecognized as an empty string rather than
                // failing the whole restore over one variable.
                _ => mash::env::VarValue::String(String::new()),
            };
            (
                name.clone(),
                mash::env::Variable {
                    value,
                    exported: pv.exported,
                    readonly: pv.readonly,
                    integer: pv.integer,
                },
            )
        })
        .collect();

    let options = mash::env::ShellOptions {
        allexport: persisted.options.allexport,
        errexit: persisted.options.errexit,
        nounset: persisted.options.nounset,
        pipefail: persisted.options.pipefail,
        xtrace: persisted.options.xtrace,
        verbose: persisted.options.verbose,
        noglob: persisted.options.noglob,
        notify: persisted.options.notify,
        monitor: persisted.options.monitor,
        noclobber: persisted.options.noclobber,
        noexec: persisted.options.noexec,
        nonlexicalctrl: persisted.options.nonlexicalctrl,
        hash_cmds: persisted.options.hash_cmds,
        nolog: persisted.options.nolog,
        sourcepath: persisted.options.sourcepath,
    };

    mash::env::EnvSnapshot {
        variables,
        options,
        aliases: persisted.aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        functions: persisted
            .functions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        dir_stack: persisted.dir_stack.clone(),
        cwd: persisted.cwd.clone(),
        traps: persisted.traps.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
    }
}

/// Build a `PersistedSession` from current runtime state.
///
/// Called from the `Snapshot` command handler — runs on the session thread,
/// so all access to `env` is unsynchronized by design.
fn command_error_output(error: DaemonError) -> CommandOutput {
    CommandOutput {
        command_id: 0,
        output: String::new(),
        stderr: format!("malt: {error}\n"),
        exit_code: 1,
    }
}

fn build_persisted_session(
    session_id: &SessionId,
    focused_pane: &PaneId,
    name: Option<&str>,
    isolation: IsolationTier,
    env: &EnvSnapshot,
) -> PersistedSession {
    let shell_path = snapshot_string(env, "SHELL").unwrap_or_else(default_shell_path);
    let cwd = snapshot_string(env, "PWD").unwrap_or_else(|| {
        if env.cwd.is_empty() { ".".to_string() } else { env.cwd.clone() }
    });
    let env_snapshot = Some(to_persisted_env_snapshot(env));

    let pane = PersistedPane {
        cwd,
        title: None,
        pane_type: PersistedPaneType::Shell {
            shell_path,
            env_snapshot,
        },
        _unknown: vec![],
    };

    let mut panes = std::collections::BTreeMap::new();
    panes.insert(focused_pane.0, pane);

    // NOTE: Layout is hardcoded as a single Leaf node — this is correct for
    // the current single-pane model. Phase F multi-pane will need to read
    // the actual layout state from the session executor and pass it here.
    PersistedSession {
        schema_version: 1,
        id: session_id.clone(),
        name: name.map(|s| s.to_string()),
        layout: malt_protocol::common::LayoutNode::Leaf {
            pane_id: focused_pane.clone(),
        },
        focus: focused_pane.clone(),
        panes,
        theme: None,
        group: None,
        isolation,
        _unknown: vec![],
    }
}

fn snapshot_string(snapshot: &EnvSnapshot, name: &str) -> Option<String> {
    match snapshot.variables.get(name).map(|variable| &variable.value) {
        Some(mash::env::VarValue::String(value)) => Some(value.clone()),
        Some(mash::env::VarValue::Integer(value, _)) => Some(value.to_string()),
        _ => None,
    }
}

fn default_shell_path() -> String {
    #[cfg(unix)]
    {
        "/bin/sh".to_string()
    }
    #[cfg(not(unix))]
    {
        "cmd.exe".to_string()
    }
}

#[cfg(all(test, windows))]
mod isolation_tier_tests {
    use super::*;

    #[test]
    fn bare_and_restricted_get_uncapped_job_objects() {
        assert_eq!(job_object_limits_for_tier(IsolationTier::Bare), (0, 0));
        assert_eq!(
            job_object_limits_for_tier(IsolationTier::Restricted),
            (0, 0),
            "Restricted should be group-kill only, no resource caps"
        );
    }

    #[test]
    fn capped_and_contained_get_real_nonzero_limits() {
        let capped = job_object_limits_for_tier(IsolationTier::Capped);
        let contained = job_object_limits_for_tier(IsolationTier::Contained);
        assert_ne!(
            capped,
            (0, 0),
            "Capped must get a real resource limit, not the same uncapped \
             treatment as Restricted -- that was the confirmed tier-blind bug"
        );
        assert_eq!(
            capped, contained,
            "Contained doesn't yet get anything beyond Capped-level Job \
             Object containment (see the doc comment on apply_session_isolation) \
             -- this test pins that honestly rather than silently drifting"
        );
    }

    #[test]
    fn restricted_and_capped_are_actually_different() {
        assert_ne!(
            job_object_limits_for_tier(IsolationTier::Restricted),
            job_object_limits_for_tier(IsolationTier::Capped),
            "this is the core tier-blindness fix: Restricted and Capped must \
             no longer produce identical Job Object parameters"
        );
    }
}
