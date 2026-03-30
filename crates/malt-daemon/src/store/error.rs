#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("session not found: {0:?}")]
    SessionNotFound(malt_protocol::common::SessionId),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("corrupt file: {path}")]
    CorruptFile { path: String },
}
