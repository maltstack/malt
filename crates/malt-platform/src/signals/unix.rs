//! Unix signal implementation via nix.

use super::{SignalError, SignalKind};

/// Convert a [`SignalKind`] to the corresponding `nix::sys::signal::Signal`.
///
/// This is the **single source of truth** for `SignalKind → nix::Signal`
/// mapping on Unix.
fn to_nix(signal: SignalKind) -> Result<nix::sys::signal::Signal, SignalError> {
    use nix::sys::signal::Signal as NixSignal;
    match signal {
        SignalKind::Int => Ok(NixSignal::SIGINT),
        SignalKind::Tstp => Ok(NixSignal::SIGTSTP),
        SignalKind::Quit => Ok(NixSignal::SIGQUIT),
        SignalKind::Term => Ok(NixSignal::SIGTERM),
        SignalKind::Hup => Ok(NixSignal::SIGHUP),
        SignalKind::Usr1 => Ok(NixSignal::SIGUSR1),
        SignalKind::Usr2 => Ok(NixSignal::SIGUSR2),
        other => Err(SignalError::Unsupported { signal: other }),
    }
}

/// Send a signal to the process identified by `pid`.
pub fn send_signal(pid: u32, signal: SignalKind) -> Result<(), SignalError> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let nix_signal = to_nix(signal)?;
    let raw_pid = i32::try_from(pid).map_err(|_| SignalError::NoSuchProcess { pid })?;

    kill(Pid::from_raw(raw_pid), nix_signal).map_err(|e| match e {
        nix::errno::Errno::ESRCH => SignalError::NoSuchProcess { pid },
        nix::errno::Errno::EPERM => SignalError::PermissionDenied { pid },
        other => SignalError::Io(std::io::Error::from_raw_os_error(other as i32)),
    })
}

/// Check whether a process with the given PID exists.
pub fn process_exists(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let raw_pid = match i32::try_from(pid) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // kill(pid, None) sends signal 0: checks existence without sending.
    kill(Pid::from_raw(raw_pid), None).is_ok()
}

// ── UnixSignalBroker (tokio feature) ─────────────────────────────────────

#[cfg(feature = "tokio")]
pub struct UnixSignalBroker {
    tx: tokio::sync::broadcast::Sender<SignalKind>,
}

#[cfg(feature = "tokio")]
impl UnixSignalBroker {
    /// Default capacity of the signal broadcast channel.
    const BROADCAST_CAPACITY: usize = 64;

    /// Create a new broker and start listening for OS signals.
    ///
    /// Must be called from within a tokio runtime.
    pub fn start() -> Result<Self, SignalError> {
        let (tx, _) = tokio::sync::broadcast::channel(Self::BROADCAST_CAPACITY);
        let broker = Self { tx };
        broker.spawn_listeners();
        Ok(broker)
    }

    fn spawn_listeners(&self) {
        use tokio::signal::unix::{signal, SignalKind as TokioSignalKind};

        let signals = [
            (TokioSignalKind::interrupt(), SignalKind::Int),
            (TokioSignalKind::terminate(), SignalKind::Term),
            (TokioSignalKind::hangup(), SignalKind::Hup),
            (TokioSignalKind::quit(), SignalKind::Quit),
            (
                TokioSignalKind::from_raw(nix::sys::signal::Signal::SIGUSR1 as i32),
                SignalKind::Usr1,
            ),
            (
                TokioSignalKind::from_raw(nix::sys::signal::Signal::SIGUSR2 as i32),
                SignalKind::Usr2,
            ),
        ];

        for (kind, mapped) in signals {
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let mut stream = match signal(kind) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            signal = ?mapped,
                            error = %e,
                            "failed to register signal handler"
                        );
                        return;
                    }
                };
                loop {
                    stream.recv().await;
                    let _ = tx.send(mapped);
                }
            });
        }
    }
}

#[cfg(feature = "tokio")]
impl super::SignalBroker for UnixSignalBroker {
    fn send(&self, pid: u32, signal: SignalKind) -> Result<(), SignalError> {
        send_signal(pid, signal)
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SignalKind> {
        self.tx.subscribe()
    }
}
