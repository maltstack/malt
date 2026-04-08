mod app;
mod connection;
mod input;
mod render;
mod style;

use app::App;
use connection::{DaemonConnection, HttpConnection, MockConnection, VnpConnection};
use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Parse simple args: malt-tui [--session ID] [--api-addr ADDR] [--vnp ADDR]
    let mut session_id: Option<u32> = None;
    let mut api_addr = "http://127.0.0.1:7700".to_string();
    let mut vnp_addr: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--session" if i + 1 < args.len() => {
                session_id = args[i + 1].parse().ok();
                i += 2;
            }
            "--api-addr" if i + 1 < args.len() => {
                api_addr = args[i + 1].clone();
                i += 2;
            }
            "--vnp" if i + 1 < args.len() => {
                vnp_addr = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Setup terminal
    let mut terminal = ratatui::try_init()?;

    let result = if let Some(vnp) = vnp_addr {
        // VNP socket connection
        let sid = session_id.unwrap_or(1);
        let conn = VnpConnection::connect(&vnp, sid)?;
        run_loop(&mut terminal, conn)
    } else if let Some(sid) = session_id {
        // HTTP connection to daemon
        let conn = HttpConnection::new(&api_addr, sid);
        run_loop(&mut terminal, conn)
    } else {
        // Demo mode with mock data
        let conn = mock_connection();
        run_loop(&mut terminal, conn)
    };

    // Restore terminal
    ratatui::try_restore()?;

    result
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut conn: impl DaemonConnection,
) -> anyhow::Result<()> {
    let mut app = App::new();

    // Initial render — poll once immediately
    if let Some(commands) = conn.poll_commands() {
        terminal.draw(|frame| {
            app.process_commands(&commands, frame.buffer_mut());
        })?;
    }

    while app.is_running() {
        // Handle input (50ms poll — responsive typing)
        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    // Ctrl+Q or Ctrl+C to quit
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('c'))
                    {
                        app.quit();
                        continue;
                    }
                    if let Some(vnp_key) = input::convert_key_event(&key) {
                        conn.send_key_event(&vnp_key);
                    }
                }
                Event::Resize(cols, rows) => {
                    conn.send_resize(cols, rows);
                }
                _ => {}
            }
        }

        // Poll for new output from daemon
        if let Some(commands) = conn.poll_commands() {
            terminal.draw(|frame| {
                app.process_commands(&commands, frame.buffer_mut());
            })?;
        }
    }

    Ok(())
}

fn mock_connection() -> MockConnection {
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
        token_name: None,
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
        token_name: None,
        _unknown: Vec::new(),
    };
    MockConnection::new(vec![
        RenderCommand::DrawText {
            x: 2,
            y: 1,
            text: "MALT Terminal (malt-tui)".to_string(),
            style: title_style,
        },
        RenderCommand::DrawText {
            x: 2,
            y: 3,
            text: "Press Ctrl+Q to quit".to_string(),
            style: body_style.clone(),
        },
        RenderCommand::DrawText {
            x: 2,
            y: 4,
            text: "Run with --session ID --api-addr URL to connect to daemon".to_string(),
            style: body_style,
        },
    ])
}
