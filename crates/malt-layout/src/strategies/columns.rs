//! Columns tiling strategy — equal vertical columns.
//!
//! Each pane gets its own equal-width column. When `max_columns` is set and the
//! pane count exceeds it, overflow panes are stacked horizontally in the last
//! column.

use crate::ops::collect_pane_ids;
use crate::strategies::make_equal_split;
use crate::strategy::LayoutStrategy;
use crate::Rect;
use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Columns tiling strategy.
pub struct ColumnsStrategy {
    /// Maximum number of columns. `None` means unlimited (one column per pane).
    pub max_columns: Option<usize>,
}

impl LayoutStrategy for ColumnsStrategy {
    fn name(&self) -> &str {
        "columns"
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

        // If no max or panes fit within max, simple equal vertical split.
        let max = self.max_columns.unwrap_or(usize::MAX);
        if panes.len() <= max {
            return make_equal_split(panes, Direction::Vertical, 1);
        }

        // Overflow: first (max-1) panes each get a column, remaining stack in last column.
        let (head, overflow) = panes.split_at(max - 1);
        let n = max;
        let ratio = 1.0 / n as f32;

        let mut children: Vec<LayoutNode> = head
            .iter()
            .map(|id| LayoutNode::Leaf {
                pane_id: id.clone(),
            })
            .collect();

        // Last column: horizontal stack of overflow panes.
        let last_col = make_equal_split(overflow, Direction::Horizontal, 2);
        children.push(last_col);

        LayoutNode::Split {
            split_id: SplitId(1),
            direction: Direction::Vertical,
            sizes: (0..n).map(|_| SplitSize::Ratio { value: ratio }).collect(),
            children,
        }
    }
}
