//! Privileged helper named-pipe server.

use std::io;

use malt_platform::ipc::{NamedPipeConnection, NamedPipeServer, PeerIdentity};
use malt_platform::service::StopSignal;
use malt_protocol::elevate_channel::{HELLO, HELLO_ACK, REQUEST, RESPONSE};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use crate::auth::ReplayGuard;
use crate::capability::PROTOCOL_VERSION;
use crate::dispatch::{dispatch_request, refused};
use crate::error::ElevateError;
use crate::protocol::{
    ElevateHello, ElevateHelloAck, ElevateRequestEnvelope, ElevateResponse, ReasonCode,
};

/// Configuration for one authorised daemon connection.
#[derive(Debug)]
pub struct ServerConfig {
    pub pipe_name: String,
    pub authorized_principal: String,
    pub replay_capacity: usize,
}

/// Serve authenticated named-pipe clients until the Service Control Manager
/// requests the host to stop.
pub fn serve(config: &ServerConfig, stop: &StopSignal) -> Result<(), ElevateError> {
    let guard = ReplayGuard::new(config.replay_capacity)?;
    loop {
        if stop.is_requested() {
            return Ok(());
        }
        let server =
            NamedPipeServer::create_for_principal(&config.pipe_name, &config.authorized_principal)
                .map_err(ElevateError::Connection)?;
        let connection = server.accept().map_err(ElevateError::Connection)?;
        if stop.is_requested() {
            return Ok(());
        }
        let identity = connection
            .peer_identity()
            .map_err(ElevateError::Connection)?;
        if let Err(error) = authorize(identity, &config.authorized_principal) {
            tracing::warn!(error = %error, "refused unauthorised helper pipe client");
            continue;
        }
        if let Err(error) = serve_connection(connection, &guard) {
            tracing::warn!(error = %error, "helper pipe client session ended without a valid completion");
        }
    }
}

fn serve_connection(
    mut connection: NamedPipeConnection,
    guard: &ReplayGuard,
) -> Result<(), ElevateError> {
    let hello = decode_hello(&read_frame(&mut connection)?)?;
    let accepted = hello.version == PROTOCOL_VERSION;
    write_hello_ack(
        &mut connection,
        ElevateHelloAck {
            nonce: hello.nonce,
            accepted,
            reason: (!accepted).then(|| {
                format!(
                    "helper protocol version {PROTOCOL_VERSION} does not match daemon {}",
                    hello.version
                )
            }),
            version: PROTOCOL_VERSION,
            _unknown: Vec::new(),
        },
    )?;
    if !accepted {
        return Ok(());
    }
    loop {
        let frame = read_frame(&mut connection)?;
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

fn read_frame(connection: &mut NamedPipeConnection) -> Result<Frame, ElevateError> {
    match FrameReader::new(connection.file()).read_frame() {
        Ok(frame) => Ok(frame),
        Err(malt_protocol::framing::FrameError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
            ) =>
        {
            Err(ElevateError::Connection(io::Error::new(
                error.kind(),
                "helper pipe client disconnected",
            )))
        }
        Err(error) => Err(frame_error(error)),
    }
}

fn authorize(peer: PeerIdentity, expected_principal: &str) -> Result<(), ElevateError> {
    if peer.principal == expected_principal {
        Ok(())
    } else {
        Err(ElevateError::AuthFailed(format!(
            "named-pipe peer process {} has principal {}; expected {}",
            peer.process_id, peer.principal, expected_principal
        )))
    }
}

fn decode_request(frame: &Frame) -> Result<ElevateRequestEnvelope, ElevateError> {
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(ElevateError::Protocol("empty elevate frame".into()));
    };
    if tag != REQUEST {
        return Err(ElevateError::Protocol(format!(
            "unexpected elevate message tag {tag}"
        )));
    }
    let mut reader = BitReader::new(body);
    ElevateRequestEnvelope::unpack(&mut reader)
        .map_err(|error| ElevateError::Protocol(format!("invalid elevate request: {error}")))
}

fn decode_hello(frame: &Frame) -> Result<ElevateHello, ElevateError> {
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(ElevateError::Protocol("empty elevate frame".into()));
    };
    if tag != HELLO {
        return Err(ElevateError::Protocol(format!(
            "expected elevate hello message, received tag {tag}"
        )));
    }
    let mut reader = BitReader::new(body);
    ElevateHello::unpack(&mut reader)
        .map_err(|error| ElevateError::Protocol(format!("invalid elevate hello: {error}")))
}

fn write_hello_ack(
    connection: &mut NamedPipeConnection,
    acknowledgement: ElevateHelloAck,
) -> Result<(), ElevateError> {
    let mut writer = BitWriter::new();
    acknowledgement.pack(&mut writer).map_err(|error| {
        ElevateError::Protocol(format!("encode elevate hello acknowledgement: {error}"))
    })?;
    let mut payload = vec![HELLO_ACK];
    payload.extend(writer.finish());
    FrameWriter::new(connection.file())
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload,
        })
        .map_err(frame_error)
}

fn encode_response(response: &ElevateResponse) -> Result<Frame, ElevateError> {
    let mut writer = BitWriter::new();
    response
        .pack(&mut writer)
        .map_err(|error| ElevateError::Protocol(format!("encode elevate response: {error}")))?;
    let mut payload = vec![RESPONSE];
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
