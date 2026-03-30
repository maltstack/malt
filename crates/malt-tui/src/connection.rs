// Connection module: trait + mock for receiving render commands from the daemon.

use malt_protocol::render::RenderCommand;

/// Trait for receiving render commands from the daemon.
pub trait DaemonConnection {
    /// Get the next batch of render commands (non-blocking).
    /// Returns `None` if no new commands are available.
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>>;
}

/// Mock connection that returns a static set of commands once.
pub struct MockConnection {
    commands: Option<Vec<RenderCommand>>,
}

impl MockConnection {
    pub fn new(commands: Vec<RenderCommand>) -> Self {
        Self {
            commands: Some(commands),
        }
    }

    pub fn empty() -> Self {
        Self { commands: None }
    }
}

impl DaemonConnection for MockConnection {
    fn poll_commands(&mut self) -> Option<Vec<RenderCommand>> {
        self.commands.take()
    }
}
