use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;
use malt_renderer::dirty::DirtyTracker;

fn draw_text(x: u16, y: u16, text: &str) -> RenderCommand {
    RenderCommand::DrawText {
        x,
        y,
        text: text.to_string(),
        style: ResolvedStyle {
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
        },
    }
}

#[test]
fn first_frame_emits_all() {
    let mut tracker = DirtyTracker::new();
    let commands = vec![draw_text(0, 0, "hello"), draw_text(0, 1, "world")];
    let delta = tracker.diff(&commands);
    assert_eq!(delta.len(), 2);
    assert_eq!(delta, commands);
}

#[test]
fn identical_frames_emit_nothing() {
    let mut tracker = DirtyTracker::new();
    let commands = vec![draw_text(0, 0, "hello"), draw_text(0, 1, "world")];
    let _ = tracker.diff(&commands);
    let delta = tracker.diff(&commands);
    assert!(delta.is_empty());
}

#[test]
fn changed_text_emits_update() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "hello")];
    let _ = tracker.diff(&frame1);

    let frame2 = vec![draw_text(0, 0, "world")];
    let delta = tracker.diff(&frame2);
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0], draw_text(0, 0, "world"));
}

#[test]
fn added_element_emits_new() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "hello")];
    let _ = tracker.diff(&frame1);

    let frame2 = vec![draw_text(0, 0, "hello"), draw_text(0, 1, "world")];
    let delta = tracker.diff(&frame2);
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0], draw_text(0, 1, "world"));
}

#[test]
fn removed_element_emits_clear() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "hello"), draw_text(0, 1, "world")];
    let _ = tracker.diff(&frame1);

    let frame2 = vec![draw_text(0, 0, "hello")];
    let delta = tracker.diff(&frame2);
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0], RenderCommand::Clear {});
}
