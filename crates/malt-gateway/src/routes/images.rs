use crate::{
    backend::GatewayBackend,
    error::GatewayError,
    types::{ApiResponse, ImageResponse, ProvisionImageRequest},
};
use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
pub async fn provision(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Json(request): Json<ProvisionImageRequest>,
) -> Result<Json<ApiResponse<ImageResponse>>, GatewayError> {
    Ok(Json(ApiResponse::success(
        backend.provision_image(request.reference)?,
    )))
}
pub async fn list(
    State(backend): State<Arc<dyn GatewayBackend>>,
) -> Result<Json<ApiResponse<Vec<ImageResponse>>>, GatewayError> {
    Ok(Json(ApiResponse::success(backend.list_images()?)))
}
pub async fn inspect(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ImageResponse>>, GatewayError> {
    Ok(Json(ApiResponse::success(backend.inspect_image(id)?)))
}
pub async fn remove(
    State(backend): State<Arc<dyn GatewayBackend>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    backend.remove_image(id)?;
    Ok(Json(serde_json::json!({"ok":true,"data":null})))
}
