use crate::executor::coordinator::Coordinator;
use crate::executor::session_thread::SessionCommand;
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::types::{ExecResult, PaneResponse, SessionResponse};
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

fn parse_isolation(s: Option<String>) -> IsolationTier {
    match s.as_deref() {
        Some("Restricted") | Some("restricted") => IsolationTier::Restricted,
        Some("Capped") | Some("capped") => IsolationTier::Capped,
        Some("Contained") | Some("contained") => IsolationTier::Contained,
        _ => IsolationTier::Bare,
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
        let tier = parse_isolation(isolation);
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

    fn exec_command(
        &self,
        session_id: u32,
        command: String,
    ) -> Result<ExecResult, GatewayError> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .send_command(
                    SessionId(session_id),
                    SessionCommand::RunCommand {
                        command,
                        reply: reply_tx,
                    },
                )
                .map_err(|e| GatewayError::Internal(e.to_string()))?;
        } // Release lock before waiting

        let output = reply_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| GatewayError::Internal("command timed out".to_string()))?;

        Ok(ExecResult {
            command_id: 0,
            output,
            exit_code: None,
        })
    }

    fn send_input(&self, session_id: u32, input: String) -> Result<(), GatewayError> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            coord
                .send_command(
                    SessionId(session_id),
                    SessionCommand::RunCommand {
                        command: input,
                        reply: reply_tx,
                    },
                )
                .map_err(|e| GatewayError::Internal(e.to_string()))?;
        }
        // Wait for completion but discard output
        let _ = reply_rx.recv_timeout(std::time::Duration::from_secs(30));
        Ok(())
    }

    fn get_output(&self, session_id: u32) -> Result<serde_json::Value, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let raw = coord
            .get_session_output(SessionId(session_id))
            .map_err(|e| GatewayError::Internal(e.to_string()))?;
        // raw is a JSON string (array of rows with styled spans)
        let rows: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Array(vec![]));
        Ok(serde_json::json!({
            "type": "StyledGrid",
            "rows": rows,
        }))
    }

    fn list_panes(&self, session_id: u32) -> Result<Vec<PaneResponse>, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        if !coord.has_session(SessionId(session_id)) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(vec![PaneResponse {
            id: 1,
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
