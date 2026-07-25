// Session CRUD endpoints — /api/sessions.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures_core::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::backend::GatewayBackend;
use crate::error::GatewayError;
use crate::types::{
    ApiResponse, CommandHistoryEntry, CreateSessionRequest, ExecRequest, LifecycleEventDto,
    SendInputRequest,
};

/// Optional `?resume_from=` query parameter, an alternative to the
/// `Last-Event-ID` header for clients that cannot set headers easily.
#[derive(Debug, serde::Deserialize)]
pub struct EventsQuery {
    pub resume_from: Option<u64>,
}

pub async fn list(
    State(backend): State<Arc<dyn GatewayBackend>>,
) -> Result<Json<ApiResponse<Vec<crate::types::SessionResponse>>>, GatewayError> {
    let sessions = backend.list_sessions()?;
    Ok(Json(ApiResponse::success(sessions)))
}

pub async fn create(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<ApiResponse<crate::types::SessionResponse>>, GatewayError> {
    let session = backend.create_session(req.name, req.isolation)?;
    Ok(Json(ApiResponse::success(session)))
}

pub async fn get(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<crate::types::SessionResponse>>, GatewayError> {
    let session = backend.get_session(id)?;
    Ok(Json(ApiResponse::success(session)))
}

pub async fn destroy(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    backend.destroy_session(id)?;
    Ok(Json(serde_json::json!({"ok": true, "data": null})))
}

pub async fn exec(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
    Json(req): Json<ExecRequest>,
) -> Result<Json<ApiResponse<crate::types::ExecResult>>, GatewayError> {
    let result = backend.exec_command(id, req.command)?;
    Ok(Json(ApiResponse::success(result)))
}

pub async fn send_input(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
    Json(req): Json<SendInputRequest>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    backend.send_input(id, req.input)?;
    Ok(Json(serde_json::json!({"ok": true, "data": null})))
}

pub async fn end_input(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    backend.end_input(id)?;
    Ok(Json(serde_json::json!({"ok": true, "data": null})))
}

pub async fn output(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<serde_json::Value>>, GatewayError> {
    let data = backend.get_output(id)?;
    Ok(Json(ApiResponse::success(data)))
}

/// This session's command execution history, oldest first.
pub async fn history(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<Vec<CommandHistoryEntry>>>, GatewayError> {
    let entries = backend.get_command_history(id)?;
    Ok(Json(ApiResponse::success(entries)))
}

/// Stream this session's command lifecycle events as Server-Sent Events.
///
/// Resume position comes from `Last-Event-ID` (what a standard SSE client
/// sends automatically on reconnect) or an explicit `?resume_from=`. A
/// malformed value means "start from now" rather than an error — a
/// first-time subscriber must not be forced to process the session's past.
///
/// Every failure is returned as an HTTP status *before* the stream opens,
/// because an SSE client treats an established stream as success.
pub async fn events(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, GatewayError> {
    let resume_from = query.resume_from.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    });

    let rx = backend.subscribe_events(id, resume_from)?;
    let stream = ReceiverStream::new(rx).map(|dto| Ok(to_sse_event(&dto)));
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Render one event as an SSE frame: `id` is the resume position, `event` is
/// the type, `data` is the JSON payload.
fn to_sse_event(dto: &LifecycleEventDto) -> Event {
    let event = Event::default()
        .id(dto.sequence.to_string())
        .event(&dto.kind);
    match serde_json::to_string(dto) {
        Ok(json) => event.data(json),
        // Serialization of a plain data struct should not fail; if it
        // somehow does, say so in-band rather than dropping the frame
        // silently, which would create an unsignalled hole in the stream.
        Err(error) => event.data(format!(
            r#"{{"error":"failed to serialize event: {error}"}}"#
        )),
    }
}

/// Plain-text variant of `output`, for programmatic/agent consumption.
pub async fn output_text(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<serde_json::Value>>, GatewayError> {
    let text = backend.get_output_text(id)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "type": "PlainText",
        "text": text,
    }))))
}
