# Phase 2 Extension: Layout WM Strategies

## Goal

Implement 5 tiling window manager strategies for the malt-layout engine: Dwindle, MasterStack, Columns, Monocle, Grid. Each implements the existing `LayoutStrategy` trait.

## Architecture

All strategies live in `malt-layout/src/strategies/`. They use the existing tree operations (`split_pane`, `close_pane`) and produce `LayoutNode` trees that the existing `compute_resolved_panes` resolves. Pure computation — no I/O.

---

## Strategies

### Dwindle (default)

Hyprland's spiral layout. Each new pane splits the smallest existing pane, alternating direction.

```rust
pub struct DwindleStrategy {
    pub ratio: f32,              // split ratio (default 0.5)
}
```

- `add_pane`: find the leaf with smallest resolved area, split it. Direction alternates by depth (even=Vertical, odd=Horizontal).
- `rebuild`: recursively pair panes into a binary tree — `[A,B,C,D]` → Split(V, Split(H,A,B), Split(H,C,D))
- `remove_pane`: delegate to `close_pane`, no restructuring needed (tree self-balances on next add)

### MasterStack

One large master area + stacked secondary panes.

```rust
pub struct MasterStackStrategy {
    pub master_count: usize,     // panes in master area (default 1)
    pub master_ratio: f32,       // master area width ratio (default 0.55)
    pub stack_direction: Direction, // how stack panes arrange (default Horizontal)
}
```

- `add_pane`: if total panes <= master_count, add to master. Else add to stack.
- `rebuild`: if N <= master_count, all panes in a single Split(master_direction). Else Split(Vertical, [master_panes with master_ratio], [stack_panes with remaining ratio]).
- `remove_pane`: rebuild from remaining panes (maintains master/stack balance)

### Columns

Equal-width vertical columns.

```rust
pub struct ColumnsStrategy {
    pub max_columns: Option<usize>,  // None = unlimited
}
```

- `add_pane`: add a new column to the root Split. All columns equal ratio.
- `rebuild`: N panes → root Split(Vertical, [Ratio(1/N)] * N, [Leaf] * N)
- `remove_pane`: rebuild from remaining panes

### Monocle

All panes fullscreen, tabbed. Only active visible.

```rust
pub struct MonocleStrategy;
```

- `add_pane`: add to root Tabbed node, new pane becomes active
- `rebuild`: single Tabbed node containing all panes
- `remove_pane`: remove from Tabbed children

### Grid

Automatic NxM grid.

```rust
pub struct GridStrategy;
```

- `add_pane`: rebuild with N+1 panes
- `rebuild`: for N panes, compute best grid (cols = ceil(sqrt(N)), rows = ceil(N/cols)). Create rows of Horizontal splits inside a Vertical split. Last row may have fewer panes.
- `remove_pane`: rebuild with remaining panes

---

## Testing

### Dwindle (5 tests)
- Add to empty → single leaf
- Add second → vertical split with 0.5 ratio
- Add third → nested split alternating direction
- Rebuild 4 panes → balanced binary tree
- Resolution produces non-overlapping rects

### MasterStack (5 tests)
- Single pane → single leaf
- Two panes → master + one stack
- Three panes → master + two stacked
- Master ratio applies (master gets ~55% width)
- master_count=2 puts two panes in master area

### Columns (5 tests)
- One pane → single leaf
- Three panes → three equal columns
- Resolution: columns have equal width
- max_columns=3 stacks within last column after limit
- Remove pane rebuilds with fewer columns

### Monocle (5 tests)
- Add creates Tabbed node
- Multiple panes → all in Tabbed, only active visible
- Resolution: all panes same rect, one visible
- Remove from Tabbed
- Rebuild preserves active index (or clamps)

### Grid (5 tests)
- 1 pane → single leaf
- 4 panes → 2x2 grid
- 5 panes → 3 cols (2+2+1 or 3+2)
- 9 panes → 3x3 grid
- Resolution: all cells approximately equal size
