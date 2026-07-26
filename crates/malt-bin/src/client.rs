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
    pub isolation: IsolationData,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IsolationData {
    pub effective: String,
    pub requested: String,
    pub basis: String,
    pub mechanism: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IsolationCapabilityData {
    pub tier: String,
    pub available: bool,
    pub basis: String,
    pub mechanism: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageData {
    pub id: String,
    pub manifest_digest: String,
    pub platform: String,
    pub os_version: Option<String>,
    pub ready: bool,
    pub reason: Option<String>,
    pub active_sessions: u32,
}

/// Payload sent to the existing create-session endpoint.
#[derive(Debug, Serialize)]
struct CreateSessionRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    isolation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    isolation_policy: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<&'a str>,
}

impl<'a> CreateSessionRequest<'a> {
    fn new(
        name: Option<&'a str>,
        isolation: Option<&'a str>,
        isolation_policy: Option<&'a str>,
        image: Option<&'a str>,
    ) -> Self {
        Self {
            name,
            isolation,
            isolation_policy,
            image,
        }
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
    /// `#[serde(default)]` so this client keeps working against an older
    /// daemon that predates truncation reporting.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub omitted_bytes: u64,
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

/// One entry from `GET /sessions/{id}/history`.
///
/// `finished_at`/`exit_code` are absent when the command is not confirmed
/// complete — still running, or interrupted by a daemon stop.
#[derive(Debug, Deserialize)]
pub struct CommandHistoryEntry {
    pub command_id: u32,
    pub cmd: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    #[allow(dead_code)]
    pub pane_id: u32,
}

/// An opened event stream, or the reason the server refused one.
///
/// A refusal is reported here rather than as a transport error because it is
/// terminal — retrying cannot make an unknown session exist or a token gain
/// scope, so the reconnect loop must not treat it as a blip.
pub struct EventStream {
    pub body: Option<reqwest::blocking::Response>,
    pub refusal: Option<String>,
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
    pub fn provision_image(&self, reference: &str) -> Result<ImageData> {
        let resp = self
            .authed(self.http.post(self.url("/images")))
            .json(&serde_json::json!({"reference":reference}))
            .send()
            .context("failed to reach daemon")?;
        resp.json::<ApiEnvelope<ImageData>>()
            .context("invalid image provision response")?
            .into_data("image provision")
    }
    pub fn list_images(&self) -> Result<Vec<ImageData>> {
        let resp = self
            .authed(self.http.get(self.url("/images")))
            .send()
            .context("failed to reach daemon")?;
        resp.json::<ApiEnvelope<Vec<ImageData>>>()
            .context("invalid image list response")?
            .into_data("image list")
    }
    pub fn inspect_image(&self, id: &str) -> Result<ImageData> {
        let resp = self
            .authed(self.http.get(self.url(&format!("/images/{id}"))))
            .send()
            .context("failed to reach daemon")?;
        resp.json::<ApiEnvelope<ImageData>>()
            .context("invalid image inspect response")?
            .into_data("image inspect")
    }
    pub fn remove_image(&self, id: &str) -> Result<()> {
        let response = self
            .authed(self.http.delete(self.url(&format!("/images/{id}"))))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            response.json().context("invalid image remove response")?;
        if envelope.ok {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                envelope
                    .error
                    .map(ApiError::into_message)
                    .unwrap_or_else(|| "image remove failed".to_string())
            ))
        }
    }
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
    fn authed(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
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

    pub fn isolation_capabilities(&self) -> Result<Vec<IsolationCapabilityData>> {
        let resp = self
            .authed(self.http.get(self.url("/isolation/capabilities")))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<Vec<IsolationCapabilityData>> = resp
            .json()
            .context("invalid isolation capabilities response")?;
        envelope.into_data("isolation capabilities")
    }

    pub fn create_session(
        &self,
        name: Option<&str>,
        isolation: Option<&str>,
        isolation_policy: Option<&str>,
        image: Option<&str>,
    ) -> Result<SessionData> {
        let req =
            self.authed(self.http.post(self.url("/sessions")))
                .json(&CreateSessionRequest::new(
                    name,
                    isolation,
                    isolation_policy,
                    image,
                ));
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
            .authed(
                self.http
                    .get(self.url(&format!("/sessions/{id}/output/text"))),
            )
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<OutputTextData> =
            resp.json().context("invalid output response")?;
        envelope.into_data("get output")
    }

    /// Fetch the session's command execution history, oldest first.
    pub fn get_command_history(&self, id: u32) -> Result<Vec<CommandHistoryEntry>> {
        let resp = self
            .authed(self.http.get(self.url(&format!("/sessions/{id}/history"))))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<Vec<CommandHistoryEntry>> =
            resp.json().context("invalid history response")?;
        envelope.into_data("get command history")
    }

    /// Open the lifecycle event stream, resuming after `from` when given.
    ///
    /// Returns the raw response for incremental reading — deliberately not
    /// `.json()`, which would block until the stream ended (i.e. forever).
    pub fn open_event_stream(&self, id: u32, from: Option<u64>) -> Result<EventStream> {
        self.open_sse_stream(&format!("/sessions/{id}/events"), from)
    }

    /// Open the output-chunk stream, resuming after `from` when given. Same
    /// shape and transport as `open_event_stream` -- see that method's doc.
    pub fn open_output_stream(&self, id: u32, from: Option<u64>) -> Result<EventStream> {
        self.open_sse_stream(&format!("/sessions/{id}/output/stream"), from)
    }

    fn open_sse_stream(&self, path: &str, from: Option<u64>) -> Result<EventStream> {
        let mut req = self
            .authed(self.http.get(self.url(path)))
            .header("accept", "text/event-stream");
        if let Some(seq) = from {
            req = req.header("last-event-id", seq.to_string());
        }
        // A long read timeout: an idle stream is normal, not a failure, and
        // the server sends SSE keep-alive comments to hold it open.
        let resp = req
            .timeout(std::time::Duration::from_secs(86_400))
            .send()
            .context("failed to reach daemon")?;

        if resp.status().is_success() {
            return Ok(EventStream {
                body: Some(resp),
                refusal: None,
            });
        }

        let status = resp.status();
        let message = resp
            .json::<ApiEnvelope<serde_json::Value>>()
            .ok()
            .and_then(|e| e.error)
            .map(ApiError::into_message)
            .unwrap_or_else(|| format!("event stream refused with HTTP {status}"));
        Ok(EventStream {
            body: None,
            refusal: Some(message),
        })
    }

    pub fn end_input(&self, id: u32) -> Result<()> {
        let resp = self
            .authed(self.http.post(self.url(&format!("/sessions/{id}/eof"))))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid eof response")?;
        if envelope.ok {
            return Ok(());
        }
        Err(anyhow::anyhow!(
            "{}",
            envelope
                .error
                .map(ApiError::into_message)
                .unwrap_or_else(|| "end-of-input failed".to_string())
        ))
    }

    pub fn send_input(&self, id: u32, input: &str) -> Result<()> {
        let resp = self
            .authed(self.http.post(self.url(&format!("/sessions/{id}/send"))))
            .json(&serde_json::json!({ "input": input }))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid send response")?;
        // `send` returns `data: null` on success, so `into_data` -- which
        // treats a missing payload as a failure -- reported "response
        // contained no data" for a request that had in fact succeeded. Check
        // the `ok` flag and surface the server's message on failure instead.
        if envelope.ok {
            return Ok(());
        }
        Err(anyhow::anyhow!(
            "{}",
            envelope
                .error
                .map(ApiError::into_message)
                .unwrap_or_else(|| "send input failed".to_string())
        ))
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
        let empty = CreateSessionRequest::new(None, None, None, None);
        assert_eq!(serde_json::to_value(empty).unwrap(), serde_json::json!({}));

        let named = CreateSessionRequest::new(Some("build"), None, None, None);
        assert_eq!(
            serde_json::to_value(named).unwrap(),
            serde_json::json!({ "name": "build" })
        );

        let tier_only = CreateSessionRequest::new(None, Some("restricted"), None, None);
        assert_eq!(
            serde_json::to_value(tier_only).unwrap(),
            serde_json::json!({ "isolation": "restricted" })
        );

        let named_tier = CreateSessionRequest::new(Some("build"), Some("capped"), None, None);
        assert_eq!(
            serde_json::to_value(named_tier).unwrap(),
            serde_json::json!({ "name": "build", "isolation": "capped" })
        );

        let contained = CreateSessionRequest::new(
            Some("proof"),
            Some("contained"),
            Some("required"),
            Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        );
        assert_eq!(
            serde_json::to_value(contained).unwrap(),
            serde_json::json!({
                "name": "proof",
                "isolation": "contained",
                "isolation_policy": "required",
                "image": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            })
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
                    "isolation": {"effective":"bare","requested":"bare","basis":"none"},
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
        assert_eq!(sessions[0].isolation.effective, "bare");
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
