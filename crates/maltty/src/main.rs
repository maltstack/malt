mod app;
mod connection;
mod input;
mod renderer;
mod text;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct Maltty {
    app: Option<app::App>,
    api_addr: String,
    session_id: u32,
}

impl ApplicationHandler for Maltty {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none() {
            let window = match event_loop.create_window(
                Window::default_attributes()
                    .with_title("maltty")
                    .with_inner_size(winit::dpi::LogicalSize::new(1024, 768)),
            ) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("failed to create window: {e}");
                    event_loop.exit();
                    return;
                }
            };
            match app::App::new(window, &self.api_addr, self.session_id) {
                Ok(app) => self.app = Some(app),
                Err(error) => {
                    eprintln!("failed to initialize GPU renderer: {error}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(app) = &mut self.app else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => app.render(),
            WindowEvent::Resized(size) => app.resize(size),
            WindowEvent::KeyboardInput { event, .. } => app.handle_key(event),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &mut self.app {
            app.poll_output();
            if let Some(window) = app.window() {
                window.request_redraw();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Parse simple args: maltty [--session ID] [--api-addr ADDR]
    let mut session_id: u32 = 1;
    let mut api_addr = "http://127.0.0.1:7700".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--session" if i + 1 < args.len() => {
                session_id = args[i + 1].parse().unwrap_or(1);
                i += 2;
            }
            "--api-addr" if i + 1 < args.len() => {
                api_addr = args[i + 1].clone();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let event_loop = EventLoop::new()?;
    let mut maltty = Maltty {
        app: None,
        api_addr,
        session_id,
    };
    event_loop.run_app(&mut maltty)?;
    Ok(())
}
