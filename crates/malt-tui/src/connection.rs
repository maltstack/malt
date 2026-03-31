use malt_protocol::common::ResolvedStyle;
use malt_protocol::envelope::{encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::Hello;
use malt_protocol::render::RenderCommand;
use std::io::BufReader;
use std::net::TcpStream;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

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

/// Parse an RGB array `[r, g, b]` from JSON.
fn parse_rgb(val: Option<&serde_json::Value>) -> (u8, u8, u8) {
    val.and_then(|v| v.as_array())
        .and_then(|arr| {
            if arr.len() == 3 {
                Some((
                    arr[0].as_u64().unwrap_or(204) as u8,
                    arr[1].as_u64().unwrap_or(204) as u8,
                    arr[2].as_u64().unwrap_or(204) as u8,
                ))
            } else {
                None
            }
        })
        .unwrap_or((204, 204, 204))
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
        let data = json.get("data")?;

        // Check content hash to avoid re-rendering identical frames
        let raw = data.to_string();
        if raw == self.last_output {
            return None;
        }
        self.last_output = raw;

        let mut commands = vec![RenderCommand::Clear {}];

        // Try styled grid format first
        if let Some(rows) = data.get("rows").and_then(|r| r.as_array()) {
            for (y, row) in rows.iter().enumerate() {
                if let Some(spans) = row.as_array() {
                    let mut x: u16 = 0;
                    for span in spans {
                        let text = span.get("t").and_then(|t| t.as_str()).unwrap_or("");
                        if text.trim().is_empty() {
                            x += text.len() as u16;
                            continue;
                        }
                        let fg = parse_rgb(span.get("fg"));
                        let bg = parse_rgb(span.get("bg"));
                        let bold = span.get("b").and_then(|b| b.as_bool()).unwrap_or(false);

                        commands.push(RenderCommand::DrawText {
                            x,
                            y: y as u16,
                            text: text.to_string(),
                            style: ResolvedStyle {
                                fg,
                                bg,
                                bold,
                                italic: false,
                                underline: false,
                                dim: false,
                                strikethrough: false,
                                reverse: false,
                                blink: false,
                                _unknown: Vec::new(),
                            },
                        });
                        x += text.len() as u16;
                    }
                }
            }
        } else if let Some(text) = data.get("text").and_then(|t| t.as_str()) {
            // Fallback: plain text
            let default_style = ResolvedStyle {
                fg: (204, 204, 204),
                bg: (0, 0, 0),
                bold: false, italic: false, underline: false,
                dim: false, strikethrough: false, reverse: false, blink: false,
                _unknown: Vec::new(),
            };
            for (i, line) in text.lines().enumerate() {
                if !line.is_empty() {
                    commands.push(RenderCommand::DrawText {
                        x: 0,
                        y: i as u16,
                        text: line.to_string(),
                        style: default_style.clone(),
                    });
                }
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

/// VNP socket connection to the MALT daemon.
///
/// Uses VNP framing with JSON payloads for the simplified protocol.
/// Connects via TCP, performs VNP handshake, then exchanges JSON frames.
pub struct VnpConnection {
    reader: FrameReader<BufReader<TcpStream>>,
    writer: FrameWriter<TcpStream>,
    session_id: u32,
    last_output: String,
}

impl VnpConnection {
    /// Connect to the VNP listener, perform handshake, and attach to a session.
    pub fn connect(addr: &str, session_id: u32) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(addr)?;

        let write_stream = stream.try_clone()?;
        let read_stream = BufReader::new(stream);

        let mut frame_writer = FrameWriter::new(write_stream);
        let mut frame_reader = FrameReader::new(read_stream);

        // Send Hello
        let hello = Hello {
            version: 1,
            client_type: "malt-tui".to_string(),
            capabilities: malt_protocol::common::ClientCapabilities {
                color_depth: malt_protocol::common::ColorDepth::TrueColor,
                unicode: malt_protocol::common::UnicodeLevel::Full,
                image_protocol: malt_protocol::common::ImageProtocol::None,
                overlay: false,
                vt_passthrough: true,
                max_fps: 60,
                _unknown: Vec::new(),
            },
            _unknown: Vec::new(),
        };

        let hello_envelope = Envelope {
            wire_version: 0,
            domain: 0,
            msg_type: 0x01,
            session_id: 0,
            timestamp: 0,
            msg_id: None,
            _unknown: Vec::new(),
        };

        let mut w = BitWriter::new();
        hello.pack(&mut w)?;
        let hello_bytes = w.finish();
        let combined = encode_message(&hello_envelope, &hello_bytes)?;
        let frame = Frame {
            flags: FrameFlags::new(),
            payload: combined,
        };
        frame_writer.write_frame(&frame)?;

        // Read HelloAck
        let ack_frame = frame_reader.read_frame()?;
        let (envelope, _msg_bytes) =
            malt_protocol::envelope::decode_envelope(&ack_frame.payload)?;
        if envelope.msg_type != 0x02 {
            anyhow::bail!(
                "expected HelloAck (type=0x02), got type={:#x}",
                envelope.msg_type
            );
        }

        // Set non-blocking for poll-based reads
        // We need to get at the underlying TcpStream inside BufReader inside FrameReader.
        // Instead, we set a short read timeout on the write_stream clone's original.
        // Actually we set it on the read side via the original stream reference.
        // Since we moved the stream into BufReader, we need to use the write clone.
        // TcpStream clones share the same socket, so setting timeout on one affects both.
        // However we want the writer to be blocking. So use set_read_timeout instead.
        // Unfortunately we cannot access the inner stream of FrameReader<BufReader<TcpStream>>.
        // We'll create a fresh clone before moving into the reader.

        // Workaround: the write_stream is a clone of the same socket, so we can
        // set the read timeout via it (affects the same underlying socket).
        if let Err(e) = frame_writer.inner_ref().set_read_timeout(Some(
            std::time::Duration::from_millis(10),
        )) {
            // Non-fatal — will just block on reads
            let _ = e;
        }

        // Send attach message
        let attach = serde_json::json!({
            "type": "attach",
            "session": session_id,
        });
        let payload = attach.to_string().into_bytes();
        let mut flags = FrameFlags::new();
        flags.set_json_encoded(true);
        let frame = Frame { flags, payload };
        frame_writer.write_frame(&frame)?;

        Ok(Self {
            reader: frame_reader,
            writer: frame_writer,
            session_id,
            last_output: String::new(),
        })
    }
}

impl DaemonConnection for VnpConnection {
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>> {
        // Try to read a frame (non-blocking via read timeout)
        match self.reader.read_frame() {
            Ok(frame) => {
                let payload = String::from_utf8_lossy(&frame.payload);
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&payload) {
                    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if msg_type == "output" {
                        let raw = msg.to_string();
                        if raw == self.last_output {
                            return None;
                        }
                        self.last_output = raw;

                        let mut commands = vec![RenderCommand::Clear {}];
                        let default_style = ResolvedStyle {
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

                        if let Some(rows) = msg.get("rows").and_then(|r| r.as_array()) {
                            for (y, row) in rows.iter().enumerate() {
                                if let Some(spans) = row.as_array() {
                                    let mut x: u16 = 0;
                                    for span in spans {
                                        let text = span
                                            .get("t")
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("");
                                        if text.trim().is_empty() {
                                            x += text.len() as u16;
                                            continue;
                                        }
                                        let fg = parse_rgb(span.get("fg"));
                                        let bg = parse_rgb(span.get("bg"));
                                        let bold = span
                                            .get("b")
                                            .and_then(|b| b.as_bool())
                                            .unwrap_or(false);
                                        commands.push(RenderCommand::DrawText {
                                            x,
                                            y: y as u16,
                                            text: text.to_string(),
                                            style: ResolvedStyle {
                                                fg,
                                                bg,
                                                bold,
                                                italic: false,
                                                underline: false,
                                                dim: false,
                                                strikethrough: false,
                                                reverse: false,
                                                blink: false,
                                                _unknown: Vec::new(),
                                            },
                                        });
                                        x += text.len() as u16;
                                    }
                                }
                            }
                        } else if let Some(text) =
                            msg.get("text").and_then(|t| t.as_str())
                        {
                            for (i, line) in text.lines().enumerate() {
                                if !line.is_empty() {
                                    commands.push(RenderCommand::DrawText {
                                        x: 0,
                                        y: i as u16,
                                        text: line.to_string(),
                                        style: default_style.clone(),
                                    });
                                }
                            }
                        }

                        return Some(commands);
                    }
                }
                None
            }
            Err(malt_protocol::framing::FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                None
            }
            Err(malt_protocol::framing::FrameError::UnexpectedEof) => None,
            Err(_) => None,
        }
    }

    fn send_input(&mut self, input: &str) {
        let msg = serde_json::json!({
            "type": "input",
            "session": self.session_id,
            "data": input,
        });
        let payload = msg.to_string().into_bytes();
        let mut flags = FrameFlags::new();
        flags.set_json_encoded(true);
        let frame = Frame { flags, payload };
        let _ = self.writer.write_frame(&frame);
    }
}
