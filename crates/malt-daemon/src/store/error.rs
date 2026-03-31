#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("session not found: {0:?}")]
    SessionNotFound(malt_protocol::common::SessionId),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("corrupt file at {}: {reason} (moved to {})", path.display(), moved_to.display())]
    CorruptFile {
        path: std::path::PathBuf,
        reason: String,
        moved_to: std::path::PathBuf,
    },
}
