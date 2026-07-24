//! Master-Stack tiling strategy — dwm / xmonad style.
//!
//! The first N panes (configurable via `master_count`) occupy a "master" area on
//! the left side of the screen.  All remaining panes are stacked on the right.
//! `master_ratio` controls how much horizontal space the master area receives.

use crate::ops::collect_pane_ids;
use crate::strategies::make_equal_split;
use crate::strategy::LayoutStrategy;
use crate::Rect;
use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Master-Stack tiling strategy.
pub struct MasterStackStrategy {
    /// How many panes occupy the master area (default 1).
    pub master_count: usize,
    /// Fraction of width given to the master area (default 0.55).
    pub master_ratio: f32,
    /// Direction in which stack panes are arranged (default Horizontal = stacked vertically).
    pub stack_direction: Direction,
}

impl Default for MasterStackStrategy {
    fn default() -> Self {
        Self {
            master_count: 1,
            master_ratio: 0.55,
            stack_direction: Direction::Horizontal,
        }
    }
}

impl LayoutStrategy for MasterStackStrategy {
    fn name(&self) -> &str {
        "master-stack"
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

        // If all panes fit within master_count, just arrange them evenly.
        if panes.len() <= self.master_count {
            if panes.len() == 1 {
                return LayoutNode::Leaf {
                    pane_id: panes[0].clone(),
                };
            }
            // Multiple masters, no stack: split them vertically (side by side).
            return make_equal_split(panes, Direction::Vertical, 1);
        }

        let (masters, stack) = panes.split_at(self.master_count);

        let master_node = if masters.len() == 1 {
            LayoutNode::Leaf {
                pane_id: masters[0].clone(),
            }
        } else {
            // Multiple masters stacked horizontally (top/bottom within master area).
            make_equal_split(masters, Direction::Horizontal, 2)
        };

        let stack_node = make_equal_split(stack, self.stack_direction, 3);

        // Root split: master on left, stack on right.
        LayoutNode::Split {
            split_id: SplitId(1),
            direction: Direction::Vertical,
            sizes: vec![
                SplitSize::Ratio {
                    value: self.master_ratio,
                },
                SplitSize::Ratio {
                    value: 1.0 - self.master_ratio,
                },
            ],
            children: vec![master_node, stack_node],
        }
    }
}
