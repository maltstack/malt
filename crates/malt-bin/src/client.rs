use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionData {
    pub id: u32,
    pub name: Option<String>,
    pub pane_count: u32,
    pub isolation: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ExecResultData {
    pub command_id: u32,
    pub output: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct HealthData {
    pub status: String,
}

pub struct MaltClient {
    addr: String,
    http: Client,
}

impl MaltClient {
    pub fn new(addr: &str) -> Self {
        let addr = addr.trim_end_matches('/').to_owned();
        Self {
            addr,
            http: Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.addr, path)
    }

    pub fn health(&self) -> Result<HealthData> {
        let resp = self
            .http
            .get(self.url("/health"))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<HealthData> = resp.json().context("invalid health response")?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in health response"))
    }

    pub fn shutdown(&self) -> Result<()> {
        self.http
            .post(self.url("/shutdown"))
            .send()
            .context("failed to reach daemon")?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionData>> {
        let resp = self
            .http
            .get(self.url("/sessions"))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<Vec<SessionData>> =
            resp.json().context("invalid session list response")?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in session list response"))
    }

    pub fn create_session(&self, name: Option<&str>) -> Result<SessionData> {
        let mut req = self.http.post(self.url("/sessions"));
        if let Some(n) = name {
            req = req.json(&serde_json::json!({ "name": n }));
        } else {
            req = req.json(&serde_json::json!({}));
        }
        let resp = req.send().context("failed to reach daemon")?;
        let envelope: ApiEnvelope<SessionData> =
            resp.json().context("invalid create session response")?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in create session response"))
    }

    pub fn destroy_session(&self, id: u32) -> Result<()> {
        let resp = self
            .http
            .delete(self.url(&format!("/sessions/{id}")))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid destroy session response")?;
        if envelope.ok {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                envelope.error.unwrap_or_else(|| "unknown error".into())
            ))
        }
    }

    pub fn exec_command(&self, id: u32, cmd: &str) -> Result<ExecResultData> {
        let resp = self
            .http
            .post(self.url(&format!("/sessions/{id}/exec")))
            .json(&serde_json::json!({ "command": cmd }))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<ExecResultData> =
            resp.json().context("invalid exec response")?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in exec response"))
    }

    pub fn send_input(&self, id: u32, input: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/sessions/{id}/send")))
            .json(&serde_json::json!({ "input": input }))
            .send()
            .context("failed to reach daemon")?;
        let envelope: ApiEnvelope<serde_json::Value> =
            resp.json().context("invalid send response")?;
        if envelope.ok {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                envelope.error.unwrap_or_else(|| "unknown error".into())
            ))
        }
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
    }
}
