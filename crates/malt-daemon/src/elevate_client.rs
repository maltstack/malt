//! Client and lifecycle operations for the privileged helper service.
//!
//! SCM registration is only installation state. `status` treats a helper as
//! reachable after its generated VNP hello/ack exchange completes.

use std::io;
use std::path::Path;

pub const HELPER_SERVICE_NAME: &str = "MALT-Elevate";
pub const HELPER_PIPE_NAME: &str = "malt-elevate";
pub const HELPER_PROTOCOL_VERSION: u32 = 2;

/// Observable helper state. Reachability is never inferred from SCM alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperState {
    NotInstalled,
    InstalledStopped,
    InstalledUnreachable,
    Reachable { protocol_version: u32 },
    VersionMismatch { expected: u32, actual: u32 },
}

/// Send one request through the helper after checking state. A request that
/// may have crossed the process boundary but yields no response is explicitly
/// indeterminate; callers must not reinterpret it as a refusal or success.
#[cfg(windows)]
pub fn send_request(
    envelope: malt_protocol::elevate::ElevateRequestEnvelope,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_protocol::elevate::ReasonCode;
    use std::sync::mpsc;
    use std::time::Duration;

    let request_id = envelope.request_id;
    match status()? {
        HelperState::Reachable { .. } => {}
        HelperState::VersionMismatch { .. } => {
            return Ok(refused(
                request_id,
                ReasonCode::HelperUnavailable,
                "helper protocol version does not match; no operation was attempted",
            ))
        }
        HelperState::NotInstalled
        | HelperState::InstalledStopped
        | HelperState::InstalledUnreachable => {
            return Ok(refused(
                request_id,
                ReasonCode::HelperUnavailable,
                "privileged helper is not reachable; no operation was attempted",
            ))
        }
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(send_once(envelope));
    });
    match receiver.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Ok(indeterminate(
            request_id,
            format!("helper request may have started but no response was received: {error}"),
        )),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(indeterminate(
            request_id,
            "helper request timed out after 30 seconds; its outcome is unknown".to_string(),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(indeterminate(
            request_id,
            "helper request worker exited without reporting an outcome".to_string(),
        )),
    }
}

/// Return whether this process already has the elevation required for a
/// service-management operation.
#[cfg(windows)]
pub fn is_current_process_elevated() -> io::Result<bool> {
    malt_platform::service::is_current_process_elevated()
}

/// Request UAC consent for the explicitly supplied MALT command and wait for
/// the elevated child to complete.
#[cfg(windows)]
pub fn run_elevated(executable: &Path, arguments: &[&str]) -> io::Result<u32> {
    malt_platform::service::run_elevated(executable, arguments)
}

/// Query helper state, including a bounded VNP round trip when SCM says the
/// service is running.
#[cfg(windows)]
pub fn status() -> io::Result<HelperState> {
    match malt_platform::service::status(HELPER_SERVICE_NAME)? {
        malt_platform::service::ServiceStatus::NotInstalled => Ok(HelperState::NotInstalled),
        malt_platform::service::ServiceStatus::Stopped
        | malt_platform::service::ServiceStatus::Other => Ok(HelperState::InstalledStopped),
        malt_platform::service::ServiceStatus::Running => match probe()? {
            ProbeResult::Reachable { protocol_version } => {
                Ok(HelperState::Reachable { protocol_version })
            }
            ProbeResult::VersionMismatch { actual } => Ok(HelperState::VersionMismatch {
                expected: HELPER_PROTOCOL_VERSION,
                actual,
            }),
            ProbeResult::Unavailable => Ok(HelperState::InstalledUnreachable),
        },
    }
}

/// Explicitly register and start the helper service for the current Windows
/// principal. The caller needs an already-elevated process; this function
/// neither suppresses nor attempts a UAC prompt.
#[cfg(windows)]
pub fn install(helper_executable: &Path) -> io::Result<()> {
    let principal = malt_platform::ipc::current_process_principal()?;
    malt_platform::service::install(
        HELPER_SERVICE_NAME,
        helper_executable,
        &[
            "--service",
            "--pipe",
            HELPER_PIPE_NAME,
            "--authorized-principal",
            &principal,
        ],
    )
}

/// Explicitly remove the helper service. The caller needs an already-elevated
/// process, and no other MALT command invokes this operation.
#[cfg(windows)]
pub fn uninstall() -> io::Result<()> {
    malt_platform::service::uninstall(HELPER_SERVICE_NAME)
}

/// Explicitly enrol one running daemon process after UAC approval.
#[cfg(windows)]
pub fn enroll_daemon(pid: u32) -> io::Result<()> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::{DaemonEnrollmentRequest, DaemonEnrollmentResponse, ElevateHello};
    use malt_protocol::elevate_channel::{
        DAEMON_ENROLLMENT_REQUEST, DAEMON_ENROLLMENT_RESPONSE, HELLO, HELLO_ACK,
    };
    use malt_protocol::vexil_runtime::{BitWriter, Pack};

    match status()? {
        HelperState::Reachable { .. } => {}
        state => {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("helper must be reachable before daemon enrollment: {state:?}"),
            ))
        }
    }
    let mut connection = NamedPipeClient::connect(HELPER_PIPE_NAME)?;
    let nonce = enrollment_nonce(pid);
    let hello = ElevateHello {
        nonce,
        version: HELPER_PROTOCOL_VERSION,
        _unknown: Vec::new(),
    };
    let mut writer = BitWriter::new();
    hello
        .pack(&mut writer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![HELLO];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    let acknowledgement =
        read_tagged::<malt_protocol::elevate::ElevateHelloAck>(&mut connection, HELLO_ACK)?;
    if acknowledgement.nonce != nonce
        || acknowledgement.version != HELPER_PROTOCOL_VERSION
        || !acknowledgement.accepted
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "helper rejected the enrollment handshake",
        ));
    }
    let mut writer = BitWriter::new();
    DaemonEnrollmentRequest {
        pid,
        _unknown: Vec::new(),
    }
    .pack(&mut writer)
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![DAEMON_ENROLLMENT_REQUEST];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    let response =
        read_tagged::<DaemonEnrollmentResponse>(&mut connection, DAEMON_ENROLLMENT_RESPONSE)?;
    if response.accepted {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            response
                .reason
                .unwrap_or_else(|| "helper refused daemon enrollment".to_string()),
        ))
    }
}

/// Register the resources a previously enrolled daemon may name for one
/// session. The helper canonicalizes and independently re-observes them.
#[cfg(windows)]
pub fn register_session_entitlement(
    session_id: malt_protocol::common::SessionId,
    storage_root: &Path,
    pids: &[u32],
) -> io::Result<()> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::{
        ElevateHello, SessionEntitlementRequest, SessionEntitlementResponse,
    };
    use malt_protocol::elevate_channel::{
        HELLO, HELLO_ACK, SESSION_ENTITLEMENT_REQUEST, SESSION_ENTITLEMENT_RESPONSE,
    };
    use malt_protocol::vexil_runtime::{BitWriter, Pack};

    if !matches!(status()?, HelperState::Reachable { .. }) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "helper is not reachable",
        ));
    }
    let storage_root = storage_root.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "session root is not valid UTF-8",
        )
    })?;
    let mut connection = NamedPipeClient::connect(HELPER_PIPE_NAME)?;
    let nonce = enrollment_nonce(session_id.0);
    let hello = ElevateHello {
        nonce,
        version: HELPER_PROTOCOL_VERSION,
        _unknown: Vec::new(),
    };
    let mut writer = BitWriter::new();
    hello
        .pack(&mut writer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![HELLO];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    let acknowledgement =
        read_tagged::<malt_protocol::elevate::ElevateHelloAck>(&mut connection, HELLO_ACK)?;
    if acknowledgement.nonce != nonce
        || acknowledgement.version != HELPER_PROTOCOL_VERSION
        || !acknowledgement.accepted
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "helper rejected session registration handshake",
        ));
    }
    let mut writer = BitWriter::new();
    SessionEntitlementRequest {
        session_id,
        storage_root: storage_root.to_string(),
        pids: pids.to_vec(),
        _unknown: Vec::new(),
    }
    .pack(&mut writer)
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![SESSION_ENTITLEMENT_REQUEST];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    let response =
        read_tagged::<SessionEntitlementResponse>(&mut connection, SESSION_ENTITLEMENT_RESPONSE)?;
    if response.accepted {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            response
                .reason
                .unwrap_or_else(|| "helper refused session registration".to_string()),
        ))
    }
}

#[cfg(windows)]
fn enrollment_nonce(pid: u32) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_NONCE: AtomicU64 = AtomicU64::new(1);
    (u64::from(pid) << 32) ^ NEXT_NONCE.fetch_add(1, Ordering::Relaxed)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use malt_protocol::elevate::{OutcomeKind, ReasonCode};

    #[test]
    fn unknown_outcome_is_not_reported_as_refusal_or_success() {
        let response = indeterminate(17, "response lost");
        assert_eq!(response.request_id, 17);
        assert_eq!(response.kind, OutcomeKind::Indeterminate);
        assert_eq!(response.reason, Some(ReasonCode::TimedOut));
    }

    #[test]
    fn unavailable_helper_refusal_names_the_cause() {
        let response = refused(8, ReasonCode::HelperUnavailable, "not reachable");
        assert_eq!(response.kind, OutcomeKind::Refused);
        assert_eq!(response.reason, Some(ReasonCode::HelperUnavailable));
    }
}

#[cfg(windows)]
enum ProbeResult {
    Reachable { protocol_version: u32 },
    VersionMismatch { actual: u32 },
    Unavailable,
}

#[cfg(windows)]
fn probe() -> io::Result<ProbeResult> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    static NEXT_NONCE: AtomicU64 = AtomicU64::new(1);
    let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(probe_once(nonce));
    });
    match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(ProbeResult::Unavailable),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(ProbeResult::Unavailable),
    }
}

#[cfg(windows)]
fn probe_once(nonce: u64) -> io::Result<ProbeResult> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::ElevateHello;
    use malt_protocol::elevate_channel::{HELLO, HELLO_ACK};
    use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
    use malt_protocol::vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

    let mut connection = match NamedPipeClient::connect(HELPER_PIPE_NAME) {
        Ok(connection) => connection,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(ProbeResult::Unavailable)
        }
        Err(error) => return Err(error),
    };
    let hello = ElevateHello {
        nonce,
        version: HELPER_PROTOCOL_VERSION,
        _unknown: Vec::new(),
    };
    let mut writer = BitWriter::new();
    hello
        .pack(&mut writer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![HELLO];
    payload.extend(writer.finish());
    FrameWriter::new(connection.file())
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload,
        })
        .map_err(frame_error)?;
    let frame = FrameReader::new(connection.file())
        .read_frame()
        .map_err(frame_error)?;
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty helper acknowledgement",
        ));
    };
    if tag != HELLO_ACK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected helper acknowledgement tag {tag}"),
        ));
    }
    let mut reader = BitReader::new(body);
    let acknowledgement = malt_protocol::elevate::ElevateHelloAck::unpack(&mut reader)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if acknowledgement.nonce != nonce {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "helper acknowledgement nonce did not match the probe",
        ));
    }
    if acknowledgement.version != HELPER_PROTOCOL_VERSION || !acknowledgement.accepted {
        return Ok(ProbeResult::VersionMismatch {
            actual: acknowledgement.version,
        });
    }
    Ok(ProbeResult::Reachable {
        protocol_version: acknowledgement.version,
    })
}

#[cfg(windows)]
fn send_once(
    envelope: malt_protocol::elevate::ElevateRequestEnvelope,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::{ElevateHello, ElevateHelloAck, ElevateResponse};
    use malt_protocol::elevate_channel::{HELLO, HELLO_ACK, REQUEST, RESPONSE};
    use malt_protocol::vexil_runtime::{BitWriter, Pack};

    let mut connection = NamedPipeClient::connect(HELPER_PIPE_NAME)?;
    let hello = ElevateHello {
        nonce: envelope.nonce,
        version: HELPER_PROTOCOL_VERSION,
        _unknown: Vec::new(),
    };
    let mut writer = BitWriter::new();
    hello
        .pack(&mut writer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![HELLO];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    let acknowledgement = read_tagged::<ElevateHelloAck>(&mut connection, HELLO_ACK)?;
    if acknowledgement.nonce != envelope.nonce
        || acknowledgement.version != HELPER_PROTOCOL_VERSION
        || !acknowledgement.accepted
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "helper rejected the operation handshake",
        ));
    }
    let mut writer = BitWriter::new();
    envelope
        .pack(&mut writer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut payload = vec![REQUEST];
    payload.extend(writer.finish());
    write_frame(&mut connection, payload)?;
    read_tagged::<ElevateResponse>(&mut connection, RESPONSE)
}

#[cfg(windows)]
fn write_frame(
    connection: &mut malt_platform::ipc::NamedPipeConnection,
    payload: Vec<u8>,
) -> io::Result<()> {
    use malt_protocol::framing::{Frame, FrameFlags, FrameWriter};

    FrameWriter::new(connection.file())
        .write_frame(&Frame {
            flags: FrameFlags::new(),
            payload,
        })
        .map_err(frame_error)
}

#[cfg(windows)]
fn read_tagged<T>(
    connection: &mut malt_platform::ipc::NamedPipeConnection,
    expected_tag: u8,
) -> io::Result<T>
where
    T: malt_protocol::vexil_runtime::Unpack,
{
    use malt_protocol::framing::FrameReader;
    use malt_protocol::vexil_runtime::BitReader;

    let frame = FrameReader::new(connection.file())
        .read_frame()
        .map_err(frame_error)?;
    let Some((&tag, body)) = frame.payload.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty helper frame",
        ));
    };
    if tag != expected_tag {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected helper message tag {tag}"),
        ));
    }
    let mut reader = BitReader::new(body);
    T::unpack(&mut reader)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[cfg(windows)]
fn refused(
    request_id: u32,
    reason: malt_protocol::elevate::ReasonCode,
    detail: impl Into<String>,
) -> malt_protocol::elevate::ElevateResponse {
    response(
        request_id,
        malt_protocol::elevate::OutcomeKind::Refused,
        Some(reason),
        detail,
    )
}

#[cfg(windows)]
fn indeterminate(
    request_id: u32,
    detail: impl Into<String>,
) -> malt_protocol::elevate::ElevateResponse {
    response(
        request_id,
        malt_protocol::elevate::OutcomeKind::Indeterminate,
        Some(malt_protocol::elevate::ReasonCode::TimedOut),
        detail,
    )
}

#[cfg(windows)]
fn response(
    request_id: u32,
    kind: malt_protocol::elevate::OutcomeKind,
    reason: Option<malt_protocol::elevate::ReasonCode>,
    detail: impl Into<String>,
) -> malt_protocol::elevate::ElevateResponse {
    malt_protocol::elevate::ElevateResponse {
        request_id,
        kind,
        reason,
        detail: Some(detail.into()),
        payload: None,
        _unknown: Vec::new(),
    }
}

#[cfg(windows)]
fn frame_error(error: malt_protocol::framing::FrameError) -> io::Error {
    match error {
        malt_protocol::framing::FrameError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

#[cfg(not(windows))]
pub fn status() -> io::Result<HelperState> {
    Ok(HelperState::InstalledUnreachable)
}

#[cfg(not(windows))]
pub fn send_request(
    envelope: malt_protocol::elevate::ElevateRequestEnvelope,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    Ok(malt_protocol::elevate::ElevateResponse {
        request_id: envelope.request_id,
        kind: malt_protocol::elevate::OutcomeKind::Refused,
        reason: Some(malt_protocol::elevate::ReasonCode::UnsupportedPlatform),
        detail: Some("the privileged helper service is only available on Windows".into()),
        payload: None,
        _unknown: Vec::new(),
    })
}

#[cfg(not(windows))]
pub fn is_current_process_elevated() -> io::Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn run_elevated(_executable: &Path, _arguments: &[&str]) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows UAC elevation is not available on this platform",
    ))
}

#[cfg(not(windows))]
pub fn install(_helper_executable: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the privileged helper service is only available on Windows",
    ))
}

#[cfg(not(windows))]
pub fn uninstall() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the privileged helper service is only available on Windows",
    ))
}
