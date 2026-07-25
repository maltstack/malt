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

    /// Draw a one-line notice across the bottom of the screen.
    ///
    /// Used for things the session tells the client about itself rather than
    /// content the session produced -- losing input authority, most of all.
    /// It is written straight to the buffer rather than pushed through the
    /// render pipeline because it is not session output and must not be
    /// mistaken for it in scrollback.
    pub fn show_notice(&mut self, notice: &str, buf: &mut ratatui::buffer::Buffer) {
        let area = *buf.area();
        if area.height == 0 || area.width == 0 {
            return;
        }
        let row = area.height.saturating_sub(1);
        let style = ratatui::style::Style::default()
            .fg(ratatui::style::Color::Black)
            .bg(ratatui::style::Color::Yellow);
        for (offset, ch) in notice.chars().enumerate() {
            let col = offset as u16;
            if col >= area.width {
                break;
            }
            let cell = &mut buf[(col, row)];
            cell.set_char(ch);
            cell.set_style(style);
        }
    }

    /// Process incoming render commands and apply them to the buffer.
    pub fn process_commands(
        &mut self,
        commands: &[RenderCommand],
        buf: &mut ratatui::buffer::Buffer,
    ) {
        self.renderer.apply(commands, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
