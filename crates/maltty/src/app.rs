use crate::connection::HttpConnection;
use crate::renderer::GpuRenderer;
use std::sync::Arc;
use winit::event::{ElementState, KeyEvent};
use winit::window::Window;

/// A styled span of text for rendering.
///
/// Fields are populated by the connection layer and will be consumed
/// by the GPU text renderer once glyph atlas rendering is implemented.
#[allow(dead_code)]
pub struct StyledSpan {
    pub text: String,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
}

/// Top-level application state: owns the window, GPU renderer, and daemon connection.
pub struct App {
    window: Arc<Window>,
    renderer: GpuRenderer,
    connection: HttpConnection,
    lines: Vec<Vec<StyledSpan>>,
}

impl App {
    pub fn new(window: Window, api_addr: &str, session_id: u32) -> anyhow::Result<Self> {
        let window = Arc::new(window);
        let renderer = pollster::block_on(GpuRenderer::new(window.clone()))?;
        let connection = HttpConnection::new(api_addr, session_id);
        Ok(Self {
            window,
            renderer,
            connection,
            lines: Vec::new(),
        })
    }

    pub fn window(&self) -> Option<&Window> {
        Some(&self.window)
    }

    pub fn render(&mut self) {
        self.renderer.render(&self.lines);
    }

    pub fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.renderer.resize(size);
    }

    pub fn poll_output(&mut self) {
        if let Some(new_lines) = self.connection.poll_styled() {
            self.lines = new_lines;
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let text = crate::input::map_key(&event);
        if let Some(text) = text {
            self.connection.send_input(&text);
        }
    }
}
