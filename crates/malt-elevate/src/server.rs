//! Privileged helper named-pipe server.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const MAX_CONCURRENT_CLIENTS: usize = 16;

use malt_platform::ipc::{NamedPipeConnection, NamedPipeServer, PeerIdentity};
use malt_platform::service::StopSignal;
use malt_protocol::elevate_channel::{
    DAEMON_ENROLLMENT_REQUEST, DAEMON_ENROLLMENT_RESPONSE, HELLO, HELLO_ACK, REQUEST, RESPONSE,
    SESSION_ENTITLEMENT_REQUEST, SESSION_ENTITLEMENT_RESPONSE,
};
use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
use malt_protocol::vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

use crate::auth::ReplayGuard;
use crate::capability::PROTOCOL_VERSION;
use crate::dispatch::refused;
use crate::entitlement::EnrollmentRegistry;
use crate::error::ElevateError;
use crate::protocol::{
    DaemonEnrollmentRequest, DaemonEnrollmentResponse, ElevateHello, ElevateHelloAck,
    ElevateRequestEnvelope, ElevateResponse, ReasonCode, SessionEntitlementRequest,
    SessionEntitlementResponse,
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
    let guard = Arc::new(ReplayGuard::new(config.replay_capacity)?);
    let active_clients = Arc::new(AtomicUsize::new(0));
    let enrollments = Arc::new(std::sync::Mutex::new(EnrollmentRegistry::default()));
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
        if let Err(error) = authorize(&identity, &config.authorized_principal) {
            tracing::warn!(error = %error, "refused unauthorised helper pipe client");
            continue;
        }
        if !try_acquire_client_slot(&active_clients) {
            tracing::warn!(
                limit = MAX_CONCURRENT_CLIENTS,
                "refused helper pipe client because the service is at its concurrent-client limit"
            );
            continue;
        }
        let guard = Arc::clone(&guard);
        let active_clients_for_thread = Arc::clone(&active_clients);
        let enrollments = Arc::clone(&enrollments);
        if let Err(error) = std::thread::Builder::new()
            .name("malt-elevate-client".to_string())
            .spawn(move || {
                let _slot = ClientSlot {
                    active_clients: active_clients_for_thread,
                };
                if let Err(error) = serve_connection(connection, identity, &guard, &enrollments) {
                    tracing::warn!(error = %error, "helper pipe client session ended without a valid completion");
                }
            })
        {
            active_clients.fetch_sub(1, Ordering::AcqRel);
            return Err(ElevateError::Connection(error));
        }
    }
}

fn try_acquire_client_slot(active_clients: &AtomicUsize) -> bool {
    active_clients
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < MAX_CONCURRENT_CLIENTS).then_some(active + 1)
        })
        .is_ok()
}

struct ClientSlot {
    active_clients: Arc<AtomicUsize>,
}

impl Drop for ClientSlot {
    fn drop(&mut self) {
        self.active_clients.fetch_sub(1, Ordering::AcqRel);
    }
}

fn serve_connection(
    mut connection: NamedPipeConnection,
    peer: PeerIdentity,
    guard: &ReplayGuard,
    enrollments: &std::sync::Mutex<EnrollmentRegistry>,
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
        let Some((&tag, _)) = frame.payload.split_first() else {
            return Err(ElevateError::Protocol("empty elevate frame".to_string()));
        };
        if tag == DAEMON_ENROLLMENT_REQUEST {
            let response = enroll_daemon(&frame, &peer, enrollments)?;
            let frame = encode_enrollment_response(&response)?;
            FrameWriter::new(connection.file())
                .write_frame(&frame)
                .map_err(frame_error)?;
            continue;
        }
        if tag == SESSION_ENTITLEMENT_REQUEST {
            let response = register_session(&frame, &peer, enrollments)?;
            FrameWriter::new(connection.file())
                .write_frame(&encode_session_entitlement_response(&response)?)
                .map_err(frame_error)?;
            continue;
        }
        let response = match decode_request(&frame) {
            Ok(envelope) if guard.consume(envelope.nonce) => {
                let enrolled = enrollments
                    .lock()
                    .map_err(|_| {
                        ElevateError::AuthFailed("enrollment registry lock poisoned".to_string())
                    })?
                    .is_currently_enrolled(&peer)?;
                if enrolled {
                    refuse_without_entitlement_authority(envelope.request_id)
                } else {
                    refused(
                        envelope.request_id,
                        ReasonCode::NotEntitled,
                        "caller is not an explicitly enrolled daemon process",
                    )
                }
            }
            Ok(envelope) => refused(
                envelope.request_id,
                ReasonCode::InvalidParameters,
                "request nonce has already been consumed",
            ),
            Err(error) => return Err(error),
        };
        FrameWriter::new(connection.file())
            .write_frame(&encode_response(&response)?)
            .map_err(frame_error)?;
    }
}

fn enroll_daemon(
    frame: &Frame,
    peer: &PeerIdentity,
    enrollments: &std::sync::Mutex<EnrollmentRegistry>,
) -> Result<DaemonEnrollmentResponse, ElevateError> {
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(ElevateError::Protocol("empty elevate frame".to_string()));
    };
    if tag != DAEMON_ENROLLMENT_REQUEST {
        return Err(ElevateError::Protocol(format!(
            "unexpected enrollment message tag {tag}"
        )));
    }
    let mut reader = BitReader::new(body);
    let request = DaemonEnrollmentRequest::unpack(&mut reader)
        .map_err(|error| ElevateError::Protocol(format!("invalid enrollment request: {error}")))?;
    let requester =
        malt_platform::ipc::process_identity(peer.process_id).map_err(ElevateError::Connection)?;
    let result = enrollments
        .lock()
        .map_err(|_| ElevateError::AuthFailed("enrollment registry lock poisoned".to_string()))?
        .enroll(peer, requester.elevated, request.pid);
    Ok(match result {
        Ok(()) => DaemonEnrollmentResponse {
            accepted: true,
            reason: None,
            _unknown: Vec::new(),
        },
        Err(error) => DaemonEnrollmentResponse {
            accepted: false,
            reason: Some(error.to_string()),
            _unknown: Vec::new(),
        },
    })
}

fn register_session(
    frame: &Frame,
    peer: &PeerIdentity,
    enrollments: &std::sync::Mutex<EnrollmentRegistry>,
) -> Result<SessionEntitlementResponse, ElevateError> {
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(ElevateError::Protocol("empty elevate frame".to_string()));
    };
    if tag != SESSION_ENTITLEMENT_REQUEST {
        return Err(ElevateError::Protocol(format!(
            "unexpected session entitlement message tag {tag}"
        )));
    }
    let mut reader = BitReader::new(body);
    let request = SessionEntitlementRequest::unpack(&mut reader).map_err(|error| {
        ElevateError::Protocol(format!("invalid session entitlement request: {error}"))
    })?;
    let result = enrollments
        .lock()
        .map_err(|_| ElevateError::AuthFailed("enrollment registry lock poisoned".to_string()))?
        .register_session(
            peer,
            request.session_id.0,
            &request.storage_root,
            &request.pids,
        );
    Ok(match result {
        Ok(()) => SessionEntitlementResponse {
            accepted: true,
            reason: None,
            _unknown: Vec::new(),
        },
        Err(error) => SessionEntitlementResponse {
            accepted: false,
            reason: Some(error.to_string()),
            _unknown: Vec::new(),
        },
    })
}

/// Refuse privileged operations until the helper can verify the envelope's
/// session, process, and path claims from helper-owned authority.
///
/// A named-pipe SID only identifies the Windows user.  It does not establish
/// that the connecting process is MALT's daemon, nor does it entitle that
/// process to act on an arbitrary session.  Dispatching an operation before
/// those claims are independently verified would turn the elevated service
/// into a same-user privilege-escalation primitive.
fn refuse_without_entitlement_authority(request_id: u32) -> ElevateResponse {
    tracing::warn!(
        request_id,
        "refused privileged operation without helper-side entitlement authority"
    );
    refused(
        request_id,
        ReasonCode::NotEntitled,
        "the helper has no independent authority for this session, process, or path; refusing operation",
    )
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

fn authorize(peer: &PeerIdentity, expected_principal: &str) -> Result<(), ElevateError> {
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

fn encode_enrollment_response(response: &DaemonEnrollmentResponse) -> Result<Frame, ElevateError> {
    let mut writer = BitWriter::new();
    response
        .pack(&mut writer)
        .map_err(|error| ElevateError::Protocol(format!("encode enrollment response: {error}")))?;
    let mut payload = vec![DAEMON_ENROLLMENT_RESPONSE];
    payload.extend(writer.finish());
    Ok(Frame {
        flags: FrameFlags::new(),
        payload,
    })
}

fn encode_session_entitlement_response(
    response: &SessionEntitlementResponse,
) -> Result<Frame, ElevateError> {
    let mut writer = BitWriter::new();
    response.pack(&mut writer).map_err(|error| {
        ElevateError::Protocol(format!("encode session entitlement response: {error}"))
    })?;
    let mut payload = vec![SESSION_ENTITLEMENT_RESPONSE];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::OutcomeKind;

    #[test]
    fn operations_are_refused_without_helper_side_entitlement_authority() {
        let response = refuse_without_entitlement_authority(41);

        assert_eq!(response.request_id, 41);
        assert_eq!(response.kind, OutcomeKind::Refused);
        assert_eq!(response.reason, Some(ReasonCode::NotEntitled));
        assert!(response.payload.is_none());
    }

    #[test]
    fn concurrent_client_limit_is_bounded() {
        let active = AtomicUsize::new(MAX_CONCURRENT_CLIENTS - 1);
        assert!(try_acquire_client_slot(&active));
        assert_eq!(active.load(Ordering::Acquire), MAX_CONCURRENT_CLIENTS);
        assert!(!try_acquire_client_slot(&active));
    }
}
