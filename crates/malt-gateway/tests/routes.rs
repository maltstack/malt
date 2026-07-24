use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use malt_gateway::auth::{AuthScope, TokenStore};
use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::rate_limit::RateLimiter;
use malt_gateway::server::build_router;
use malt_gateway::types::{ExecResult, PaneResponse, SessionResponse};
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
                isolation: "Bare".to_string(),
                state: "Active".to_string(),
            }],
        }
    }
}

impl GatewayBackend for MockBackend {
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
            isolation: isolation.unwrap_or_else(|| "Bare".to_string()),
            state: "Active".to_string(),
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
        })
    }

    fn send_input(&self, session_id: u32, _input: String) -> Result<(), GatewayError> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(())
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
