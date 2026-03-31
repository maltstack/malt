use crate::app::StyledSpan;

/// HTTP connection to the MALT daemon.
///
/// Polls `/sessions/:id/output` for terminal content and sends
/// keystrokes via `/sessions/:id/send`. Same protocol as malt-tui.
pub struct HttpConnection {
    base_url: String,
    session_id: u32,
    http: reqwest::blocking::Client,
    last_output: String,
}

impl HttpConnection {
    pub fn new(api_addr: &str, session_id: u32) -> Self {
        Self {
            base_url: api_addr.trim_end_matches('/').to_string(),
            session_id,
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
            last_output: String::new(),
        }
    }

    /// Poll the daemon for styled terminal output.
    ///
    /// Returns `None` if no new content is available or the daemon is unreachable.
    pub fn poll_styled(&mut self) -> Option<Vec<Vec<StyledSpan>>> {
        let url = format!(
            "{}/sessions/{}/output",
            self.base_url, self.session_id
        );
        let resp = self.http.get(&url).send().ok()?;
        let json: serde_json::Value = resp.json().ok()?;
        let data = json.get("data")?;

        // Check content hash to avoid re-rendering identical frames
        let raw = data.to_string();
        if raw == self.last_output {
            return None;
        }
        self.last_output = raw;

        let mut lines = Vec::new();

        if let Some(rows) = data.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                let mut spans = Vec::new();
                if let Some(arr) = row.as_array() {
                    for span in arr {
                        let text = span
                            .get("t")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        let fg = parse_rgb(span.get("fg"));
                        let bg = parse_rgb(span.get("bg"));
                        let bold = span
                            .get("b")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(false);
                        spans.push(StyledSpan { text, fg, bg, bold });
                    }
                }
                lines.push(spans);
            }
        } else if let Some(text) = data.get("text").and_then(|t| t.as_str()) {
            // Fallback: plain text
            for line in text.lines() {
                lines.push(vec![StyledSpan {
                    text: line.to_string(),
                    fg: [204, 204, 204],
                    bg: [0, 0, 0],
                    bold: false,
                }]);
            }
        }

        Some(lines)
    }

    /// Send raw input text to the daemon session.
    pub fn send_input(&mut self, input: &str) {
        let url = format!(
            "{}/sessions/{}/send",
            self.base_url, self.session_id
        );
        let _ = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "input": input }))
            .send();
    }
}

fn parse_rgb(val: Option<&serde_json::Value>) -> [u8; 3] {
    val.and_then(|v| v.as_array())
        .and_then(|arr| {
            if arr.len() == 3 {
                Some([
                    arr[0].as_u64().unwrap_or(204) as u8,
                    arr[1].as_u64().unwrap_or(204) as u8,
                    arr[2].as_u64().unwrap_or(204) as u8,
                ])
            } else {
                None
            }
        })
        .unwrap_or([204, 204, 204])
}
