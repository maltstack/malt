//! Monocle tiling strategy — fullscreen tabbed.
//!
//! All panes are placed in a single Tabbed node. Only the active pane is visible
//! at full size. This mirrors the "monocle" mode found in bspwm, dwm, etc.

use crate::ops::collect_pane_ids;
use crate::strategy::LayoutStrategy;
use crate::Rect;
use malt_protocol::common::{LayoutNode, PaneId, SplitId};

/// Monocle tiling strategy — one pane visible at a time, fullscreen.
pub struct MonocleStrategy;

impl LayoutStrategy for MonocleStrategy {
    fn name(&self) -> &str {
        "monocle"
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
        // Rebuild with new pane as active (last index).
        let tree = self.rebuild(&panes, terminal);
        match tree {
            LayoutNode::Tabbed {
                split_id, children, ..
            } => LayoutNode::Tabbed {
                split_id,
                active: (children.len().saturating_sub(1)) as u16,
                children,
            },
            other => other,
        }
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

        LayoutNode::Tabbed {
            split_id: SplitId(1),
            active: 0,
            children: panes
                .iter()
                .map(|id| LayoutNode::Leaf {
                    pane_id: id.clone(),
                })
                .collect(),
        }
    }
}
