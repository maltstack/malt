use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::*;

fn default_config() -> LayoutConfig {
    LayoutConfig::default()
}

#[test]
fn single_leaf_fills_terminal() {
    let tree = LayoutNode::Leaf {
        pane_id: PaneId(1),
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].width, 80);
    assert_eq!(panes[0].height, 24);
    assert!(panes[0].focused);
}

#[test]
fn vertical_split_two_children() {
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
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].width, 40);
    assert_eq!(panes[1].width, 40);
    assert_eq!(panes[0].x, 0);
    assert_eq!(panes[1].x, 40);
}

#[test]
fn horizontal_split_two_children() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Horizontal,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].height, 12);
    assert_eq!(panes[1].height, 12);
}

#[test]
fn three_way_split_ratios() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.25 },
            SplitSize::Ratio { value: 0.25 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
            LayoutNode::Leaf { pane_id: PaneId(3) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes.len(), 3);
    assert_eq!(panes[0].width, 40); // 50%
    assert_eq!(panes[1].width, 20); // 25%
}

#[test]
fn fixed_size_allocation() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Fixed { value: 20 },
            SplitSize::Ratio { value: 1.0 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes[0].width, 20);
    assert_eq!(panes[1].width, 60);
}

#[test]
fn inner_gap_creates_spacing() {
    let config = LayoutConfig {
        inner_gap: 2,
        ..Default::default()
    };
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
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    // 80 - 2 (gap) = 78, split into 39 + 39
    assert_eq!(panes[0].width + panes[1].width + config.inner_gap, 80);
    assert!(panes[1].x > panes[0].x + panes[0].width); // gap between
}

#[test]
fn outer_gap_insets() {
    let config = LayoutConfig {
        outer_gap: 1,
        ..Default::default()
    };
    let tree = LayoutNode::Leaf {
        pane_id: PaneId(1),
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    assert_eq!(panes[0].x, 1);
    assert_eq!(panes[0].y, 1);
    assert_eq!(panes[0].width, 78);
    assert_eq!(panes[0].height, 22);
}

#[test]
fn tabbed_only_active_visible() {
    let tree = LayoutNode::Tabbed {
        split_id: SplitId(1),
        active: 1,
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
            LayoutNode::Leaf { pane_id: PaneId(3) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(2), &default_config());
    assert_eq!(panes.len(), 3);
    let visible: Vec<_> = panes.iter().filter(|p| p.visible).collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].pane_id, PaneId(2));
}

#[test]
fn float_absolute_position() {
    let tree = LayoutNode::Float {
        pane_id: PaneId(1),
        x: 10,
        y: 5,
        width: 30,
        height: 10,
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &default_config());
    assert_eq!(panes[0].x, 10);
    assert_eq!(panes[0].y, 5);
    assert_eq!(panes[0].width, 30);
    assert_eq!(panes[0].z_order, 1); // above tiled
}

#[test]
fn min_size_enforcement() {
    let config = LayoutConfig {
        min_cols: 8,
        ..Default::default()
    };
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.99 },
            SplitSize::Ratio { value: 0.01 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Leaf { pane_id: PaneId(2) },
        ],
    };
    let panes = compute_resolved_panes(&tree, Rect::new(0, 0, 80, 24), PaneId(1), &config);
    assert!(panes[1].width >= 8); // enforced minimum
}
