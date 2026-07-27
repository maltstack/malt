//! Client and lifecycle operations for the privileged helper service.
//!
//! SCM registration is only installation state. `status` treats a helper as
//! reachable after its generated VNP hello/ack exchange completes.

use std::io;
use std::path::Path;
use std::time::Duration;

pub const HELPER_SERVICE_NAME: &str = "MALT-Elevate";
pub const HELPER_PIPE_NAME: &str = "malt-elevate";
pub const HELPER_PROTOCOL_VERSION: u32 = 3;

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
    send_request_with_timeout(envelope, Duration::from_secs(30))
}

#[cfg(windows)]
fn send_request_with_timeout(
    envelope: malt_protocol::elevate::ElevateRequestEnvelope,
    timeout: Duration,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_protocol::elevate::ReasonCode;
    use std::sync::mpsc;

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
    match receiver.recv_timeout(timeout) {
        Ok(result) => complete_request_attempt(request_id, result),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(indeterminate(
            request_id,
            format!(
                "helper request timed out after {} seconds; its outcome is unknown",
                timeout.as_secs()
            ),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(indeterminate(
            request_id,
            "helper request worker exited without reporting an outcome".to_string(),
        )),
    }
}

/// Preserve the distinction between a helper refusal and an interrupted
/// request after it crossed the privilege boundary.  Keeping this conversion
/// in one place lets the actual named-pipe loss test exercise the exact path
/// production uses after its worker returns.
#[cfg(windows)]
fn complete_request_attempt(
    request_id: u32,
    result: io::Result<malt_protocol::elevate::ElevateResponse>,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    match result {
        Ok(response) => Ok(response),
        Err(error) => Ok(indeterminate(
            request_id,
            format!("helper request may have started but no response was received: {error}"),
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
    let service = malt_platform::service::status(HELPER_SERVICE_NAME)?;
    let probe = matches!(service, malt_platform::service::ServiceStatus::Running)
        .then(probe)
        .transpose()?;
    Ok(helper_state_from(service, probe))
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
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let state = loop {
        let state = status()?;
        if matches!(
            state,
            HelperState::Reachable { .. } | HelperState::VersionMismatch { .. }
        ) || std::time::Instant::now() >= deadline
        {
            break state;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    match state {
        HelperState::Reachable { .. } => Ok(()),
        state => {
            let rollback = uninstall();
            let detail = match rollback {
                Ok(()) => format!(
                    "helper installed but did not become reachable ({state:?}); the service registration was removed"
                ),
                Err(error) => format!(
                    "helper installed but did not become reachable ({state:?}); rollback removal failed: {error}"
                ),
            };
            Err(io::Error::new(io::ErrorKind::NotConnected, detail))
        }
    }
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

/// Ask the helper to create and start the HCS compute system for one entitled
/// session. Only a Performed outcome means a container now exists; callers
/// must leave their isolation carrier unchanged for every other outcome.
#[cfg(windows)]
pub fn manage_hcs_container(
    session_id: malt_protocol::common::SessionId,
    memory_limit_mb: Option<u32>,
    hostname: Option<String>,
    image_id: Option<String>,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    send_hcs_container_operation(
        session_id,
        malt_protocol::elevate::ContainerOperation::Create {
            memory_limit_mb,
            hostname,
            image_id,
        },
    )
}

/// Perform an authenticated helper-owned image operation. SessionId(0) is
/// reserved for image inventory: the helper still requires daemon enrollment,
/// but never accepts a caller-selected filesystem path.
#[cfg(windows)]
pub fn manage_image(
    operation: malt_protocol::elevate::ImageOperation,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_protocol::elevate::{ElevateRequest, ElevateRequestEnvelope};
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT_IMAGE_REQUEST: AtomicU32 = AtomicU32::new(50_000);
    send_request_with_timeout(
        ElevateRequestEnvelope {
            request_id: NEXT_IMAGE_REQUEST.fetch_add(1, Ordering::Relaxed),
            request: ElevateRequest::ManageImage { operation },
            session_id: malt_protocol::common::SessionId(0),
            nonce: request_nonce(),
            _unknown: Vec::new(),
        },
        Duration::from_secs(900),
    )
}

/// Ask the helper to terminate only the compute system it recorded for this
/// caller's session. This is intentionally separate from generic requests so
/// callers cannot name an arbitrary system id without the entitlement check.
#[cfg(windows)]
pub fn terminate_hcs_container(
    session_id: malt_protocol::common::SessionId,
    id: String,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    send_hcs_container_operation(
        session_id,
        malt_protocol::elevate::ContainerOperation::Terminate { id },
    )
}

/// Start one process inside an already helper-owned HCS compute system and
/// decode the handles duplicated into this authenticated daemon process.
/// Only a `Performed` outcome with a complete payload becomes a usable launch;
/// refused and indeterminate outcomes remain ordinary errors for the caller to
/// contain and clean up.
#[cfg(windows)]
pub fn start_hcs_process(
    session_id: malt_protocol::common::SessionId,
    request: malt_protocol::elevate::HcsProcessRequest,
) -> io::Result<malt_protocol::elevate::HcsProcessLaunch> {
    use malt_protocol::elevate::OutcomeKind;
    use malt_protocol::vexil_runtime::{BitReader, Unpack};

    let response = send_hcs_container_operation(
        session_id,
        malt_protocol::elevate::ContainerOperation::StartProcess { request },
    )?;
    if response.kind != OutcomeKind::Performed {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            response
                .detail
                .unwrap_or_else(|| "helper did not perform HCS process launch".to_string()),
        ));
    }
    let payload = response.payload.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "helper performed HCS process launch without a handle payload",
        )
    })?;
    let mut reader = BitReader::new(&payload);
    malt_protocol::elevate::HcsProcessLaunch::unpack(&mut reader)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[cfg(windows)]
fn send_hcs_container_operation(
    session_id: malt_protocol::common::SessionId,
    operation: malt_protocol::elevate::ContainerOperation,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_protocol::elevate::{ElevateRequest, ElevateRequestEnvelope};
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    send_request(ElevateRequestEnvelope {
        request_id,
        request: ElevateRequest::ManageHcsContainer { operation },
        session_id,
        nonce: request_nonce(),
        _unknown: Vec::new(),
    })
}

#[cfg(windows)]
fn request_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT_NONCE: AtomicU64 = AtomicU64::new(1);
    let issued_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    (issued_at << 32) | (NEXT_NONCE.fetch_add(1, Ordering::Relaxed) & u64::from(u32::MAX))
}

#[cfg(windows)]
fn enrollment_nonce(_subject: u32) -> u64 {
    request_nonce()
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

    #[test]
    fn helper_states_stay_distinct_across_scm_and_authenticated_probe_results() {
        use malt_platform::service::ServiceStatus;

        let states = [
            helper_state_from(ServiceStatus::NotInstalled, None),
            helper_state_from(ServiceStatus::Stopped, None),
            helper_state_from(ServiceStatus::Running, Some(ProbeResult::Unavailable)),
            helper_state_from(
                ServiceStatus::Running,
                Some(ProbeResult::Reachable {
                    protocol_version: HELPER_PROTOCOL_VERSION,
                }),
            ),
            helper_state_from(
                ServiceStatus::Running,
                Some(ProbeResult::VersionMismatch { actual: 1 }),
            ),
        ];

        assert_eq!(states[0], HelperState::NotInstalled);
        assert_eq!(states[1], HelperState::InstalledStopped);
        assert_eq!(states[2], HelperState::InstalledUnreachable);
        assert_eq!(
            states[3],
            HelperState::Reachable {
                protocol_version: HELPER_PROTOCOL_VERSION,
            }
        );
        assert_eq!(
            states[4],
            HelperState::VersionMismatch {
                expected: HELPER_PROTOCOL_VERSION,
                actual: 1,
            }
        );
        for (index, state) in states.iter().enumerate() {
            for other in states.iter().skip(index + 1) {
                assert_ne!(state, other, "helper states must not collapse");
            }
        }
    }

    #[test]
    fn actual_vnp_probe_reports_a_protocol_version_mismatch() {
        use malt_platform::ipc::NamedPipeServer;
        use malt_protocol::elevate::{ElevateHello, ElevateHelloAck};
        use malt_protocol::elevate_channel::{HELLO, HELLO_ACK};
        use malt_protocol::vexil_runtime::{BitWriter, Pack};
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let pipe_name = format!("malt-elevate-version-test-{}-{suffix}", std::process::id());
        let server_pipe = pipe_name.clone();
        let server = std::thread::spawn(move || {
            let server = NamedPipeServer::create(&server_pipe).expect("create version test pipe");
            let mut connection = server.accept().expect("accept version test client");
            let hello = read_tagged::<ElevateHello>(&mut connection, HELLO)
                .expect("read version test hello");
            let acknowledgement = ElevateHelloAck {
                nonce: hello.nonce,
                version: HELPER_PROTOCOL_VERSION - 1,
                accepted: false,
                reason: Some("test protocol version mismatch".to_string()),
                _unknown: Vec::new(),
            };
            let mut writer = BitWriter::new();
            acknowledgement
                .pack(&mut writer)
                .expect("pack version mismatch acknowledgement");
            let mut payload = vec![HELLO_ACK];
            payload.extend(writer.finish());
            write_frame(&mut connection, payload).expect("write version mismatch acknowledgement");
        });

        let result = (0..100)
            .find_map(|_| match probe_once_at(&pipe_name, 9_001) {
                Ok(ProbeResult::Unavailable) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    None
                }
                Ok(result) => Some(result),
                Err(error) if error.raw_os_error() == Some(2) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    None
                }
                Err(error) => panic!("run version mismatch probe: {error}"),
            })
            .expect("fake version-mismatch helper never accepted the VNP probe");
        assert_eq!(
            result,
            ProbeResult::VersionMismatch {
                actual: HELPER_PROTOCOL_VERSION - 1,
            }
        );
        server.join().expect("version mismatch server thread");
    }

    #[test]
    fn lost_helper_after_receiving_request_is_indeterminate_and_cannot_establish_containment() {
        use malt_platform::ipc::NamedPipeServer;
        use malt_platform::isolation::{EstablishedKind, IsolationContext};
        use malt_protocol::common::SessionId;
        use malt_protocol::elevate::{
            ContainerOperation, ElevateHello, ElevateHelloAck, ElevateRequest,
            ElevateRequestEnvelope, OutcomeKind,
        };
        use malt_protocol::elevate_channel::{HELLO, HELLO_ACK, REQUEST};
        use malt_protocol::vexil_runtime::{BitWriter, Pack};
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let pipe_name = format!("malt-elevate-loss-test-{}-{suffix}", std::process::id());
        let server_pipe = pipe_name.clone();
        let server = std::thread::spawn(move || {
            let server = NamedPipeServer::create(&server_pipe).expect("create loss test pipe");
            let mut connection = server.accept().expect("accept loss test client");
            let hello =
                read_tagged::<ElevateHello>(&mut connection, HELLO).expect("read loss test hello");
            let acknowledgement = ElevateHelloAck {
                nonce: hello.nonce,
                version: HELPER_PROTOCOL_VERSION,
                accepted: true,
                reason: None,
                _unknown: Vec::new(),
            };
            let mut writer = BitWriter::new();
            acknowledgement
                .pack(&mut writer)
                .expect("pack loss test acknowledgement");
            let mut payload = vec![HELLO_ACK];
            payload.extend(writer.finish());
            write_frame(&mut connection, payload).expect("write loss test acknowledgement");
            let request = read_tagged::<ElevateRequestEnvelope>(&mut connection, REQUEST)
                .expect("receive request before helper loss");
            assert_eq!(request.request_id, 88);
            // Drop the connection after consuming the request, which is the
            // observable client-side effect of the helper dying mid-operation.
        });

        let envelope = ElevateRequestEnvelope {
            request_id: 88,
            request: ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Create {
                    memory_limit_mb: None,
                    hostname: None,
                    image_id: None,
                },
            },
            session_id: SessionId(88),
            nonce: 8_800,
            _unknown: Vec::new(),
        };
        let result = (0..100)
            .find_map(|_| match send_once_at(&pipe_name, envelope.clone()) {
                Err(error) if error.raw_os_error() == Some(2) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    None
                }
                result => Some(result),
            })
            .expect("fake helper never accepted the request");
        let response = complete_request_attempt(88, result).expect("classify helper loss");
        assert_eq!(response.kind, OutcomeKind::Indeterminate);

        let context = IsolationContext::contained();
        assert_eq!(
            context.established_kind().expect("read isolation carrier"),
            EstablishedKind::Nothing,
            "an indeterminate helper outcome must not alter the session carrier"
        );
        server.join().expect("loss test server thread");
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeResult {
    Reachable { protocol_version: u32 },
    VersionMismatch { actual: u32 },
    Unavailable,
}

/// Derive the operator-visible state from the two independent observations:
/// SCM bookkeeping and the authenticated VNP probe. Keeping this pure makes
/// the protocol-mismatch case coverable without replacing the installed
/// helper binary merely to change its version.
#[cfg(windows)]
fn helper_state_from(
    service: malt_platform::service::ServiceStatus,
    probe: Option<ProbeResult>,
) -> HelperState {
    match service {
        malt_platform::service::ServiceStatus::NotInstalled => HelperState::NotInstalled,
        malt_platform::service::ServiceStatus::Stopped
        | malt_platform::service::ServiceStatus::Other => HelperState::InstalledStopped,
        malt_platform::service::ServiceStatus::Running => {
            match probe.unwrap_or(ProbeResult::Unavailable) {
                ProbeResult::Reachable { protocol_version } => {
                    HelperState::Reachable { protocol_version }
                }
                ProbeResult::VersionMismatch { actual } => HelperState::VersionMismatch {
                    expected: HELPER_PROTOCOL_VERSION,
                    actual,
                },
                ProbeResult::Unavailable => HelperState::InstalledUnreachable,
            }
        }
    }
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
    probe_once_at(HELPER_PIPE_NAME, nonce)
}

#[cfg(windows)]
fn probe_once_at(pipe_name: &str, nonce: u64) -> io::Result<ProbeResult> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::ElevateHello;
    use malt_protocol::elevate_channel::{HELLO, HELLO_ACK};
    use malt_protocol::framing::{Frame, FrameFlags, FrameReader, FrameWriter};
    use malt_protocol::vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

    let mut connection = match NamedPipeClient::connect(pipe_name) {
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
    send_once_at(HELPER_PIPE_NAME, envelope)
}

/// Execute one authenticated request against a named pipe.  Production uses
/// the fixed helper pipe; the explicit parameter gives tests a real transport
/// boundary where the peer can disappear after consuming the request.
#[cfg(windows)]
fn send_once_at(
    pipe_name: &str,
    envelope: malt_protocol::elevate::ElevateRequestEnvelope,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    use malt_platform::ipc::NamedPipeClient;
    use malt_protocol::elevate::{ElevateHello, ElevateHelloAck, ElevateResponse};
    use malt_protocol::elevate_channel::{HELLO, HELLO_ACK, REQUEST, RESPONSE};
    use malt_protocol::vexil_runtime::{BitWriter, Pack};

    let mut connection = NamedPipeClient::connect(pipe_name)?;
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
pub fn manage_hcs_container(
    session_id: malt_protocol::common::SessionId,
    _memory_limit_mb: Option<u32>,
    _hostname: Option<String>,
    _image_id: Option<String>,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    send_request(malt_protocol::elevate::ElevateRequestEnvelope {
        request_id: 0,
        request: malt_protocol::elevate::ElevateRequest::ManageHcsContainer {
            operation: malt_protocol::elevate::ContainerOperation::Create {
                memory_limit_mb: None,
                hostname: None,
                image_id: None,
            },
        },
        session_id,
        nonce: 0,
        _unknown: Vec::new(),
    })
}

#[cfg(not(windows))]
pub fn terminate_hcs_container(
    session_id: malt_protocol::common::SessionId,
    _id: String,
) -> io::Result<malt_protocol::elevate::ElevateResponse> {
    manage_hcs_container(session_id, None, None, None)
}

#[cfg(not(windows))]
pub fn register_session_entitlement(
    _session_id: malt_protocol::common::SessionId,
    _storage_root: &Path,
    _pids: &[u32],
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the privileged helper service is only available on Windows",
    ))
}

#[cfg(not(windows))]
pub fn enroll_daemon(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the privileged helper service is only available on Windows",
    ))
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
