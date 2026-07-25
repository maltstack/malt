use serde::{Deserialize, Serialize};

/// Wrapper for successful API responses.
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub ok: bool,
    pub data: T,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self { ok: true, data }
    }
}

/// Session summary returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct SessionResponse {
    pub id: u32,
    pub name: Option<String>,
    pub pane_count: u16,
    pub isolation: String,
    pub state: String,
}

/// Pane summary returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct PaneResponse {
    pub id: u32,
    pub kind: String,
    pub title: Option<String>,
    pub focused: bool,
}

/// Request body for creating a session.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub name: Option<String>,
    pub isolation: Option<String>,
}

/// Request body for executing a command.
#[derive(Debug, Deserialize)]
pub struct ExecRequest {
    pub command: String,
}

/// Result of a command execution.
#[derive(Debug, Clone, Serialize)]
pub struct ExecResult {
    pub command_id: u32,
    pub output: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

/// One command execution record returned by the history API.
///
/// `finished_at` and `exit_code` are both `None` when the command is not
/// confirmed complete — either still running, or interrupted by a daemon
/// stop. Neither case is ever reported as a successful exit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandHistoryEntry {
    pub command_id: u32,
    pub cmd: String,
    /// Epoch milliseconds.
    pub started_at: u64,
    /// Epoch milliseconds; `None` if not confirmed complete.
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub pane_id: u32,
}

/// One lifecycle event as delivered to a client.
///
/// `kind` names the event (`command_started`, `command_finished`, `gap`);
/// the type-specific fields are flattened alongside it and omitted when not
/// applicable, so a client reads one flat object per frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LifecycleEventDto {
    #[serde(skip)]
    pub sequence: u64,
    #[serde(skip)]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_from: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_through: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Request body for sending raw input.
#[derive(Debug, Deserialize)]
pub struct SendInputRequest {
    pub input: String,
}

/// Request body for splitting a pane.
#[derive(Debug, Deserialize)]
pub struct SplitPaneRequest {
    pub target_pane_id: u32,
    #[serde(default = "default_direction")]
    pub direction: String,
}

fn default_direction() -> String {
    "vertical".to_string()
}
