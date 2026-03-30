mod app;
mod connection;
mod input;
mod render;
mod style;

use app::App;
use connection::{DaemonConnection, MockConnection};
use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;
use ratatui::crossterm::event::{self, Event, KeyCode};

fn main() -> anyhow::Result<()> {
    // Setup terminal (raw mode + alternate screen + panic hook).
    let mut terminal = ratatui::try_init()?;

    // Create mock connection with demo content.
    let title_style = ResolvedStyle {
        fg: (0, 255, 128),
        bg: (0, 0, 0),
        bold: true,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        _unknown: Vec::new(),
    };
    let body_style = ResolvedStyle {
        fg: (180, 180, 180),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        _unknown: Vec::new(),
    };
    let mut conn = MockConnection::new(vec![
        RenderCommand::DrawText {
            x: 2,
            y: 1,
            text: "MALT Terminal (malt-tui)".to_string(),
            style: title_style,
        },
        RenderCommand::DrawText {
            x: 2,
            y: 3,
            text: "Press 'q' to quit".to_string(),
            style: body_style,
        },
    ]);

    let mut app = App::new();

    // Main loop.
    while app.is_running() {
        // Poll for new commands from the connection.
        if let Some(commands) = conn.poll_commands() {
            terminal.draw(|frame| {
                app.process_commands(&commands, frame.buffer_mut());
            })?;
        }

        // Handle input (100ms poll timeout).
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    app.quit();
                }
            }
        }
    }

    // Restore terminal.
    ratatui::try_restore()?;

    Ok(())
}
