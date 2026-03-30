// Pane management endpoints — /api/sessions/:id/panes.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use crate::backend::GatewayBackend;
use crate::error::GatewayError;
use crate::types::{ApiResponse, PaneResponse, SplitPaneRequest};

pub async fn list(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(session_id): Path<u32>,
) -> Result<Json<ApiResponse<Vec<PaneResponse>>>, GatewayError> {
    let panes = backend.list_panes(session_id)?;
    Ok(Json(ApiResponse::success(panes)))
}

pub async fn split(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(session_id): Path<u32>,
    Json(req): Json<SplitPaneRequest>,
) -> Result<Json<ApiResponse<PaneResponse>>, GatewayError> {
    let pane = backend.split_pane(session_id, req.target_pane_id, req.direction)?;
    Ok(Json(ApiResponse::success(pane)))
}

pub async fn close(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path((session_id, pane_id)): Path<(u32, u32)>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    backend.close_pane(session_id, pane_id)?;
    Ok(Json(serde_json::json!({"ok": true, "data": null})))
}
