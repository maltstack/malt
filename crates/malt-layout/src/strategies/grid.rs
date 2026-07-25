//! Grid tiling strategy — NxM auto-grid.
//!
//! Panes are arranged in a grid with `ceil(sqrt(N))` columns and `ceil(N/cols)`
//! rows. The last row may contain fewer cells than the others. All cells have
//! equal size (Ratio-based).

use crate::ops::collect_pane_ids;
use crate::strategies::make_equal_split;
use crate::strategy::LayoutStrategy;
use crate::Rect;
use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Grid tiling strategy.
pub struct GridStrategy;

impl LayoutStrategy for GridStrategy {
    fn name(&self) -> &str {
        "grid"
    }

    fn add_pane(
        &self,
        tree: &LayoutNode,
        terminal: Rect,
        new_pane: PaneId,
        _new_split: SplitId,
    ) -> LayoutNode {
        let mut panes = collect_pane_ids(tree);
        panes.push(new_pane);
        self.rebuild(&panes, terminal)
    }

    fn remove_pane(&self, tree: &LayoutNode, target: PaneId) -> Option<LayoutNode> {
        let panes: Vec<PaneId> = collect_pane_ids(tree)
            .into_iter()
            .filter(|id| *id != target)
            .collect();
        if panes.is_empty() {
            return None;
        }
        Some(self.rebuild(&panes, Rect::new(0, 0, 0, 0)))
    }

    fn rebuild(&self, panes: &[PaneId], _terminal: Rect) -> LayoutNode {
        if panes.is_empty() {
            return LayoutNode::Leaf { pane_id: PaneId(0) };
        }
        if panes.len() == 1 {
            return LayoutNode::Leaf {
                pane_id: panes[0].clone(),
            };
        }

        let n = panes.len();
        let cols = ceil_sqrt(n);
        let rows = n.div_ceil(cols); // ceil(n / cols)

        let row_ratio = 1.0 / rows as f32;

        let mut row_nodes: Vec<LayoutNode> = Vec::with_capacity(rows);
        let mut offset = 0;

        for r in 0..rows {
            let row_count = if r < rows - 1 {
                cols
            } else {
                n - offset // last row gets remaining panes
            };
            let row_panes = &panes[offset..offset + row_count];
            // Each row is a vertical split of cells (or a single leaf).
            let row_node = make_equal_split(row_panes, Direction::Vertical, (r as u32 + 1) * 10);
            row_nodes.push(row_node);
            offset += row_count;
        }

        if row_nodes.len() == 1 {
            return row_nodes.into_iter().next().expect("checked non-empty");
        }

        LayoutNode::Split {
            split_id: SplitId(1),
            direction: Direction::Horizontal,
            sizes: (0..rows)
                .map(|_| SplitSize::Ratio { value: row_ratio })
                .collect(),
            children: row_nodes,
        }
    }
}

/// Integer ceiling of sqrt(n). For n=0 returns 0.
fn ceil_sqrt(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let s = (n as f64).sqrt().ceil() as usize;
    // Guard against float imprecision: if (s-1)*(s-1) >= n, use s-1.
    if s > 1 && (s - 1) * (s - 1) >= n {
        s - 1
    } else {
        s
    }
}
