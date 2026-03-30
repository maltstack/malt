# Phase 3C: Renderer Host Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Renderer Host that transforms FrameElement trees into RenderCommand deltas for clients, with per-client capability degradation, dirty tracking, and backpressure.

**Architecture:** Pure pipeline: FrameElement trees + ResolvedPane layout in, RenderCommand batches out. Per-client state tracks frame sequencing, ack status, and capabilities. The crate has no bus dependency — integration into malt-daemon happens separately.

**Tech Stack:** Rust, malt-protocol (FrameElement, RenderCommand, RenderBatch, ClientCapabilities, ResolvedPane), thiserror, tracing

---

## File Structure

```
crates/malt-renderer/
  Cargo.toml
  src/
    lib.rs              — crate root, module declarations, re-exports
    error.rs            — RendererError enum
    theme.rs            — ThemeResolver: token → RGB stub
    walker.rs           — FrameWalker: tree traversal → RenderCommand list
    dirty.rs            — DirtyTracker: diff previous vs current commands
    client_state.rs     — ClientState: frame_seq, ack tracking, lagging/shedding
    host.rs             — RendererHost: orchestrates pipeline, owns client states
  tests/
    walker.rs           — tree walking, limits, capability degradation
    dirty.rs            — dirty tracking diff tests
    client_state.rs     — frame sequencing, slow client
    host.rs             — end-to-end integration
```

---

### Task 1: Crate Scaffolding

**Files:**
- Create: `crates/malt-renderer/Cargo.toml`
- Create: `crates/malt-renderer/src/lib.rs`
- Create: `crates/malt-renderer/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "malt-renderer"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Renderer Host for MALT — FrameElement to RenderCommand pipeline"

[dependencies]
malt-protocol = { path = "../malt-protocol" }
thiserror = "2"
tracing = "0.1"
```

- [ ] **Step 2: Create error.rs**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RendererError {
    #[error("client not found: {0}")]
    ClientNotFound(u64),

    #[error("frame too large: {size} bytes exceeds {max} byte limit")]
    FrameTooLarge { size: usize, max: usize },
}
```

- [ ] **Step 3: Create lib.rs**

```rust
pub mod client_state;
pub mod dirty;
pub mod error;
pub mod host;
pub mod theme;
pub mod walker;

pub use error::RendererError;
pub use host::RendererHost;
```

- [ ] **Step 4: Create stub modules**

Create `crates/malt-renderer/src/theme.rs`:
```rust
// ThemeResolver — Task 2
```

Create `crates/malt-renderer/src/walker.rs`:
```rust
// FrameWalker — Task 3
```

Create `crates/malt-renderer/src/dirty.rs`:
```rust
// DirtyTracker — Task 4
```

Create `crates/malt-renderer/src/client_state.rs`:
```rust
// ClientState — Task 5
```

Create `crates/malt-renderer/src/host.rs`:
```rust
// RendererHost — Task 6
```

- [ ] **Step 5: Add malt-renderer to workspace**

In root `Cargo.toml`, add `"crates/malt-renderer"` to the workspace members list.

- [ ] **Step 6: Verify it compiles**

Run: `cargo check -p malt-renderer`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add crates/malt-renderer/ Cargo.toml
git commit -m "feat(malt-renderer): crate scaffolding with module stubs"
```

---

### Task 2: Theme Resolver

**Files:**
- Create: `crates/malt-renderer/src/theme.rs`

- [ ] **Step 1: Implement ThemeResolver stub**

```rust
use malt_protocol::common::ResolvedStyle;

/// Default foreground: white
const DEFAULT_FG: (u8, u8, u8) = (204, 204, 204);
/// Default background: black
const DEFAULT_BG: (u8, u8, u8) = (0, 0, 0);

/// Resolves theme tokens to concrete RGB values.
///
/// Currently a stub returning default colors. Will be extended with
/// actual theme files in Phase 5 (Ecosystem).
#[derive(Debug, Clone)]
pub struct ThemeResolver;

impl ThemeResolver {
    pub fn new() -> Self {
        Self
    }

    /// Returns a default style with white-on-black colors.
    pub fn default_style(&self) -> ResolvedStyle {
        ResolvedStyle {
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
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

    /// Resolve a style, filling in defaults for any missing values.
    /// Currently a pass-through since styles are already resolved.
    pub fn resolve(&self, style: &ResolvedStyle) -> ResolvedStyle {
        style.clone()
    }
}

impl Default for ThemeResolver {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p malt-renderer`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add crates/malt-renderer/src/theme.rs
git commit -m "feat(malt-renderer): theme resolver stub — default white-on-black"
```

---

### Task 3: Frame Walker

**Files:**
- Create: `crates/malt-renderer/src/walker.rs`
- Create: `crates/malt-renderer/tests/walker.rs`

- [ ] **Step 1: Write failing tests for basic element walking**

Create `crates/malt-renderer/tests/walker.rs`:
```rust
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, Direction, ImageProtocol, ResolvedStyle, UnicodeLevel,
};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::RenderCommand;
use malt_renderer::walker::{walk_frame, WalkConfig, WalkResult};

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

fn full_caps() -> ClientCapabilities {
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

#[test]
fn text_element_produces_draw_text() {
    let elem = FrameElement::Text {
        text: "hello".to_string(),
        style: default_style(),
        _unknown: Vec::new(),
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::DrawText { x, y, text, .. } => {
            assert_eq!(*x, 0);
            assert_eq!(*y, 0);
            assert_eq!(text, "hello");
        }
        other => panic!("expected DrawText, got {other:?}"),
    }
}

#[test]
fn empty_element_produces_nothing() {
    let elem = FrameElement::Empty { _unknown: Vec::new() };
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert!(result.commands.is_empty());
}

#[test]
fn paragraph_produces_multiple_draw_texts() {
    let elem = FrameElement::Paragraph {
        lines: vec!["line 1".to_string(), "line 2".to_string()],
        style: default_style(),
        _unknown: Vec::new(),
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert_eq!(result.commands.len(), 2);
    match &result.commands[0] {
        RenderCommand::DrawText { y, text, .. } => {
            assert_eq!(*y, 0);
            assert_eq!(text, "line 1");
        }
        other => panic!("expected DrawText, got {other:?}"),
    }
    match &result.commands[1] {
        RenderCommand::DrawText { y, text, .. } => {
            assert_eq!(*y, 1);
            assert_eq!(text, "line 2");
        }
        other => panic!("expected DrawText, got {other:?}"),
    }
}

#[test]
fn vt_passthrough_produces_write_raw() {
    let elem = FrameElement::VtPassthrough {
        data: vec![27, 91, 50, 74], // ESC[2J
        _unknown: Vec::new(),
    };
    let result = walk_frame(&elem, 5, 3, 40, 10, &full_caps(), &WalkConfig::default());
    assert_eq!(result.commands.len(), 1);
    match &result.commands[0] {
        RenderCommand::WriteRaw { data, x, y, width, height, .. } => {
            assert_eq!(data, &vec![27, 91, 50, 74]);
            assert_eq!(*x, 5);
            assert_eq!(*y, 3);
            assert_eq!(*width, 40);
            assert_eq!(*height, 10);
        }
        other => panic!("expected WriteRaw, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-renderer --test walker`
Expected: FAIL — `walk_frame` not implemented

- [ ] **Step 3: Implement walk_frame**

Write `crates/malt-renderer/src/walker.rs`:
```rust
use malt_protocol::common::{ClientCapabilities, ColorDepth, Direction, ResolvedStyle, SplitSize};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::RenderCommand;
use tracing::warn;

/// Configuration for the frame walker.
#[derive(Debug, Clone)]
pub struct WalkConfig {
    /// Maximum tree depth before truncation.
    pub max_depth: usize,
    /// Maximum number of nodes to process per frame.
    pub max_nodes: usize,
    /// Maximum total output size in bytes (approximate).
    pub max_output_bytes: usize,
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 10_000,
            max_output_bytes: 1_048_576, // 1 MiB
        }
    }
}

/// Result of walking a frame element tree.
#[derive(Debug)]
pub struct WalkResult {
    pub commands: Vec<RenderCommand>,
    pub nodes_visited: usize,
    pub truncated: bool,
}

struct WalkState {
    commands: Vec<RenderCommand>,
    nodes_visited: usize,
    truncated: bool,
    config: WalkConfig,
}

impl WalkState {
    fn new(config: WalkConfig) -> Self {
        Self {
            commands: Vec::new(),
            nodes_visited: 0,
            truncated: false,
            config,
        }
    }

    fn at_limit(&self) -> bool {
        self.truncated || self.nodes_visited >= self.config.max_nodes
    }

    fn push(&mut self, cmd: RenderCommand) {
        self.commands.push(cmd);
    }
}

/// Walk a FrameElement tree and produce RenderCommands.
///
/// `x`, `y`, `w`, `h` define the bounding rectangle for this element.
/// Capability-based degradation is applied per element.
pub fn walk_frame(
    element: &FrameElement,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    caps: &ClientCapabilities,
    config: &WalkConfig,
) -> WalkResult {
    let mut state = WalkState::new(config.clone());
    walk_element(element, x, y, w, h, 0, caps, &mut state);
    WalkResult {
        commands: state.commands,
        nodes_visited: state.nodes_visited,
        truncated: state.truncated,
    }
}

fn walk_element(
    element: &FrameElement,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    depth: usize,
    caps: &ClientCapabilities,
    state: &mut WalkState,
) {
    if state.at_limit() {
        return;
    }
    if depth >= state.config.max_depth {
        state.truncated = true;
        warn!(depth, "frame tree depth limit reached, truncating");
        return;
    }
    state.nodes_visited += 1;
    if state.nodes_visited >= state.config.max_nodes {
        state.truncated = true;
        warn!(nodes = state.nodes_visited, "frame node limit reached, truncating");
        return;
    }

    match element {
        FrameElement::Text { text, style, .. } => {
            let style = degrade_style(style, caps);
            state.push(RenderCommand::DrawText {
                x,
                y,
                text: text.clone(),
                style,
                _unknown: Vec::new(),
            });
        }
        FrameElement::Paragraph { lines, style, .. } => {
            let style = degrade_style(style, caps);
            for (i, line) in lines.iter().enumerate() {
                if y + (i as u16) >= y + h {
                    break;
                }
                state.push(RenderCommand::DrawText {
                    x,
                    y: y + i as u16,
                    text: line.clone(),
                    style: style.clone(),
                    _unknown: Vec::new(),
                });
            }
        }
        FrameElement::Empty { .. } => {}
        FrameElement::Split {
            direction,
            sizes,
            children,
            ..
        } => {
            walk_split(direction, sizes, children, x, y, w, h, depth, caps, state);
        }
        FrameElement::Stack { children, .. } => {
            // Stack: each child occupies the full area, painted in order (last on top)
            for child in children {
                walk_element(child, x, y, w, h, depth + 1, caps, state);
            }
        }
        FrameElement::Padded {
            top,
            right,
            bottom,
            left,
            child,
            ..
        } => {
            let inner_x = x + left;
            let inner_y = y + top;
            let inner_w = w.saturating_sub(left + right);
            let inner_h = h.saturating_sub(top + bottom);
            if inner_w > 0 && inner_h > 0 {
                walk_element(child, inner_x, inner_y, inner_w, inner_h, depth + 1, caps, state);
            }
        }
        FrameElement::Centered { child, .. } => {
            // Center child in the available space (pass through for now)
            walk_element(child, x, y, w, h, depth + 1, caps, state);
        }
        FrameElement::Scrollable { offset, child, .. } => {
            // Scrollable: offset applied to y, child rendered in clipped region
            let _ = offset; // Scrollback integration deferred to Phase 3D
            walk_element(child, x, y, w, h, depth + 1, caps, state);
        }
        FrameElement::VtPassthrough { data, .. } => {
            if caps.vt_passthrough {
                state.push(RenderCommand::WriteRaw {
                    data: data.clone(),
                    x,
                    y,
                    width: w,
                    height: h,
                    _unknown: Vec::new(),
                });
            }
            // Clients without VT passthrough: graceful degradation (skip)
        }
        FrameElement::Custom { fallback, .. } => {
            // Render fallback if available, skip otherwise
            if let Some(fb) = fallback {
                walk_element(fb, x, y, w, h, depth + 1, caps, state);
            }
        }
        _ => {
            // Unknown variant (forward compatibility) — skip
        }
    }
}

fn walk_split(
    direction: &Direction,
    sizes: &[SplitSize],
    children: &[FrameElement],
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    depth: usize,
    caps: &ClientCapabilities,
    state: &mut WalkState,
) {
    if children.is_empty() {
        return;
    }

    let total = match direction {
        Direction::Horizontal => h as f32,
        Direction::Vertical => w as f32,
        _ => w as f32,
    };

    // Calculate sizes for each child
    let mut offsets = Vec::with_capacity(children.len());
    let mut pos: f32 = 0.0;
    for (i, _child) in children.iter().enumerate() {
        offsets.push(pos);
        let size = if i < sizes.len() {
            match &sizes[i] {
                SplitSize::Ratio { value, .. } => total * value,
                SplitSize::Fixed { value, .. } => *value as f32,
                _ => total / children.len() as f32,
            }
        } else {
            total / children.len() as f32
        };
        pos += size;
    }

    for (i, child) in children.iter().enumerate() {
        let offset = offsets[i] as u16;
        let next = if i + 1 < offsets.len() {
            offsets[i + 1] as u16
        } else {
            match direction {
                Direction::Horizontal => h,
                Direction::Vertical => w,
                _ => w,
            }
        };
        let size = next.saturating_sub(offset);

        let (cx, cy, cw, ch) = match direction {
            Direction::Vertical => (x + offset, y, size, h),
            Direction::Horizontal => (x, y + offset, w, size),
            _ => (x + offset, y, size, h),
        };

        if cw > 0 && ch > 0 {
            walk_element(child, cx, cy, cw, ch, depth + 1, caps, state);
        }
    }
}

/// Degrade a style based on client capabilities.
fn degrade_style(style: &ResolvedStyle, caps: &ClientCapabilities) -> ResolvedStyle {
    match caps.color_depth {
        ColorDepth::TrueColor => style.clone(),
        ColorDepth::Basic256 => {
            let mut s = style.clone();
            s.fg = nearest_256(style.fg);
            s.bg = nearest_256(style.bg);
            s
        }
        ColorDepth::None => {
            let mut s = style.clone();
            s.fg = (255, 255, 255);
            s.bg = (0, 0, 0);
            s
        }
        _ => style.clone(),
    }
}

/// Approximate RGB to nearest 256-color palette value.
/// Uses the 6x6x6 color cube (indices 16-231).
fn nearest_256(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    let r = ((rgb.0 as u16 * 5 + 127) / 255) as u8;
    let g = ((rgb.1 as u16 * 5 + 127) / 255) as u8;
    let b = ((rgb.2 as u16 * 5 + 127) / 255) as u8;
    // Map back from 6-level to approximate RGB
    let to_rgb = |v: u8| -> u8 { if v == 0 { 0 } else { 55 + 40 * v } };
    (to_rgb(r), to_rgb(g), to_rgb(b))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-renderer --test walker`
Expected: all 4 tests PASS

- [ ] **Step 5: Write tests for depth and node limits**

Append to `crates/malt-renderer/tests/walker.rs`:
```rust
#[test]
fn depth_limit_truncates() {
    // Build a tree 70 levels deep (exceeds 64 limit)
    let mut elem = FrameElement::Text {
        text: "deep".to_string(),
        style: default_style(),
        _unknown: Vec::new(),
    };
    for _ in 0..70 {
        elem = FrameElement::Padded {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
            child: Box::new(elem),
            _unknown: Vec::new(),
        };
    }
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert!(result.truncated);
    // The Text at the bottom should NOT be reached
    assert!(result.commands.is_empty());
}

#[test]
fn node_limit_truncates() {
    // Build a Stack with 10001 children (exceeds 10000 limit)
    let children: Vec<FrameElement> = (0..10_001)
        .map(|i| FrameElement::Text {
            text: format!("node {i}"),
            style: default_style(),
            _unknown: Vec::new(),
        })
        .collect();
    let elem = FrameElement::Stack {
        children,
        _unknown: Vec::new(),
    };
    let config = WalkConfig::default();
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &config);
    assert!(result.truncated);
    // Should have processed close to but not exceeding 10000 nodes
    assert!(result.nodes_visited <= 10_001);
}

#[test]
fn capability_degradation_basic256() {
    let style = ResolvedStyle {
        fg: (128, 64, 192),
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
    let elem = FrameElement::Text {
        text: "test".to_string(),
        style,
        _unknown: Vec::new(),
    };
    let mut caps = full_caps();
    caps.color_depth = ColorDepth::Basic256;
    let result = walk_frame(&elem, 0, 0, 80, 24, &caps, &WalkConfig::default());
    match &result.commands[0] {
        RenderCommand::DrawText { style, .. } => {
            // Colors should be quantized to 6x6x6 cube, not original values
            assert_ne!(style.fg, (128, 64, 192));
        }
        other => panic!("expected DrawText, got {other:?}"),
    }
}

#[test]
fn vt_passthrough_skipped_without_capability() {
    let elem = FrameElement::VtPassthrough {
        data: vec![27, 91, 50, 74],
        _unknown: Vec::new(),
    };
    let mut caps = full_caps();
    caps.vt_passthrough = false;
    let result = walk_frame(&elem, 0, 0, 80, 24, &caps, &WalkConfig::default());
    assert!(result.commands.is_empty());
}

#[test]
fn unknown_variant_skipped() {
    // Custom without fallback — should be skipped
    let elem = FrameElement::Custom {
        type_id: "unknown-widget".to_string(),
        data: vec![],
        fallback: None,
        _unknown: Vec::new(),
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert!(result.commands.is_empty());
}

#[test]
fn custom_with_fallback_renders_fallback() {
    let elem = FrameElement::Custom {
        type_id: "widget".to_string(),
        data: vec![],
        fallback: Some(Box::new(FrameElement::Text {
            text: "fallback".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        })),
        _unknown: Vec::new(),
    };
    let result = walk_frame(&elem, 0, 0, 80, 24, &full_caps(), &WalkConfig::default());
    assert_eq!(result.commands.len(), 1);
}
```

- [ ] **Step 6: Run all walker tests**

Run: `cargo test -p malt-renderer --test walker`
Expected: all 10 tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/malt-renderer/src/walker.rs crates/malt-renderer/tests/walker.rs
git commit -m "feat(malt-renderer): frame walker — tree traversal, limits, capability degradation"
```

---

### Task 4: Dirty Tracker

**Files:**
- Create: `crates/malt-renderer/src/dirty.rs`
- Create: `crates/malt-renderer/tests/dirty.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-renderer/tests/dirty.rs`:
```rust
use malt_protocol::common::ResolvedStyle;
use malt_protocol::render::RenderCommand;
use malt_renderer::dirty::DirtyTracker;

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

fn draw_text(x: u16, y: u16, text: &str) -> RenderCommand {
    RenderCommand::DrawText {
        x,
        y,
        text: text.to_string(),
        style: default_style(),
        _unknown: Vec::new(),
    }
}

#[test]
fn first_frame_emits_all() {
    let mut tracker = DirtyTracker::new();
    let commands = vec![draw_text(0, 0, "hello"), draw_text(0, 1, "world")];
    let delta = tracker.diff(&commands);
    assert_eq!(delta.len(), 2);
}

#[test]
fn identical_frames_emit_nothing() {
    let mut tracker = DirtyTracker::new();
    let commands = vec![draw_text(0, 0, "hello")];
    tracker.diff(&commands); // first frame
    let delta = tracker.diff(&commands); // identical
    assert!(delta.is_empty());
}

#[test]
fn changed_text_emits_update() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "hello")];
    tracker.diff(&frame1);
    let frame2 = vec![draw_text(0, 0, "world")];
    let delta = tracker.diff(&frame2);
    assert_eq!(delta.len(), 1);
    match &delta[0] {
        RenderCommand::DrawText { text, .. } => assert_eq!(text, "world"),
        other => panic!("expected DrawText, got {other:?}"),
    }
}

#[test]
fn added_element_emits_new() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "a")];
    tracker.diff(&frame1);
    let frame2 = vec![draw_text(0, 0, "a"), draw_text(0, 1, "b")];
    let delta = tracker.diff(&frame2);
    assert_eq!(delta.len(), 1);
    match &delta[0] {
        RenderCommand::DrawText { text, .. } => assert_eq!(text, "b"),
        other => panic!("expected DrawText, got {other:?}"),
    }
}

#[test]
fn removed_element_emits_clear() {
    let mut tracker = DirtyTracker::new();
    let frame1 = vec![draw_text(0, 0, "a"), draw_text(0, 1, "b")];
    tracker.diff(&frame1);
    let frame2 = vec![draw_text(0, 0, "a")];
    let delta = tracker.diff(&frame2);
    // Should contain at least a clear for the removed region
    assert!(!delta.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-renderer --test dirty`
Expected: FAIL — `DirtyTracker` not implemented

- [ ] **Step 3: Implement DirtyTracker**

Write `crates/malt-renderer/src/dirty.rs`:
```rust
use malt_protocol::render::RenderCommand;

/// Tracks the previous frame's commands and diffs against the current frame.
///
/// Comparison is by-value equality of the serialized RenderCommand list.
/// This is a simple approach: if a command at position N changed, emit it.
/// Commands beyond the previous frame's length are always emitted.
/// If the current frame is shorter, emit a Clear for the removed region.
#[derive(Debug)]
pub struct DirtyTracker {
    previous: Vec<RenderCommand>,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
        }
    }

    /// Diff the current commands against the previous frame.
    /// Returns only the commands that changed (the delta).
    /// Updates internal state to the current frame.
    pub fn diff(&mut self, current: &[RenderCommand]) -> Vec<RenderCommand> {
        if self.previous.is_empty() {
            // First frame: emit everything
            self.previous = current.to_vec();
            return current.to_vec();
        }

        let mut delta = Vec::new();

        // Compare common prefix
        let common_len = self.previous.len().min(current.len());
        for i in 0..common_len {
            if !commands_equal(&self.previous[i], &current[i]) {
                delta.push(current[i].clone());
            }
        }

        // New commands beyond previous length
        for cmd in current.iter().skip(self.previous.len()) {
            delta.push(cmd.clone());
        }

        // If current is shorter, emit Clear to wipe removed content
        if current.len() < self.previous.len() {
            delta.push(RenderCommand::Clear { _unknown: Vec::new() });
        }

        self.previous = current.to_vec();
        delta
    }

    /// Reset tracker state, forcing a full re-render on next diff.
    pub fn reset(&mut self) {
        self.previous.clear();
    }
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Compare two RenderCommands for equality.
/// Uses Debug representation as a proxy since generated types
/// derive Debug and PartialEq.
fn commands_equal(a: &RenderCommand, b: &RenderCommand) -> bool {
    a == b
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-renderer --test dirty`
Expected: all 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-renderer/src/dirty.rs crates/malt-renderer/tests/dirty.rs
git commit -m "feat(malt-renderer): dirty tracker — frame diffing for delta emission"
```

---

### Task 5: Client State

**Files:**
- Create: `crates/malt-renderer/src/client_state.rs`
- Create: `crates/malt-renderer/tests/client_state.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-renderer/tests/client_state.rs`:
```rust
use malt_protocol::common::{ClientCapabilities, ColorDepth, ImageProtocol, UnicodeLevel};
use malt_renderer::client_state::{ClientState, AckStatus};

fn full_caps() -> ClientCapabilities {
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

#[test]
fn new_client_starts_at_seq_zero() {
    let state = ClientState::new(1, full_caps());
    assert_eq!(state.frame_seq(), 0);
    assert_eq!(state.unacked_count(), 0);
}

#[test]
fn advance_seq_increments() {
    let mut state = ClientState::new(1, full_caps());
    let seq = state.advance_seq();
    assert_eq!(seq, 1);
    assert_eq!(state.frame_seq(), 1);
    assert_eq!(state.unacked_count(), 1);
}

#[test]
fn ack_reduces_unacked() {
    let mut state = ClientState::new(1, full_caps());
    state.advance_seq(); // seq 1
    state.advance_seq(); // seq 2
    assert_eq!(state.unacked_count(), 2);
    state.ack(2);
    assert_eq!(state.unacked_count(), 0);
}

#[test]
fn ack_partial() {
    let mut state = ClientState::new(1, full_caps());
    state.advance_seq(); // 1
    state.advance_seq(); // 2
    state.advance_seq(); // 3
    state.ack(2);
    assert_eq!(state.unacked_count(), 1); // only seq 3 unacked
}

#[test]
fn lagging_at_30_unacked() {
    let mut state = ClientState::new(1, full_caps());
    for _ in 0..30 {
        state.advance_seq();
    }
    assert_eq!(state.check_status(), AckStatus::Lagging);
}

#[test]
fn not_lagging_below_30() {
    let mut state = ClientState::new(1, full_caps());
    for _ in 0..29 {
        state.advance_seq();
    }
    assert_eq!(state.check_status(), AckStatus::Ok);
}

#[test]
fn ack_clears_lagging() {
    let mut state = ClientState::new(1, full_caps());
    for _ in 0..35 {
        state.advance_seq();
    }
    assert_eq!(state.check_status(), AckStatus::Lagging);
    state.ack(35);
    assert_eq!(state.check_status(), AckStatus::Ok);
}

#[test]
fn shed_after_timeout() {
    let mut state = ClientState::new(1, full_caps());
    state.advance_seq();
    // Simulate 10 seconds passing with no ack
    assert!(!state.should_shed(9_999));
    assert!(state.should_shed(10_000));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-renderer --test client_state`
Expected: FAIL — `ClientState` not implemented

- [ ] **Step 3: Implement ClientState**

Write `crates/malt-renderer/src/client_state.rs`:
```rust
use malt_protocol::common::ClientCapabilities;

const LAGGING_THRESHOLD: u64 = 30;
const SHED_TIMEOUT_MS: u64 = 10_000;

/// Status of a client's ack state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckStatus {
    /// Client is keeping up with frames.
    Ok,
    /// Client has > 30 unacked frames — stop producing for it.
    Lagging,
}

/// Per-client renderer state.
///
/// Tracks frame sequencing, ack state, and capabilities.
#[derive(Debug)]
pub struct ClientState {
    id: u64,
    capabilities: ClientCapabilities,
    frame_seq: u64,
    last_acked_seq: u64,
    last_ack_time_ms: u64,
}

impl ClientState {
    pub fn new(id: u64, capabilities: ClientCapabilities) -> Self {
        Self {
            id,
            capabilities,
            frame_seq: 0,
            last_acked_seq: 0,
            last_ack_time_ms: 0,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn capabilities(&self) -> &ClientCapabilities {
        &self.capabilities
    }

    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }

    /// Advance the frame sequence counter. Returns the new sequence number.
    pub fn advance_seq(&mut self) -> u64 {
        self.frame_seq += 1;
        self.frame_seq
    }

    /// Number of frames sent but not yet acknowledged.
    pub fn unacked_count(&self) -> u64 {
        self.frame_seq - self.last_acked_seq
    }

    /// Process a frame acknowledgment from the client.
    pub fn ack(&mut self, seq: u64) {
        if seq > self.last_acked_seq {
            self.last_acked_seq = seq;
            self.last_ack_time_ms = 0; // Reset timer on ack
        }
    }

    /// Check the client's ack status.
    pub fn check_status(&self) -> AckStatus {
        if self.unacked_count() >= LAGGING_THRESHOLD {
            AckStatus::Lagging
        } else {
            AckStatus::Ok
        }
    }

    /// Check if the client should be shed (disconnected).
    /// `elapsed_since_last_ack_ms` is the time since the last ack or connection.
    pub fn should_shed(&self, elapsed_since_last_ack_ms: u64) -> bool {
        self.unacked_count() > 0 && elapsed_since_last_ack_ms >= SHED_TIMEOUT_MS
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-renderer --test client_state`
Expected: all 8 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-renderer/src/client_state.rs crates/malt-renderer/tests/client_state.rs
git commit -m "feat(malt-renderer): client state — frame sequencing, lagging detection, shedding"
```

---

### Task 6: Renderer Host

**Files:**
- Create: `crates/malt-renderer/src/host.rs`
- Create: `crates/malt-renderer/tests/host.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/malt-renderer/tests/host.rs`:
```rust
use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, PaneId, ResolvedPane, ResolvedStyle,
    UnicodeLevel,
};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::RenderCommand;
use malt_renderer::client_state::AckStatus;
use malt_renderer::host::{PaneFrame, RendererHost};

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

fn full_caps() -> ClientCapabilities {
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

fn make_pane(id: u32, x: u16, y: u16, w: u16, h: u16) -> ResolvedPane {
    ResolvedPane {
        pane_id: PaneId(id),
        x,
        y,
        width: w,
        height: h,
        focused: id == 1,
        visible: true,
        z_order: 0,
        tab_context: None,
        _unknown: Vec::new(),
    }
}

#[test]
fn register_client_and_render() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hello".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];

    let batches = host.process_frame(&frames, &layout);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].client_id, 1);
    assert_eq!(batches[0].batch.frame_seq, 1);
    assert!(!batches[0].batch.commands.is_empty());
}

#[test]
fn lagging_client_skipped() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hi".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];

    // Produce 30 frames without acking
    for _ in 0..30 {
        host.process_frame(&frames, &layout);
    }

    // 31st frame: client should be lagging, no batch produced
    let batches = host.process_frame(&frames, &layout);
    assert!(batches.is_empty());
}

#[test]
fn ack_resumes_production() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hi".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];

    for _ in 0..30 {
        host.process_frame(&frames, &layout);
    }
    // Lagging — ack everything
    host.ack_frame(1, 30);
    let batches = host.process_frame(&frames, &layout);
    assert_eq!(batches.len(), 1);
}

#[test]
fn unregistered_client_no_batches() {
    let mut host = RendererHost::new();
    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hi".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];
    let batches = host.process_frame(&frames, &layout);
    assert!(batches.is_empty());
}

#[test]
fn initial_state_snapshot() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hello".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];

    let initial = host.snapshot_initial_state(&frames, &layout, 1);
    assert!(!initial.commands.is_empty());
    assert_eq!(initial.panes.len(), 1);
}

#[test]
fn remove_client() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());
    host.remove_client(1);

    let frames = vec![PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hi".to_string(),
            style: default_style(),
            _unknown: Vec::new(),
        },
    }];
    let layout = vec![make_pane(1, 0, 0, 80, 24)];
    let batches = host.process_frame(&frames, &layout);
    assert!(batches.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p malt-renderer --test host`
Expected: FAIL — `RendererHost` not implemented

- [ ] **Step 3: Implement RendererHost**

Write `crates/malt-renderer/src/host.rs`:
```rust
use crate::client_state::{AckStatus, ClientState};
use crate::dirty::DirtyTracker;
use crate::theme::ThemeResolver;
use crate::walker::{walk_frame, WalkConfig};
use malt_protocol::common::{ClientCapabilities, LayoutNode, PaneId, ResolvedPane};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::{InitialState, RenderBatch, RenderCommand};
use std::collections::HashMap;

/// A pane's FrameElement tree.
#[derive(Debug, Clone)]
pub struct PaneFrame {
    pub pane_id: PaneId,
    pub element: FrameElement,
}

/// A render batch targeted to a specific client.
#[derive(Debug)]
pub struct ClientRenderBatch {
    pub client_id: u64,
    pub batch: RenderBatch,
}

struct ClientEntry {
    state: ClientState,
    dirty: DirtyTracker,
}

/// Renderer Host: orchestrates the FrameElement → RenderCommand pipeline.
///
/// Manages per-client state, dirty tracking, capability degradation,
/// and frame sequencing with backpressure.
pub struct RendererHost {
    clients: HashMap<u64, ClientEntry>,
    theme: ThemeResolver,
    walk_config: WalkConfig,
}

impl RendererHost {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            theme: ThemeResolver::new(),
            walk_config: WalkConfig::default(),
        }
    }

    /// Register a new client with its capabilities.
    pub fn register_client(&mut self, client_id: u64, capabilities: ClientCapabilities) {
        self.clients.insert(
            client_id,
            ClientEntry {
                state: ClientState::new(client_id, capabilities),
                dirty: DirtyTracker::new(),
            },
        );
    }

    /// Remove a client.
    pub fn remove_client(&mut self, client_id: u64) {
        self.clients.remove(&client_id);
    }

    /// Process a frame acknowledgment from a client.
    pub fn ack_frame(&mut self, client_id: u64, frame_seq: u64) {
        if let Some(entry) = self.clients.get_mut(&client_id) {
            entry.state.ack(frame_seq);
        }
    }

    /// Process a frame: walk all pane elements, diff, and produce per-client render batches.
    ///
    /// Returns render batches only for clients that are not lagging.
    pub fn process_frame(
        &mut self,
        panes: &[PaneFrame],
        layout: &[ResolvedPane],
    ) -> Vec<ClientRenderBatch> {
        let mut batches = Vec::new();

        // Build a map of pane_id → resolved position
        let pane_layout: HashMap<u32, &ResolvedPane> =
            layout.iter().filter(|p| p.visible).map(|p| (p.pane_id.0, p)).collect();

        let client_ids: Vec<u64> = self.clients.keys().copied().collect();
        for client_id in client_ids {
            let entry = self.clients.get_mut(&client_id).unwrap();

            // Skip lagging clients
            if entry.state.check_status() == AckStatus::Lagging {
                continue;
            }

            // Walk all visible panes and collect commands
            let mut all_commands = Vec::new();
            for pane in panes {
                if let Some(resolved) = pane_layout.get(&pane.pane_id.0) {
                    let result = walk_frame(
                        &pane.element,
                        resolved.x,
                        resolved.y,
                        resolved.width,
                        resolved.height,
                        entry.state.capabilities(),
                        &self.walk_config,
                    );
                    all_commands.extend(result.commands);
                }
            }

            // Diff against previous frame
            let delta = entry.dirty.diff(&all_commands);
            if delta.is_empty() {
                continue;
            }

            let seq = entry.state.advance_seq();
            batches.push(ClientRenderBatch {
                client_id,
                batch: RenderBatch {
                    frame_seq: seq,
                    commands: delta,
                    _unknown: Vec::new(),
                },
            });
        }

        batches
    }

    /// Snapshot the current state for a newly attached client.
    pub fn snapshot_initial_state(
        &self,
        panes: &[PaneFrame],
        layout: &[ResolvedPane],
        client_id: u64,
    ) -> InitialState {
        let caps = self
            .clients
            .get(&client_id)
            .map(|e| e.state.capabilities().clone())
            .unwrap_or_else(|| ClientCapabilities {
                color_depth: malt_protocol::common::ColorDepth::TrueColor,
                unicode: malt_protocol::common::UnicodeLevel::Full,
                image_protocol: malt_protocol::common::ImageProtocol::None,
                overlay: false,
                vt_passthrough: true,
                max_fps: 60,
                _unknown: Vec::new(),
            });

        let pane_layout: HashMap<u32, &ResolvedPane> =
            layout.iter().filter(|p| p.visible).map(|p| (p.pane_id.0, p)).collect();

        let mut commands = Vec::new();
        for pane in panes {
            if let Some(resolved) = pane_layout.get(&pane.pane_id.0) {
                let result = walk_frame(
                    &pane.element,
                    resolved.x,
                    resolved.y,
                    resolved.width,
                    resolved.height,
                    &caps,
                    &self.walk_config,
                );
                commands.extend(result.commands);
            }
        }

        InitialState {
            frame_seq: self
                .clients
                .get(&client_id)
                .map(|e| e.state.frame_seq())
                .unwrap_or(0),
            layout: LayoutNode::Leaf {
                pane_id: PaneId(0),
                _unknown: Vec::new(),
            }, // Placeholder — real layout tree passed through when daemon integrates
            panes: layout.to_vec(),
            commands,
            _unknown: Vec::new(),
        }
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

impl Default for RendererHost {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p malt-renderer --test host`
Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/malt-renderer/src/host.rs crates/malt-renderer/tests/host.rs
git commit -m "feat(malt-renderer): renderer host — pipeline orchestration, per-client state"
```

---

### Task 7: Module Re-exports and Final Verification

**Files:**
- Modify: `crates/malt-renderer/src/lib.rs`

- [ ] **Step 1: Update lib.rs with clean re-exports**

```rust
pub mod client_state;
pub mod dirty;
pub mod error;
pub mod host;
pub mod theme;
pub mod walker;

pub use error::RendererError;
pub use host::{ClientRenderBatch, PaneFrame, RendererHost};
pub use walker::{WalkConfig, WalkResult};
```

- [ ] **Step 2: Run all malt-renderer tests**

Run: `cargo test -p malt-renderer`
Expected: all tests PASS (10 walker + 5 dirty + 8 client_state + 6 host = 29 total)

- [ ] **Step 3: Run full workspace check**

Run: `cargo check --workspace`
Expected: compiles

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -p malt-renderer -- -W clippy::all -A unused-imports`
Expected: no warnings from malt-renderer code

- [ ] **Step 5: Fix any clippy issues**

- [ ] **Step 6: Commit**

```bash
git add crates/malt-renderer/src/lib.rs
git commit -m "feat(malt-renderer): clean re-exports for public API"
```

- [ ] **Step 7: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all workspace tests PASS (~711 total: 682 existing + 29 new)
