use malt_layout::ops::collect_pane_ids;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::strategies::columns::ColumnsStrategy;
use malt_layout::strategies::dwindle::DwindleStrategy;
use malt_layout::strategies::grid::GridStrategy;
use malt_layout::strategies::master_stack::MasterStackStrategy;
use malt_layout::strategies::monocle::MonocleStrategy;
use malt_layout::strategy::LayoutStrategy;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId};

// ── Dwindle ──

#[test]
fn dwindle_single_pane() {
    let s = DwindleStrategy::default();
    let tree = s.rebuild(&[PaneId(1)], Rect::new(0, 0, 80, 24));
    assert!(matches!(tree, LayoutNode::Leaf { .. }));
}

#[test]
fn dwindle_two_panes() {
    let s = DwindleStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2)], Rect::new(0, 0, 80, 24));
    let ids = collect_pane_ids(&tree);
    assert_eq!(ids.len(), 2);
}

#[test]
fn dwindle_four_panes_resolution() {
    let s = DwindleStrategy::default();
    let tree = s.rebuild(
        &[PaneId(1), PaneId(2), PaneId(3), PaneId(4)],
        Rect::new(0, 0, 80, 24),
    );
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 4);
    for p in &panes {
        assert!(
            p.width > 0 && p.height > 0,
            "pane {:?} has zero dimension",
            p.pane_id
        );
    }
}

#[test]
fn dwindle_alternates_direction() {
    let s = DwindleStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    // Root split (depth 0) should be Vertical
    match &tree {
        LayoutNode::Split {
            direction,
            children,
            ..
        } => {
            assert_eq!(*direction, Direction::Vertical);
            // Second child is the nested split (depth 1) — should be Horizontal
            match &children[1] {
                LayoutNode::Split {
                    direction: inner_dir,
                    ..
                } => {
                    assert_eq!(*inner_dir, Direction::Horizontal);
                }
                _ => panic!("expected nested Split for 3-pane dwindle"),
            }
        }
        _ => panic!("expected Split at root"),
    }
}

#[test]
fn dwindle_add_pane() {
    let s = DwindleStrategy::default();
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let tree2 = s.add_pane(&tree, Rect::new(0, 0, 80, 24), PaneId(2), SplitId(1));
    let ids = collect_pane_ids(&tree2);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&PaneId(1)));
    assert!(ids.contains(&PaneId(2)));
}

#[test]
fn dwindle_remove_pane() {
    let s = DwindleStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let tree2 = s.remove_pane(&tree, PaneId(2));
    assert!(tree2.is_some());
    let ids = collect_pane_ids(&tree2.unwrap());
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&PaneId(2)));
}

#[test]
fn dwindle_remove_last_pane_returns_none() {
    let s = DwindleStrategy::default();
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    assert!(s.remove_pane(&tree, PaneId(1)).is_none());
}

#[test]
fn dwindle_custom_ratio() {
    let s = DwindleStrategy { ratio: 0.6 };
    let tree = s.rebuild(&[PaneId(1), PaneId(2)], Rect::new(0, 0, 100, 50));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 100, 50),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 2);
    let p1 = panes.iter().find(|p| p.pane_id == PaneId(1)).unwrap();
    // With ratio 0.6, first pane should be ~60 cols out of 100
    assert!(
        p1.width >= 58 && p1.width <= 62,
        "expected ~60, got {}",
        p1.width
    );
}

// ── MasterStack ──

#[test]
fn master_stack_single_pane() {
    let s = MasterStackStrategy::default();
    let tree = s.rebuild(&[PaneId(1)], Rect::new(0, 0, 80, 24));
    assert!(matches!(tree, LayoutNode::Leaf { .. }));
}

#[test]
fn master_stack_two_panes() {
    let s = MasterStackStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2)], Rect::new(0, 0, 80, 24));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 2);
    // Master (PaneId(1)) should be wider (~55% of 80 = 44)
    let master = panes.iter().find(|p| p.pane_id == PaneId(1)).unwrap();
    assert!(
        master.width > 40,
        "master width {} should be > 40",
        master.width
    );
}

#[test]
fn master_stack_three_panes_stack() {
    let s = MasterStackStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 3);
    let master = panes.iter().find(|p| p.pane_id == PaneId(1)).unwrap();
    let stack2 = panes.iter().find(|p| p.pane_id == PaneId(2)).unwrap();
    let stack3 = panes.iter().find(|p| p.pane_id == PaneId(3)).unwrap();
    // Master is wider than stack panes
    assert!(master.width > stack2.width);
    // Stack panes share the remaining space equally (same width, split height)
    assert_eq!(stack2.width, stack3.width);
}

#[test]
fn master_stack_master_count_2() {
    let s = MasterStackStrategy {
        master_count: 2,
        ..Default::default()
    };
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let ids = collect_pane_ids(&tree);
    assert_eq!(ids.len(), 3);
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 3);
    // All should have non-zero dimensions
    for p in &panes {
        assert!(p.width > 0 && p.height > 0);
    }
}

#[test]
fn master_stack_all_in_master() {
    // When pane count <= master_count, no stack area should exist.
    let s = MasterStackStrategy {
        master_count: 3,
        ..Default::default()
    };
    let tree = s.rebuild(&[PaneId(1), PaneId(2)], Rect::new(0, 0, 80, 24));
    let ids = collect_pane_ids(&tree);
    assert_eq!(ids.len(), 2);
    // Should be a simple equal split, not master/stack
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    let p1 = panes.iter().find(|p| p.pane_id == PaneId(1)).unwrap();
    let p2 = panes.iter().find(|p| p.pane_id == PaneId(2)).unwrap();
    assert_eq!(p1.width, p2.width);
}

#[test]
fn master_stack_add_pane() {
    let s = MasterStackStrategy::default();
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let tree2 = s.add_pane(&tree, Rect::new(0, 0, 80, 24), PaneId(2), SplitId(1));
    let ids = collect_pane_ids(&tree2);
    assert_eq!(ids.len(), 2);
}

#[test]
fn master_stack_remove_pane() {
    let s = MasterStackStrategy::default();
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let tree2 = s.remove_pane(&tree, PaneId(2));
    assert!(tree2.is_some());
    let ids = collect_pane_ids(&tree2.unwrap());
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&PaneId(2)));
}

#[test]
fn master_stack_remove_last_returns_none() {
    let s = MasterStackStrategy::default();
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    assert!(s.remove_pane(&tree, PaneId(1)).is_none());
}

// ── Columns ──

#[test]
fn columns_single_pane() {
    let s = ColumnsStrategy { max_columns: None };
    let tree = s.rebuild(&[PaneId(1)], Rect::new(0, 0, 80, 24));
    assert!(matches!(tree, LayoutNode::Leaf { .. }));
}

#[test]
fn columns_three_equal() {
    let s = ColumnsStrategy { max_columns: None };
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 90, 24));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 90, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 3);
    for p in &panes {
        assert!(p.width >= 28 && p.width <= 32, "width={}", p.width);
    }
}

#[test]
fn columns_max_stacks_overflow() {
    let s = ColumnsStrategy {
        max_columns: Some(2),
    };
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let ids = collect_pane_ids(&tree);
    assert_eq!(ids.len(), 3);
}

#[test]
fn columns_add_pane() {
    let s = ColumnsStrategy { max_columns: None };
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let tree2 = s.add_pane(&tree, Rect::new(0, 0, 80, 24), PaneId(2), SplitId(1));
    assert_eq!(collect_pane_ids(&tree2).len(), 2);
}

// ── Monocle ──

#[test]
fn monocle_single() {
    let s = MonocleStrategy;
    let tree = s.rebuild(&[PaneId(1)], Rect::new(0, 0, 80, 24));
    assert!(matches!(tree, LayoutNode::Leaf { .. }));
}

#[test]
fn monocle_three_panes_tabbed() {
    let s = MonocleStrategy;
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    match &tree {
        LayoutNode::Tabbed {
            children, active, ..
        } => {
            assert_eq!(children.len(), 3);
            assert_eq!(*active, 0);
        }
        _ => panic!("expected Tabbed"),
    }
}

#[test]
fn monocle_only_active_visible() {
    let s = MonocleStrategy;
    let tree = s.rebuild(&[PaneId(1), PaneId(2), PaneId(3)], Rect::new(0, 0, 80, 24));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    let visible: Vec<_> = panes.iter().filter(|p| p.visible).collect();
    assert_eq!(visible.len(), 1);
}

#[test]
fn monocle_fullscreen() {
    let s = MonocleStrategy;
    let tree = s.rebuild(&[PaneId(1), PaneId(2)], Rect::new(0, 0, 80, 24));
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    let active = panes.iter().find(|p| p.visible).unwrap();
    assert_eq!(active.width, 80);
    assert_eq!(active.height, 24);
}

// ── Grid ──

#[test]
fn grid_single() {
    let s = GridStrategy;
    let tree = s.rebuild(&[PaneId(1)], Rect::new(0, 0, 80, 24));
    assert!(matches!(tree, LayoutNode::Leaf { .. }));
}

#[test]
fn grid_four_panes_2x2() {
    let s = GridStrategy;
    let tree = s.rebuild(
        &[PaneId(1), PaneId(2), PaneId(3), PaneId(4)],
        Rect::new(0, 0, 80, 24),
    );
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 80, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 4);
    for p in &panes {
        assert!(p.width >= 35 && p.width <= 45, "width={}", p.width);
        assert!(p.height >= 10 && p.height <= 14, "height={}", p.height);
    }
}

#[test]
fn grid_nine_panes_3x3() {
    let s = GridStrategy;
    let tree = s.rebuild(
        &[
            PaneId(1),
            PaneId(2),
            PaneId(3),
            PaneId(4),
            PaneId(5),
            PaneId(6),
            PaneId(7),
            PaneId(8),
            PaneId(9),
        ],
        Rect::new(0, 0, 90, 24),
    );
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 90, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 9);
}

#[test]
fn grid_five_panes_handles_uneven() {
    let s = GridStrategy;
    let tree = s.rebuild(
        &[PaneId(1), PaneId(2), PaneId(3), PaneId(4), PaneId(5)],
        Rect::new(0, 0, 90, 24),
    );
    let panes = compute_resolved_panes(
        &tree,
        Rect::new(0, 0, 90, 24),
        PaneId(1),
        &LayoutConfig::default(),
    );
    assert_eq!(panes.len(), 5);
}

#[test]
fn grid_add_pane() {
    let s = GridStrategy;
    let tree = LayoutNode::Leaf { pane_id: PaneId(1) };
    let tree2 = s.add_pane(&tree, Rect::new(0, 0, 80, 24), PaneId(2), SplitId(1));
    assert_eq!(collect_pane_ids(&tree2).len(), 2);
}
