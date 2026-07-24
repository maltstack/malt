// Session CRUD endpoints — /api/sessions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use crate::backend::GatewayBackend;
use crate::error::GatewayError;
use crate::types::{ApiResponse, CreateSessionRequest, ExecRequest, SendInputRequest};

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

pub async fn output(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<serde_json::Value>>, GatewayError> {
    let data = backend.get_output(id)?;
    Ok(Json(ApiResponse::success(data)))
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
