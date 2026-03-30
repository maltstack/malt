//! LayoutStrategy trait for WM-style auto-tiling.

use crate::Rect;
use malt_protocol::common::{LayoutNode, PaneId, SplitId};

/// Layout strategy for WM-style auto-tiling.
///
/// Implementations (Dwindle, MasterStack, etc.) come in a follow-up sub-project.
pub trait LayoutStrategy: Send + Sync {
    /// Human-readable name for this strategy (e.g. "dwindle", "master-stack").
    fn name(&self) -> &str;

    /// Insert a new pane into the layout tree according to strategy rules.
    fn add_pane(
        &self,
        tree: &LayoutNode,
        terminal: Rect,
        new_pane: PaneId,
        new_split: SplitId,
    ) -> LayoutNode;

    /// Remove a pane. Default delegates to [`crate::ops::close_pane`].
    fn remove_pane(&self, tree: &LayoutNode, target: PaneId) -> Option<LayoutNode> {
        crate::ops::close_pane(tree, target)
    }

    /// Rebuild the entire layout from scratch given a list of pane IDs.
    fn rebuild(&self, panes: &[PaneId], terminal: Rect) -> LayoutNode;
}
