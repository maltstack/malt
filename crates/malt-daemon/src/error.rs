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

    #[error("name conflict: cannot find unique name for '{0}' after 100 attempts")]
    NameConflict(String),

    #[error(
        "app pane restore is not supported until Phase G plugin infrastructure is implemented"
    )]
    AppRestoreNotSupported,

    #[error("failed to restore session {0:?}: {1}")]
    RestoreFailed(malt_protocol::common::SessionId, String),

    #[error("session {0:?} is dormant — attach to restore it")]
    SessionDormant(malt_protocol::common::SessionId),

    #[error("session {session_id:?} execution queue is full (capacity {capacity})")]
    ExecutionQueueFull {
        session_id: malt_protocol::common::SessionId,
        capacity: usize,
    },

    #[error("session {0:?} execution worker is unavailable")]
    ExecutionUnavailable(malt_protocol::common::SessionId),

    #[error("session {0:?} is shutting down and no longer accepts execution")]
    SessionShuttingDown(malt_protocol::common::SessionId),

    #[error("invalid pool configuration: {0}")]
    InvalidPoolConfig(String),

    #[error("session {0:?} input buffer is full; the command has not consumed prior input")]
    InputBufferFull(malt_protocol::common::SessionId),

    /// Input refused because the sender does not hold input authority.
    ///
    /// Carries the reason as text so the holder's identity survives to the
    /// caller; a bare "forbidden" would leave a client unable to decide
    /// whether to claim authority (FR-014, FR-015).
    #[error("{0}")]
    InputNotAuthorized(String),
}
