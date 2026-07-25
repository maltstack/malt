use crate::executor::coordinator::Coordinator;
use crate::DaemonError;
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::types::{CommandHistoryEntry, ExecResult, PaneResponse, SessionResponse};
use malt_protocol::common::{IsolationTier, SessionId};
use std::sync::{Arc, Mutex};

/// Bridges the HTTP gateway to the real daemon Coordinator.
pub struct DaemonBackend {
    coordinator: Arc<Mutex<Coordinator>>,
}

impl DaemonBackend {
    pub fn new(coordinator: Arc<Mutex<Coordinator>>) -> Self {
        Self { coordinator }
    }

    pub fn coordinator(&self) -> &Arc<Mutex<Coordinator>> {
        &self.coordinator
    }
}

fn map_execution_error(error: DaemonError) -> GatewayError {
    let message = error.to_string();
    match error {
        DaemonError::ExecutionQueueFull { .. } => GatewayError::ExecutionQueueFull(message),
        DaemonError::ExecutionUnavailable(_) => GatewayError::ExecutionUnavailable(message),
        DaemonError::SessionShuttingDown(_) => GatewayError::SessionShuttingDown(message),
        DaemonError::SessionNotFound(id) => GatewayError::SessionNotFound(id.0),
        other => GatewayError::Internal(other.to_string()),
    }
}

/// Parse the `isolation` field of a `CreateSession` request.
///
/// An omitted field defaults to `Bare` (preserves existing behavior for
/// callers that don't opt in). An unrecognized value is a client error, not
/// a silent fallback to `Bare` — a typo'd isolation string (e.g. from an
/// agent constructing the request JSON itself) must not silently produce a
/// weaker session than requested with no indication anything was wrong.
fn parse_isolation(s: Option<String>) -> Result<IsolationTier, GatewayError> {
    match s.as_deref() {
        None => Ok(IsolationTier::Bare),
        Some("Bare") | Some("bare") => Ok(IsolationTier::Bare),
        Some("Restricted") | Some("restricted") => Ok(IsolationTier::Restricted),
        Some("Capped") | Some("capped") => Ok(IsolationTier::Capped),
        Some("Contained") | Some("contained") => Ok(IsolationTier::Contained),
        Some(other) => Err(GatewayError::BadRequest(format!(
            "unrecognized isolation tier {other:?} (expected one of: bare, restricted, capped, contained)"
        ))),
    }
}

impl GatewayBackend for DaemonBackend {
    fn list_sessions(&self) -> Result<Vec<SessionResponse>, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let sessions = coord.list_sessions();
        Ok(sessions
            .into_iter()
            .map(|s| SessionResponse {
                id: s.session_id.0,
                name: s.name,
                pane_count: s.pane_count,
                isolation: format!("{:?}", s.isolation),
                state: format!("{:?}", s.state),
            })
            .collect())
    }

    fn create_session(
        &self,
        name: Option<String>,
        isolation: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        let tier = parse_isolation(isolation)?;
        let mut coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let session_id = coord
            .create_session(name.clone(), tier, None)
            .map_err(|e| GatewayError::Internal(e.to_string()))?;

        Ok(SessionResponse {
            id: session_id.0,
            name,
            pane_count: 1,
            isolation: format!("{:?}", tier),
            state: "Active".to_string(),
        })
    }

    fn get_session(&self, id: u32) -> Result<SessionResponse, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let sessions = coord.list_sessions();
        sessions
            .into_iter()
            .find(|s| s.session_id.0 == id)
            .map(|s| SessionResponse {
                id: s.session_id.0,
                name: s.name,
                pane_count: s.pane_count,
                isolation: format!("{:?}", s.isolation),
                state: format!("{:?}", s.state),
            })
            .ok_or(GatewayError::SessionNotFound(id))
    }

    fn destroy_session(&self, id: u32) -> Result<(), GatewayError> {
        let mut coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        coord.destroy_session(SessionId(id));
        Ok(())
    }

    fn exec_command(&self, session_id: u32, command: String) -> Result<ExecResult, GatewayError> {
        let reply_rx = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .submit_execution(SessionId(session_id), command)
                .map_err(map_execution_error)?
        };

        let result = reply_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| GatewayError::Internal("command timed out".to_string()))?;
        let result = result.map_err(map_execution_error)?;

        Ok(ExecResult {
            command_id: result.command_id,
            output: result.output,
            stderr: result.stderr,
            exit_code: Some(result.exit_code),
        })
    }

    fn send_input(&self, session_id: u32, input: String) -> Result<(), GatewayError> {
        let reply_rx = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .submit_execution(SessionId(session_id), input)
                .map_err(map_execution_error)?
        };
        // Wait for completion but discard output
        match reply_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return Err(map_execution_error(error)),
            Err(_) => return Err(GatewayError::Internal("command timed out".to_string())),
        }
        Ok(())
    }

    fn get_output(&self, session_id: u32) -> Result<serde_json::Value, GatewayError> {
        let reply = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .begin_get_session_output(SessionId(session_id))
                .map_err(map_execution_error)?
        };
        let raw = reply
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| GatewayError::Internal("session output timed out".to_string()))?;
        // raw is a JSON string (array of rows with styled spans)
        let rows: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Array(vec![]));
        Ok(serde_json::json!({
            "type": "StyledGrid",
            "rows": rows,
        }))
    }

    fn get_output_text(&self, session_id: u32) -> Result<String, GatewayError> {
        let reply = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .begin_get_session_output_text(SessionId(session_id))
                .map_err(map_execution_error)?
        };
        reply
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| GatewayError::Internal("session output timed out".to_string()))
    }

    fn get_command_history(
        &self,
        session_id: u32,
    ) -> Result<Vec<CommandHistoryEntry>, GatewayError> {
        // Take the coordinator lock only long enough to resolve the pane and
        // hand off the request; waiting under it would stall unrelated
        // sessions. Resolving the pane first doubles as the existence check,
        // so an unknown session is a 404 rather than an empty history that
        // looks like a real session which has run nothing.
        let (pane_id, reply) = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            let pane_id = coord
                .session_first_pane(SessionId(session_id))
                .ok_or(GatewayError::SessionNotFound(session_id))?;
            let reply = coord
                .begin_get_session_command_history(SessionId(session_id))
                .map_err(map_execution_error)?;
            (pane_id, reply)
        };
        let blocks = reply
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| GatewayError::Internal("session history timed out".to_string()))?;
        Ok(blocks
            .into_iter()
            .map(|b| CommandHistoryEntry {
                command_id: b.command_id,
                cmd: b.cmd,
                started_at: b.started_at,
                finished_at: b.finished_at,
                exit_code: b.exit_code,
                pane_id: pane_id.0,
            })
            .collect())
    }

    fn list_panes(&self, session_id: u32) -> Result<Vec<PaneResponse>, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let pane_id = coord
            .session_first_pane(SessionId(session_id))
            .ok_or(GatewayError::SessionNotFound(session_id))?;
        // Every session has exactly one pane under today's single-pane
        // model, and every reachable pane is Shell-kind today (Compat-pane
        // creation is only reachable via session restore, which is itself
        // a confirmed stub -- see docs/BACKLOG.md). `kind`/`title` will
        // need to reflect the real PaneKind/title once multi-pane sessions
        // exist; the id below is real, not a hardcoded placeholder.
        Ok(vec![PaneResponse {
            id: pane_id.0,
            kind: "Shell".to_string(),
            title: None,
            focused: true,
        }])
    }

    fn split_pane(
        &self,
        _session_id: u32,
        _target_pane_id: u32,
        _direction: String,
    ) -> Result<PaneResponse, GatewayError> {
        // Stub — split pane integration deferred
        Ok(PaneResponse {
            id: 0,
            kind: "Shell".to_string(),
            title: None,
            focused: false,
        })
    }

    fn close_pane(&self, _session_id: u32, _pane_id: u32) -> Result<(), GatewayError> {
        Ok(())
    }
}
