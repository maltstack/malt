//! Signal abstraction — send, subscribe, name/number lookups.
//!
//! Provides cross-platform signal delivery and a mapping table between
//! [`SignalKind`] variants, canonical names, and POSIX signal numbers.
//! Platform-specific implementations live in sibling modules; this module
//! contains the public API and platform-independent logic.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use thiserror::Error;

pub use malt_protocol::common::SignalKind;

#[cfg(unix)]
pub use unix::process_exists;
#[cfg(unix)]
pub use unix::send_signal;
#[cfg(unix)]
pub use unix::terminate_process;

#[cfg(windows)]
pub use windows::process_exists;
#[cfg(windows)]
pub use windows::send_signal;
#[cfg(windows)]
pub use windows::terminate_process;

// ── Error type ───────────────────────────────────────────────────────────

/// Errors that can occur when sending or resolving signals.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SignalError {
    /// The target process does not exist.
    #[error("no such process (pid {pid})")]
    NoSuchProcess { pid: u32 },

    /// The caller lacks permission to signal the target process.
    #[error("permission denied signalling pid {pid}")]
    PermissionDenied { pid: u32 },

    /// The requested signal is not supported on this platform.
    #[error("signal {signal:?} is not supported on this platform")]
    Unsupported { signal: SignalKind },

    /// An OS-level I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ── Signal mapping table ─────────────────────────────────────────────────

/// Signal mapping entry: (canonical name, POSIX number, variant).
///
/// This is the **single source of truth** for name/number resolution.
/// POSIX standard numbers are used on all platforms.
const SIGNAL_TABLE: &[(&str, i32, SignalKind)] = &[
    ("HUP", 1, SignalKind::Hup),
    ("INT", 2, SignalKind::Int),
    ("QUIT", 3, SignalKind::Quit),
    ("TERM", 15, SignalKind::Term),
    ("USR1", 10, SignalKind::Usr1),
    ("USR2", 12, SignalKind::Usr2),
    ("TSTP", 20, SignalKind::Tstp),
];

/// Parse a signal name or number string into a [`SignalKind`].
///
/// Accepts names with or without a `SIG` prefix (e.g. `"TERM"`, `"SIGTERM"`)
/// and POSIX signal numbers as strings (e.g. `"15"`). Case-insensitive.
///
/// This is the **single source of truth** for signal-name resolution in the
/// workspace. All crates must call this instead of rolling their own mapping.
pub fn signal_by_name(s: &str) -> Option<SignalKind> {
    let raw = s.to_uppercase();
    let upper = raw.strip_prefix("SIG").unwrap_or(&raw);

    // Try by name first.
    for &(name, _, kind) in SIGNAL_TABLE {
        if upper == name {
            return Some(kind);
        }
    }

    // Try by number.
    if let Ok(num) = upper.parse::<i32>() {
        for &(_, number, kind) in SIGNAL_TABLE {
            if num == number {
                return Some(kind);
            }
        }
    }

    None
}

/// Return the canonical short name for a signal (e.g. `"TERM"`, `"USR1"`).
pub fn signal_name(signal: SignalKind) -> &'static str {
    for &(name, _, kind) in SIGNAL_TABLE {
        if kind == signal {
            return name;
        }
    }
    "UNKNOWN"
}

/// Return the POSIX signal number for a [`SignalKind`].
///
/// Standard POSIX numbers are used on all platforms for shell compatibility.
pub fn signal_number(signal: SignalKind) -> i32 {
    for &(_, number, kind) in SIGNAL_TABLE {
        if kind == signal {
            return number;
        }
    }
    0
}

// ── SignalBroker trait (tokio feature) ────────────────────────────────────

/// A broker that can send signals and provide a subscription stream.
///
/// The shell and daemon use this exclusively — they never call platform
/// signal APIs directly.
#[cfg(feature = "tokio")]
pub trait SignalBroker: Send + Sync {
    /// Send a signal to the process with the given PID.
    fn send(&self, pid: u32, signal: SignalKind) -> Result<(), SignalError>;

    /// Subscribe to signals received by this process.
    ///
    /// Returns a broadcast receiver that yields [`SignalKind`] values
    /// whenever the OS delivers a signal that the broker is listening for.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SignalKind>;
}

#[cfg(feature = "tokio")]
#[cfg(unix)]
pub use unix::UnixSignalBroker;

#[cfg(feature = "tokio")]
#[cfg(windows)]
pub use windows::WindowsSignalBroker;
