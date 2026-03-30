# `malt-layout` Tiling Layout Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the tiling layout engine — N-ary tree resolution with gaps, tree operations, focus navigation, and the LayoutStrategy trait for WM-style auto-tiling.

**Architecture:** Pure computation crate. Takes abstract `LayoutNode` tree + terminal dimensions + gap config, produces `Vec<ResolvedPane>`. All operations return new trees (immutable). Uses schema types from `malt-protocol`. The `LayoutStrategy` trait enables future WM strategies (Dwindle, MasterStack, etc.).

**Tech Stack:** Rust, malt-protocol (schema types only)

**Spec:** `malt/specs/phase2-malt-layout.md`

**Reference:** `C:\Users\mamuk\projects\vexil-v2\vexil-layout\src\lib.rs` (951 lines, binary-tree) — rewrite as N-ary with tabbed, float, gaps.

---

## File Structure

```
orix/malt/crates/malt-layout/
  Cargo.toml
  src/
    lib.rs              # LayoutConfig, Rect, re-exports
    resolve.rs          # compute_resolved_panes()
    ops.rs              # split_pane, close_pane, swap_panes, resize_split, collect_pane_ids
    focus.rs            # focus_direction()
    strategy.rs         # LayoutStrategy trait
  tests/
    resolve.rs
    ops.rs
    focus.rs
```

---

## Task 1: Crate Scaffold + Rect + LayoutConfig

**Files:**
- Modify: `orix/malt/Cargo.toml` (add workspace member)
- Create: `orix/malt/crates/malt-layout/Cargo.toml`
- Create: all `src/` files as stubs
- Create: `tests/` stubs

- [ ] **Step 1: Create crate**

Add `"crates/malt-layout"` to workspace members.

Create `Cargo.toml`:
```toml
[package]
name = "malt-layout"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Tiling layout engine for MALT — N-ary splits, tabbed, float, gaps"

[dependencies]
malt-protocol = { path = "../malt-protocol" }
```

- [ ] **Step 2: Write lib.rs with core types**

```rust
//! Tiling layout engine — N-ary splits, tabbed, float, gaps, WM strategy support.

pub mod resolve;
pub mod ops;
pub mod focus;
pub mod strategy;

/// Layout configuration — gaps and minimum pane sizes.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub inner_gap: u16,
    pub outer_gap: u16,
    pub min_cols: u16,
    pub min_rows: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { inner_gap: 0, outer_gap: 0, min_cols: 8, min_rows: 4 }
    }
}

/// Screen rectangle in cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self { Self { x, y, w, h } }
    pub fn center_x(&self) -> u16 { self.x + self.w / 2 }
    pub fn center_y(&self) -> u16 { self.y + self.h / 2 }

    /// Inset the rect by `gap` on all sides.
    pub fn inset(&self, gap: u16) -> Self {
        Self {
            x: self.x + gap,
            y: self.y + gap,
            w: self.w.saturating_sub(gap * 2),
            h: self.h.saturating_sub(gap * 2),
        }
    }
}
```

Create stub modules (each with `use malt_protocol::*;` or just doc comment).

- [ ] **Step 3: Verify and commit**

```bash
cd orix/malt && cargo check -p malt-layout
git add Cargo.toml crates/malt-layout/
git commit -m "feat(malt-layout): scaffold crate — LayoutConfig, Rect, module structure"
```

---

## Task 2: Resolution Algorithm

**Files:**
- Modify: `src/resolve.rs`
- Create: `tests/resolve.rs`

The core algorithm. Port logic from reference's `compute_rects()` but extended for N-ary, tabbed, float, and gaps.

- [ ] **Step 1: Implement compute_resolved_panes**

```rust
//! Layout resolution — tree to concrete screen positions.

use crate::{LayoutConfig, Rect};
use malt_protocol::common::*;

pub fn compute_resolved_panes(
    tree: &LayoutNode,
    terminal: Rect,
    focused: PaneId,
    config: &LayoutConfig,
) -> Vec<ResolvedPane> {
    let usable = terminal.inset(config.outer_gap);
    let mut panes = Vec::new();
    let mut float_z = 1u32;
    resolve_node(tree, usable, focused, config, &mut panes, &mut float_z, 0);
    panes
}
```

The internal `resolve_node` function:

**Leaf:** Create `ResolvedPane` with the given rect. Set `focused = (pane_id == focused_param)`.

**Split (N-ary):**
1. Determine axis: Horizontal → split height, Vertical → split width
2. Calculate total available = axis_size - (N-1) * inner_gap
3. First pass: allocate Fixed sizes, collect Min/Max constraints
4. Second pass: distribute remaining space to Ratio children proportionally
5. Apply Min/Max clamping
6. Assign sub-rects to each child, spaced by inner_gap
7. Recurse into each child

**Tabbed:**
All children get the same rect. `active` child: `visible: true`. Others: `visible: false`, `tab_context: Some(TabContext { label: "Pane N", is_active: false, tab_index: i })`.

**Float:**
Use absolute `x, y, width, height`. `z_order = float_z` (increment per float). Don't subtract from tiled space.

- [ ] **Step 2: Write resolution tests**

```rust
use malt_layout::{LayoutConfig, Rect};
use malt_layout::resolve::compute_resolved_panes;
use malt_protocol::common::*;

fn default_config() -> LayoutConfig { LayoutConfig::default() }

#[test]
fn single_leaf_fills_terminal() {
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].width, 80);
    assert_eq!(panes[0].height, 24);
    assert!(panes[0].focused);
}

#[test]
fn vertical_split_two_children() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![SplitSize::Ratio { value: 0.5 }, SplitSize::Ratio { value: 0.5 }],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].width, 40);
    assert_eq!(panes[1].width, 40);
    assert_eq!(panes[0].x, 0);
    assert_eq!(panes[1].x, 40);
}

#[test]
fn horizontal_split_two_children() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Horizontal,
        sizes: vec![SplitSize::Ratio { value: 0.5 }, SplitSize::Ratio { value: 0.5 }],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].height, 12);
    assert_eq!(panes[1].height, 12);
}

#[test]
fn three_way_split_ratios() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.25 },
            SplitSize::Ratio { value: 0.25 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
            LayoutNode::Leaf { pane_id: PaneId(3) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 3);
    assert_eq!(panes[0].width, 40); // 50%
    assert_eq!(panes[1].width, 20); // 25%
}

#[test]
fn fixed_size_allocation() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Fixed { value: 20 },
            SplitSize::Ratio { value: 1.0 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes[0].width, 20);
    assert_eq!(panes[1].width, 60);
}

#[test]
fn inner_gap_creates_spacing() {
    let config = LayoutConfig { inner_gap: 2, ..Default::default() };
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![SplitSize::Ratio { value: 0.5 }, SplitSize::Ratio { value: 0.5 }],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    // 80 - 2 (gap) = 78, split into 39 + 39
    assert_eq!(panes[0].width + panes[1].width + config.inner_gap, 80);
    assert!(panes[1].x > panes[0].x + panes[0].width); // gap between
}

#[test]
fn outer_gap_insets() {
    let config = LayoutConfig { outer_gap: 1, ..Default::default() };
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    assert_eq!(panes[0].x, 1);
    assert_eq!(panes[0].y, 1);
    assert_eq!(panes[0].width, 78);
    assert_eq!(panes[0].height, 22);
}

#[test]
fn tabbed_only_active_visible() {
    let tree = LayoutNode::Tabbed {
        split_id: SplitId(1),
        active: 1,
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
            LayoutNode::Leaf { pane_id: PaneId(3) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(2), &default_config());
    assert_eq!(panes.len(), 3);
    let visible: Vec<_> = panes.iter().filter(|p| p.visible).collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].pane_id, PaneId(2));
}

#[test]
fn float_absolute_position() {
    let tree = LayoutNode::Float {
        pane_id: PaneId(1),
        x: 10, y: 5, width: 30, height: 10,
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes[0].x, 10);
    assert_eq!(panes[0].y, 5);
    assert_eq!(panes[0].width, 30);
    assert_eq!(panes[0].z_order, 1); // above tiled
}

#[test]
fn min_size_enforcement() {
    let config = LayoutConfig { min_cols: 8, ..Default::default() };
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![SplitSize::Ratio { value: 0.99 }, SplitSize::Ratio { value: 0.01 }],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    assert!(panes[1].width >= 8); // enforced minimum
}
```

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt && cargo test -p malt-layout --test resolve
git add crates/malt-layout/
git commit -m "feat(malt-layout): N-ary resolution — splits, tabbed, float, gaps, min sizes"
```

---

## Task 3: Tree Operations

**Files:**
- Modify: `src/ops.rs`
- Create: `tests/ops.rs`

Implement: `split_pane`, `close_pane`, `swap_panes`, `resize_split`, `collect_pane_ids`.

- [ ] **Step 1: Implement tree operations**

All operations walk the tree recursively, creating new nodes (immutable pattern). Key algorithms:

**split_pane:** Find Leaf with target PaneId. If parent is a Split with same direction, insert new child. Otherwise, replace Leaf with new Split containing old + new.

**close_pane:** Find Leaf, remove it. If parent Split has 2→1 child, collapse to remaining child. If N→N-1, just remove from array.

**swap_panes:** Walk tree, when finding Leaf(a) replace with Leaf(b) and vice versa.

**resize_split:** Find Split by SplitId, update the size array at child_index.

**collect_pane_ids:** Recursive traversal collecting all PaneIds.

Port logic from reference's `split()`, `close()`, `swap()` but adapted for N-ary.

- [ ] **Step 2: Write operation tests**

```rust
#[test]
fn split_creates_new_child() { ... }
#[test]
fn split_same_direction_extends() { ... } // no nesting
#[test]
fn close_collapses_two_children() { ... }
#[test]
fn close_removes_from_three() { ... }
#[test]
fn close_root_returns_none() { ... }
#[test]
fn swap_exchanges_pane_ids() { ... }
#[test]
fn resize_changes_split_size() { ... }
#[test]
fn collect_ids_all_variants() { ... }
```

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(malt-layout): tree operations — split, close, swap, resize, collect"
```

---

## Task 4: Focus Navigation + Strategy Trait

**Files:**
- Modify: `src/focus.rs`
- Modify: `src/strategy.rs`
- Create: `tests/focus.rs`

- [ ] **Step 1: Implement focus_direction**

Geometry-based: filter visible panes in requested direction from focused center, return nearest by Euclidean distance. Next/Prev: cycle in traversal order.

```rust
pub fn focus_direction(
    panes: &[ResolvedPane], focused: PaneId, dir: FocusDir,
) -> Option<PaneId> { ... }
```

- [ ] **Step 2: Define LayoutStrategy trait**

```rust
pub trait LayoutStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn add_pane(&self, tree: &LayoutNode, terminal: Rect, new_pane: PaneId, new_split: SplitId) -> LayoutNode;
    fn remove_pane(&self, tree: &LayoutNode, target: PaneId) -> Option<LayoutNode> {
        crate::ops::close_pane(tree, target)
    }
    fn rebuild(&self, panes: &[PaneId], terminal: Rect) -> LayoutNode;
}
```

Trait only — implementations come in the WM strategies sub-project.

- [ ] **Step 3: Write focus tests**

```rust
#[test]
fn focus_right_in_vsplit() { ... }
#[test]
fn focus_down_in_hsplit() { ... }
#[test]
fn focus_next_cycles() { ... }
#[test]
fn focus_skips_invisible() { ... }
#[test]
fn focus_single_pane_returns_none() { ... }
```

- [ ] **Step 4: Run full suite and commit**

```bash
cd orix/malt && cargo test -p malt-layout
cargo test --workspace
git commit -m "feat(malt-layout): focus navigation + LayoutStrategy trait"
```

---

## Verification

After all tasks:

1. `cargo test -p malt-layout` — all tests pass
2. `cargo test --workspace` — 470+ tests, 0 failures
3. Resolution handles all 4 LayoutNode variants with gaps
4. Operations maintain tree integrity (split, close, swap, resize)
5. Focus works geometrically across splits, skips invisible tabs
6. LayoutStrategy trait compiles (no implementations yet)
