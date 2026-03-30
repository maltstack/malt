//! WM-style tiling strategies for automatic layout management.

pub mod columns;
pub mod dwindle;
pub mod grid;
pub mod master_stack;
pub mod monocle;

use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Create an N-ary split with equal-ratio children.
///
/// Shared helper used by multiple strategies. If `panes` has a single element,
/// returns a Leaf instead of a Split.
pub(crate) fn make_equal_split(panes: &[PaneId], direction: Direction, base_split_id: u32) -> LayoutNode {
    debug_assert!(!panes.is_empty());
    if panes.len() == 1 {
        return LayoutNode::Leaf {
            pane_id: panes[0].clone(),
        };
    }
    let n = panes.len();
    let ratio = 1.0 / n as f32;
    LayoutNode::Split {
        split_id: SplitId(base_split_id),
        direction,
        sizes: (0..n).map(|_| SplitSize::Ratio { value: ratio }).collect(),
        children: panes
            .iter()
            .map(|id| LayoutNode::Leaf {
                pane_id: id.clone(),
            })
            .collect(),
    }
}
