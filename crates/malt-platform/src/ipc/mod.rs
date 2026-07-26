//! Authenticated local inter-process transport primitives.
//!
//! The implementation lives here because named-pipe creation and peer
//! attribution are OS operations. Callers receive a normal `Read + Write`
//! connection plus an OS-attributed peer process identifier.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    current_process_principal, process_identity, NamedPipeClient, NamedPipeConnection,
    NamedPipeServer, PeerIdentity, ProcessIdentity,
};

#[cfg(not(windows))]
#[derive(Debug, thiserror::Error)]
#[error("named-pipe IPC is only available on Windows")]
pub struct UnsupportedPlatform;
