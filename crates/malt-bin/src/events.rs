//! Client-side consumption of the lifecycle event stream.
//!
//! Deliberately free of CLI assumptions — no printing, no `clap` types, no
//! `process::exit`. This module is the intended nucleus of a future
//! `malt-gateway-sdk`; extracting it should be a move, not a rewrite. See
//! `specs/004-command-lifecycle-events/research.md` R10.

use std::io::{BufRead, BufReader};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::client::MaltClient;

/// Longest gap between reconnection attempts.
const MAX_BACKOFF_MS: u64 = 5_000;
/// First reconnection delay; doubles up to `MAX_BACKOFF_MS`.
const BASE_BACKOFF_MS: u64 = 250;

/// One parsed frame from the stream.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamEvent {
    /// Resume position, from the SSE `id:` field.
    pub sequence: u64,
    /// Event type, from the SSE `event:` field.
    pub kind: String,
    /// Decoded payload.
    pub payload: EventPayload,
}

/// Payload fields, flattened as the Gateway sends them. Every field is
/// optional because which ones appear depends on the event type; a client
/// reads the ones its `kind` implies.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct EventPayload {
    pub command_id: Option<u32>,
    pub cmd: Option<String>,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub duration_us: Option<u64>,
    pub missed_from: Option<u64>,
    pub missed_through: Option<u64>,
    pub reason: Option<String>,
}

/// Accumulates SSE lines into frames.
///
/// SSE separates frames with a blank line; `id:`, `event:` and `data:` lines
/// accumulate until then. An unrecognized field is ignored, per the SSE
/// specification, so a future server addition cannot break an existing
/// client.
#[derive(Debug, Default)]
pub struct FrameParser {
    id: Option<u64>,
    event: Option<String>,
    data: String,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line. Returns a frame when the line completes one.
    ///
    /// A frame whose payload does not parse is skipped rather than aborting
    /// the stream: one malformed frame must not end a subscription that is
    /// otherwise healthy.
    pub fn push_line(&mut self, line: &str) -> Option<StreamEvent> {
        // `BufRead::read_line` keeps the terminator, so strip it here rather
        // than requiring every caller to. Getting this wrong is invisible to
        // a test that feeds pre-trimmed lines: `"id: 1\n"` parses as the
        // number `"1\n"` (which fails), and `"\n"` never matches the
        // end-of-frame check, so no frame is ever emitted.
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);

        if line.is_empty() {
            return self.finish_frame();
        }
        if line.starts_with(':') {
            // A comment — SSE keep-alives arrive this way.
            return None;
        }

        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };

        match field {
            "id" => self.id = value.parse::<u64>().ok(),
            "event" => self.event = Some(value.to_string()),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
        None
    }

    fn finish_frame(&mut self) -> Option<StreamEvent> {
        let id = self.id.take();
        let event = self.event.take();
        let data = std::mem::take(&mut self.data);

        // A blank line with nothing accumulated is just a separator.
        let (Some(sequence), Some(kind)) = (id, event) else {
            return None;
        };

        let payload = if data.is_empty() {
            EventPayload::default()
        } else {
            serde_json::from_str(&data).unwrap_or_default()
        };

        Some(StreamEvent {
            sequence,
            kind,
            payload,
        })
    }
}

/// Consume the event stream, invoking `on_event` for each frame.
///
/// Reconnects automatically on transport failure, resuming from the highest
/// sequence seen so far so a dropped connection continues rather than
/// restarting. Returns only when `on_event` asks to stop or the server
/// refuses the subscription outright (an unknown session, or a permission
/// failure — retrying those would loop forever).
pub fn watch_events<F>(
    client: &MaltClient,
    session_id: u32,
    resume_from: Option<u64>,
    mut on_event: F,
) -> Result<()>
where
    F: FnMut(&StreamEvent) -> ControlFlow,
{
    let mut last_seen = resume_from;
    #[allow(unused_assignments)]
    let mut backoff = BASE_BACKOFF_MS;

    loop {
        let response = client
            .open_event_stream(session_id, last_seen)
            .context("failed to open the event stream")?;

        // A refusal is terminal: reconnecting cannot make an unknown session
        // exist or a token gain scope.
        if let Some(message) = response.refusal {
            anyhow::bail!(message);
        }
        let Some(body) = response.body else {
            anyhow::bail!("event stream returned no body");
        };

        backoff = BASE_BACKOFF_MS;
        let mut parser = FrameParser::new();
        let mut reader = BufReader::new(body);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // server closed the stream
                Ok(_) => {
                    if let Some(event) = parser.push_line(&line) {
                        last_seen = Some(event.sequence);
                        if on_event(&event) == ControlFlow::Stop {
                            return Ok(());
                        }
                    }
                }
                Err(_) => break, // transport failure — reconnect below
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(backoff));
        backoff = (backoff * 2).min(MAX_BACKOFF_MS);
    }
}

/// Whether the caller wants more events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    Continue,
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed lines the way `BufRead::read_line` actually delivers them —
    /// terminator included. Feeding pre-trimmed lines here would let a
    /// newline-handling bug pass every test in this module while the real
    /// client received nothing, which is exactly what happened once.
    fn feed(parser: &mut FrameParser, lines: &[&str]) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        for line in lines {
            let with_terminator = format!("{line}\n");
            if let Some(event) = parser.push_line(&with_terminator) {
                out.push(event);
            }
        }
        out
    }

    /// Same, for a server that terminates lines with CRLF.
    fn feed_crlf(parser: &mut FrameParser, lines: &[&str]) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        for line in lines {
            let with_terminator = format!("{line}\r\n");
            if let Some(event) = parser.push_line(&with_terminator) {
                out.push(event);
            }
        }
        out
    }

    #[test]
    fn parses_a_well_formed_frame() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 12",
                "event: command_started",
                r#"data: {"command_id":4,"cmd":"cargo test","started_at":1784070000123}"#,
                "",
            ],
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.sequence, 12);
        assert_eq!(event.kind, "command_started");
        assert_eq!(event.payload.command_id, Some(4));
        assert_eq!(event.payload.cmd.as_deref(), Some("cargo test"));
        assert_eq!(event.payload.started_at, Some(1_784_070_000_123));
    }

    #[test]
    fn splits_consecutive_frames_on_blank_lines() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 1",
                "event: command_started",
                r#"data: {"command_id":1,"cmd":"echo hi"}"#,
                "",
                "id: 2",
                "event: command_finished",
                r#"data: {"command_id":1,"exit_code":0}"#,
                "",
            ],
        );

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[1].payload.exit_code, Some(0));
        assert_eq!(
            events[1].payload.cmd, None,
            "fields from the previous frame must not leak into the next"
        );
    }

    #[test]
    fn parses_a_gap_frame() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 41",
                "event: gap",
                r#"data: {"missed_from":14,"missed_through":40,"reason":"retention_exceeded"}"#,
                "",
            ],
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "gap");
        assert_eq!(events[0].payload.missed_from, Some(14));
        assert_eq!(events[0].payload.missed_through, Some(40));
        assert_eq!(
            events[0].payload.reason.as_deref(),
            Some("retention_exceeded")
        );
    }

    #[test]
    fn an_unknown_event_type_is_delivered_rather_than_aborting_the_stream() {
        // Forward compatibility: a server that gains a new event type must
        // not break existing clients.
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 5",
                "event: something_new",
                r#"data: {"command_id":9}"#,
                "",
                "id: 6",
                "event: command_finished",
                r#"data: {"command_id":9,"exit_code":0}"#,
                "",
            ],
        );

        assert_eq!(
            events.len(),
            2,
            "the stream must continue past an unknown type"
        );
        assert_eq!(events[0].kind, "something_new");
        assert_eq!(events[1].kind, "command_finished");
    }

    #[test]
    fn malformed_payload_yields_an_empty_payload_not_a_lost_stream() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 7",
                "event: command_started",
                "data: {not valid json",
                "",
                "id: 8",
                "event: command_finished",
                r#"data: {"command_id":1,"exit_code":0}"#,
                "",
            ],
        );

        assert_eq!(
            events.len(),
            2,
            "one bad frame must not end the subscription"
        );
        assert_eq!(events[0].payload, EventPayload::default());
        assert_eq!(events[1].payload.exit_code, Some(0));
    }

    #[test]
    fn keep_alive_comments_and_stray_blank_lines_are_ignored() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "",
                ": keep-alive",
                "",
                "id: 3",
                "event: command_started",
                r#"data: {"command_id":2}"#,
                "",
            ],
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 3);
    }

    #[test]
    fn multi_line_data_is_joined_with_newlines() {
        let mut parser = FrameParser::new();
        let events = feed(
            &mut parser,
            &[
                "id: 9",
                "event: command_started",
                "data: {",
                r#"data: "command_id": 3}"#,
                "",
            ],
        );
        // Not valid JSON split this way, but the parser must have joined the
        // lines rather than keeping only the last.
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 9);
    }

    #[test]
    fn crlf_line_endings_parse_identically() {
        let mut parser = FrameParser::new();
        let events = feed_crlf(
            &mut parser,
            &[
                "id: 15",
                "event: command_started",
                "data: {\"command_id\":7}",
                "",
            ],
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 15);
        assert_eq!(events[0].payload.command_id, Some(7));
    }

    #[test]
    fn a_bare_terminator_ends_a_frame() {
        // Regression: the end-of-frame check compared against "" while
        // read_line supplies "\n", so frames were never emitted at all.
        let mut parser = FrameParser::new();
        assert!(parser.push_line("id: 1\n").is_none());
        assert!(parser.push_line("event: command_started\n").is_none());
        assert!(parser.push_line("data: {}\n").is_none());
        let event = parser
            .push_line("\n")
            .expect("a bare newline must terminate the frame");
        assert_eq!(event.sequence, 1);
        assert_eq!(event.kind, "command_started");
    }

    #[test]
    fn an_id_with_a_terminator_still_parses_as_a_number() {
        // The other half of the same bug: "1\n".parse::<u64>() fails, which
        // silently discarded the resume position.
        let mut parser = FrameParser::new();
        parser.push_line("id: 42\n");
        parser.push_line("event: command_started\n");
        parser.push_line("data: {}\n");
        let event = parser.push_line("\n").expect("frame");
        assert_eq!(
            event.sequence, 42,
            "the sequence must survive the line terminator -- it is the resume token"
        );
    }
}
