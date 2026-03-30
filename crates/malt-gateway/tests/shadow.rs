use malt_gateway::shadow::frame_to_json;
use malt_protocol::common::{Direction, ResolvedStyle, SplitSize};
use malt_protocol::frame_element::FrameElement;

fn default_style() -> Box<ResolvedStyle> {
    Box::new(ResolvedStyle {
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
    })
}

#[test]
fn text_to_json() {
    let elem = FrameElement::Text {
        text: "hello".to_string(),
        style: default_style(),
    };
    let json = frame_to_json(&elem);
    assert_eq!(json["type"], "Text");
    assert_eq!(json["text"], "hello");
    assert_eq!(json["style"]["fg"], "#cccccc");
    assert_eq!(json["style"]["bold"], false);
}

#[test]
fn paragraph_to_json() {
    let elem = FrameElement::Paragraph {
        lines: vec!["line one".to_string(), "line two".to_string()],
        style: default_style(),
    };
    let json = frame_to_json(&elem);
    assert_eq!(json["type"], "Paragraph");
    assert_eq!(json["lines"].as_array().unwrap().len(), 2);
    assert_eq!(json["lines"][0], "line one");
    assert_eq!(json["lines"][1], "line two");
}

#[test]
fn split_to_json() {
    let elem = FrameElement::Split {
        direction: Box::new(Direction::Vertical),
        sizes: vec![
            SplitSize::Fixed { value: 10 },
            SplitSize::Fixed { value: 20 },
        ],
        children: vec![
            FrameElement::Text {
                text: "left".to_string(),
                style: default_style(),
            },
            FrameElement::Text {
                text: "right".to_string(),
                style: default_style(),
            },
        ],
    };
    let json = frame_to_json(&elem);
    assert_eq!(json["type"], "Split");
    assert_eq!(json["direction"], "Vertical");
    assert_eq!(json["children"].as_array().unwrap().len(), 2);
}

#[test]
fn vt_passthrough_to_json() {
    let elem = FrameElement::VtPassthrough {
        data: vec![0x1b, 0x5b, 0x31, 0x6d],
    };
    let json = frame_to_json(&elem);
    assert_eq!(json["type"], "VtPassthrough");
    assert_eq!(json["raw_bytes"], 4);
}
