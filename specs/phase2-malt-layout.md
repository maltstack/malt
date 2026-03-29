# Phase 2: `malt-layout` — Tiling Layout Engine Core

## Goal

Build the tiling layout engine — N-ary tree resolution with gaps, tree mutation operations, directional focus navigation, and the LayoutStrategy trait for WM-style auto-tiling. This is the backbone of the tiling window manager that makes MALT novel.

## Architecture

Pure computation crate — no I/O, no platform deps. Takes abstract `LayoutNode` tree + terminal dimensions + config, produces `Vec<ResolvedPane>` with concrete screen positions. All operations return new trees (persistent/immutable pattern). The `LayoutStrategy` trait is defined here; implementations (Dwindle, MasterStack, Columns, Monocle, Grid) come in a follow-up sub-project.

Uses schema types from `malt-protocol`: `LayoutNode`, `SplitSize`, `ResolvedPane`, `PaneId`, `SplitId`, `Direction`, `FocusDir`, `TabContext`.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-layout\src\lib.rs` (951 lines, binary-tree only). We're rewriting as N-ary with tabbed, float, gaps, and WM strategy support.

---

## Crate Structure

```
orix/malt/crates/malt-layout/
  Cargo.toml
  src/
    lib.rs              # Re-exports, LayoutConfig, Rect
    resolve.rs          # compute_resolved_panes() — N-ary with gaps
    ops.rs              # Tree mutations: split, close, swap, resize
    focus.rs            # Directional + cycle focus navigation
    strategy.rs         # LayoutStrategy trait definition
  tests/
    resolve.rs
    ops.rs
    focus.rs
```

---

## Dependencies

```toml
[dependencies]
malt-protocol = { path = "../malt-protocol" }
```

No other dependencies. Pure computation.

---

## Core Types

```rust
/// Layout configuration — gaps, minimum pane sizes.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Space between panes in cells (default 0).
    pub inner_gap: u16,
    /// Space from terminal edges in cells (default 0).
    pub outer_gap: u16,
    /// Minimum pane width in columns (default 8).
    pub min_cols: u16,
    /// Minimum pane height in rows (default 4).
    pub min_rows: u16,
}

/// Screen rectangle in cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}
```

---

## Resolution Algorithm

```rust
pub fn compute_resolved_panes(
    tree: &LayoutNode,
    terminal: Rect,
    focused: PaneId,
    config: &LayoutConfig,
) -> Vec<ResolvedPane>;
```

Walks the `LayoutNode` tree and resolves each variant:

### Leaf
Single pane fills the available rect. `focused` set by comparing with `focused` param. `visible: true`, `z_order: 0`.

### Split (N-ary)
1. Subtract `(N-1) * inner_gap` from available space (width for Vertical, height for Horizontal)
2. First pass: allocate Fixed sizes. Calculate remaining space.
3. Second pass: distribute remaining space among Ratio children proportionally. Apply Min/Max constraints.
4. Clamp: enforce `min_cols`/`min_rows` per child
5. Assign sub-rects with `inner_gap` spacing between them
6. Recurse into each child with its sub-rect

### Tabbed
All children share the same rect. The `active` child (by index) gets `visible: true`. Others get `visible: false` with `TabContext { label, is_active: false, tab_index }`. All children are resolved to the same position (only active is rendered).

### Float
Uses absolute `x, y, width, height` from the node. `z_order` set to 1+ (above tiled panes). Resolved independently — floats overlay the tiled tree.

### Gaps
```
Outer gap: inset terminal rect by outer_gap on all sides before processing
Inner gap: between each pair of adjacent children in a Split
```

### Minimum sizes
If a child's allocated space is below `min_cols`/`min_rows`, it gets the minimum and others are reduced to compensate. If total space can't accommodate all minimums, children are clipped.

---

## Tree Operations

All return a new `LayoutNode` (immutable tree pattern). Return `None` if the operation can't be performed.

### split_pane
```rust
pub fn split_pane(
    tree: &LayoutNode, target: PaneId, dir: Direction,
    size: SplitSize, new_pane: PaneId, new_split: SplitId,
) -> Option<LayoutNode>;
```
Find the Leaf with `target` PaneId. Replace it with a new Split containing the original + new pane. If the target is already inside a Split with the same direction, insert as a new child (extend the N-ary list) instead of nesting.

### close_pane
```rust
pub fn close_pane(tree: &LayoutNode, target: PaneId) -> Option<LayoutNode>;
```
Remove the Leaf. If parent Split has 2 children → collapse to the remaining child. If N>2 children → remove from array, redistribute sizes. If closing root Leaf → return None.

### swap_panes
```rust
pub fn swap_panes(tree: &LayoutNode, a: PaneId, b: PaneId) -> Option<LayoutNode>;
```
Exchange PaneIds of two Leaf nodes. Structure unchanged.

### resize_split
```rust
pub fn resize_split(
    tree: &LayoutNode, split_id: SplitId,
    child_index: u16, new_size: SplitSize,
) -> Option<LayoutNode>;
```
Find Split by SplitId, update the size at `child_index`. Redistributes remaining ratios among other Ratio children.

### collect_pane_ids
```rust
pub fn collect_pane_ids(tree: &LayoutNode) -> Vec<PaneId>;
```
In-order traversal collecting all PaneIds (including tabbed inactive and floats).

---

## Focus Navigation

```rust
pub fn focus_direction(
    panes: &[ResolvedPane], focused: PaneId, dir: FocusDir,
) -> Option<PaneId>;
```

Geometry-based navigation on resolved pane positions:

- **Up/Down/Left/Right** — Filter to visible panes in the requested direction from focused pane's center. Return nearest by Euclidean distance. Break ties by perpendicular distance.
- **Next/Prev** — Cycle through panes in tree traversal order (wrapping at ends).

Only considers `visible: true` panes (skips inactive tabs).

---

## LayoutStrategy Trait

Defined here, implemented in sub-project 2:

```rust
pub trait LayoutStrategy: Send + Sync {
    /// Strategy name for display/config.
    fn name(&self) -> &str;

    /// Add a new pane to the tree. Strategy decides placement.
    fn add_pane(
        &self, tree: &LayoutNode, terminal: Rect,
        new_pane: PaneId, new_split: SplitId,
    ) -> LayoutNode;

    /// Remove a pane and reorganize. Falls back to close_pane if not overridden.
    fn remove_pane(&self, tree: &LayoutNode, target: PaneId) -> Option<LayoutNode> {
        close_pane(tree, target)
    }

    /// Rebuild the entire tree from a list of panes. Used when switching strategies.
    fn rebuild(&self, panes: &[PaneId], terminal: Rect) -> LayoutNode;
}
```

**Manual mode** — no strategy. User calls `split_pane()` / `close_pane()` directly (tmux-style). The daemon can use either manual operations or a strategy.

**Future strategies (sub-project 2):**
- **Dwindle** — new pane splits the smallest pane, alternating direction (Hyprland default)
- **MasterStack** — one master pane (configurable ratio) + vertical/horizontal stack
- **Columns** — equal-width columns
- **Monocle** — all panes fullscreen, only active visible (tabbed at root)
- **Grid** — automatic NxM grid arrangement

---

## Testing Strategy

### Resolution (10+ tests)
- Single leaf fills terminal
- 2-way vertical split divides width
- 2-way horizontal split divides height
- 3-way split with Ratio sizes distributes proportionally
- Fixed + Ratio mixed sizes
- Min/Max constraints enforced
- Tabbed: active visible, others hidden with TabContext
- Float: absolute position, z_order > 0
- Inner gaps create spacing between panes
- Outer gaps inset from terminal edges
- Minimum size enforcement (small terminal)

### Operations (8+ tests)
- Split creates new child in parent
- Split same direction extends (no nesting)
- Close with 2 siblings collapses parent
- Close with 3+ siblings removes from array
- Close root returns None
- Swap exchanges two PaneIds
- Resize changes split size
- Collect pane ids traverses all variants

### Focus (6+ tests)
- Up/Down/Left/Right in 2x2 grid
- Next/Prev cycle
- Skip invisible (tabbed inactive)
- Float panes included in navigation
- Single pane: all directions return None
