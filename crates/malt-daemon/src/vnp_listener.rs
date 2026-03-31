//! VNP socket listener — accepts TCP connections from TUI/terminal clients.
//!
//! Each connected client runs in its own thread. After VNP handshake,
//! the client exchanges simplified JSON-in-frames messages with the daemon.

use crate::connection::handshake::perform_server_handshake;
use crate::executor::coordinator::Coordinator;
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{info, warn};

/// Start the VNP listener on the given address (e.g. `"127.0.0.1:7701"`).
///
/// This function blocks the calling thread. It accepts connections in a loop
/// and spawns a handler thread per client.
pub fn start_vnp_listener(addr: &str, coordinator: Arc<Mutex<Coordinator>>) {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            warn!(addr, error = %e, "VNP listener failed to bind");
            return;
        }
    };
    info!(addr, "VNP listener started");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let coord = coordinator.clone();
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                info!(peer = %peer, "VNP client connected");
                thread::Builder::new()
                    .name(format!("vnp-client-{peer}"))
                    .spawn(move || {
                        handle_client(stream, coord);
                    })
                    .ok();
            }
            Err(e) => {
                warn!(error = %e, "VNP accept error");
            }
        }
    }
}

/// Handle a single VNP client connection.
///
/// 1. Perform VNP handshake (Hello / HelloAck).
/// 2. Enter message loop: read JSON commands from client, poll output, send back.
fn handle_client(stream: TcpStream, coordinator: Arc<Mutex<Coordinator>>) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Split stream for reading/writing
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            warn!(peer = %peer, error = %e, "failed to clone stream");
            return;
        }
    };

    let mut read_stream = BufReader::new(stream);

    // Handshake
    let sessions = coordinator
        .lock()
        .map(|c| c.list_sessions())
        .unwrap_or_default();

    let handshake = perform_server_handshake(&mut read_stream, &mut &write_stream, &sessions);
    match &handshake {
        Ok(result) => {
            info!(
                peer = %peer,
                client_type = %result.client_type,
                version = result.negotiated_version,
                "VNP handshake completed"
            );
        }
        Err(e) => {
            warn!(peer = %peer, error = %e, "VNP handshake failed");
            return;
        }
    }

    // Message loop
    let mut frame_reader = FrameReader::new(read_stream);
    let mut frame_writer = FrameWriter::new(write_stream);
    let mut attached_session: Option<u32> = None;
    let mut last_output = String::new();

    // Set a read timeout so we can poll output periodically
    // The underlying stream is inside BufReader inside FrameReader,
    // so we need to set it before entering the loop.
    // We'll use a blocking approach with a short timeout instead.

    loop {
        // Try to read a frame from the client (with timeout via non-blocking polling)
        match frame_reader.read_frame() {
            Ok(frame) => {
                let payload = String::from_utf8_lossy(&frame.payload);
                match serde_json::from_str::<serde_json::Value>(&payload) {
                    Ok(msg) => {
                        let msg_type = msg
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        match msg_type {
                            "attach" => {
                                let sid = msg
                                    .get("session")
                                    .and_then(|s| s.as_u64())
                                    .map(|s| s as u32);
                                attached_session = sid;
                                info!(peer = %peer, session = ?sid, "client attached");

                                // Send immediate output snapshot
                                if let Some(sid) = attached_session {
                                    send_output_frame(
                                        sid,
                                        &coordinator,
                                        &mut frame_writer,
                                        &mut last_output,
                                    );
                                }
                            }
                            "input" => {
                                let sid = msg
                                    .get("session")
                                    .and_then(|s| s.as_u64())
                                    .map(|s| s as u32)
                                    .or(attached_session);
                                let data = msg
                                    .get("data")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("");
                                if let Some(sid) = sid {
                                    if let Ok(mut coord) = coordinator.lock() {
                                        let _ = coord.write_to_session(sid, data.as_bytes());
                                    }
                                }
                            }
                            "ping" => {
                                // Respond with pong
                                let pong = serde_json::json!({"type": "pong"});
                                let payload = pong.to_string().into_bytes();
                                let mut flags = FrameFlags::new();
                                flags.set_json_encoded(true);
                                let frame = Frame { flags, payload };
                                if frame_writer.write_frame(&frame).is_err() {
                                    break;
                                }
                            }
                            _ => {
                                warn!(peer = %peer, msg_type, "unknown message type");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(peer = %peer, error = %e, "invalid JSON in frame");
                    }
                }

                // After processing input, send any pending output
                if let Some(sid) = attached_session {
                    send_output_frame(sid, &coordinator, &mut frame_writer, &mut last_output);
                }
            }
            Err(malt_protocol::framing::FrameError::UnexpectedEof) => {
                info!(peer = %peer, "VNP client disconnected");
                break;
            }
            Err(malt_protocol::framing::FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Timeout — just poll output
                if let Some(sid) = attached_session {
                    send_output_frame(sid, &coordinator, &mut frame_writer, &mut last_output);
                }
                continue;
            }
            Err(e) => {
                warn!(peer = %peer, error = %e, "VNP frame read error");
                break;
            }
        }
    }
}

/// Poll session output and send to the client if it changed.
fn send_output_frame<W: std::io::Write>(
    session_id: u32,
    coordinator: &Arc<Mutex<Coordinator>>,
    writer: &mut FrameWriter<W>,
    last_output: &mut String,
) {
    let output = coordinator
        .lock()
        .ok()
        .and_then(|c| c.get_session_output(session_id).ok())
        .unwrap_or_default();

    if output == *last_output {
        return;
    }
    *last_output = output.clone();

    let msg = serde_json::json!({
        "type": "output",
        "session": session_id,
        "text": output,
    });
    let payload = msg.to_string().into_bytes();
    let mut flags = FrameFlags::new();
    flags.set_json_encoded(true);
    let frame = Frame { flags, payload };
    let _ = writer.write_frame(&frame);
}
