use malt_session::session::SessionError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    #[error("session not found: {0:?}")]
    SessionNotFound(malt_protocol::common::SessionId),

    #[error("session unreachable (thread may have panicked): {0:?}")]
    SessionUnreachable(malt_protocol::common::SessionId),

    #[error("session error: {0}")]
    Session(#[from] SessionError),

    #[error("transport error: {0}")]
    Transport(#[from] malt_platform::sockets::TransportError),

    #[error("handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("bus full for subscriber {0}")]
    BusFull(u64),

    #[error("frame error: {0}")]
    Frame(#[from] malt_protocol::framing::FrameError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
