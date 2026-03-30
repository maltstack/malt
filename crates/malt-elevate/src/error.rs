//! Error types for the elevated helper.

/// Errors that can occur in the elevated helper binary.
#[derive(Debug, thiserror::Error)]
pub enum ElevateError {
    /// Authentication failed — nonce mismatch or missing nonce file.
    #[error("auth failed: {0}")]
    AuthFailed(String),

    /// Failed to read or parse the nonce file.
    #[error("nonce file error: {0}")]
    NonceFile(#[source] std::io::Error),

    /// IPC connection error.
    #[error("connection error: {0}")]
    Connection(#[source] std::io::Error),

    /// Protocol-level error (malformed frames, unexpected messages).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// An argument was invalid or missing.
    #[error("invalid argument: {0}")]
    InvalidArg(String),

    /// The requested operation failed.
    #[error("operation failed: {0}")]
    OperationFailed(String),
}
