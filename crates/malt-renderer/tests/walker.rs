use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, ResolvedStyle, UnicodeLevel,
};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::RenderCommand;
use malt_renderer::walker::{walk_frame, WalkConfig};

fn default_style() -> ResolvedStyle {
    ResolvedStyle {
        fg: (204, 204, 204),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        _unknown: Vec::new(),
    }
}

fn truecolor_caps() -> ClientCapabilities {
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}

fn default_config() -> WalkConfig {
    WalkConfig::default()
}

#[test]
fn text_element_produces_draw_text() {
    let elem = FrameElement::Text {
        text: "hello".to_string(),
        style: Box::new(default_style()),
    };
    let result = walk_frame(&elem, 5, 10, 80, 24, &truecolor_caps(), &default_config());
    assert!(!result.truncated);
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::DrawText { x, y, text, style } => {
            assert_eq!(*x, 5);
            assert_eq!(*y, 10);
            assert_eq!(text, "hello");
            assert_eq!(style.fg, (204, 204, 204));
        }
        other => panic!("expected DrawText, got {:?}", other),
    }
}

#[test]
fn empty_element_produces_nothing() {
    let elem = FrameElement::Empty {};
    let result = walk_frame(&elem, 0, 0, 80, 24, &truecolor_caps(), &default_config());
    assert!(result.commands.is_empty());
    assert!(!result.truncated);
}

#[test]
fn paragraph_produces_multiple_draw_texts() {
    let elem = FrameElement::Paragraph {
        lines: vec!["line one".to_string(), "line two".to_string()],
        style: Box::new(default_style()),
    };
    let result = walk_frame(&elem, 3, 7, 80, 24, &truecolor_caps(), &default_config());
    assert_eq!(result.commands.len(), 2);
    match &result.commands[0] {
        RenderCommand::DrawText { x, y, text, .. } => {
            assert_eq!(*x, 3);
            assert_eq!(*y, 7);
            assert_eq!(text, "line one");
        }
        other => panic!("expected DrawText, got {:?}", other),
    }
    match &result.commands[1] {
        RenderCommand::DrawText { x, y, text, .. } => {
            assert_eq!(*x, 3);
            assert_eq!(*y, 8);
            assert_eq!(text, "line two");
        }
        other => panic!("expected DrawText, got {:?}", other),
    }
}

#[test]
fn vt_passthrough_produces_write_raw() {
    let data = vec![0x1b, 0x5b, 0x31, 0x6d]; // ESC[1m
    let elem = FrameElement::VtPassthrough { data: data.clone() };
    let result = walk_frame(&elem, 2, 3, 40, 10, &truecolor_caps(), &default_config());
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::WriteRaw {
            data: d,
            x,
            y,
            width,
            height,
        } => {
            assert_eq!(d, &data);
            assert_eq!(*x, 2);
            assert_eq!(*y, 3);
            assert_eq!(*width, 40);
            assert_eq!(*height, 10);
        }
        other => panic!("expected WriteRaw, got {:?}", other),
    }
}

#[test]
fn depth_limit_truncates() {
    // Build 70-deep Padded nesting (exceeds default 64 limit)
    let mut elem = FrameElement::Text {
        text: "deep".to_string(),
        style: Box::new(default_style()),
    };
    for _ in 0..70 {
        elem = FrameElement::Padded {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
            child: Box::new(elem),
        };
    }
    let result = walk_frame(&elem, 0, 0, 80, 24, &truecolor_caps(), &default_config());
    assert!(result.truncated);
    // The inner Text should not have been reached
    assert!(result.commands.is_empty());
}

#[test]
fn node_limit_truncates() {
    // Stack with 10001 Text children
    let children: Vec<FrameElement> = (0..10_001)
        .map(|i| FrameElement::Text {
            text: format!("t{}", i),
            style: Box::new(default_style()),
        })
        .collect();
    let elem = FrameElement::Stack { children };
    let config = WalkConfig {
        max_depth: 64,
        max_nodes: 10_000,
        max_output_bytes: 1_048_576,
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &truecolor_caps(), &config);
    assert!(result.truncated);
}

#[test]
fn capability_degradation_basic256() {
    let elem = FrameElement::Text {
        text: "colored".to_string(),
        style: Box::new(ResolvedStyle {
            fg: (100, 150, 200),
            bg: (10, 20, 30),
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
            blink: false,
            _unknown: Vec::new(),
        }),
    };
    let mut caps = truecolor_caps();
    caps.color_depth = ColorDepth::Basic256;
    let result = walk_frame(&elem, 0, 0, 80, 24, &caps, &default_config());
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::DrawText { style, .. } => {
            // Quantized to 6x6x6 cube: each channel = round(val/255*5) * 51
            // fg: (100,150,200) -> (2,3,4) -> (102,153,204)
            // bg: (10,20,30) -> (0,0,1) -> (0,0,51)
            assert_eq!(style.fg, (102, 153, 204));
            assert_eq!(style.bg, (0, 0, 51));
        }
        other => panic!("expected DrawText, got {:?}", other),
    }
}

#[test]
fn vt_passthrough_skipped_without_capability() {
    let elem = FrameElement::VtPassthrough {
        data: vec![0x1b, 0x5b, 0x31, 0x6d],
    };
    let mut caps = truecolor_caps();
    caps.vt_passthrough = false;
    let result = walk_frame(&elem, 0, 0, 80, 24, &caps, &default_config());
    assert!(result.commands.is_empty());
}

#[test]
fn unknown_variant_skipped() {
    // Custom without fallback acts like an unknown widget
    let elem = FrameElement::Custom {
        type_id: "com.example.widget".to_string(),
        data: vec![1, 2, 3],
        fallback: None,
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &truecolor_caps(), &default_config());
    assert!(result.commands.is_empty());
}

#[test]
fn custom_with_fallback_renders_fallback() {
    let fallback = FrameElement::Text {
        text: "fallback text".to_string(),
        style: Box::new(default_style()),
    };
    let elem = FrameElement::Custom {
        type_id: "com.example.widget".to_string(),
        data: vec![1, 2, 3],
        fallback: Some(Box::new(fallback)),
    };
    let result = walk_frame(&elem, 4, 5, 80, 24, &truecolor_caps(), &default_config());
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::DrawText { x, y, text, .. } => {
            assert_eq!(*x, 4);
            assert_eq!(*y, 5);
            assert_eq!(text, "fallback text");
        }
        other => panic!("expected DrawText, got {:?}", other),
    }
}
