// Health check endpoint — GET /health.

use axum::Json;
use crate::types::ApiResponse;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
}

pub async fn health() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse::success(HealthResponse {
        status: "ok".to_string(),
    }))
}
