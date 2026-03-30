use malt_layout::ops::{close_pane, collect_pane_ids, split_pane, swap_panes};
use malt_protocol::common::*;

#[test]
fn split_creates_new_child() {
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let result = split_pane(
        &tree,
        PaneId(1),
        Direction::Vertical,
        SplitSize::Ratio { value: 0.5 },
        PaneId(2),
        SplitId(1),
    );
    assert!(result.is_some());
    let ids = collect_pane_ids(&result.unwrap());
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&PaneId(1)));
    assert!(ids.contains(&PaneId(2)));
}

#[test]
fn close_collapses_binary_split() {
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
    let result = close_pane(&tree, PaneId(1)).unwrap();
    let ids = collect_pane_ids(&result);
    assert_eq!(ids, vec![PaneId(2)]);
}

#[test]
fn close_root_returns_none() {
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    assert!(close_pane(&tree, PaneId(1)).is_none());
}

#[test]
fn swap_exchanges_ids() {
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
    let result = swap_panes(&tree, PaneId(1), PaneId(2)).unwrap();
    let ids = collect_pane_ids(&result);
    assert_eq!(ids, vec![PaneId(2), PaneId(1)]); // swapped order
}

#[test]
fn collect_ids_all_variants() {
    let tree = LayoutNode::Split {
        split_id: SplitId(1),
        direction: Direction::Vertical,
        sizes: vec![
            SplitSize::Ratio { value: 0.5 },
            SplitSize::Ratio { value: 0.5 },
        ],
        children: vec![
            LayoutNode::Leaf { pane_id: PaneId(1) },
            LayoutNode::Tabbed {
                split_id: SplitId(2),
                active: 0,
                children: vec![
                    LayoutNode::Leaf { pane_id: PaneId(2) },
                    LayoutNode::Leaf { pane_id: PaneId(3) },
                ],
            },
        ],
    };
    let ids = collect_pane_ids(&tree);
    assert_eq!(ids.len(), 3);
}
