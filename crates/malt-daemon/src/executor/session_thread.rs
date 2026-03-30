use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use crate::DaemonError;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{IsolationTier, PaneId, SessionId};
use malt_session::session::SessionRuntime;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use tracing::{info, warn};

/// Commands sent from the coordinator to a session executor.
#[derive(Debug)]
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
    /// Graceful shutdown.
    Shutdown,
}

/// Per-session executor running on a dedicated thread with its own tokio runtime.
pub struct SessionExecutor {
    session: SessionRuntime,
    bus: Bus,
    authority: AuthorityTracker,
    terminal_size: Rect,
    layout_config: LayoutConfig,
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
                };
                executor.run(rx);
            })
            .map_err(|e| DaemonError::Io(e))?;
        Ok((tx, handle))
    }

    fn run(&mut self, rx: mpsc::Receiver<SessionCommand>) {
        info!(session = ?self.session.id(), "session executor started");
        loop {
            match rx.recv() {
                Ok(SessionCommand::Shutdown) => {
                    info!(session = ?self.session.id(), "session executor shutting down");
                    break;
                }
                Ok(SessionCommand::Deliver(msg)) => {
                    self.bus.publish(msg);
                }
                Ok(SessionCommand::AttachClient { client_id, authority }) => {
                    self.authority.attach(client_id, authority);
                    let _ = self.session.attach(client_id, authority);
                }
                Ok(SessionCommand::DetachClient { client_id }) => {
                    self.authority.detach(client_id);
                    let _ = self.session.detach(client_id);
                }
                Ok(SessionCommand::Resize { cols, rows }) => {
                    self.terminal_size = Rect::new(0, 0, cols, rows);
                    self.recompute_layout();
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
        // Layout results will be published to bus when renderer is integrated (Phase 3C)
    }
}
