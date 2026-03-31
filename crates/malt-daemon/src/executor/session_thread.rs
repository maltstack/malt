use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use crate::DaemonError;
use malt_compat::CompatTranslator;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{IsolationTier, PaneId, SessionId};
use malt_protocol::priority::Priority;
use malt_session::session::SessionRuntime;
use std::fs::File;
use std::io::Write;
use std::sync::mpsc;
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
    /// Write input to PTY stdin.
    WriteInput { data: Vec<u8> },
    /// Get the current output snapshot (requester sends back via channel).
    GetOutput {
        reply: mpsc::Sender<String>,
    },
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
            Self::WriteInput { data } => f
                .debug_struct("WriteInput")
                .field("len", &data.len())
                .finish(),
            Self::GetOutput { .. } => f.debug_struct("GetOutput").finish(),
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
    pty_writer: Option<File>,
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
                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                    compat: None,
                    pty_writer: None,
                };
                executor.run(rx);
            })
            .map_err(DaemonError::Io)?;
        Ok((tx, handle))
    }

    /// Set the PTY writer for this session (called after spawn via command).
    pub fn set_pty_writer(&mut self, writer: File) {
        self.pty_writer = Some(writer);
    }

    /// Initialize the compat translator for this session.
    pub fn init_compat(&mut self, cols: u16, rows: u16) {
        self.compat = Some(CompatTranslator::new(cols, rows));
    }

    fn run(&mut self, rx: mpsc::Receiver<SessionCommand>) {
        info!(session = ?self.session.id(), "session executor started");
        // Initialize compat translator with default terminal size
        self.init_compat(self.terminal_size.w, self.terminal_size.h);

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
                    if let Some(writer) = &mut self.pty_writer {
                        if let Err(e) = writer.write_all(&data) {
                            warn!(error = %e, "failed to write to PTY");
                        }
                    }
                }
                Ok(SessionCommand::GetOutput { reply }) => {
                    let output = self.get_grid_text();
                    let _ = reply.send(output);
                }
                Err(_) => {
                    warn!(session = ?self.session.id(), "command channel closed");
                    break;
                }
            }
        }
    }

    fn recompute_layout(&mut self) {
        let _resolved = compute_resolved_panes(
            self.session.layout(),
            self.terminal_size,
            self.session.focused_pane().clone(),
            &self.layout_config,
        );
    }

    /// Extract visible text from the compat translator's grid.
    fn get_grid_text(&self) -> String {
        let Some(compat) = &self.compat else {
            return String::new();
        };
        let grid = compat.grid();
        let mut lines = Vec::new();
        for row in grid.rows_data() {
            let line: String = row.cells.iter().map(|c| c.ch).collect();
            lines.push(line.trim_end().to_string());
        }
        // Trim trailing empty lines
        while lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}
