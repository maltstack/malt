use crate::error::GatewayError;
use crate::types::{CommandHistoryEntry, ExecResult, PaneResponse, SessionResponse};

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

    fn get_session(&self, id: u32) -> Result<SessionResponse, GatewayError>;

    fn destroy_session(&self, id: u32) -> Result<(), GatewayError>;

    fn exec_command(&self, session_id: u32, command: String) -> Result<ExecResult, GatewayError>;

    fn send_input(&self, session_id: u32, input: String) -> Result<(), GatewayError>;

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

    fn list_panes(&self, session_id: u32) -> Result<Vec<PaneResponse>, GatewayError>;

    fn split_pane(
        &self,
        session_id: u32,
        target_pane_id: u32,
        direction: String,
    ) -> Result<PaneResponse, GatewayError>;

    fn close_pane(&self, session_id: u32, pane_id: u32) -> Result<(), GatewayError>;
}
