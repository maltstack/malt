use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const SERVER_NAME: &str = "malt-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn tools_schema() -> Value {
    json!([
        {
            "name": "list_sessions",
            "description": "List all terminal sessions",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "create_session",
            "description": "Create a new terminal session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Optional session name"}
                }
            }
        },
        {
            "name": "run_command",
            "description": "Run a shell command in a session and return output",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "integer", "description": "Session ID"},
                    "command": {"type": "string", "description": "Shell command to execute"}
                },
                "required": ["session_id", "command"]
            }
        },
        {
            "name": "get_output",
            "description": "Get current terminal output from a session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "integer", "description": "Session ID"}
                },
                "required": ["session_id"]
            }
        },
        {
            "name": "get_command_history",
            "description": "List a session's command execution history (command text, start/finish times, exit codes), oldest first. Entries with null finished_at/exit_code are not confirmed complete -- still running, or interrupted by a daemon stop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "integer", "description": "Session ID"}
                },
                "required": ["session_id"]
            }
        },
        {
            "name": "send_input",
            "description": "Send raw input to a session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "integer", "description": "Session ID"},
                    "input": {"type": "string", "description": "Raw input to send"}
                },
                "required": ["session_id", "input"]
            }
        },
        {
            "name": "destroy_session",
            "description": "Destroy a terminal session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "integer", "description": "Session ID"}
                },
                "required": ["session_id"]
            }
        }
    ])
}

/// Where the Gateway's default API token lives. Deliberately duplicated
/// from `malt_gateway::auth::dirs_token_path()` rather than depending on
/// `malt-gateway` -- `malt-mcp` is intentionally zero-internal-dependency
/// (ADR-0002), and a whole crate dependency for one path constant would
/// undermine that.
fn default_api_token_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("malt")
        .join("api-token")
}

fn read_default_api_token() -> Option<String> {
    std::fs::read_to_string(default_api_token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Attach the bearer token, if present, to an outgoing request.
fn authed(
    builder: reqwest::blocking::RequestBuilder,
    token: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    match token {
        Some(t) => builder.bearer_auth(t),
        None => builder,
    }
}

fn handle_request(
    request: &Value,
    client: &reqwest::blocking::Client,
    api_addr: &str,
    token: Option<&str>,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                }
            }
        }),

        "notifications/initialized" => {
            // Notification — no response required, but return null id for consistency
            return Value::Null;
        }

        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools_schema()
            }
        }),

        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match dispatch_tool(tool_name, &arguments, client, api_addr, token) {
                Ok(text) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": text}]
                    }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32000,
                        "message": e.to_string()
                    }
                }),
            }
        }

        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {method}")
            }
        }),
    }
}

/// Build the request body for `create_session`. Omits `name` entirely when
/// not provided, matching malt-bin's `CreateSessionRequest` (which uses
/// `skip_serializing_if`) -- defaulting to the literal string "default"
/// would create a real, named session that a CLI- or curl-created session
/// with no name never gets (shown as `-` in `malt list`).
fn create_session_body(arguments: &Value) -> Value {
    let mut body = json!({});
    if let Some(name) = arguments.get("name").and_then(|n| n.as_str()) {
        body["name"] = json!(name);
    }
    body
}

fn dispatch_tool(
    name: &str,
    arguments: &Value,
    client: &reqwest::blocking::Client,
    api_addr: &str,
    token: Option<&str>,
) -> anyhow::Result<String> {
    match name {
        "list_sessions" => {
            let resp = authed(client.get(format!("{api_addr}/sessions")), token).send()?;
            Ok(resp.text()?)
        }

        "create_session" => {
            let body = create_session_body(arguments);
            let resp = authed(client.post(format!("{api_addr}/sessions")), token)
                .json(&body)
                .send()?;
            Ok(resp.text()?)
        }

        "run_command" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let command = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing command"))?;
            let resp = authed(
                client.post(format!("{api_addr}/sessions/{session_id}/exec")),
                token,
            )
            .json(&json!({"command": command}))
            .send()?;
            Ok(resp.text()?)
        }

        "get_output" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            // Plain-text variant, not the StyledGrid route -- an agent has
            // no use for character-cell RGB/bold spans built for human
            // rendering clients.
            let resp = authed(
                client.get(format!("{api_addr}/sessions/{session_id}/output/text")),
                token,
            )
            .send()?;
            Ok(resp.text()?)
        }

        "get_command_history" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let resp = authed(
                client.get(format!("{api_addr}/sessions/{session_id}/history")),
                token,
            )
            .send()?;
            Ok(resp.text()?)
        }

        "send_input" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let input = arguments
                .get("input")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing input"))?;
            let resp = authed(
                client.post(format!("{api_addr}/sessions/{session_id}/send")),
                token,
            )
            .json(&json!({"input": input}))
            .send()?;
            Ok(resp.text()?)
        }

        "destroy_session" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let resp = authed(
                client.delete(format!("{api_addr}/sessions/{session_id}")),
                token,
            )
            .send()?;
            Ok(resp.text()?)
        }

        _ => Err(anyhow::anyhow!("unknown tool: {name}")),
    }
}

fn main() -> anyhow::Result<()> {
    let api_addr =
        std::env::var("MALT_API_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7700".to_string());
    let client = reqwest::blocking::Client::new();
    // Read once at startup, same as malt-bin -- the Gateway now enforces
    // real auth, so every tool call needs this to get anything but 401.
    let token = read_default_api_token();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = serde_json::from_str(&line)?;
        let response = handle_request(&request, &client, &api_addr, token.as_deref());

        // Notifications produce Value::Null — no response needed
        if response.is_null() {
            continue;
        }

        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_session_body_omits_name_when_absent() {
        let body = create_session_body(&json!({}));
        assert!(
            body.get("name").is_none(),
            "an omitted name argument must not become the literal string \
             \"default\" -- that diverges from malt-bin's own \
             CreateSessionRequest, which omits the field entirely; got: {body:?}"
        );
    }

    #[test]
    fn create_session_body_includes_name_when_present() {
        let body = create_session_body(&json!({"name": "my-session"}));
        assert_eq!(body["name"], "my-session");
    }

    #[test]
    fn handle_initialize() {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        assert_eq!(resp["result"]["serverInfo"]["name"], "malt-mcp");
    }

    #[test]
    fn handle_tools_list() {
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        let tools = resp["result"]["tools"]
            .as_array()
            .expect("tools should be an array");
        assert!(tools.len() >= 5);
    }

    #[test]
    fn get_command_history_tool_is_advertised_with_a_session_id_schema() {
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        let tools = resp["result"]["tools"]
            .as_array()
            .expect("tools should be an array");

        let tool = tools
            .iter()
            .find(|t| t["name"] == "get_command_history")
            .expect("get_command_history must be advertised to agents");

        assert_eq!(tool["inputSchema"]["properties"]["session_id"]["type"], "integer");
        assert_eq!(
            tool["inputSchema"]["required"],
            json!(["session_id"]),
            "session_id must be required -- there is no sensible default session"
        );
    }

    #[test]
    fn get_command_history_requires_a_session_id_argument() {
        // Dispatch must reject the call before it ever builds a request URL,
        // rather than silently querying some fallback session.
        let client = reqwest::blocking::Client::new();
        let err = dispatch_tool(
            "get_command_history",
            &json!({}),
            &client,
            "http://localhost:9999",
            None,
        )
        .expect_err("a missing session_id must be an error");
        assert!(
            err.to_string().contains("session_id"),
            "the error must name the missing argument, got: {err}"
        );
    }

    #[test]
    fn handle_unknown_method() {
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "unknown/method"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        assert!(resp.get("error").is_some());
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn handle_tools_call_unknown_tool() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "nonexistent", "arguments": {}}
        });
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        assert!(resp.get("error").is_some());
    }

    #[test]
    fn handle_notification_returns_null() {
        let req = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999", None);
        assert!(resp.is_null());
    }

    #[test]
    fn tools_schema_has_required_tools() {
        let tools = tools_schema();
        let arr = tools.as_array().expect("should be array");
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"list_sessions"));
        assert!(names.contains(&"create_session"));
        assert!(names.contains(&"run_command"));
        assert!(names.contains(&"get_output"));
        assert!(names.contains(&"send_input"));
        assert!(names.contains(&"destroy_session"));
    }
}
