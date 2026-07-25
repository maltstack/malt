use crate::executor::coordinator::Coordinator;
use crate::executor::events::{GapReason, LifecycleEvent, LifecycleEventKind};
use crate::DaemonError;
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::types::{
    CommandHistoryEntry, ExecResult, LifecycleEventDto, PaneResponse, SessionResponse,
};
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

/// Depth of the daemon→wire forwarding channel. Small on purpose: the real
/// buffering bound is the session-side subscriber sink, and a second deep
/// queue here would just relocate the unbounded-growth problem.
const SUBSCRIBER_FORWARD_BUFFER: usize = 64;

/// Project a daemon lifecycle event onto the flat wire shape.
fn to_event_dto(event: LifecycleEvent) -> LifecycleEventDto {
    let mut dto = LifecycleEventDto {
        sequence: event.sequence,
        kind: String::new(),
        command_id: None,
        cmd: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
        duration_us: None,
        missed_from: None,
        missed_through: None,
        reason: None,
    };
    match event.kind {
        LifecycleEventKind::CommandStarted {
            command_id,
            cmd,
            started_at,
        } => {
            dto.kind = "command_started".to_string();
            dto.command_id = Some(command_id);
            dto.cmd = Some(cmd);
            dto.started_at = Some(started_at);
        }
        LifecycleEventKind::CommandFinished {
            command_id,
            exit_code,
            finished_at,
            duration_us,
        } => {
            dto.kind = "command_finished".to_string();
            dto.command_id = Some(command_id);
            dto.exit_code = Some(exit_code);
            dto.finished_at = Some(finished_at);
            dto.duration_us = Some(duration_us);
        }
        LifecycleEventKind::Gap {
            missed_from,
            missed_through,
            reason,
        } => {
            dto.kind = "gap".to_string();
            dto.missed_from = Some(missed_from);
            dto.missed_through = Some(missed_through);
            dto.reason = Some(
                match reason {
                    GapReason::RetentionExceeded => "retention_exceeded",
                    GapReason::SubscriberLagged => "subscriber_lagged",
                }
                .to_string(),
            );
        }
    }
    dto
}

fn map_execution_error(error: DaemonError) -> GatewayError {
    let message = error.to_string();
    match error {
        DaemonError::ExecutionQueueFull { .. } => GatewayError::ExecutionQueueFull(message),
        DaemonError::ExecutionUnavailable(_) => GatewayError::ExecutionUnavailable(message),
        DaemonError::SessionShuttingDown(_) => GatewayError::SessionShuttingDown(message),
        // A dormant session is a real, caller-actionable state ("attach to
        // restore it"), not a server fault -- reporting it as a 500 told
        // clients the daemon had broken when nothing had.
        DaemonError::SessionDormant(_) => GatewayError::SessionDormant(message),
        DaemonError::InputBufferFull(_) => GatewayError::InputBufferFull(message),
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
        // Raw input to whatever is reading, NOT a command to execute.
        //
        // This previously submitted the payload as a new execution and waited
        // up to 30 seconds for it to run, which meant `send` could not answer
        // a prompt and could run a command the caller never intended. A caller
        // that wants to run something uses `exec`.
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        coord
            .write_session_input(
                SessionId(session_id),
                // The HTTP surface has no per-connection identity, so its
                // input is unattributed: accepted while nobody holds
                // authority, refused once someone does.
                crate::executor::session_thread::InputOrigin::Unattributed,
                input.into_bytes(),
            )
            .map_err(map_execution_error)
    }

    fn end_input(&self, session_id: u32) -> Result<(), GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        coord
            .end_session_input(SessionId(session_id))
            .map_err(map_execution_error)
    }

    fn input_authority(&self, session_id: u32) -> Result<Option<u64>, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        coord
            .input_authority_holder(&SessionId(session_id))
            .map_err(map_execution_error)
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

    fn subscribe_events(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<LifecycleEventDto>, GatewayError> {
        // Establish the subscription under the lock, then release it: a
        // long-lived stream must never hold the coordinator mutex.
        let rx = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            // Existence check first, so an unknown session is a 404 before
            // any stream is opened rather than an empty stream that a client
            // would read as success.
            if coord.session_first_pane(SessionId(session_id)).is_none() {
                return Err(GatewayError::SessionNotFound(session_id));
            }
            coord
                .begin_subscribe_events(SessionId(session_id), resume_from)
                .map_err(map_execution_error)?
        };

        // Translate daemon events into the wire DTO on a small forwarding
        // task. The channel stays bounded end to end, so a stalled HTTP
        // client still cannot grow memory without limit.
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(SUBSCRIBER_FORWARD_BUFFER);
        let mut rx = rx;
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if out_tx.send(to_event_dto(event)).await.is_err() {
                    break;
                }
            }
        });
        Ok(out_rx)
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
