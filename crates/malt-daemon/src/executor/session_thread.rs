use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
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
use mash::env::{Env, Variable};
use mash::executor::execute_list;
use mash::parser;
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use tracing::{info, warn};

/// Apply a session's isolation tier to its MASH environment: sets the opaque
/// isolation context token, and on Windows, creates a Job Object every
/// externally-spawned command in this session gets assigned to (see
/// `mash::executor`'s spawn call site). Best-effort — if job object creation
/// fails, the session still runs, just without process containment.
///
/// Bare tier does nothing (no job object needed).
fn apply_session_isolation(env: &mut Env, session_id: SessionId, isolation: IsolationTier) {
    env.set_isolation_context(malt_platform::isolation::IsolationContext::from(isolation));

    #[cfg(windows)]
    {
        if isolation == IsolationTier::Bare {
            return;
        }
        let job_name = format!("malt-session-{}", session_id.0);
        match malt_platform::isolation::job_objects::create_job_object(&job_name, 0, 0) {
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
    /// Write input to PTY stdin.
    WriteInput { data: Vec<u8> },
    /// Get the current output snapshot (requester sends back via channel).
    GetOutput { reply: mpsc::Sender<String> },
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
            Self::WriteInput { data } => f
                .debug_struct("WriteInput")
                .field("len", &data.len())
                .finish(),
            Self::GetOutput { .. } => f.debug_struct("GetOutput").finish(),
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
    mash_env: Env,
    renderer: RendererHost,
    editor: Editor,
    render_pushers: HashMap<u64, SyncSender<RenderBatch>>,
    resolved_panes: Vec<ResolvedPane>,
    /// Monotonically increasing id assigned to each command run via
    /// `run_mash_command`, starting at 1 (0 is reserved to mean "no real id
    /// assigned yet" for any response path that hasn't been wired to this
    /// counter). Does not survive a session restore — see
    /// `docs/BACKLOG.md`'s persistent-execution-history item.
    next_command_id: u32,
}

impl SessionExecutor {
    /// Spawn a new session executor on a dedicated thread.
    /// Returns the command sender and thread handle.
    pub fn spawn(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
    ) -> Result<(mpsc::Sender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(format!("session-{}", session_id.0))
            .spawn(move || {
                let mut env = Env::from_os();
                env.set_interactive(true);
                apply_session_isolation(&mut env, session_id.clone(), isolation);
                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                    compat: None,
                    mash_env: env,
                    renderer: RendererHost::new(),
                    editor: Editor::new(EditMode::Emacs),
                    render_pushers: HashMap::new(),
                    resolved_panes: Vec::new(),
                    next_command_id: 0,
                };
                executor.run(rx);
            })
            .map_err(DaemonError::Io)?;
        Ok((tx, handle))
    }

    /// Spawn a new session executor on a dedicated thread, setting the working directory.
    /// If `initial_cwd` is not a directory at spawn time, a warning is logged and the
    /// OS-inherited cwd is used instead.
    pub fn spawn_with_cwd(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
    ) -> Result<(mpsc::Sender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(format!("session-{}", session_id.0))
            .spawn(move || {
                let mut env = Env::from_os();
                env.set_interactive(true);
                apply_session_isolation(&mut env, session_id.clone(), isolation);

                if initial_cwd.is_dir() {
                    let cwd_str = initial_cwd.to_string_lossy().to_string();
                    if let Err(e) = env.set_global("PWD", Variable::exported_string(cwd_str)) {
                        warn!(?initial_cwd, error = %e, "spawn_with_cwd: failed to set PWD in mash env");
                    }
                } else {
                    warn!(
                        ?initial_cwd,
                        "spawn_with_cwd: directory no longer exists; falling back to OS cwd"
                    );
                }

                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                    compat: None,
                    mash_env: env,
                    renderer: RendererHost::new(),
                    editor: Editor::new(EditMode::Emacs),
                    render_pushers: HashMap::new(),
                    resolved_panes: Vec::new(),
                    next_command_id: 0,
                };
                executor.run(rx);
            })
            .map_err(DaemonError::Io)?;
        Ok((tx, handle))
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
            match rx.recv() {
                Ok(SessionCommand::Shutdown) => {
                    info!(session = ?self.session.id(), "session executor shutting down");
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
                    let output = self.run_mash_command(&command);
                    let _ = reply.send(output);
                }
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
                }
                Ok(SessionCommand::WriteInput { data }) => {
                    let input = String::from_utf8_lossy(&data);
                    let input = input.trim();
                    if !input.is_empty() {
                        let _ = self.run_mash_command(input);
                    }
                }
                Ok(SessionCommand::GetOutput { reply }) => {
                    let output = self.get_grid_output();
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
                                let _ = self.run_mash_command(&line);
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
                        &self.mash_env,
                    );
                    let _ = reply.send(persisted);
                }
                Err(_) => {
                    warn!(session = ?self.session.id(), "command channel closed");
                    break;
                }
            }
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

    /// Parse and execute a command string via mash, feeding output through the
    /// compat translator and returning the plain stdout text.
    fn run_mash_command(&mut self, input: &str) -> CommandOutput {
        // Every call is one distinct execution, including parse errors and
        // empty input -- assign the id once, up front, so all three return
        // paths below get a real, unique, monotonically-increasing id.
        self.next_command_id += 1;
        let command_id = self.next_command_id;

        let commands = match parser::parse(input) {
            Ok(cmds) => cmds,
            Err(e) => {
                let err_msg = format!("mash: parse error: {e}\n");
                if let Some(compat) = &mut self.compat {
                    compat.feed(err_msg.as_bytes());
                }
                // Matches mash's own CLI convention (crates/mash/src/main.rs)
                // of exiting 1 on a parse error.
                return CommandOutput {
                    command_id,
                    output: err_msg,
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
        };

        if commands.is_empty() {
            return CommandOutput {
                command_id,
                output: String::new(),
                stderr: String::new(),
                exit_code: 0,
            };
        }

        let result = execute_list(&commands, input, &mut self.mash_env);

        // Feed stdout through compat translator for grid rendering
        if !result.stdout.is_empty() {
            if let Some(compat) = &mut self.compat {
                compat.feed(&result.stdout);
            }
            // Publish output to bus
            self.bus.publish(BusMessage {
                domain: 1,   // Shell
                msg_type: 4, // OutputChunk
                priority: Priority::Normal,
                producer_id: 0,
                payload: result.stdout.clone(),
            });
        }

        // Feed stderr through compat translator too
        if !result.stderr.is_empty() {
            if let Some(compat) = &mut self.compat {
                compat.feed(&result.stderr);
            }
        }

        CommandOutput {
            command_id,
            output: String::from_utf8_lossy(&result.stdout).to_string(),
            stderr: String::from_utf8_lossy(&result.stderr).to_string(),
            exit_code: result.exit_code,
        }
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

/// Build a `PersistedSession` from current runtime state.
///
/// Called from the `Snapshot` command handler — runs on the session thread,
/// so all access to `env` is unsynchronized by design.
fn build_persisted_session(
    session_id: &SessionId,
    focused_pane: &PaneId,
    name: Option<&str>,
    isolation: IsolationTier,
    env: &Env,
) -> PersistedSession {
    let shell_path = {
        let s = env.get_str("SHELL");
        if s.is_empty() {
            #[cfg(unix)]
            {
                "/bin/sh".to_string()
            }
            #[cfg(not(unix))]
            {
                "cmd.exe".to_string()
            }
        } else {
            s.to_string()
        }
    };

    let cwd = {
        let s = env.get_str("PWD");
        if s.is_empty() {
            ".".to_string()
        } else {
            s.to_string()
        }
    };

    let pane = PersistedPane {
        cwd,
        title: None,
        pane_type: PersistedPaneType::Shell { shell_path },
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
