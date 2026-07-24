/// Configuration for shared thread pools.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Thread pool for WASM plugin execution.
    pub wasm_threads: usize,
    /// Thread pool for PTY read I/O.
    pub pty_io_threads: usize,
    /// Thread pool for disk I/O (persistence, scrollback).
    pub disk_io_threads: usize,
    /// Bounded channel capacity from coordinator to session.
    pub session_channel_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            wasm_threads: cpus.max(4) / 2,
            pty_io_threads: cpus.max(4),
            disk_io_threads: 4,
            session_channel_size: 256,
        }
    }
}

impl PoolConfig {
    /// Validate configuration that changes correctness, rather than merely
    /// tuning performance. A zero pending-execution capacity would make every
    /// session reject work and previously led to an opaque channel failure.
    pub fn validate(&self) -> Result<(), crate::DaemonError> {
        if self.session_channel_size == 0 {
            return Err(crate::DaemonError::InvalidPoolConfig(
                "session_channel_size must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}
