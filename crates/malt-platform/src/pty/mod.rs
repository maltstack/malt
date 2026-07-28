//! PTY abstraction — open, read/write, resize.

pub mod spawn;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::sync::Arc;

pub use spawn::{spawn_with_pty, PtyProcess, PtySpawnError};

/// PTY window size in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
}

/// Handle to a pseudo-terminal that supports resize.
pub trait Pty: Send + Sync {
    /// Resize the terminal to the given dimensions.
    fn resize(&self, size: WinSize) -> Result<(), PtyError>;
}

/// Open a new PTY pair.
///
/// Returns `(pty_handle, reader, writer)` with independent ownership:
/// - `pty_handle` — resize control (implements [`Pty`])
/// - `reader` — reads output from the child process
/// - `writer` — writes input to the child process
pub fn open_pty(
    size: WinSize,
) -> Result<
    (
        Arc<dyn Pty>,
        std::fs::File,
        std::fs::File,
        Option<std::fs::File>,
    ),
    PtyError,
> {
    #[cfg(unix)]
    {
        unix::open_pty(size)
    }
    #[cfg(windows)]
    {
        windows::open_pty(size)
    }
}

/// Errors that can occur during PTY operations.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    /// Failed to open a new PTY pair.
    #[error("failed to open PTY: {0}")]
    Open(std::io::Error),
    /// Failed to resize the PTY.
    #[error("failed to resize PTY: {0}")]
    Resize(std::io::Error),
    /// Windows ConPTY returned a failing HRESULT.
    #[cfg(windows)]
    #[error("ConPTY error: HRESULT {code:#010x}")]
    ConPty { code: u32 },
}
