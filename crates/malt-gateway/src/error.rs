use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Errors returned by the API gateway.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GatewayError {
    #[error("session not found: {0}")]
    SessionNotFound(u32),

    #[error("pane not found: {0}")]
    PaneNotFound(u32),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: requires {required}")]
    Forbidden { required: String },

    #[error("rate limited")]
    RateLimited,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            GatewayError::SessionNotFound(_) => (StatusCode::NOT_FOUND, "session_not_found"),
            GatewayError::PaneNotFound(_) => (StatusCode::NOT_FOUND, "pane_not_found"),
            GatewayError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
            GatewayError::Forbidden { .. } => (StatusCode::FORBIDDEN, "forbidden"),
            GatewayError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            GatewayError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            GatewayError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };

        let body = ErrorBody {
            ok: false,
            error: ErrorDetail {
                code,
                message: self.to_string(),
            },
        };

        (status, axum::Json(body)).into_response()
    }
}
