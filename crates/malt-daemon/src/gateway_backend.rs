use crate::executor::coordinator::Coordinator;
use crate::executor::events::{GapReason, LifecycleEvent, LifecycleEventKind};
use crate::executor::output_log::{OutputEvent, OutputEventKind};
use crate::DaemonError;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::types::{
    CommandHistoryEntry, ExecResult, ImageResponse, IsolationCapabilityResponse,
    IsolationStatusResponse, LifecycleEventDto, OutputChunkDto, PaneResponse, SessionResponse,
};
use malt_protocol::common::{IsolationPolicy, IsolationTier, SessionId, SessionState};
use malt_protocol::shell::OutputStream;
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

    /// Return the coordinator's currently live contained-session references
    /// for an immutable image id. The helper has a separate, narrower registry
    /// of HCS workspaces; this view survives an uncertain helper reply and
    /// prevents the gateway from treating a live daemon session as removable.
    fn active_image_sessions(&self, image_id: &str) -> Result<Vec<u32>, GatewayError> {
        let coordinator = self
            .coordinator
            .lock()
            .map_err(|_| GatewayError::Internal("coordinator lock poisoned".to_string()))?;
        Ok(coordinator
            .list_sessions()
            .into_iter()
            .filter(|session| {
                session.state == SessionState::Active
                    && session.selected_image.as_deref() == Some(image_id)
            })
            .map(|session| session.session_id.0)
            .collect())
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

/// Project a daemon output event onto the flat wire shape. `data` is
/// base64: output is arbitrary bytes and a multi-byte character may be
/// split across chunks, so lossy text decoding at the transport would be
/// unrecoverable by the client (research R6).
fn to_output_dto(event: OutputEvent) -> OutputChunkDto {
    let mut dto = OutputChunkDto {
        sequence: event.sequence,
        kind: String::new(),
        command_id: None,
        stream: None,
        data: None,
        produced_at: None,
        from: None,
        to: None,
        reason: None,
    };
    match event.kind {
        OutputEventKind::Chunk {
            command_id,
            stream,
            data,
            produced_at,
        } => {
            dto.kind = "output".to_string();
            dto.command_id = Some(command_id);
            dto.stream = Some(
                match stream {
                    OutputStream::Stdout => "stdout",
                    OutputStream::Stderr => "stderr",
                    // The wire enum is #[non_exhaustive]; a future variant
                    // this daemon build doesn't know about is reported
                    // honestly rather than misreported as stdout.
                    _ => "unknown",
                }
                .to_string(),
            );
            dto.data = Some(BASE64.encode(&data));
            dto.produced_at = Some(produced_at);
        }
        OutputEventKind::Gap {
            missed_from,
            missed_through,
            reason,
        } => {
            dto.kind = "gap".to_string();
            dto.from = Some(missed_from);
            dto.to = Some(missed_through);
            dto.reason = Some(
                match reason {
                    crate::executor::output_log::GapReason::RetentionExceeded => {
                        "retention_exceeded"
                    }
                    crate::executor::output_log::GapReason::SubscriberLagged => "subscriber_lagged",
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

fn parse_isolation_policy(
    s: Option<String>,
    tier: IsolationTier,
) -> Result<IsolationPolicy, GatewayError> {
    match s.as_deref() {
        None if tier == IsolationTier::Bare => Ok(IsolationPolicy::Disabled),
        None => Ok(IsolationPolicy::Required),
        Some("required") => Ok(IsolationPolicy::Required),
        Some("preferred") => Ok(IsolationPolicy::Preferred),
        Some("disabled") => Ok(IsolationPolicy::Disabled),
        Some(other) => Err(GatewayError::BadRequest(format!(
            "unrecognized isolation policy {other:?} (expected one of: required, preferred, disabled)"
        ))),
    }
}

fn isolation_status_response(
    status: malt_protocol::common::IsolationStatus,
) -> IsolationStatusResponse {
    IsolationStatusResponse {
        effective: format!("{:?}", status.effective).to_ascii_lowercase(),
        requested: format!("{:?}", status.requested).to_ascii_lowercase(),
        basis: format!("{:?}", status.basis).to_ascii_lowercase(),
        mechanism: status.mechanism,
        detail: status.detail,
    }
}

fn performed_payload(
    response: malt_protocol::elevate::ElevateResponse,
) -> Result<Vec<u8>, GatewayError> {
    if response.kind != malt_protocol::elevate::OutcomeKind::Performed {
        return Err(GatewayError::BadRequest(response.detail.unwrap_or_else(
            || "helper did not perform image operation".to_string(),
        )));
    }
    response.payload.ok_or_else(|| {
        GatewayError::Internal("helper performed image operation without payload".to_string())
    })
}
fn to_image_response(image: malt_protocol::elevate::ProvisionedImage) -> ImageResponse {
    ImageResponse {
        id: image.id,
        manifest_digest: image.manifest_digest,
        platform: image.platform,
        os_version: image.os_version,
        ready: image.ready,
        reason: image.reason,
        active_sessions: image.active_sessions,
        readiness_evidence: image.readiness_evidence,
    }
}

fn reconcile_image_response(mut image: ImageResponse, daemon_sessions: &[u32]) -> ImageResponse {
    let daemon_count = u32::try_from(daemon_sessions.len()).map_or(u32::MAX, |count| count);
    // The helper is authoritative for its live compute systems; the daemon is
    // authoritative for session admission. Keep the conservative maximum when
    // a helper restart or lost response makes either observation incomplete.
    image.active_sessions = image.active_sessions.max(daemon_count);
    image
}
fn image_response(
    response: malt_protocol::elevate::ElevateResponse,
) -> Result<ImageResponse, GatewayError> {
    let payload = performed_payload(response)?;
    let mut reader = malt_protocol::vexil_runtime::BitReader::new(&payload);
    let image =
        <malt_protocol::elevate::ProvisionedImage as malt_protocol::vexil_runtime::Unpack>::unpack(
            &mut reader,
        )
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(to_image_response(image))
}

/// Derive the contained-session capability from the helper's owned image
/// inventory. The platform report deliberately cannot make this claim on its
/// own: a live HCS route still needs a helper-owned prepared image selected by
/// the daemon.
fn contained_capability_from_images(
    images: &[malt_protocol::elevate::ProvisionedImage],
) -> IsolationCapabilityResponse {
    let ready_image = images
        .iter()
        .filter(|image| image.ready)
        .max_by_key(|image| image.readiness_evidence == "live-proven");

    match ready_image {
        Some(image) => IsolationCapabilityResponse {
            tier: "contained".to_string(),
            available: true,
            basis: "verified".to_string(),
            mechanism: Some("hcs-container".to_string()),
            detail: Some(format!(
                "helper reported {} image {}; session creation revalidates host compatibility immediately before HCS construction",
                image.readiness_evidence, image.id
            )),
        },
        None => IsolationCapabilityResponse {
            tier: "contained".to_string(),
            available: false,
            basis: "none".to_string(),
            mechanism: None,
            detail: Some(
                "the helper reported no HCS-prepared Windows image selectable by the contained session path"
                    .to_string(),
            ),
        },
    }
}

/// Query the privilege boundary rather than inferring contained availability
/// from a host primitive. A failed query is itself actionable evidence that
/// this daemon cannot currently use the helper-owned HCS route.
fn contained_capability_from_helper() -> IsolationCapabilityResponse {
    let result = (|| -> Result<_, GatewayError> {
        let response =
            crate::elevate_client::manage_image(malt_protocol::elevate::ImageOperation::List {})
                .map_err(|error| GatewayError::Internal(error.to_string()))?;
        let payload = performed_payload(response)?;
        let mut reader = malt_protocol::vexil_runtime::BitReader::new(&payload);
        <malt_protocol::elevate::ProvisionedImageList as malt_protocol::vexil_runtime::Unpack>::unpack(
            &mut reader,
        )
        .map_err(|error| GatewayError::Internal(error.to_string()))
    })();

    match result {
        Ok(images) => contained_capability_from_images(&images.images),
        Err(error) => IsolationCapabilityResponse {
            tier: "contained".to_string(),
            available: false,
            basis: "none".to_string(),
            mechanism: None,
            detail: Some(format!(
                "the helper-owned HCS session path could not be assessed: {error}"
            )),
        },
    }
}

impl GatewayBackend for DaemonBackend {
    fn provision_image(&self, reference: String) -> Result<ImageResponse, GatewayError> {
        image_response(
            crate::elevate_client::manage_image(
                malt_protocol::elevate::ImageOperation::Provision { reference },
            )
            .map_err(|e| GatewayError::Internal(e.to_string()))?,
        )
    }
    fn list_images(&self) -> Result<Vec<ImageResponse>, GatewayError> {
        let response =
            crate::elevate_client::manage_image(malt_protocol::elevate::ImageOperation::List {})
                .map_err(|e| GatewayError::Internal(e.to_string()))?;
        let payload = performed_payload(response)?;
        let mut reader = malt_protocol::vexil_runtime::BitReader::new(&payload);
        let list = <malt_protocol::elevate::ProvisionedImageList as malt_protocol::vexil_runtime::Unpack>::unpack(&mut reader).map_err(|e| GatewayError::Internal(e.to_string()))?;
        list.images
            .into_iter()
            .map(|image| {
                let image = to_image_response(image);
                let sessions = self.active_image_sessions(&image.id)?;
                Ok(reconcile_image_response(image, &sessions))
            })
            .collect()
    }
    fn inspect_image(&self, id: String) -> Result<ImageResponse, GatewayError> {
        let image = image_response(
            crate::elevate_client::manage_image(malt_protocol::elevate::ImageOperation::Inspect {
                id,
            })
            .map_err(|e| GatewayError::Internal(e.to_string()))?,
        )?;
        let sessions = self.active_image_sessions(&image.id)?;
        Ok(reconcile_image_response(image, &sessions))
    }
    fn remove_image(&self, id: String) -> Result<(), GatewayError> {
        let dependent_sessions = self.active_image_sessions(&id)?;
        if !dependent_sessions.is_empty() {
            return Err(GatewayError::BadRequest(format!(
                "cannot remove image while contained session{} {} {} active",
                if dependent_sessions.len() == 1 {
                    ""
                } else {
                    "s"
                },
                dependent_sessions
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                if dependent_sessions.len() == 1 {
                    "is"
                } else {
                    "are"
                },
            )));
        }
        let _ = performed_payload(
            crate::elevate_client::manage_image(malt_protocol::elevate::ImageOperation::Remove {
                id,
            })
            .map_err(|e| GatewayError::Internal(e.to_string()))?,
        )?;
        Ok(())
    }
    fn isolation_capabilities(&self) -> Result<Vec<IsolationCapabilityResponse>, GatewayError> {
        let mut capabilities = malt_platform::isolation::session_tier_capabilities()
            .into_iter()
            .map(|capability| IsolationCapabilityResponse {
                tier: format!("{:?}", capability.tier).to_ascii_lowercase(),
                available: capability.available,
                basis: format!("{:?}", capability.basis).to_ascii_lowercase(),
                mechanism: capability
                    .mechanism
                    .map(|mechanism| format!("{:?}", mechanism).to_ascii_lowercase()),
                detail: capability.detail,
            })
            .collect::<Vec<_>>();
        if let Some(contained) = capabilities
            .iter_mut()
            .find(|capability| capability.tier == "contained")
        {
            *contained = contained_capability_from_helper();
        }
        Ok(capabilities)
    }

    fn list_sessions(&self) -> Result<Vec<SessionResponse>, GatewayError> {
        let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let sessions = coord.list_sessions();
        Ok(sessions
            .into_iter()
            .map(|s| SessionResponse {
                id: s.session_id.0,
                name: s.name,
                pane_count: s.pane_count,
                isolation: isolation_status_response(s.isolation),
                state: format!("{:?}", s.state),
                selected_image: s.selected_image,
            })
            .collect())
    }

    fn create_session(
        &self,
        name: Option<String>,
        isolation: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        self.create_session_with_policy(name, isolation, None)
    }

    fn create_session_with_policy(
        &self,
        name: Option<String>,
        isolation: Option<String>,
        isolation_policy: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        self.create_session_with_policy_and_image(name, isolation, isolation_policy, None)
    }

    fn create_session_with_policy_and_image(
        &self,
        name: Option<String>,
        isolation: Option<String>,
        isolation_policy: Option<String>,
        image: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        let tier = parse_isolation(isolation)?;
        let policy = parse_isolation_policy(isolation_policy, tier)?;
        let mut coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        let session_id = coord
            .create_session_with_policy_and_image(name.clone(), tier, policy, None, image)
            .map_err(|e| match e {
                DaemonError::IsolationUnavailable(detail) => GatewayError::IsolationUnavailable {
                    message: format!(
                        "{tier:?} was required but could not be established: {detail}. Retry with isolation_policy=preferred to accept a lower level."
                    ),
                    requested: format!("{tier:?}").to_ascii_lowercase(),
                    best_available: "bare".to_string(),
                },
                other => GatewayError::Internal(other.to_string()),
            })?;

        let session = coord
            .list_sessions()
            .into_iter()
            .find(|s| s.session_id == session_id);
        Ok(SessionResponse {
            id: session_id.0,
            name,
            pane_count: 1,
            isolation: session
                .as_ref()
                .map(|s| isolation_status_response(s.isolation.clone()))
                .unwrap_or(IsolationStatusResponse {
                    effective: format!("{tier:?}").to_ascii_lowercase(),
                    requested: format!("{tier:?}").to_ascii_lowercase(),
                    basis: "none".to_string(),
                    mechanism: None,
                    detail: None,
                }),
            state: "Active".to_string(),
            selected_image: session.and_then(|s| s.selected_image),
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
                isolation: isolation_status_response(s.isolation),
                state: format!("{:?}", s.state),
                selected_image: s.selected_image,
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
            truncated: result.truncated,
            omitted_bytes: result.omitted_bytes,
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

    fn subscribe_output(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<OutputChunkDto>, GatewayError> {
        // Mirrors subscribe_events exactly -- see that method's doc.
        let rx = {
            let coord = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
            if coord.session_first_pane(SessionId(session_id)).is_none() {
                return Err(GatewayError::SessionNotFound(session_id));
            }
            coord
                .begin_subscribe_output(SessionId(session_id), resume_from)
                .map_err(map_execution_error)?
        };

        let (out_tx, out_rx) = tokio::sync::mpsc::channel(SUBSCRIBER_FORWARD_BUFFER);
        let mut rx = rx;
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if out_tx.send(to_output_dto(event)).await.is_err() {
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

#[cfg(test)]
mod contained_capability_tests {
    use super::contained_capability_from_images;
    use malt_protocol::elevate::ProvisionedImage;

    fn image(id: &str, ready: bool, readiness_evidence: &str) -> ProvisionedImage {
        ProvisionedImage {
            id: id.to_string(),
            manifest_digest: id.to_string(),
            platform: "windows/amd64".to_string(),
            os_version: Some("10.0.20348.0".to_string()),
            ready,
            reason: None,
            active_sessions: 0,
            readiness_evidence: readiness_evidence.to_string(),
            _unknown: Vec::new(),
        }
    }

    #[test]
    fn contained_capability_requires_a_helper_prepared_image() {
        let capability =
            contained_capability_from_images(&[image("sha256:acquired", false, "acquired")]);

        assert!(!capability.available);
        assert_eq!(capability.basis, "none");
        assert!(capability.mechanism.is_none());
    }

    #[test]
    fn contained_capability_prefers_live_proven_helper_evidence() {
        let capability = contained_capability_from_images(&[
            image("sha256:prepared", true, "hcs-prepared"),
            image("sha256:live", true, "live-proven"),
        ]);

        assert!(capability.available);
        assert_eq!(capability.basis, "verified");
        assert_eq!(capability.mechanism.as_deref(), Some("hcs-container"));
        assert!(capability
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("live-proven image sha256:live")));
    }
}
