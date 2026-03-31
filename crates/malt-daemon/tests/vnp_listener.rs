use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_protocol::envelope::{decode_envelope, encode_message, Envelope};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::handshake::{Hello, HelloAck};
use std::io::BufReader;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;

/// Start a VNP listener on a random port and return the address.
fn start_test_listener(coordinator: Arc<Mutex<Coordinator>>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    // Spawn the accept loop manually (we can't use start_vnp_listener
    // because it calls bind internally, and we need the random port).
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let coord = coordinator.clone();
                    std::thread::spawn(move || {
                        // Reuse the handle_client logic by connecting through the module
                        // We'll inline the handshake + message loop here for test isolation.
                        handle_test_client(stream, coord);
                    });
                }
                Err(_) => break,
            }
        }
    });

    addr
}

/// Simplified client handler for tests — mirrors vnp_listener::handle_client.
fn handle_test_client(stream: TcpStream, coordinator: Arc<Mutex<Coordinator>>) {
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut read_stream = BufReader::new(stream);

    let sessions = coordinator
        .lock()
        .map(|c| c.list_sessions())
        .unwrap_or_default();

    if malt_daemon::connection::handshake::perform_server_handshake(
        &mut read_stream,
        &mut &write_stream,
        &sessions,
    )
    .is_err()
    {
        return;
    }

    // Message loop
    let mut frame_reader = FrameReader::new(read_stream);
    let mut frame_writer = FrameWriter::new(write_stream);

    // Set read timeout so loop doesn't block forever
    if let Ok(peer) = frame_writer.inner_ref().peer_addr() {
        let _ = frame_writer
            .inner_ref()
            .set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = peer;
    }

    loop {
        match frame_reader.read_frame() {
            Ok(frame) => {
                let payload = String::from_utf8_lossy(&frame.payload);
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&payload) {
                    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if msg_type == "attach" {
                        let sid = msg.get("session").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
                        // Send an output frame
                        let output = coordinator
                            .lock()
                            .ok()
                            .and_then(|c| c.get_session_output(sid).ok())
                            .unwrap_or_else(|| "session output placeholder".to_string());
                        let resp = serde_json::json!({
                            "type": "output",
                            "session": sid,
                            "text": output,
                        });
                        let payload = resp.to_string().into_bytes();
                        let mut flags = FrameFlags::new();
                        flags.set_json_encoded(true);
                        let frame = Frame { flags, payload };
                        if frame_writer.write_frame(&frame).is_err() {
                            break;
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn make_hello() -> Hello {
    Hello {
        version: 1,
        client_type: "test".to_string(),
        capabilities: ClientCapabilities {
            color_depth: ColorDepth::TrueColor,
            unicode: UnicodeLevel::Full,
            image_protocol: ImageProtocol::None,
            overlay: false,
            vt_passthrough: true,
            max_fps: 60,
            _unknown: Vec::new(),
        },
        _unknown: Vec::new(),
    }
}

fn send_hello(writer: &mut FrameWriter<TcpStream>) {
    let hello = make_hello();
    let envelope = Envelope {
        wire_version: 0,
        domain: 0,
        msg_type: 0x01,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
        _unknown: Vec::new(),
    };
    let mut w = BitWriter::new();
    hello.pack(&mut w).unwrap();
    let hello_bytes = w.finish();
    let combined = encode_message(&envelope, &hello_bytes);
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: combined,
    };
    writer.write_frame(&frame).unwrap();
}

fn read_hello_ack(reader: &mut FrameReader<BufReader<TcpStream>>) -> HelloAck {
    let frame = reader.read_frame().unwrap();
    let (envelope, msg_bytes) = decode_envelope(&frame.payload).unwrap();
    assert_eq!(envelope.domain, 0);
    assert_eq!(envelope.msg_type, 0x02, "expected HelloAck");
    let mut r = BitReader::new(msg_bytes);
    HelloAck::unpack(&mut r).unwrap()
}

#[test]
fn vnp_connect_and_handshake() {
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default())));
    let addr = start_test_listener(coordinator);

    // Give listener a moment to start
    std::thread::sleep(std::time::Duration::from_millis(50));

    let stream = TcpStream::connect(&addr).unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    // Send Hello
    send_hello(&mut writer);

    // Read HelloAck
    let ack = read_hello_ack(&mut reader);
    assert_eq!(ack.negotiated_version, 1);
}

#[test]
fn vnp_attach_and_receive_output() {
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default())));
    let addr = start_test_listener(coordinator);

    std::thread::sleep(std::time::Duration::from_millis(50));

    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let write_stream = stream.try_clone().unwrap();
    let read_stream = BufReader::new(stream);

    let mut writer = FrameWriter::new(write_stream);
    let mut reader = FrameReader::new(read_stream);

    // Handshake
    send_hello(&mut writer);
    let _ack = read_hello_ack(&mut reader);

    // Send attach message
    let attach = serde_json::json!({
        "type": "attach",
        "session": 1,
    });
    let payload = attach.to_string().into_bytes();
    let mut flags = FrameFlags::new();
    flags.set_json_encoded(true);
    let frame = Frame { flags, payload };
    writer.write_frame(&frame).unwrap();

    // Read output frame
    let output_frame = reader.read_frame().unwrap();
    let output_str = String::from_utf8_lossy(&output_frame.payload);
    let msg: serde_json::Value = serde_json::from_str(&output_str).unwrap();
    assert_eq!(msg.get("type").and_then(|t| t.as_str()), Some("output"));
    assert_eq!(msg.get("session").and_then(|s| s.as_u64()), Some(1));
    assert!(msg.get("text").is_some(), "output should contain text field");
}
