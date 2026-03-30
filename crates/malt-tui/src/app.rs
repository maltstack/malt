// Application state for the TUI client.

use crate::render::TuiRenderer;
use malt_protocol::render::RenderCommand;

/// Application state for the TUI client.
///
/// Owns the renderer and tracks whether the event loop should continue.
pub struct App {
    renderer: TuiRenderer,
    running: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            renderer: TuiRenderer::new(),
            running: true,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Process incoming render commands and apply them to the buffer.
    pub fn process_commands(&mut self, commands: &[RenderCommand], buf: &mut ratatui::buffer::Buffer) {
        self.renderer.apply(commands, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
