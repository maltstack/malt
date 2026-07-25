//! Transport abstraction — Unix sockets, named pipes, TCP.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::net::SocketAddr;
#[allow(unused_imports)]
use std::path::PathBuf;

/// Transport type for daemon connections.
pub enum Transport {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    #[cfg(windows)]
    NamedPipe(String),
    Tcp(SocketAddr),
}

impl Transport {
    /// Platform-appropriate default local transport.
    pub fn default_local() -> Self {
        #[cfg(unix)]
        {
            let path = crate::env::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".malt")
                .join("daemon.sock");
            Transport::UnixSocket(path)
        }
        #[cfg(windows)]
        {
            Transport::NamedPipe(r"\\.\pipe\malt-daemon".to_string())
        }
    }

    /// Serialize for MALT_SOCKET env var.
    pub fn to_env_string(&self) -> String {
        match self {
            #[cfg(unix)]
            Transport::UnixSocket(path) => path.to_string_lossy().to_string(),
            #[cfg(windows)]
            Transport::NamedPipe(name) => name.clone(),
            Transport::Tcp(addr) => format!("tcp://{addr}"),
        }
    }

    /// Parse from MALT_SOCKET env var.
    pub fn from_env_string(s: &str) -> Result<Self, TransportError> {
        if let Some(rest) = s.strip_prefix("tcp://") {
            let addr: SocketAddr = rest
                .parse()
                .map_err(|e| TransportError::InvalidAddress(format!("{e}")))?;
            return Ok(Transport::Tcp(addr));
        }

        #[cfg(unix)]
        {
            Ok(Transport::UnixSocket(PathBuf::from(s)))
        }
        #[cfg(windows)]
        {
            if s.starts_with(r"\\") {
                Ok(Transport::NamedPipe(s.to_string()))
            } else {
                Err(TransportError::InvalidAddress(format!(
                    "not a named pipe path: {s}"
                )))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("invalid transport address: {0}")]
    InvalidAddress(String),
    #[error("connection failed: {0}")]
    Io(#[from] std::io::Error),
}
