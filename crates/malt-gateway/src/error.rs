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

    #[error("execution queue full: {0}")]
    ExecutionQueueFull(String),

    #[error("execution unavailable: {0}")]
    ExecutionUnavailable(String),

    #[error("session shutting down: {0}")]
    SessionShuttingDown(String),

    /// The session exists but is dormant, so it cannot service the request
    /// until something attaches and restores it. A caller-actionable state,
    /// not a server fault — the same class as `SessionShuttingDown`.
    #[error("session dormant: {0}")]
    SessionDormant(String),

    /// The session is holding as much unread type-ahead as it will. The
    /// caller should retry once the command consumes some.
    #[error("input buffer full: {0}")]
    InputBufferFull(String),

    #[error("isolation unavailable: {message}")]
    IsolationUnavailable {
        message: String,
        requested: String,
        best_available: String,
    },

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
    #[serde(skip_serializing_if = "Option::is_none")]
    requested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_available: Option<String>,
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
            GatewayError::ExecutionQueueFull(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "execution_queue_full")
            }
            GatewayError::ExecutionUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "execution_unavailable")
            }
            GatewayError::SessionShuttingDown(_) => (StatusCode::CONFLICT, "session_shutting_down"),
            GatewayError::SessionDormant(_) => (StatusCode::CONFLICT, "session_dormant"),
            GatewayError::InputBufferFull(_) => {
                (StatusCode::TOO_MANY_REQUESTS, "input_buffer_full")
            }
            GatewayError::IsolationUnavailable { .. } => {
                (StatusCode::CONFLICT, "isolation_unavailable")
            }
            GatewayError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };

        let body = ErrorBody {
            ok: false,
            error: ErrorDetail {
                code,
                message: self.to_string(),
                requested: match &self {
                    GatewayError::IsolationUnavailable { requested, .. } => Some(requested.clone()),
                    _ => None,
                },
                best_available: match &self {
                    GatewayError::IsolationUnavailable { best_available, .. } => {
                        Some(best_available.clone())
                    }
                    _ => None,
                },
            },
        };

        (status, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn isolation_unavailable_is_a_structured_conflict() {
        let response = GatewayError::IsolationUnavailable {
            message: "contained unavailable; retry preferred".to_string(),
            requested: "contained".to_string(),
            best_available: "bare".to_string(),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["code"], "isolation_unavailable");
        assert_eq!(json["error"]["requested"], "contained");
        assert_eq!(json["error"]["best_available"], "bare");
    }
}
