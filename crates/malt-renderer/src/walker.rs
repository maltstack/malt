// FrameElement tree walker — emits RenderCommands from a FrameElement tree.

use malt_protocol::common::{ClientCapabilities, ColorDepth, Direction, ResolvedStyle, SplitSize};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::RenderCommand;

/// Configuration limits for the frame walker.
#[derive(Debug, Clone)]
pub struct WalkConfig {
    /// Maximum recursion depth before truncation.
    pub max_depth: usize,
    /// Maximum number of nodes visited before truncation.
    pub max_nodes: usize,
    /// Maximum estimated output size in bytes before truncation.
    pub max_output_bytes: usize,
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 10_000,
            max_output_bytes: 1_048_576,
        }
    }
}

/// Result of walking a FrameElement tree.
#[derive(Debug, Clone)]
pub struct WalkResult {
    /// Emitted render commands.
    pub commands: Vec<RenderCommand>,
    /// Total nodes visited during the walk.
    pub nodes_visited: usize,
    /// Whether the walk was truncated due to limits.
    pub truncated: bool,
}

/// Walks a `FrameElement` tree and produces `RenderCommand`s.
///
/// The walker traverses the element tree depth-first, translating semantic
/// elements into concrete drawing instructions. It respects client capabilities
/// for color degradation and VT passthrough, and enforces depth/node limits
/// to prevent pathological trees from consuming unbounded resources.
pub fn walk_frame(
    element: &FrameElement,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    caps: &ClientCapabilities,
    config: &WalkConfig,
) -> WalkResult {
    let mut state = WalkState {
        commands: Vec::new(),
        nodes_visited: 0,
        truncated: false,
        config,
        caps,
    };
    walk_recursive(element, x, y, w, h, 0, &mut state);
    WalkResult {
        commands: state.commands,
        nodes_visited: state.nodes_visited,
        truncated: state.truncated,
    }
}

struct WalkState<'a> {
    commands: Vec<RenderCommand>,
    nodes_visited: usize,
    truncated: bool,
    config: &'a WalkConfig,
    caps: &'a ClientCapabilities,
}

fn walk_recursive(
    element: &FrameElement,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    depth: usize,
    state: &mut WalkState<'_>,
) {
    if state.truncated {
        return;
    }

    if depth > state.config.max_depth {
        state.truncated = true;
        return;
    }

    state.nodes_visited += 1;
    if state.nodes_visited > state.config.max_nodes {
        state.truncated = true;
        return;
    }

    match element {
        FrameElement::Text { text, style } => {
            let resolved = degrade_style(style, state.caps);
            state.commands.push(RenderCommand::DrawText {
                x,
                y,
                text: text.clone(),
                style: resolved,
            });
        }

        FrameElement::Paragraph { lines, style } => {
            let resolved = degrade_style(style, state.caps);
            for (i, line) in lines.iter().enumerate() {
                let line_y = y.saturating_add(i as u16);
                if line_y >= y.saturating_add(h) {
                    break;
                }
                state.commands.push(RenderCommand::DrawText {
                    x,
                    y: line_y,
                    text: line.clone(),
                    style: resolved.clone(),
                });
            }
        }

        FrameElement::Empty {} => {
            // Nothing to render.
        }

        FrameElement::Split {
            direction,
            sizes,
            children,
        } => {
            let rects = compute_split_rects(x, y, w, h, direction, sizes, children.len());
            for (child, (cx, cy, cw, ch)) in children.iter().zip(rects) {
                walk_recursive(child, cx, cy, cw, ch, depth + 1, state);
                if state.truncated {
                    return;
                }
            }
        }

        FrameElement::Stack { children } => {
            for child in children {
                walk_recursive(child, x, y, w, h, depth + 1, state);
                if state.truncated {
                    return;
                }
            }
        }

        FrameElement::Padded {
            top,
            right,
            bottom,
            left,
            child,
        } => {
            let px = x.saturating_add(*left);
            let py = y.saturating_add(*top);
            let pw = w.saturating_sub(*left).saturating_sub(*right);
            let ph = h.saturating_sub(*top).saturating_sub(*bottom);
            if pw > 0 && ph > 0 {
                walk_recursive(child, px, py, pw, ph, depth + 1, state);
            }
        }

        FrameElement::Centered { child } => {
            // Pass through — centering logic deferred.
            walk_recursive(child, x, y, w, h, depth + 1, state);
        }

        FrameElement::Scrollable { child, .. } => {
            // Pass through — scrollback deferred.
            walk_recursive(child, x, y, w, h, depth + 1, state);
        }

        FrameElement::VtPassthrough { data } => {
            if state.caps.vt_passthrough {
                state.commands.push(RenderCommand::WriteRaw {
                    data: data.clone(),
                    x,
                    y,
                    width: w,
                    height: h,
                });
            }
        }

        FrameElement::Custom {
            fallback: Some(fb), ..
        } => {
            walk_recursive(fb, x, y, w, h, depth + 1, state);
        }

        FrameElement::Custom { fallback: None, .. } => {
            // No fallback — skip.
        }

        // Unknown / future variants — skip for forward compatibility.
        _ => {}
    }
}

/// Degrade a style based on client color capabilities.
fn degrade_style(style: &ResolvedStyle, caps: &ClientCapabilities) -> ResolvedStyle {
    match caps.color_depth {
        ColorDepth::TrueColor => style.clone(),
        ColorDepth::Basic256 => {
            let mut s = style.clone();
            s.fg = quantize_to_cube(s.fg);
            s.bg = quantize_to_cube(s.bg);
            s
        }
        ColorDepth::None => ResolvedStyle {
            fg: (255, 255, 255),
            bg: (0, 0, 0),
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            dim: style.dim,
            strikethrough: style.strikethrough,
            reverse: style.reverse,
            blink: style.blink,
            _unknown: Vec::new(),
        },
        // Future color depth variants — fall back to TrueColor passthrough.
        _ => style.clone(),
    }
}

/// Quantize an RGB triplet to the 6x6x6 color cube (xterm-256 color space).
/// Each channel is mapped: round(val / 255 * 5) * 51.
fn quantize_to_cube(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    fn q(v: u8) -> u8 {
        let idx = ((v as u16 * 5 + 127) / 255) as u8;
        idx * 51
    }
    (q(rgb.0), q(rgb.1), q(rgb.2))
}

/// Compute child rectangles for a Split element.
fn compute_split_rects(
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    direction: &Direction,
    sizes: &[SplitSize],
    child_count: usize,
) -> Vec<(u16, u16, u16, u16)> {
    if child_count == 0 {
        return Vec::new();
    }

    let total = match direction {
        Direction::Horizontal => w,
        Direction::Vertical => h,
    };

    // Compute size for each child
    let mut child_sizes: Vec<u16> = Vec::with_capacity(child_count);

    for i in 0..child_count {
        let size = if i < sizes.len() {
            match &sizes[i] {
                SplitSize::Fixed { value } => *value,
                SplitSize::Ratio { value } => {
                    ((*value) * (total as f32)).round() as u16
                }
                SplitSize::Min { value } => *value,
                SplitSize::Max { value } => (*value).min(total),
                _ => {
                    // Unknown size variant — equal share
                    total / child_count as u16
                }
            }
        } else {
            // No size specified — equal share of remaining
            total / child_count as u16
        };
        child_sizes.push(size);
    }

    // Build rectangles
    let mut rects = Vec::with_capacity(child_count);
    let mut offset: u16 = 0;

    for size in &child_sizes {
        let rect = match direction {
            Direction::Horizontal => (x.saturating_add(offset), y, *size, h),
            Direction::Vertical => (x, y.saturating_add(offset), w, *size),
        };
        rects.push(rect);
        offset = offset.saturating_add(*size);
    }

    rects
}
