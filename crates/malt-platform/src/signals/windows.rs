//! Windows signal implementation.
//!
//! Maps POSIX signals to Win32 equivalents:
//! - `Int` → `GenerateConsoleCtrlEvent(CTRL_C_EVENT)`
//! - `Term` / `Quit` → `OpenProcess(PROCESS_TERMINATE)` + `TerminateProcess()`
//! - Others → `SignalError::Unsupported`

use super::{signal_number, SignalError, SignalKind};

/// Send a signal to the process identified by `pid`.
///
/// On Windows only a subset of POSIX signals are meaningful:
/// - `Int` generates a console Ctrl+C event
/// - `Term` and `Quit` terminate the process with exit code 128+signum
pub fn send_signal(pid: u32, signal: SignalKind) -> Result<(), SignalError> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_C_EVENT};
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    match signal {
        SignalKind::Int => {
            // SAFETY: GenerateConsoleCtrlEvent sends CTRL_C_EVENT to the
            // process group identified by `pid`. Returns 0 on failure.
            let result = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid) };
            if result == 0 {
                let err = std::io::Error::last_os_error();
                return match err.raw_os_error() {
                    // ERROR_INVALID_PARAMETER (87) when PID doesn't exist
                    Some(87) => Err(SignalError::NoSuchProcess { pid }),
                    _ => Err(SignalError::Io(err)),
                };
            }
            Ok(())
        }
        SignalKind::Term | SignalKind::Quit => {
            // Use 128+signal_number as the exit code so that POSIX `wait`
            // semantics work: $? after signalled process should be >= 128.
            let exit_code: u32 = 128 + signal_number(signal) as u32;

            // SAFETY: OpenProcess with PROCESS_TERMINATE access right
            // returns a valid handle or null on failure.
            let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
            if handle.is_null() {
                let err = std::io::Error::last_os_error();
                return match err.raw_os_error() {
                    // ERROR_INVALID_PARAMETER (87) when PID doesn't exist
                    Some(87) => Err(SignalError::NoSuchProcess { pid }),
                    Some(5) => Err(SignalError::PermissionDenied { pid }),
                    _ => Err(SignalError::Io(err)),
                };
            }

            // SAFETY: handle is valid with PROCESS_TERMINATE access.
            let result = unsafe { TerminateProcess(handle, exit_code) };

            // SAFETY: handle was obtained from OpenProcess and must be closed.
            unsafe { CloseHandle(handle) };

            if result == 0 {
                let err = std::io::Error::last_os_error();
                return Err(SignalError::Io(err));
            }
            Ok(())
        }
        other => Err(SignalError::Unsupported { signal: other }),
    }
}

/// Check whether a process with the given PID exists.
pub fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: OpenProcess is a standard Win32 call; the handle is checked.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: handle is valid and was just opened; must be closed.
    unsafe { CloseHandle(handle) };
    true
}

// ── WindowsSignalBroker (tokio feature) ──────────────────────────────────

#[cfg(feature = "tokio")]
pub struct WindowsSignalBroker {
    tx: tokio::sync::broadcast::Sender<SignalKind>,
}

#[cfg(feature = "tokio")]
impl WindowsSignalBroker {
    /// Default capacity of the signal broadcast channel.
    const BROADCAST_CAPACITY: usize = 64;

    /// Create a new broker and start listening for `Ctrl+C`.
    ///
    /// Must be called from within a tokio runtime.
    pub fn start() -> Result<Self, SignalError> {
        let (tx, _) = tokio::sync::broadcast::channel(Self::BROADCAST_CAPACITY);
        let broker = Self { tx };
        broker.spawn_listener();
        Ok(broker)
    }

    fn spawn_listener(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                match tokio::signal::ctrl_c().await {
                    Ok(()) => {
                        let _ = tx.send(SignalKind::Int);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "ctrl_c handler failed");
                        return;
                    }
                }
            }
        });
    }
}

#[cfg(feature = "tokio")]
impl super::SignalBroker for WindowsSignalBroker {
    fn send(&self, pid: u32, signal: SignalKind) -> Result<(), SignalError> {
        send_signal(pid, signal)
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SignalKind> {
        self.tx.subscribe()
    }
}
