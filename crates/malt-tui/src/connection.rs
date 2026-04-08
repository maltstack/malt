use malt_protocol::codec::{
    make_envelope, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION, MSG_ATTACH_SESSION,
    MSG_FRAME_ACK, MSG_HELLO, MSG_HELLO_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT, MSG_RENDER_BATCH,
    MSG_RESIZE,
};
use malt_protocol::common::{InputAuthority, ResolvedStyle, SessionId};
use malt_protocol::envelope::{decode_envelope, encode_message};
use malt_protocol::framing::{Frame, FrameError, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::Hello;
use malt_protocol::input::{KeyEvent, Resize};
use malt_protocol::render::{FrameAck, InitialState, RenderBatch, RenderCommand};
use malt_protocol::session::AttachSession;
use std::io::BufReader;
use std::net::TcpStream;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

/// Trait for a live connection to the MALT daemon.
///
/// The connection is non-blocking: `poll_commands` returns `None` immediately
/// when no new data is available.
pub trait DaemonConnection {
    /// Get the next batch of render commands (non-blocking).
    /// Returns `None` if no new commands are available.
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>>;

    /// Send a typed keyboard input event to the daemon.
    fn send_key_event(&mut self, event: &KeyEvent);

    /// Notify the daemon that the client terminal was resized.
    fn send_resize(&mut self, cols: u16, rows: u16);
}

// ── MockConnection ────────────────────────────────────────────────────────────

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

    fn send_key_event(&mut self, _event: &KeyEvent) {}

    fn send_resize(&mut self, _cols: u16, _rows: u16) {}
}

// ── parse_rgb (shared utility for HTTP JSON parsing) ──────────────────────────

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

/// Convert a VNP KeyEvent to a text string for the HTTP send endpoint.
///
/// Named keys are mapped to their standard terminal byte sequences.
/// Returns `None` for events that have no meaningful text representation.
fn key_event_to_text(event: &KeyEvent) -> Option<String> {
    use malt_protocol::input::{KeyValue, NamedKey};

    let ctrl = event.modifiers.contains(malt_protocol::common::KeyModifiers::CTRL);
    match &event.key {
        KeyValue::Char { codepoint } => {
            let c = char::from_u32(*codepoint)?;
            if ctrl {
                // Ctrl+letter: map to control character (0x01–0x1a).
                let byte = (*codepoint as u8).wrapping_sub(b'a').wrapping_add(1);
                Some(String::from(byte as char))
            } else {
                Some(String::from(c))
            }
        }
        KeyValue::Named { key } => {
            let seq = match key {
                NamedKey::Enter => "\r",
                NamedKey::Escape => "\x1b",
                NamedKey::Tab => "\t",
                NamedKey::Backspace => "\x08",
                NamedKey::Delete => "\x1b[3~",
                NamedKey::Insert => "\x1b[2~",
                NamedKey::Home => "\x1b[H",
                NamedKey::End => "\x1b[F",
                NamedKey::PageUp => "\x1b[5~",
                NamedKey::PageDown => "\x1b[6~",
                NamedKey::Up => "\x1b[A",
                NamedKey::Down => "\x1b[B",
                NamedKey::Right => "\x1b[C",
                NamedKey::Left => "\x1b[D",
                _ => return None,
            };
            Some(seq.to_string())
        }
        KeyValue::Function { number } => {
            // F1–F4 are \x1bOP–\x1bOS; F5+ vary — emit the most common xterm form.
            let seq = match number {
                1 => "\x1bOP",
                2 => "\x1bOQ",
                3 => "\x1bOR",
                4 => "\x1bOS",
                5 => "\x1b[15~",
                6 => "\x1b[17~",
                7 => "\x1b[18~",
                8 => "\x1b[19~",
                9 => "\x1b[20~",
                10 => "\x1b[21~",
                11 => "\x1b[23~",
                12 => "\x1b[24~",
                _ => return None,
            };
            Some(seq.to_string())
        }
        _ => None,
    }
}

// ── HttpConnection ────────────────────────────────────────────────────────────

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

        // Check content hash to avoid re-rendering identical frames.
        let raw = data.to_string();
        if raw == self.last_output {
            return None;
        }
        self.last_output = raw;

        let mut commands = vec![RenderCommand::Clear {}];

        // Try styled grid format first.
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
                                token_name: None,
                                _unknown: Vec::new(),
                            },
                        });
                        x += text.len() as u16;
                    }
                }
            }
        } else if let Some(text) = data.get("text").and_then(|t| t.as_str()) {
            // Fallback: plain text.
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
                token_name: None,
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

    fn send_key_event(&mut self, event: &KeyEvent) {
        if let Some(text) = key_event_to_text(event) {
            let url = format!("{}/sessions/{}/send", self.base_url, self.session_id);
            if let Err(e) = self
                .http
                .post(&url)
                .json(&serde_json::json!({ "input": text }))
                .send()
            {
                tracing::warn!("HTTP send_key_event failed: {e}");
            }
        }
    }

    fn send_resize(&mut self, _cols: u16, _rows: u16) {
        // HTTP API does not yet have a resize endpoint.
    }
}

// ── VnpConnection ─────────────────────────────────────────────────────────────

/// VNP socket connection to the MALT daemon.
///
/// Uses typed VNP bitpack encoding for all messages after the handshake.
/// Connects via TCP, performs the VNP Hello/HelloAck handshake, sends
/// AttachSession, and then streams RenderBatch/InitialState frames.
pub struct VnpConnection {
    reader: FrameReader<BufReader<TcpStream>>,
    writer: FrameWriter<TcpStream>,
    session_id: u32,
    /// Commands buffered from the InitialState received during connect.
    pending_commands: Option<Vec<RenderCommand>>,
}

impl VnpConnection {
    /// Connect to the VNP listener, perform handshake, and attach to a session.
    ///
    /// Steps:
    /// 1. Send Hello (domain=0, type=0x01) as bitpack frame.
    /// 2. Read HelloAck (domain=0, type=0x02).
    /// 3. Set 10 ms read timeout (shared socket — affects the reader).
    /// 4. Send AttachSession (domain=4, type=0x02) as bitpack frame.
    /// 5. Read InitialState (domain=6, type=0x03) as bitpack frame.
    /// 6. Send FrameAck for the initial state.
    /// 7. Store initial.commands as pending_commands.
    pub fn connect(addr: &str, session_id: u32) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(addr)?;

        let write_stream = stream.try_clone()?;
        let read_stream = BufReader::new(stream);

        let mut frame_writer = FrameWriter::new(write_stream);
        let mut frame_reader = FrameReader::new(read_stream);

        // ── Step 1: Send Hello ────────────────────────────────────────────────
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

        let hello_env = make_envelope(DOMAIN_HANDSHAKE_U8, MSG_HELLO, 0);
        let mut w = BitWriter::new();
        hello.pack(&mut w)?;
        let hello_bytes = w.finish();
        let combined = encode_message(&hello_env, &hello_bytes)?;
        frame_writer.write_frame(&Frame {
            flags: FrameFlags::new(),
            payload: combined,
        })?;

        // ── Step 2: Read HelloAck ─────────────────────────────────────────────
        let ack_frame = frame_reader.read_frame()?;
        let (ack_env, _) = decode_envelope(&ack_frame.payload)?;
        if ack_env.msg_type != MSG_HELLO_ACK {
            anyhow::bail!(
                "expected HelloAck (type={:#x}), got type={:#x}",
                MSG_HELLO_ACK,
                ack_env.msg_type
            );
        }

        // ── Step 3: Set 10 ms read timeout ───────────────────────────────────
        // TcpStream clones share the same underlying socket. Setting the read
        // timeout via the write-side clone affects both directions.
        if let Err(e) = frame_writer
            .inner_ref()
            .set_read_timeout(Some(std::time::Duration::from_millis(10)))
        {
            // Non-fatal — poll_commands will block briefly on reads instead.
            tracing::warn!("could not set read timeout on VNP socket: {e}");
        }

        // ── Step 4: Send AttachSession ────────────────────────────────────────
        let attach = AttachSession {
            session_id: SessionId(session_id),
            authority: InputAuthority::Exclusive,
            _unknown: Vec::new(),
        };
        let attach_env = make_envelope(DOMAIN_SESSION, MSG_ATTACH_SESSION, session_id);
        let mut w = BitWriter::new();
        attach.pack(&mut w)?;
        let attach_bytes = w.finish();
        let combined = encode_message(&attach_env, &attach_bytes)?;
        frame_writer.write_frame(&Frame {
            flags: FrameFlags::new(),
            payload: combined,
        })?;

        // ── Step 5: Read InitialState ─────────────────────────────────────────
        let initial_frame = frame_reader.read_frame()?;
        let (initial_env, msg_bytes) = decode_envelope(&initial_frame.payload)?;
        if initial_env.domain != DOMAIN_RENDER || initial_env.msg_type != MSG_INITIAL_STATE {
            anyhow::bail!(
                "expected InitialState (domain={}, type={:#x}), got domain={}, type={:#x}",
                DOMAIN_RENDER,
                MSG_INITIAL_STATE,
                initial_env.domain,
                initial_env.msg_type,
            );
        }
        let mut r = BitReader::new(msg_bytes);
        let initial = InitialState::unpack(&mut r)?;

        let pending = initial.commands;
        let initial_seq = initial.frame_seq;

        // ── Step 6: Send FrameAck ─────────────────────────────────────────────
        let mut conn = Self {
            reader: frame_reader,
            writer: frame_writer,
            session_id,
            pending_commands: None,
        };
        conn.send_frame_ack(initial_seq);

        // ── Step 7: Store pending commands ────────────────────────────────────
        if !pending.is_empty() {
            conn.pending_commands = Some(pending);
        }

        Ok(conn)
    }

    /// Encode and send a FrameAck for the given sequence number.
    fn send_frame_ack(&mut self, frame_seq: u64) {
        let ack = FrameAck {
            frame_seq,
            _unknown: Vec::new(),
        };
        let env = make_envelope(DOMAIN_RENDER, MSG_FRAME_ACK, self.session_id);
        let mut w = BitWriter::new();
        if ack.pack(&mut w).is_err() {
            return;
        }
        let payload = w.finish();
        match encode_message(&env, &payload) {
            Ok(combined) => {
                let frame = Frame {
                    flags: FrameFlags::new(),
                    payload: combined,
                };
                if let Err(e) = self.writer.write_frame(&frame) {
                    tracing::warn!("failed to write FrameAck frame: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("failed to encode FrameAck: {e}");
            }
        }
    }
}

/// The DOMAIN_HANDSHAKE constant is 0, matching codec::DOMAIN_HANDSHAKE.
/// Aliased here to keep the connect() body readable without a `as u8` cast.
const DOMAIN_HANDSHAKE_U8: u8 = malt_protocol::codec::DOMAIN_HANDSHAKE;

impl DaemonConnection for VnpConnection {
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>> {
        // Drain any commands buffered during connect() first.
        if let Some(cmds) = self.pending_commands.take() {
            return Some(cmds);
        }

        match self.reader.read_frame() {
            Ok(frame) => {
                let (env, msg_bytes) = match decode_envelope(&frame.payload) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("VNP decode_envelope failed: {e}");
                        return None;
                    }
                };
                match (env.domain, env.msg_type) {
                    (d, t) if d == DOMAIN_RENDER && t == MSG_RENDER_BATCH => {
                        let mut r = BitReader::new(msg_bytes);
                        let batch = match RenderBatch::unpack(&mut r) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("VNP RenderBatch unpack failed: {e}");
                                return None;
                            }
                        };
                        self.send_frame_ack(batch.frame_seq);
                        Some(batch.commands)
                    }
                    (d, t) if d == DOMAIN_RENDER && t == MSG_INITIAL_STATE => {
                        let mut r = BitReader::new(msg_bytes);
                        let initial = match InitialState::unpack(&mut r) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("VNP InitialState unpack failed: {e}");
                                return None;
                            }
                        };
                        self.send_frame_ack(initial.frame_seq);
                        Some(initial.commands)
                    }
                    _ => None,
                }
            }
            Err(FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                None
            }
            Err(FrameError::UnexpectedEof) => {
                tracing::warn!("VNP connection closed by daemon (EOF)");
                None
            }
            Err(e) => {
                tracing::warn!("VNP frame read error: {e}");
                None
            }
        }
    }

    fn send_key_event(&mut self, event: &KeyEvent) {
        let env = make_envelope(DOMAIN_INPUT, MSG_KEY_EVENT, self.session_id);
        let mut w = BitWriter::new();
        if event.pack(&mut w).is_err() {
            return;
        }
        let payload = w.finish();
        match encode_message(&env, &payload) {
            Ok(combined) => {
                let frame = Frame {
                    flags: FrameFlags::new(),
                    payload: combined,
                };
                if let Err(e) = self.writer.write_frame(&frame) {
                    tracing::warn!("failed to write KeyEvent frame: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("failed to encode KeyEvent: {e}");
            }
        }
    }

    fn send_resize(&mut self, cols: u16, rows: u16) {
        let resize = Resize {
            cols,
            rows,
            _unknown: Vec::new(),
        };
        let env = make_envelope(DOMAIN_INPUT, MSG_RESIZE, self.session_id);
        let mut w = BitWriter::new();
        if resize.pack(&mut w).is_err() {
            return;
        }
        let payload = w.finish();
        match encode_message(&env, &payload) {
            Ok(combined) => {
                let frame = Frame {
                    flags: FrameFlags::new(),
                    payload: combined,
                };
                if let Err(e) = self.writer.write_frame(&frame) {
                    tracing::warn!("failed to write Resize frame: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("failed to encode Resize: {e}");
            }
        }
    }
}
