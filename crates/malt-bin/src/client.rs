use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

impl<T> ApiEnvelope<T> {
    fn into_data(self, operation: &str) -> Result<T> {
        let error_message = self.error.map(ApiError::into_message);
        match (self.ok, self.data) {
            (true, Some(data)) => Ok(data),
            (true, None) => Err(anyhow::anyhow!("{operation} response contained no data")),
            (false, _) => Err(anyhow::anyhow!(
                "{}",
                error_message.unwrap_or_else(|| format!("{operation} failed"))
            )),
        }
    }
}

/// Error representations returned by gateway endpoints.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ApiError {
    Message(String),
    Detail { message: String },
}

impl ApiError {
    fn into_message(self) -> String {
        match self {
            Self::Message(message) | Self::Detail { message } => message,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionData {
    pub id: u32,
    pub name: Option<String>,
    pub pane_count: u32,
    pub isolation: String,
    pub state: String,
}

/// Payload sent to the existing create-session endpoint.
#[derive(Debug, Serialize)]
struct CreateSessionRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    isolation: Option<&'a str>,
}

impl<'a> CreateSessionRequest<'a> {
    fn new(name: Option<&'a str>, isolation: Option<&'a str>) -> Self {
        Self { name, isolation }
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ExecResultData {
    pub command_id: u32,
    pub output: String,
    #[serde(default)]
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct HealthData {
    pub status: String,
}

/// Response shape from `GET /sessions/{id}/output/text`.
#[derive(Debug, Deserialize)]
pub struct OutputTextData {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub kind: String,
    pub text: String,
}

pub struct MaltClient {
    addr: String,
    http: Client,
    /// Bearer token read once at construction from the same well-known
    /// file the daemon's `TokenStore::load_or_generate_default` writes
    /// (`malt_gateway::auth::dirs_token_path()`). `None` if the file
    /// doesn't exist yet (e.g. daemon never started) -- requests are then
    /// sent unauthenticated and will get a real 401 from the Gateway,
    /// which is correct: no local fallback bypasses real enforcement.
    token: Option<String>,
}

impl MaltClient {
    pub fn new(addr: &str) -> Self {
        let addr = addr.trim_end_matches('/').to_owned();
        let token = std::fs::read_to_string(malt_gateway::auth::dirs_token_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self {
            addr,
            http: Client::new(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.addr, path)
    }

    /// Attach the bearer token, if we have one, to an outgoing request.
    fn authed(&self, builder: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    pub fn health(&self) -> Result<HealthData> {
        let resp = self
            .authed(self.http.get(self.url("/health")))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<HealthData> = resp.json().context("invalid health response")?;
        envelope.into_data("health")
    }

    pub fn shutdown(&self) -> Result<()> {
        self.authed(self.http.post(self.url("/shutdown")))
            .send()
            .context("failed to reach daemon")?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionData>> {
        let resp = self
            .authed(self.http.get(self.url("/sessions")))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<Vec<SessionData>> =
            resp.json().context("invalid session list response")?;
        envelope.into_data("session list")
    }

    pub fn create_session(
        &self,
        name: Option<&str>,
        isolation: Option<&str>,
    ) -> Result<SessionData> {
        let req = self
            .authed(self.http.post(self.url("/sessions")))
            .json(&CreateSessionRequest::new(name, isolation));
        let resp = req.send().context("failed to reach daemon")?;
        let envelope: ApiEnvelope<SessionData> =
            resp.json().context("invalid create session response")?;
        envelope.into_data("create session")
    }

    pub fn destroy_session(&self, id: u32) -> Result<()> {
        let resp = self
            .authed(self.http.delete(self.url(&format!("/sessions/{id}"))))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid destroy session response")?;
        let _: serde_json::Value = envelope.into_data("destroy session")?;
        Ok(())
    }

    pub fn exec_command(&self, id: u32, cmd: &str) -> Result<ExecResultData> {
        let resp = self
            .authed(self.http.post(self.url(&format!("/sessions/{id}/exec"))))
            .json(&serde_json::json!({ "command": cmd }))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<ExecResultData> = resp.json().context("invalid exec response")?;
        envelope.into_data("exec")
    }

    /// Fetch the session's current output as plain text (no styling) --
    /// the agent/CLI-friendly variant, distinct from the StyledGrid shape
    /// `malt-tui`/`maltty` consume.
    pub fn get_output_text(&self, id: u32) -> Result<OutputTextData> {
        let resp = self
            .authed(self.http.get(self.url(&format!("/sessions/{id}/output/text"))))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<OutputTextData> =
            resp.json().context("invalid output response")?;
        envelope.into_data("get output")
    }

    pub fn send_input(&self, id: u32, input: &str) -> Result<()> {
        let resp = self
            .authed(self.http.post(self.url(&format!("/sessions/{id}/send"))))
            .json(&serde_json::json!({ "input": input }))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid send response")?;
        let _: serde_json::Value = envelope.into_data("send input")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_construction() {
        let client = MaltClient::new("http://127.0.0.1:7700");
        assert_eq!(client.url("/health"), "http://127.0.0.1:7700/health");
        assert_eq!(
            client.url("/sessions/1/exec"),
            "http://127.0.0.1:7700/sessions/1/exec"
        );
    }

    #[test]
    fn api_url_trailing_slash() {
        let client = MaltClient::new("http://127.0.0.1:7700/");
        assert_eq!(client.url("/health"), "http://127.0.0.1:7700/health");
    }

    #[test]
    fn create_session_payload_preserves_legacy_and_selected_tier_shapes() {
        let empty = CreateSessionRequest::new(None, None);
        assert_eq!(serde_json::to_value(empty).unwrap(), serde_json::json!({}));

        let named = CreateSessionRequest::new(Some("build"), None);
        assert_eq!(
            serde_json::to_value(named).unwrap(),
            serde_json::json!({ "name": "build" })
        );

        let tier_only = CreateSessionRequest::new(None, Some("restricted"));
        assert_eq!(
            serde_json::to_value(tier_only).unwrap(),
            serde_json::json!({ "isolation": "restricted" })
        );

        let named_tier = CreateSessionRequest::new(Some("build"), Some("capped"));
        assert_eq!(
            serde_json::to_value(named_tier).unwrap(),
            serde_json::json!({ "name": "build", "isolation": "capped" })
        );
    }

    #[test]
    fn failed_create_session_response_preserves_gateway_message() {
        let json = r#"{
            "ok": false,
            "data": null,
            "error": {
                "code": "bad_request",
                "message": "requested isolation tier is unavailable"
            }
        }"#;
        let envelope: ApiEnvelope<SessionData> = serde_json::from_str(json).unwrap();
        let error = envelope.into_data("create session").unwrap_err();
        assert_eq!(error.to_string(), "requested isolation tier is unavailable");
    }

    #[test]
    fn parse_session_list_response() {
        let json = r#"{
            "ok": true,
            "data": [
                {
                    "id": 1,
                    "name": "dev",
                    "pane_count": 2,
                    "isolation": "Bare",
                    "state": "Running"
                }
            ],
            "error": null
        }"#;
        let envelope: ApiEnvelope<Vec<SessionData>> = serde_json::from_str(json).unwrap();
        assert!(envelope.ok);
        let sessions = envelope.data.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, 1);
        assert_eq!(sessions[0].name.as_deref(), Some("dev"));
        assert_eq!(sessions[0].pane_count, 2);
        assert_eq!(sessions[0].isolation, "Bare");
        assert_eq!(sessions[0].state, "Running");
    }

    #[test]
    fn parse_health_response() {
        let json = r#"{
            "ok": true,
            "data": { "status": "ok" },
            "error": null
        }"#;
        let envelope: ApiEnvelope<HealthData> = serde_json::from_str(json).unwrap();
        assert!(envelope.ok);
        let health = envelope.data.unwrap();
        assert_eq!(health.status, "ok");
    }

    #[test]
    fn parse_output_text_response() {
        let json = r#"{
            "ok": true,
            "data": {
                "type": "PlainText",
                "text": "hello\n"
            },
            "error": null
        }"#;
        let envelope: ApiEnvelope<OutputTextData> = serde_json::from_str(json).unwrap();
        assert!(envelope.ok);
        let result = envelope.data.unwrap();
        assert_eq!(result.text, "hello\n");
    }

    #[test]
    fn parse_exec_response() {
        let json = r#"{
            "ok": true,
            "data": {
                "command_id": 1,
                "output": "hello\n",
                "exit_code": 0
            },
            "error": null
        }"#;
        let envelope: ApiEnvelope<ExecResultData> = serde_json::from_str(json).unwrap();
        assert!(envelope.ok);
        let result = envelope.data.unwrap();
        assert_eq!(result.command_id, 1);
        assert_eq!(result.output, "hello\n");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            result.stderr, "",
            "a response with no stderr field should default to empty, not fail to parse"
        );
    }

    #[test]
    fn parse_exec_response_with_stderr() {
        let json = r#"{
            "ok": true,
            "data": {
                "command_id": 2,
                "output": "",
                "stderr": "error: something failed\n",
                "exit_code": 1
            },
            "error": null
        }"#;
        let envelope: ApiEnvelope<ExecResultData> = serde_json::from_str(json).unwrap();
        let result = envelope.data.unwrap();
        assert_eq!(result.stderr, "error: something failed\n");
        assert_eq!(result.exit_code, Some(1));
    }
}
