// Server setup — build_router, listener binding, graceful shutdown.

use crate::backend::GatewayBackend;
use crate::routes;
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

/// Build the axum Router with all routes and middleware.
pub fn build_router(backend: Arc<dyn GatewayBackend>) -> Router {
    Router::new()
        .route("/health", get(routes::health::health))
        .route("/sessions", get(routes::sessions::list))
        .route("/sessions", post(routes::sessions::create))
        .route("/sessions/{id}", get(routes::sessions::get))
        .route("/sessions/{id}", delete(routes::sessions::destroy))
        .route("/sessions/{id}/exec", post(routes::sessions::exec))
        .route("/sessions/{id}/send", post(routes::sessions::send_input))
        .route("/sessions/{id}/output", get(routes::sessions::output))
        .route("/sessions/{id}/panes", get(routes::panes::list))
        .route("/sessions/{id}/panes/split", post(routes::panes::split))
        .route(
            "/sessions/{id}/panes/{pane_id}",
            delete(routes::panes::close),
        )
        .with_state(backend)
}
