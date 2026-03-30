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
    pub exit_code: Option<i32>,
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
