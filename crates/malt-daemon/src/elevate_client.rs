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
