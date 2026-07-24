use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use malt_gateway::backend::GatewayBackend;
use malt_gateway::error::GatewayError;
use malt_gateway::server::build_router;
use malt_gateway::types::{ExecResult, PaneResponse, SessionResponse};

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

fn app() -> Router {
    build_router(Arc::new(MockBackend::new()))
}

#[tokio::test]
async fn health_endpoint() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
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
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/sessions")
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
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
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
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/sessions/1")
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
    let response = app()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/sessions/1")
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
async fn session_not_found() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/sessions/999")
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
