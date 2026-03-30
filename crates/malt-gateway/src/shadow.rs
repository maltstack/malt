//! Shadow tree serialization — converts FrameElement trees to semantic JSON
//! for the REST API. Clients receive a JSON representation of the UI tree
//! rather than raw VNP-encoded bytes.

use malt_protocol::common::{Direction, ResolvedStyle};
use malt_protocol::frame_element::FrameElement;
use serde_json::{json, Value};

/// Convert an RGB tuple to a CSS hex color string.
fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

/// Serialize a `ResolvedStyle` to a JSON object with hex colors and boolean attributes.
pub fn style_to_json(style: &ResolvedStyle) -> Value {
    json!({
        "fg": rgb_to_hex(style.fg.0, style.fg.1, style.fg.2),
        "bg": rgb_to_hex(style.bg.0, style.bg.1, style.bg.2),
        "bold": style.bold,
        "italic": style.italic,
        "underline": style.underline,
        "dim": style.dim,
    })
}

/// Convert a `Direction` enum to its string representation.
fn direction_to_str(dir: &Direction) -> &'static str {
    match dir {
        Direction::Horizontal => "Horizontal",
        Direction::Vertical => "Vertical",
    }
}

/// Recursively convert a `FrameElement` tree into semantic JSON.
///
/// Each variant maps to a JSON object with a `"type"` discriminator and
/// variant-specific fields. Recursive children are serialized depth-first.
pub fn frame_to_json(element: &FrameElement) -> Value {
    match element {
        FrameElement::Text { text, style } => json!({
            "type": "Text",
            "text": text,
            "style": style_to_json(style),
        }),

        FrameElement::Paragraph { lines, style } => json!({
            "type": "Paragraph",
            "lines": lines,
            "style": style_to_json(style),
        }),

        FrameElement::Empty {} => json!({
            "type": "Empty",
        }),

        FrameElement::Split { direction, children, .. } => json!({
            "type": "Split",
            "direction": direction_to_str(direction.as_ref()),
            "children": children.iter().map(frame_to_json).collect::<Vec<_>>(),
        }),

        FrameElement::Stack { children } => json!({
            "type": "Stack",
            "children": children.iter().map(frame_to_json).collect::<Vec<_>>(),
        }),

        FrameElement::Padded { top, right, bottom, left, child } => json!({
            "type": "Padded",
            "padding": {
                "top": top,
                "right": right,
                "bottom": bottom,
                "left": left,
            },
            "child": frame_to_json(child),
        }),

        FrameElement::Centered { child } => json!({
            "type": "Centered",
            "child": frame_to_json(child),
        }),

        FrameElement::Scrollable { offset, child } => json!({
            "type": "Scrollable",
            "offset": offset,
            "child": frame_to_json(child),
        }),

        FrameElement::VtPassthrough { data } => json!({
            "type": "VtPassthrough",
            "raw_bytes": data.len(),
        }),

        FrameElement::Custom { type_id, data, fallback } => {
            let mut obj = json!({
                "type": "Custom",
                "type_id": type_id,
                "data_bytes": data.len(),
            });
            if let Some(fb) = fallback {
                obj["fallback"] = frame_to_json(fb);
            }
            obj
        }

        // Unknown variants from forward-compatible decoding
        _ => json!({
            "type": "Unknown",
        }),
    }
}
