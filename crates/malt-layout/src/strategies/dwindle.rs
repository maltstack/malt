//! Dwindle tiling strategy — Hyprland / bspwm style.
//!
//! Each new pane is placed by splitting the last pane in the list, alternating
//! between vertical and horizontal splits at increasing depth.  The first pane
//! occupies one side of the root split and all remaining panes spiral into the
//! other side, producing the characteristic "dwindling" rectangles.

use crate::ops::collect_pane_ids;
use crate::strategy::LayoutStrategy;
use crate::Rect;
use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Dwindle tiling strategy.
///
/// `ratio` controls the split proportion at each level (default 0.5 = equal halves).
pub struct DwindleStrategy {
    pub ratio: f32,
}

impl Default for DwindleStrategy {
    fn default() -> Self {
        Self { ratio: 0.5 }
    }
}

impl DwindleStrategy {
    fn build_dwindle(&self, panes: &[PaneId], depth: usize) -> LayoutNode {
        debug_assert!(!panes.is_empty());
        if panes.len() == 1 {
            return LayoutNode::Leaf {
                pane_id: panes[0].clone(),
            };
        }

        let dir = if depth % 2 == 0 {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };

        // Dwindle pattern: first pane on the left/top, rest spiral into right/bottom.
        let left = &panes[..1];
        let right = &panes[1..];

        LayoutNode::Split {
            split_id: SplitId(depth as u32 + 1),
            direction: dir,
            sizes: vec![
                SplitSize::Ratio { value: self.ratio },
                SplitSize::Ratio {
                    value: 1.0 - self.ratio,
                },
            ],
            children: vec![
                self.build_dwindle(left, depth + 1),
                self.build_dwindle(right, depth + 1),
            ],
        }
    }
}

impl LayoutStrategy for DwindleStrategy {
    fn name(&self) -> &str {
        "dwindle"
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
            // Shouldn't happen in normal use — return a placeholder leaf.
            return LayoutNode::Leaf {
                pane_id: PaneId(0),
            };
        }
        if panes.len() == 1 {
            return LayoutNode::Leaf {
                pane_id: panes[0].clone(),
            };
        }
        self.build_dwindle(panes, 0)
    }
}
