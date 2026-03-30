// Server setup — build_router, listener binding, graceful shutdown.

/// Build the axum Router with all routes and middleware.
pub fn build_router<B: crate::backend::GatewayBackend>(_backend: std::sync::Arc<B>) -> axum::Router {
    axum::Router::new()
}
