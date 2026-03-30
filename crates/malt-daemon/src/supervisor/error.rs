use malt_protocol::common::PaneId;

/// Errors that can occur during process supervision.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SupervisorError {
    #[error("process not found for pane: {0:?}")]
    ProcessNotFound(PaneId),

    #[error("spawn failed: {0}")]
    SpawnFailed(#[from] malt_platform::process::SpawnError),

    #[error("pty error: {0}")]
    PtyError(#[from] malt_platform::pty::PtyError),

    #[error("restart limit exceeded for pane: {0:?}")]
    RestartLimitExceeded(PaneId),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
