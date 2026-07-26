//! Privileged helper named-pipe server.

use std::io;

use malt_platform::ipc::{NamedPipeServer, PeerIdentity};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use crate::auth::ReplayGuard;
use crate::dispatch::{dispatch_request, refused};
use crate::error::ElevateError;
use crate::protocol::{ElevateRequestEnvelope, ElevateResponse, ReasonCode};

const REQUEST_TAG: u8 = 1;
const RESPONSE_TAG: u8 = 2;

/// Configuration for one authorised daemon connection.
#[derive(Debug)]
pub struct ServerConfig {
    pub pipe_name: String,
    pub authorized_process_id: u32,
    pub replay_capacity: usize,
}

/// Serve one authenticated named-pipe client until it disconnects.
pub fn serve(config: &ServerConfig) -> Result<(), ElevateError> {
    let server = NamedPipeServer::create(&config.pipe_name).map_err(ElevateError::Connection)?;
    let mut connection = server.accept().map_err(ElevateError::Connection)?;
    authorize(
        connection
            .peer_identity()
            .map_err(ElevateError::Connection)?,
        config.authorized_process_id,
    )?;
    let guard = ReplayGuard::new(config.replay_capacity)?;

    loop {
        let frame = FrameReader::new(connection.file())
            .read_frame()
            .map_err(frame_error)?;
        let response = match decode_request(&frame) {
            Ok(envelope) if guard.consume(envelope.nonce) => {
                dispatch_request(envelope.request_id, &envelope.request)
            }
            Ok(envelope) => refused(
                envelope.request_id,
                ReasonCode::InvalidParameters,
                "request nonce has already been consumed",
            ),
            Err(error) => return Err(error),
        };
        let frame = encode_response(&response)?;
        FrameWriter::new(connection.file())
            .write_frame(&frame)
            .map_err(frame_error)?;
    }
}

fn authorize(peer: PeerIdentity, expected_process_id: u32) -> Result<(), ElevateError> {
    if peer.process_id == expected_process_id {
        Ok(())
    } else {
        Err(ElevateError::AuthFailed(format!(
            "named-pipe peer process {} is not the authorised daemon {}",
            peer.process_id, expected_process_id
        )))
    }
}

fn decode_request(frame: &Frame) -> Result<ElevateRequestEnvelope, ElevateError> {
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(ElevateError::Protocol("empty elevate frame".into()));
    };
    if tag != REQUEST_TAG {
        return Err(ElevateError::Protocol(format!(
            "unexpected elevate message tag {tag}"
        )));
    }
    let mut reader = BitReader::new(body);
    ElevateRequestEnvelope::unpack(&mut reader)
        .map_err(|error| ElevateError::Protocol(format!("invalid elevate request: {error}")))
}

fn encode_response(response: &ElevateResponse) -> Result<Frame, ElevateError> {
    let mut writer = BitWriter::new();
    response
        .pack(&mut writer)
        .map_err(|error| ElevateError::Protocol(format!("encode elevate response: {error}")))?;
    let mut payload = vec![RESPONSE_TAG];
    payload.extend(writer.finish());
    Ok(Frame {
        flags: FrameFlags::new(),
        payload,
    })
}

fn frame_error(error: malt_protocol::framing::FrameError) -> ElevateError {
    match error {
        malt_protocol::framing::FrameError::Io(error) => ElevateError::Connection(error),
        other => ElevateError::Protocol(other.to_string()),
    }
}

impl From<io::Error> for ElevateError {
    fn from(error: io::Error) -> Self {
        Self::Connection(error)
    }
}
