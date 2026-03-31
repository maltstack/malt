use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;

/// Trait for receiving render commands from the daemon.
pub trait DaemonConnection {
    /// Get the next batch of render commands (non-blocking).
    /// Returns `None` if no new commands are available.
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>>;

    /// Send raw input text to the session.
    fn send_input(&mut self, input: &str);
}

/// Mock connection that returns a static set of commands once.
pub struct MockConnection {
    commands: Option<Vec<RenderCommand>>,
}

impl MockConnection {
    pub fn new(commands: Vec<RenderCommand>) -> Self {
        Self {
            commands: Some(commands),
        }
    }

    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self { commands: None }
    }
}

impl DaemonConnection for MockConnection {
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>> {
        self.commands.take()
    }

    fn send_input(&mut self, _input: &str) {}
}

/// Live HTTP connection to the MALT daemon.
///
/// Polls `/sessions/:id/output` for terminal content and sends
/// keystrokes via `/sessions/:id/send`.
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
}

impl DaemonConnection for HttpConnection {
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>> {
        let url = format!(
            "{}/sessions/{}/output",
            self.base_url, self.session_id
        );
        let resp = self.http.get(&url).send().ok()?;
        let json: serde_json::Value = resp.json().ok()?;

        let text = json
            .get("data")
            .and_then(|d| d.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Only update if content changed
        if text == self.last_output {
            return None;
        }
        self.last_output = text.to_string();

        // Convert text lines to DrawText commands
        let style = ResolvedStyle {
            fg: (204, 204, 204),
            bg: (0, 0, 0),
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
            blink: false,
            _unknown: Vec::new(),
        };

        let mut commands = vec![RenderCommand::Clear {}];
        for (i, line) in text.lines().enumerate() {
            if !line.is_empty() {
                commands.push(RenderCommand::DrawText {
                    x: 0,
                    y: i as u16,
                    text: line.to_string(),
                    style: style.clone(),
                });
            }
        }
        Some(commands)
    }

    fn send_input(&mut self, input: &str) {
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
