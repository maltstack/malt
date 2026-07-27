use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use malt_gateway::auth::{AuthScope, TokenStore};
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::rate_limit::RateLimiter;
use malt_gateway::server::build_router;
use malt_gateway::types::{
    CommandHistoryEntry, ExecResult, LifecycleEventDto, OutputChunkDto, PaneResponse,
    SessionResponse,
};
use malt_gateway::with_auth;

struct MockBackend {
    sessions: Vec<SessionResponse>,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            sessions: vec![SessionResponse {
                id: 1,
                name: Some("main".to_string()),
                pane_count: 1,
                isolation: malt_gateway::types::IsolationStatusResponse {
                    effective: "bare".to_string(),
                    requested: "bare".to_string(),
                    basis: "none".to_string(),
                    mechanism: None,
                    detail: None,
                },
                state: "Active".to_string(),
                selected_image: None,
            }],
        }
    }
}

impl GatewayBackend for MockBackend {
    fn isolation_capabilities(
        &self,
    ) -> Result<Vec<malt_gateway::types::IsolationCapabilityResponse>, GatewayError> {
        Ok(vec![malt_gateway::types::IsolationCapabilityResponse {
            tier: "contained".to_string(),
            available: false,
            basis: "none".to_string(),
            mechanism: None,
            detail: Some("no HCS spawn path".to_string()),
        }])
    }

    fn list_sessions(&self) -> Result<Vec<SessionResponse>, GatewayError> {
        Ok(self.sessions.clone())
    }

    fn create_session(
        &self,
        name: Option<String>,
        isolation: Option<String>,
    ) -> Result<SessionResponse, GatewayError> {
        Ok(SessionResponse {
            id: 2,
            name,
            pane_count: 1,
            isolation: malt_gateway::types::IsolationStatusResponse {
                effective: isolation.clone().unwrap_or_else(|| "bare".to_string()),
                requested: isolation.unwrap_or_else(|| "bare".to_string()),
                basis: "none".to_string(),
                mechanism: None,
                detail: None,
            },
            state: "Active".to_string(),
            selected_image: None,
        })
    }

    fn get_session(&self, id: u32) -> Result<SessionResponse, GatewayError> {
        self.sessions
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or(GatewayError::SessionNotFound(id))
    }

    fn destroy_session(&self, id: u32) -> Result<(), GatewayError> {
        if self.sessions.iter().any(|s| s.id == id) {
            Ok(())
        } else {
            Err(GatewayError::SessionNotFound(id))
        }
    }

    fn exec_command(&self, session_id: u32, command: String) -> Result<ExecResult, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(ExecResult {
            command_id: 1,
            output: format!("ran: {command}"),
            stderr: String::new(),
            exit_code: Some(0),
            truncated: false,
            omitted_bytes: 0,
        })
    }

    fn send_input(&self, session_id: u32, _input: String) -> Result<(), GatewayError> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(())
        } else {
            Err(GatewayError::SessionNotFound(session_id))
        }
    }

    fn end_input(&self, session_id: u32) -> Result<(), GatewayError> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(())
        } else {
            Err(GatewayError::SessionNotFound(session_id))
        }
    }

    fn input_authority(&self, session_id: u32) -> Result<Option<u64>, GatewayError> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(None)
        } else {
            Err(GatewayError::SessionNotFound(session_id))
        }
    }

    fn get_output(&self, session_id: u32) -> Result<serde_json::Value, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(serde_json::json!({"lines": []}))
    }

    fn get_output_text(&self, session_id: u32) -> Result<String, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok("mock plain text output".to_string())
    }

    fn get_command_history(
        &self,
        session_id: u32,
    ) -> Result<Vec<CommandHistoryEntry>, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(vec![
            CommandHistoryEntry {
                command_id: 1,
                cmd: "echo hello".to_string(),
                started_at: 1_784_070_000_123,
                finished_at: Some(1_784_070_000_456),
                exit_code: Some(0),
                pane_id: 1,
            },
            // Not confirmed complete -- exercises the null/null wire shape.
            CommandHistoryEntry {
                command_id: 2,
                cmd: "sleep 300".to_string(),
                started_at: 1_784_070_060_000,
                finished_at: None,
                exit_code: None,
                pane_id: 1,
            },
        ])
    }

    fn subscribe_events(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<LifecycleEventDto>, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut event = LifecycleEventDto {
            sequence: 1,
            kind: "command_started".to_string(),
            command_id: Some(1),
            cmd: Some("echo hello".to_string()),
            started_at: Some(1_784_070_000_123),
            finished_at: None,
            exit_code: None,
            duration_us: None,
            missed_from: None,
            missed_through: None,
            reason: None,
        };
        // Echo the resume position back through the sequence so a test can
        // prove the header actually reached the backend.
        if let Some(from) = resume_from {
            event.sequence = from + 1;
        }
        let _ = tx.try_send(event);
        Ok(rx)
    }

    fn subscribe_output(
        &self,
        session_id: u32,
        resume_from: Option<u64>,
    ) -> Result<tokio::sync::mpsc::Receiver<OutputChunkDto>, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut chunk = OutputChunkDto {
            sequence: 1,
            kind: "output".to_string(),
            command_id: Some(1),
            stream: Some("stdout".to_string()),
            data: Some("aGVsbG8=".to_string()),
            produced_at: Some(1_784_070_000_123),
            from: None,
            to: None,
            reason: None,
        };
        // Echo the resume position back through the sequence so a test can
        // prove the header actually reached the backend.
        if let Some(from) = resume_from {
            chunk.sequence = from + 1;
        }
        let _ = tx.try_send(chunk);
        Ok(rx)
    }

    fn list_panes(&self, session_id: u32) -> Result<Vec<PaneResponse>, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(vec![PaneResponse {
            id: 1,
            kind: "shell".to_string(),
            title: None,
            focused: true,
        }])
    }

    fn split_pane(
        &self,
        session_id: u32,
        _target_pane_id: u32,
        _direction: String,
    ) -> Result<PaneResponse, GatewayError> {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return Err(GatewayError::SessionNotFound(session_id));
        }
        Ok(PaneResponse {
            id: 2,
            kind: "shell".to_string(),
            title: None,
            focused: false,
        })
    }

    fn close_pane(&self, session_id: u32, _pane_id: u32) -> Result<(), GatewayError> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(())
        } else {
            Err(GatewayError::SessionNotFound(session_id))
        }
    }
}

/// Returns a fully wired (auth-enforced) router plus an Admin-scoped token
/// tests can use to authenticate their requests.
fn app() -> (Router, String) {
    let token_store = Arc::new(TokenStore::new());
    let token = token_store.generate_token(AuthScope::Admin);
    let rate_limiter = Arc::new(RateLimiter::new(1000));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );
    (router, token)
}

#[tokio::test]
async fn health_endpoint() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["status"], "ok");
}

#[tokio::test]
async fn list_sessions() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn isolation_capabilities_are_read_scoped_and_structured() {
    let store = TokenStore::new();
    let read_token = store.generate_token(AuthScope::Read);
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        Arc::new(store),
        Arc::new(RateLimiter::new(100)),
    );
    let response = router
        .oneshot(
            Request::builder()
                .uri("/isolation/capabilities")
                .header("authorization", format!("Bearer {read_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(std::str::from_utf8(&body).unwrap().contains("contained"));
}

#[tokio::test]
async fn create_session() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["id"], 2);
}

#[tokio::test]
async fn get_session() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["name"], "main");
}

#[tokio::test]
async fn destroy_session() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/sessions/1")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn output_text_returns_plain_text_shape() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/output/text")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["type"], "PlainText");
    assert_eq!(json["data"]["text"], "mock plain text output");
}

#[tokio::test]
async fn history_route_returns_chronological_entries() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/history")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);

    let entries = json["data"].as_array().expect("data must be an array");
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0]["command_id"], 1);
    assert_eq!(entries[0]["cmd"], "echo hello");
    assert_eq!(entries[0]["started_at"], 1_784_070_000_123u64);
    assert_eq!(entries[0]["finished_at"], 1_784_070_000_456u64);
    assert_eq!(entries[0]["exit_code"], 0);
    assert_eq!(entries[0]["pane_id"], 1);

    // A command that never reported completion must serialize both fields as
    // null -- never as a zero exit code, which would read as success.
    assert_eq!(entries[1]["command_id"], 2);
    assert!(
        entries[1]["finished_at"].is_null(),
        "an unfinished command must report finished_at as null, got {}",
        entries[1]["finished_at"]
    );
    assert!(
        entries[1]["exit_code"].is_null(),
        "an unfinished command must report exit_code as null, got {}",
        entries[1]["exit_code"]
    );
}

#[tokio::test]
async fn history_for_unknown_session_is_not_found() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/999/history")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Must be 404, not 200 with an empty list -- an unknown session and a
    // real session that has run nothing are different answers.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "session_not_found");
}

#[tokio::test]
async fn history_requires_a_token() {
    let (router, _token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "command history must not be readable without a token -- command \
         text can contain paths, arguments, and secrets typed at the prompt"
    );
}

#[tokio::test]
async fn history_requires_read_scope() {
    // Monitor is for liveness/inventory only; history is session content and
    // must sit behind Read, like /output.
    let token_store = Arc::new(TokenStore::new());
    let monitor_token = token_store.generate_token(AuthScope::Monitor);
    let read_token = token_store.generate_token(AuthScope::Read);
    let rate_limiter = Arc::new(RateLimiter::new(1000));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );

    let forbidden = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/1/history")
                .header("authorization", format!("Bearer {monitor_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let allowed = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/history")
                .header("authorization", format!("Bearer {read_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "a Read-scoped token must be sufficient for history"
    );
}

#[tokio::test]
async fn events_route_streams_sse() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/events")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "expected an SSE stream, got content-type {content_type:?}"
    );

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: command_started"), "frames: {text}");
    assert!(
        text.contains("id: 1"),
        "frames must carry a resume id: {text}"
    );
    assert!(text.contains("echo hello"), "frames: {text}");
}

#[tokio::test]
async fn events_route_passes_last_event_id_to_the_backend() {
    // The mock echoes resume_from into the sequence, so seeing id: 43 proves
    // the header was parsed and forwarded rather than silently ignored --
    // which would look identical to a working stream from the client side.
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/events")
                .header("authorization", format!("Bearer {token}"))
                .header("last-event-id", "42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("id: 43"),
        "resume position not forwarded: {text}"
    );
}

#[tokio::test]
async fn events_for_unknown_session_is_not_found_before_the_stream_opens() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/999/events")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Must be an HTTP error, never a 200 that then emits an error frame --
    // an SSE client reads an opened stream as success.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        !content_type.starts_with("text/event-stream"),
        "a failure must not be delivered as a stream"
    );
}

#[tokio::test]
async fn events_requires_read_scope() {
    let token_store = Arc::new(TokenStore::new());
    let monitor_token = token_store.generate_token(AuthScope::Monitor);
    let rate_limiter = Arc::new(RateLimiter::new(1000));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );

    let no_token = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

    let forbidden = router
        .oneshot(
            Request::builder()
                .uri("/sessions/1/events")
                .header("authorization", format!("Bearer {monitor_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        forbidden.status(),
        StatusCode::FORBIDDEN,
        "the event stream carries command text and must sit behind Read"
    );
}

#[tokio::test]
async fn session_not_found() {
    let (router, token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions/999")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "session_not_found");
}

#[tokio::test]
async fn request_with_no_token_is_rejected() {
    let (router, _token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "every route must require a bearer token now -- this is the whole \
         point of wiring TokenStore/AuthContext into build_router"
    );
}

#[tokio::test]
async fn request_with_invalid_token_is_rejected() {
    let (router, _token) = app();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .header("authorization", "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn insufficient_scope_is_forbidden() {
    // A Monitor-scoped token trying to create a session (requires Interact)
    // must be rejected -- proves per-route scope checking, not just
    // "any valid token gets in."
    let token_store = Arc::new(TokenStore::new());
    let monitor_token = token_store.generate_token(AuthScope::Monitor);
    let rate_limiter = Arc::new(RateLimiter::new(1000));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {monitor_token}"))
                .body(Body::from(r#"{"name":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a Monitor-scoped token must not be able to create a session \
         (requires Interact) -- per-route scope must actually be checked, \
         not just token validity"
    );
}

#[tokio::test]
async fn monitor_scope_can_still_read_health() {
    // The same low-privilege token from the test above must still work for
    // a Monitor-level route -- proves the scope check isn't accidentally
    // requiring Admin everywhere.
    let token_store = Arc::new(TokenStore::new());
    let monitor_token = token_store.generate_token(AuthScope::Monitor);
    let rate_limiter = Arc::new(RateLimiter::new(1000));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );

    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("authorization", format!("Bearer {monitor_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_exceeded_is_rejected() {
    let token_store = Arc::new(TokenStore::new());
    let token = token_store.generate_token(AuthScope::Admin);
    let rate_limiter = Arc::new(RateLimiter::new(2));
    let router = with_auth(
        build_router(Arc::new(MockBackend::new())),
        token_store,
        rate_limiter,
    );

    let make_req = || {
        Request::builder()
            .uri("/health")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };

    let r1 = router.clone().oneshot(make_req()).await.unwrap();
    let r2 = router.clone().oneshot(make_req()).await.unwrap();
    let r3 = router.clone().oneshot(make_req()).await.unwrap();

    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(
        r3.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the 3rd request within a 2-request-per-window budget must be rate limited"
    );
}

#[tokio::test]
async fn execution_admission_errors_have_stable_statuses_and_codes() {
    let cases = [
        (
            GatewayError::ExecutionQueueFull("queue full".to_string()),
            StatusCode::SERVICE_UNAVAILABLE,
            "execution_queue_full",
        ),
        (
            GatewayError::ExecutionUnavailable("worker lost".to_string()),
            StatusCode::SERVICE_UNAVAILABLE,
            "execution_unavailable",
        ),
        (
            GatewayError::SessionShuttingDown("closing".to_string()),
            StatusCode::CONFLICT,
            "session_shutting_down",
        ),
        (
            GatewayError::RateLimited,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
        ),
    ];
    for (error, status, code) in cases {
        let response = error.into_response();
        assert_eq!(response.status(), status);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], code);
    }
}
