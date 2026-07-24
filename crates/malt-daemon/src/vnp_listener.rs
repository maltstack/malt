//! VNP socket listener — accepts TCP connections from TUI/terminal clients.
//!
//! After the VNP handshake, all communication uses typed VNP envelopes with
//! bitpack-encoded payloads. No JSON is used post-handshake.

use crate::connection::handshake::perform_server_handshake;
use crate::executor::coordinator::Coordinator;
use crate::executor::session_thread::SessionCommand;
use malt_protocol::codec::{
    make_envelope, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION, MSG_ATTACH_SESSION,
    MSG_DETACH_SESSION, MSG_FRAME_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT, MSG_RENDER_BATCH,
    MSG_RESIZE,
};
use malt_protocol::common::SessionId;
use malt_protocol::envelope::{decode_envelope, encode_message};
use malt_protocol::framing::{Frame, FrameError, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::input::{KeyEvent, Resize};
use malt_protocol::render::{FrameAck, RenderBatch};
use malt_protocol::session::{AttachSession, DetachSession};
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{info, warn};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

/// Start the VNP listener on the given address (e.g. `"127.0.0.1:7701"`).
///
/// This function blocks the calling thread. It accepts connections in a loop
/// and spawns a handler thread per client. A shared `AtomicU64` counter
/// assigns monotonically increasing client IDs.
pub fn start_vnp_listener(
    addr: &str,
    coordinator: Arc<Mutex<Coordinator>>,
    client_counter: Arc<AtomicU64>,
) {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            warn!(addr, error = %e, "VNP listener failed to bind");
            return;
        }
    };
    info!(addr, "VNP listener started");
    accept_vnp_connections(listener, coordinator, client_counter);
}

/// Accept loop — separated from `start_vnp_listener` so tests can provide a
/// pre-bound `TcpListener` with a random port.
pub fn accept_vnp_connections(
    listener: TcpListener,
    coordinator: Arc<Mutex<Coordinator>>,
    client_counter: Arc<AtomicU64>,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let client_id = client_counter.fetch_add(1, Ordering::Relaxed);
                let coord = coordinator.clone();
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                info!(peer = %peer, client_id, "VNP client connected");
                if let Err(e) = thread::Builder::new()
                    .name(format!("vnp-client-{client_id}"))
                    .spawn(move || {
                        handle_client(stream, coord, client_id);
                    })
                {
                    warn!(peer = %peer, error = %e, "failed to spawn VNP client handler thread");
                }
            }
            Err(e) => {
                warn!(error = %e, "VNP accept error");
            }
        }
    }
}

/// Handle a single VNP client connection.
///
/// 1. Clone stream for the write side; wrap read side in a `BufReader`.
/// 2. Perform VNP handshake — obtain `HandshakeResult` with capabilities.
/// 3. Set 30 s read timeout for the attach phase.
/// 4. Wait for an `AttachSession` message (domain=4, type=0x02).
/// 5. Register with the coordinator to get a `render_rx` and `InitialState`.
/// 6. Send `InitialState` frame to the client (domain=6, type=0x03).
/// 7. Switch to 16 ms read timeout for the main loop.
/// 8. Loop: drain `render_rx`, encode `RenderBatch` frames; read TCP frames,
///    dispatch `KeyEvent` / `Resize` / `FrameAck`.
/// 9. On disconnect: `unregister_vnp_client`.
fn handle_client(stream: TcpStream, coordinator: Arc<Mutex<Coordinator>>, client_id: u64) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Clone stream for the write side.
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            warn!(peer = %peer, error = %e, "VNP: failed to clone stream");
            return;
        }
    };

    let mut read_stream = BufReader::new(stream);

    // Collect the current session list for HelloAck.
    let sessions = coordinator
        .lock()
        .map(|c| c.list_sessions())
        .unwrap_or_default();

    // VNP handshake.
    let handshake = match perform_server_handshake(&mut read_stream, &mut &write_stream, &sessions)
    {
        Ok(r) => r,
        Err(e) => {
            warn!(peer = %peer, error = %e, "VNP handshake failed");
            return;
        }
    };

    info!(
        peer = %peer,
        client_id,
        client_type = %handshake.client_type,
        version = handshake.negotiated_version,
        "VNP handshake completed"
    );

    // Set 30 s read timeout for the attach phase.
    if let Err(e) = read_stream
        .get_ref()
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
    {
        warn!(peer = %peer, error = %e, "VNP: failed to set attach timeout");
    }

    let mut frame_reader = FrameReader::new(read_stream);
    let mut frame_writer = FrameWriter::new(write_stream);

    // Wait for AttachSession.
    let session_id_raw = match wait_for_attach(&mut frame_reader, &peer) {
        Some(id) => id,
        None => {
            warn!(peer = %peer, client_id, "VNP: no AttachSession received; disconnecting");
            return;
        }
    };

    let session_id = SessionId(session_id_raw);
    info!(peer = %peer, client_id, session_id = session_id_raw, "VNP client attached");

    // Create a bounded sync_channel for render batches.
    let (render_tx, render_rx) = std::sync::mpsc::sync_channel::<RenderBatch>(4);

    // Register with the coordinator; obtain the InitialState snapshot.
    let initial_reply = match coordinator.lock() {
        Ok(mut coord) => {
            match coord.begin_register_vnp_client(
                session_id.clone(),
                client_id,
                handshake.capabilities,
                render_tx,
            ) {
                Ok(reply) => reply,
                Err(e) => {
                    warn!(peer = %peer, error = %e, session_id = session_id_raw,
                          "VNP: register_vnp_client failed");
                    return;
                }
            }
        }
        Err(e) => {
            warn!(peer = %peer, error = %e, "VNP: coordinator lock poisoned");
            return;
        }
    };

    // Do not retain the global coordinator mutex while waiting for the
    // session control actor. A long-running command in any session therefore
    // cannot turn an attach into global coordinator contention.
    let initial_state = match initial_reply.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(state) => state,
        Err(_) => {
            warn!(peer = %peer, session_id = session_id_raw, "VNP: InitialState timed out");
            cleanup(&coordinator, session_id.clone(), client_id, &peer);
            return;
        }
    };

    // Send InitialState to the client.
    if let Err(e) = send_vnp_msg(
        &mut frame_writer,
        DOMAIN_RENDER,
        MSG_INITIAL_STATE,
        session_id_raw,
        &initial_state,
    ) {
        warn!(peer = %peer, error = %e, "VNP: failed to send InitialState");
        cleanup(&coordinator, session_id.clone(), client_id, &peer);
        return;
    }

    // Switch to 16 ms read timeout for the main loop.
    // Set timeout on the write-side clone. Both clones share the same underlying
    // socket descriptor, so SO_RCVTIMEO applies to reads on the read-side too.
    if let Err(e) = frame_writer
        .inner_ref()
        .set_read_timeout(Some(std::time::Duration::from_millis(16)))
    {
        warn!(peer = %peer, error = %e, "VNP: failed to set main-loop timeout");
    }

    // Main loop: drain render_rx → send RenderBatch frames; read frames → dispatch.
    loop {
        // Drain all pending render batches without blocking.
        loop {
            match render_rx.try_recv() {
                Ok(batch) => {
                    if let Err(e) = send_vnp_msg(
                        &mut frame_writer,
                        DOMAIN_RENDER,
                        MSG_RENDER_BATCH,
                        session_id_raw,
                        &batch,
                    ) {
                        warn!(peer = %peer, error = %e, "VNP: failed to send RenderBatch; disconnecting");
                        cleanup(&coordinator, session_id.clone(), client_id, &peer);
                        return;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    info!(peer = %peer, client_id, "VNP: render channel closed; disconnecting");
                    cleanup(&coordinator, session_id.clone(), client_id, &peer);
                    return;
                }
            }
        }

        // Read one TCP frame.
        match frame_reader.read_frame() {
            Ok(frame) => {
                match dispatch_frame(frame, &coordinator, client_id, session_id_raw, &peer) {
                    DispatchResult::Continue => {}
                    DispatchResult::Disconnect => {
                        cleanup(&coordinator, session_id.clone(), client_id, &peer);
                        return;
                    }
                    DispatchResult::DetachedClean => {
                        // Client voluntarily detached; unregister already called inside
                        // dispatch_frame — return without calling cleanup again.
                        return;
                    }
                }
            }
            Err(FrameError::UnexpectedEof) => {
                info!(peer = %peer, client_id, "VNP client disconnected (EOF)");
                cleanup(&coordinator, session_id.clone(), client_id, &peer);
                return;
            }
            Err(FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Timeout is normal — continue to drain render_rx next iteration.
                continue;
            }
            Err(e) => {
                warn!(peer = %peer, client_id, error = %e, "VNP frame read error; disconnecting");
                cleanup(&coordinator, session_id.clone(), client_id, &peer);
                return;
            }
        }
    }
}

/// Read frames until an `AttachSession` message is found (domain=4, type=0x02).
///
/// Returns the session ID as a `u32`, or `None` if the connection closes before
/// the expected message arrives or if 64 non-matching frames are received.
fn wait_for_attach(reader: &mut FrameReader<BufReader<TcpStream>>, peer: &str) -> Option<u32> {
    let mut attempts = 0usize;
    loop {
        if attempts >= 64 {
            warn!(peer, "VNP: AttachSession not received within 64 frames");
            return None;
        }
        attempts += 1;

        let frame = match reader.read_frame() {
            Ok(f) => f,
            Err(FrameError::UnexpectedEof) => {
                info!(peer, "VNP: client disconnected before AttachSession");
                return None;
            }
            Err(FrameError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                warn!(peer, "VNP: timed out waiting for AttachSession");
                return None;
            }
            Err(e) => {
                warn!(peer, error = %e, "VNP: error reading frame in attach phase");
                return None;
            }
        };

        let (envelope, msg_bytes) = match decode_envelope(&frame.payload) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(peer, error = %e, "VNP: envelope decode failed in attach phase; skipping frame");
                continue;
            }
        };

        if envelope.domain == DOMAIN_SESSION && envelope.msg_type == MSG_ATTACH_SESSION {
            let mut bit_reader = BitReader::new(msg_bytes);
            match AttachSession::unpack(&mut bit_reader) {
                Ok(attach) => {
                    return Some(attach.session_id.0);
                }
                Err(e) => {
                    warn!(peer, error = %e, "VNP: AttachSession decode failed");
                    return None;
                }
            }
        }

        // Log and skip non-AttachSession frames during the attach phase.
        warn!(
            peer,
            domain = envelope.domain,
            msg_type = envelope.msg_type,
            "VNP: unexpected message during attach phase; skipping"
        );
    }
}

/// Result of dispatching a single inbound VNP frame.
#[derive(Debug, PartialEq)]
enum DispatchResult {
    /// The message was handled; the main loop should continue.
    Continue,
    /// A fatal error occurred; the caller must call cleanup and return.
    Disconnect,
    /// The client sent `DetachSession` and has been cleanly unregistered;
    /// the caller must return without calling cleanup again.
    DetachedClean,
}

/// Dispatch a single inbound VNP frame from the client.
///
/// Returns [`DispatchResult::Continue`] on success or when the message type is
/// unrecognised (warning logged). Returns [`DispatchResult::Disconnect`] only on
/// fatal errors such as a poisoned coordinator lock. Returns
/// [`DispatchResult::DetachedClean`] when the client voluntarily detaches.
fn dispatch_frame(
    frame: Frame,
    coordinator: &Arc<Mutex<Coordinator>>,
    client_id: u64,
    session_id: u32,
    peer: &str,
) -> DispatchResult {
    let (envelope, msg_bytes) = match decode_envelope(&frame.payload) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(peer, error = %e, "VNP: envelope decode failed; skipping frame");
            return DispatchResult::Continue;
        }
    };

    match (envelope.domain, envelope.msg_type) {
        (DOMAIN_INPUT, MSG_KEY_EVENT) => {
            let mut bit_reader = BitReader::new(msg_bytes);
            match KeyEvent::unpack(&mut bit_reader) {
                Ok(key) => match coordinator.lock() {
                    Ok(coord) => {
                        if let Err(e) = coord.send_key_input(SessionId(session_id), key) {
                            warn!(peer, error = %e, "VNP: send_key_input failed");
                        }
                    }
                    Err(e) => {
                        warn!(peer, error = %e, "VNP: coordinator lock poisoned");
                        return DispatchResult::Disconnect;
                    }
                },
                Err(e) => {
                    warn!(peer, error = %e, "VNP: KeyEvent decode failed");
                }
            }
        }
        (DOMAIN_INPUT, MSG_RESIZE) => {
            let mut bit_reader = BitReader::new(msg_bytes);
            match Resize::unpack(&mut bit_reader) {
                Ok(resize) => match coordinator.lock() {
                    Ok(coord) => {
                        if let Err(e) = coord.send_command(
                            SessionId(session_id),
                            SessionCommand::Resize {
                                cols: resize.cols,
                                rows: resize.rows,
                            },
                        ) {
                            warn!(peer, error = %e, "VNP: send Resize command failed");
                        }
                    }
                    Err(e) => {
                        warn!(peer, error = %e, "VNP: coordinator lock poisoned");
                        return DispatchResult::Disconnect;
                    }
                },
                Err(e) => {
                    warn!(peer, error = %e, "VNP: Resize decode failed");
                }
            }
        }
        (DOMAIN_RENDER, MSG_FRAME_ACK) => {
            let mut bit_reader = BitReader::new(msg_bytes);
            match FrameAck::unpack(&mut bit_reader) {
                Ok(ack) => match coordinator.lock() {
                    Ok(coord) => {
                        if let Err(e) =
                            coord.ack_frame(SessionId(session_id), client_id, ack.frame_seq)
                        {
                            warn!(peer, error = %e, "VNP: ack_frame failed");
                        }
                    }
                    Err(e) => {
                        warn!(peer, error = %e, "VNP: coordinator lock poisoned");
                        return DispatchResult::Disconnect;
                    }
                },
                Err(e) => {
                    warn!(peer, error = %e, "VNP: FrameAck decode failed");
                }
            }
        }
        (DOMAIN_SESSION, MSG_DETACH_SESSION) => {
            let mut bit_reader = BitReader::new(msg_bytes);
            match DetachSession::unpack(&mut bit_reader) {
                Ok(detach) => {
                    if detach.session_id.0 == session_id {
                        info!(
                            peer,
                            client_id, session_id, "VNP: client sent DetachSession"
                        );
                        match coordinator.lock() {
                            Ok(mut coord) => {
                                if let Err(e) =
                                    coord.unregister_vnp_client(SessionId(session_id), client_id)
                                {
                                    warn!(peer, error = %e, "VNP: unregister after DetachSession failed");
                                }
                            }
                            Err(e) => {
                                warn!(peer, error = %e, "VNP: coordinator lock poisoned during DetachSession");
                                return DispatchResult::Disconnect;
                            }
                        }
                        return DispatchResult::DetachedClean;
                    }
                }
                Err(e) => {
                    warn!(peer, error = %e, "VNP: DetachSession decode failed");
                }
            }
        }
        _ => {
            warn!(
                peer,
                domain = envelope.domain,
                msg_type = envelope.msg_type,
                "VNP: unrecognised message type; ignoring"
            );
        }
    }

    DispatchResult::Continue
}

/// Errors that can occur when encoding and sending a VNP message.
#[derive(Debug, thiserror::Error)]
enum VnpSendError {
    #[error("bitpack encode failed: {0}")]
    Pack(String),
    #[error("envelope encode failed: {0}")]
    Encode(String),
    #[error("frame write failed: {0}")]
    Write(#[from] FrameError),
}

/// Encode `msg` as a VNP envelope frame and write it to `writer`.
fn send_vnp_msg<W: std::io::Write, T: Pack>(
    writer: &mut FrameWriter<W>,
    domain: u8,
    msg_type: u8,
    session_id: u32,
    msg: &T,
) -> Result<(), VnpSendError> {
    let envelope = make_envelope(domain, msg_type, session_id);

    let mut bw = BitWriter::new();
    msg.pack(&mut bw)
        .map_err(|e| VnpSendError::Pack(e.to_string()))?;
    let msg_bytes = bw.finish();

    let payload =
        encode_message(&envelope, &msg_bytes).map_err(|e| VnpSendError::Encode(e.to_string()))?;

    let frame = Frame {
        flags: FrameFlags::new(),
        payload,
    };
    writer.write_frame(&frame)?;
    Ok(())
}

/// Unregister the client from the coordinator on disconnect.
fn cleanup(
    coordinator: &Arc<Mutex<Coordinator>>,
    session_id: SessionId,
    client_id: u64,
    peer: &str,
) {
    match coordinator.lock() {
        Ok(mut coord) => {
            if let Err(e) = coord.unregister_vnp_client(session_id, client_id) {
                warn!(peer, error = %e, "VNP: unregister_vnp_client failed");
            }
        }
        Err(e) => {
            warn!(peer, error = %e, "VNP: coordinator lock poisoned during cleanup");
        }
    }
}
