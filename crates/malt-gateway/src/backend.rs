use crate::error::GatewayError;
use crate::types::{
    CommandHistoryEntry, ExecResult, LifecycleEventDto, OutputChunkDto, PaneResponse,
    SessionResponse,
};

/// Trait abstracting the daemon operations that the gateway delegates to.
///
/// Implementations bridge the HTTP layer to the daemon's session store,
/// executor, and layout engine.
pub trait GatewayBackend: Send + Sync + 'static {
    fn list_sessions(&self) -> Result<Vec<SessionResponse>, GatewayError>;

    fn create_session(
        &self,
        name: Option<String>,
        isolation: Option<String>,
    ) -> Result<SessionResponse, GatewayError>;

    fn create_session_with_policy(
        &self,
        name: Option<String>,
        isolation: Option<String>,
        isolation_policy: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        let _ = isolation_policy;
        self.create_session(name, isolation)
    }

    fn get_session(&self, id: u32) -> Result<SessionResponse, GatewayError>;

    fn destroy_session(&self, id: u32) -> Result<(), GatewayError>;

    fn exec_command(&self, session_id: u32, command: String) -> Result<ExecResult, GatewayError>;

    fn send_input(&self, session_id: u32, input: String) -> Result<(), GatewayError>;

    /// Signal end-of-input to whatever is currently reading -- Ctrl-D.
    ///
    /// Distinct from `send_input` with an empty payload: an empty write is a
    /// zero-byte write, which a reader does not see at all, whereas this ends
    /// the read. A command consuming to the end (`cat`, `wc`) needs it to
    /// terminate, since a session's stdin has no natural end.
    fn end_input(&self, session_id: u32) -> Result<(), GatewayError>;

    /// Which client holds input authority, if any (FR-015).
    ///
    /// `None` means nobody holds it and the session is claimable -- not that
    /// the session is unusable.
    fn input_authority(&self, session_id: u32) -> Result<Option<u64>, GatewayError>;

    fn get_output(&self, session_id: u32) -> Result<serde_json::Value, GatewayError>;

    /// Plain-text variant of `get_output`, for programmatic/agent
    /// consumption — same underlying content, no styling.
    fn get_output_text(&self, session_id: u32) -> Result<String, GatewayError>;

    /// This session's command execution history, oldest first, capped at the
    /// pane's retention bound.
    fn get_command_history(
        &self,
        session_id: u32,
    ) -> Result<Vec<CommandHistoryEntry>, GatewayError>;

    /// Subscribe to a session's command lifecycle events.
    ///
    /// Returns the receiving half of a bounded channel. Errors are returned
    /// here, before any stream is established, because an SSE client treats
    /// an opened stream as success.
    fn subscribe_events(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<LifecycleEventDto>, GatewayError>;

    /// Subscribe to a session's streamed command output. Same shape and
    /// error timing as `subscribe_events` -- see that method's doc.
    fn subscribe_output(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<OutputChunkDto>, GatewayError>;

    fn list_panes(&self, session_id: u32) -> Result<Vec<PaneResponse>, GatewayError>;

    fn split_pane(
        &self,
        session_id: u32,
        target_pane_id: u32,
        direction: String,
    ) -> Result<PaneResponse, GatewayError>;

    fn close_pane(&self, session_id: u32, pane_id: u32) -> Result<(), GatewayError>;
}
