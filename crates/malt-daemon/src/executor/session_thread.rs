use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use crate::DaemonError;
use malt_compat::CompatTranslator;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{ClientCapabilities, IsolationTier, PaneId, ResolvedPane, SessionId};
use malt_protocol::input::KeyEvent;
use malt_protocol::priority::Priority;
use malt_protocol::render::{InitialState, RenderBatch};
use malt_renderer::host::{PaneFrame, RendererHost};
use malt_session::session::SessionRuntime;
use mash::env::Env;
use mash::executor::execute_list;
use mash::parser;
use malt_term::{EditMode, EditResult, Editor};
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use tracing::{info, warn};

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
        reply: mpsc::Sender<String>,
    },
    /// Write input to PTY stdin.
    WriteInput { data: Vec<u8> },
    /// Get the current output snapshot (requester sends back via channel).
    GetOutput {
        reply: mpsc::Sender<String>,
    },
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
    /// Graceful shutdown.
    Shutdown,
}

// Manual Debug impl since mpsc::Sender doesn't implement Debug
impl std::fmt::Debug for SessionCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deliver(m) => f.debug_tuple("Deliver").field(m).finish(),
            Self::AttachClient { client_id, authority } => f
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
            Self::AckFrame { client_id, frame_seq } => f
                .debug_struct("AckFrame")
                .field("client_id", client_id)
                .field("frame_seq", frame_seq)
                .finish(),
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
                        domain: 1, // Shell
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
                    let initial = self.renderer.snapshot_initial_state(&panes, &layout, client_id);
                    let _ = initial_reply.send(initial);
                }
                Ok(SessionCommand::UnregisterVnpClient { client_id }) => {
                    self.renderer.remove_client(client_id);
                    self.render_pushers.remove(&client_id);
                }
                Ok(SessionCommand::KeyInput { key }) => {
                    if let Some(input_event) =
                        crate::input_bridge::vnp_key_to_input_event(&key)
                    {
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
                Ok(SessionCommand::AckFrame { client_id, frame_seq }) => {
                    self.renderer.ack_frame(client_id, frame_seq);
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
        if self.render_pushers.is_empty() {
            return;
        }
        let element = match &self.compat {
            Some(c) => c.frame_element(),
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
    fn run_mash_command(&mut self, input: &str) -> String {
        let commands = match parser::parse(input) {
            Ok(cmds) => cmds,
            Err(e) => {
                let err_msg = format!("mash: parse error: {e}\n");
                if let Some(compat) = &mut self.compat {
                    compat.feed(err_msg.as_bytes());
                }
                return err_msg;
            }
        };

        if commands.is_empty() {
            return String::new();
        }

        let result = execute_list(&commands, input, &mut self.mash_env);

        // Feed stdout through compat translator for grid rendering
        if !result.stdout.is_empty() {
            if let Some(compat) = &mut self.compat {
                compat.feed(&result.stdout);
            }
            // Publish output to bus
            self.bus.publish(BusMessage {
                domain: 1, // Shell
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

        String::from_utf8_lossy(&result.stdout).to_string()
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
                    && a[0].get("t").and_then(|t| t.as_str()).map_or(false, |s| {
                        s.trim().is_empty()
                    })
            })
        }) {
            rows_json.pop();
        }

        serde_json::to_string(&rows_json).unwrap_or_else(|_| "[]".to_string())
    }
}
