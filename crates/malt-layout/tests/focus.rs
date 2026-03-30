use malt_layout::focus::focus_direction;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::*;

#[test]
fn focus_right_in_vsplit() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    let next = focus_direction(&panes, PaneId(1), FocusDir::Right);
    assert_eq!(next, Some(PaneId(2)));
}

#[test]
fn focus_left_returns_from_right() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(2),
        &LayoutConfig::default(),
    );
    let next = focus_direction(&panes, PaneId(2), FocusDir::Left);
    assert_eq!(next, Some(PaneId(1)));
}

#[test]
fn focus_single_pane_returns_none() {
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(focus_direction(&panes, PaneId(1), FocusDir::Right), None);
}

#[test]
fn focus_next_cycles() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(2),
        &LayoutConfig::default(),
    );
    let next = focus_direction(&panes, PaneId(2), FocusDir::Next);
    assert_eq!(next, Some(PaneId(1))); // wraps around
}
