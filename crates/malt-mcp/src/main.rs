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

fn handle_request(
    request: &Value,
    client: &reqwest::blocking::Client,
    api_addr: &str,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("");

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
            let tool_name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match dispatch_tool(tool_name, &arguments, client, api_addr) {
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

fn dispatch_tool(
    name: &str,
    arguments: &Value,
    client: &reqwest::blocking::Client,
    api_addr: &str,
) -> anyhow::Result<String> {
    match name {
        "list_sessions" => {
            let resp = client.get(format!("{api_addr}/sessions")).send()?;
            Ok(resp.text()?)
        }

        "create_session" => {
            let body = json!({
                "name": arguments.get("name").and_then(|n| n.as_str()).unwrap_or("default")
            });
            let resp = client
                .post(format!("{api_addr}/sessions"))
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
            let resp = client
                .post(format!("{api_addr}/sessions/{session_id}/exec"))
                .json(&json!({"command": command}))
                .send()?;
            Ok(resp.text()?)
        }

        "get_output" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let resp = client
                .get(format!("{api_addr}/sessions/{session_id}/output"))
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
            let resp = client
                .post(format!("{api_addr}/sessions/{session_id}/send"))
                .json(&json!({"input": input}))
                .send()?;
            Ok(resp.text()?)
        }

        "destroy_session" => {
            let session_id = arguments
                .get("session_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
            let resp = client
                .delete(format!("{api_addr}/sessions/{session_id}"))
                .send()?;
            Ok(resp.text()?)
        }

        _ => Err(anyhow::anyhow!("unknown tool: {name}")),
    }
}

fn main() -> anyhow::Result<()> {
    let api_addr = std::env::var("MALT_API_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:7700".to_string());
    let client = reqwest::blocking::Client::new();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = serde_json::from_str(&line)?;
        let response = handle_request(&request, &client, &api_addr);

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
    fn handle_initialize() {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999");
        assert_eq!(resp["result"]["serverInfo"]["name"], "malt-mcp");
    }

    #[test]
    fn handle_tools_list() {
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999");
        let tools = resp["result"]["tools"].as_array().expect("tools should be an array");
        assert!(tools.len() >= 5);
    }

    #[test]
    fn handle_unknown_method() {
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "unknown/method"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999");
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
        let resp = handle_request(&req, &client, "http://localhost:9999");
        assert!(resp.get("error").is_some());
    }

    #[test]
    fn handle_notification_returns_null() {
        let req = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let client = reqwest::blocking::Client::new();
        let resp = handle_request(&req, &client, "http://localhost:9999");
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
