//! Tree mutation operations — split, close, swap, resize.

use malt_protocol::common::{Direction, LayoutNode, PaneId, SplitId, SplitSize};

/// Split a pane, creating a new sibling. Returns a new tree.
///
/// If the target leaf is already inside a Split with the same direction,
/// the new pane is appended as an additional child (N-ary extension)
/// instead of creating a nested binary split.
pub fn split_pane(
    tree: &LayoutNode,
    target: PaneId,
    dir: Direction,
    size: SplitSize,
    new_pane: PaneId,
    new_split: SplitId,
) -> Option<LayoutNode> {
    if !contains_pane(tree, &target) {
        return None;
    }
    Some(split_node(
        tree, &target, &dir, &size, &new_pane, &new_split,
    ))
}

fn split_node(
    node: &LayoutNode,
    target: &PaneId,
    dir: &Direction,
    size: &SplitSize,
    new_pane: &PaneId,
    new_split: &SplitId,
) -> LayoutNode {
    match node {
        LayoutNode::Leaf { pane_id } if pane_id == target => LayoutNode::Split {
            split_id: new_split.clone(),
            direction: dir.clone(),
            sizes: vec![SplitSize::Ratio { value: 0.5 }, size.clone()],
            children: vec![
                LayoutNode::Leaf {
                    pane_id: target.clone(),
                },
                LayoutNode::Leaf {
                    pane_id: new_pane.clone(),
                },
            ],
        },

        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } if direction == dir && children.iter().any(|c| is_direct_leaf(c, target)) => {
            // N-ary extension: insert new pane next to the target in this split
            let mut new_sizes = sizes.clone();
            let mut new_children = children.clone();
            let idx = children
                .iter()
                .position(|c| is_direct_leaf(c, target))
                .unwrap_or(children.len());
            new_sizes.insert(idx + 1, size.clone());
            new_children.insert(
                idx + 1,
                LayoutNode::Leaf {
                    pane_id: new_pane.clone(),
                },
            );
            LayoutNode::Split {
                split_id: split_id.clone(),
                direction: direction.clone(),
                sizes: new_sizes,
                children: new_children,
            }
        }

        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } => {
            let new_children = children
                .iter()
                .map(|c| split_node(c, target, dir, size, new_pane, new_split))
                .collect();
            LayoutNode::Split {
                split_id: split_id.clone(),
                direction: direction.clone(),
                sizes: sizes.clone(),
                children: new_children,
            }
        }

        LayoutNode::Tabbed {
            split_id,
            active,
            children,
        } => {
            let new_children = children
                .iter()
                .map(|c| split_node(c, target, dir, size, new_pane, new_split))
                .collect();
            LayoutNode::Tabbed {
                split_id: split_id.clone(),
                active: *active,
                children: new_children,
            }
        }

        other => other.clone(),
    }
}

fn is_direct_leaf(node: &LayoutNode, target: &PaneId) -> bool {
    matches!(node, LayoutNode::Leaf { pane_id } if pane_id == target)
}

/// Close a pane, removing it from the tree. Returns None if closing the root leaf.
///
/// If a parent Split goes from 2 to 1 child, it collapses to the remaining child.
pub fn close_pane(tree: &LayoutNode, target: PaneId) -> Option<LayoutNode> {
    if !contains_pane(tree, &target) {
        return None;
    }
    // Root leaf case
    if is_direct_leaf(tree, &target) {
        return None;
    }
    Some(close_in_node(tree, &target))
}

fn close_in_node(node: &LayoutNode, target: &PaneId) -> LayoutNode {
    match node {
        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } => {
            // Check if target is a direct child
            if children.iter().any(|c| is_direct_leaf(c, target)) {
                let mut new_children: Vec<LayoutNode> = Vec::new();
                let mut new_sizes: Vec<SplitSize> = Vec::new();
                for (i, child) in children.iter().enumerate() {
                    if !is_direct_leaf(child, target) {
                        new_children.push(child.clone());
                        if let Some(s) = sizes.get(i) {
                            new_sizes.push(s.clone());
                        }
                    }
                }
                if new_children.len() == 1 {
                    // Collapse
                    new_children.remove(0)
                } else {
                    LayoutNode::Split {
                        split_id: split_id.clone(),
                        direction: direction.clone(),
                        sizes: new_sizes,
                        children: new_children,
                    }
                }
            } else {
                // Recurse into children
                let new_children: Vec<LayoutNode> = children
                    .iter()
                    .map(|c| {
                        if contains_pane(c, target) {
                            close_in_node(c, target)
                        } else {
                            c.clone()
                        }
                    })
                    .collect();
                LayoutNode::Split {
                    split_id: split_id.clone(),
                    direction: direction.clone(),
                    sizes: sizes.clone(),
                    children: new_children,
                }
            }
        }

        LayoutNode::Tabbed {
            split_id,
            active,
            children,
        } => {
            if children.iter().any(|c| is_direct_leaf(c, target)) {
                let new_children: Vec<LayoutNode> = children
                    .iter()
                    .filter(|c| !is_direct_leaf(c, target))
                    .cloned()
                    .collect();
                if new_children.len() == 1 {
                    new_children
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| node.clone())
                } else {
                    let new_active = if (*active as usize) >= new_children.len() {
                        (new_children.len().saturating_sub(1)) as u16
                    } else {
                        *active
                    };
                    LayoutNode::Tabbed {
                        split_id: split_id.clone(),
                        active: new_active,
                        children: new_children,
                    }
                }
            } else {
                let new_children: Vec<LayoutNode> = children
                    .iter()
                    .map(|c| {
                        if contains_pane(c, target) {
                            close_in_node(c, target)
                        } else {
                            c.clone()
                        }
                    })
                    .collect();
                LayoutNode::Tabbed {
                    split_id: split_id.clone(),
                    active: *active,
                    children: new_children,
                }
            }
        }

        other => other.clone(),
    }
}

/// Swap two pane IDs in the tree. Returns None if either is not found.
pub fn swap_panes(tree: &LayoutNode, a: PaneId, b: PaneId) -> Option<LayoutNode> {
    if !contains_pane(tree, &a) || !contains_pane(tree, &b) {
        return None;
    }
    Some(swap_in_node(tree, &a, &b))
}

fn swap_in_node(node: &LayoutNode, a: &PaneId, b: &PaneId) -> LayoutNode {
    match node {
        LayoutNode::Leaf { pane_id } => {
            if pane_id == a {
                LayoutNode::Leaf { pane_id: b.clone() }
            } else if pane_id == b {
                LayoutNode::Leaf { pane_id: a.clone() }
            } else {
                node.clone()
            }
        }

        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } => LayoutNode::Split {
            split_id: split_id.clone(),
            direction: direction.clone(),
            sizes: sizes.clone(),
            children: children.iter().map(|c| swap_in_node(c, a, b)).collect(),
        },

        LayoutNode::Tabbed {
            split_id,
            active,
            children,
        } => LayoutNode::Tabbed {
            split_id: split_id.clone(),
            active: *active,
            children: children.iter().map(|c| swap_in_node(c, a, b)).collect(),
        },

        LayoutNode::Float {
            pane_id,
            x,
            y,
            width,
            height,
        } => {
            let new_id = if pane_id == a {
                b.clone()
            } else if pane_id == b {
                a.clone()
            } else {
                pane_id.clone()
            };
            LayoutNode::Float {
                pane_id: new_id,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            }
        }

        other => other.clone(),
    }
}

/// Resize a split by replacing a child's size entry.
pub fn resize_split(
    tree: &LayoutNode,
    split_id: SplitId,
    child_index: u16,
    new_size: SplitSize,
) -> Option<LayoutNode> {
    resize_in_node(tree, &split_id, child_index, &new_size)
}

fn resize_in_node(
    node: &LayoutNode,
    target_split: &SplitId,
    child_index: u16,
    new_size: &SplitSize,
) -> Option<LayoutNode> {
    match node {
        LayoutNode::Split {
            split_id,
            direction,
            sizes,
            children,
        } => {
            if split_id == target_split {
                let idx = child_index as usize;
                if idx >= sizes.len() {
                    return None;
                }
                let mut new_sizes = sizes.clone();
                new_sizes[idx] = new_size.clone();
                Some(LayoutNode::Split {
                    split_id: split_id.clone(),
                    direction: direction.clone(),
                    sizes: new_sizes,
                    children: children.clone(),
                })
            } else {
                // Recurse
                let mut new_children = children.clone();
                let mut found = false;
                for child in &mut new_children {
                    if let Some(updated) =
                        resize_in_node(child, target_split, child_index, new_size)
                    {
                        *child = updated;
                        found = true;
                        break;
                    }
                }
                if found {
                    Some(LayoutNode::Split {
                        split_id: split_id.clone(),
                        direction: direction.clone(),
                        sizes: sizes.clone(),
                        children: new_children,
                    })
                } else {
                    None
                }
            }
        }

        LayoutNode::Tabbed {
            split_id,
            active,
            children,
        } => {
            let mut new_children = children.clone();
            let mut found = false;
            for child in &mut new_children {
                if let Some(updated) = resize_in_node(child, target_split, child_index, new_size) {
                    *child = updated;
                    found = true;
                    break;
                }
            }
            if found {
                Some(LayoutNode::Tabbed {
                    split_id: split_id.clone(),
                    active: *active,
                    children: new_children,
                })
            } else {
                None
            }
        }

        _ => None,
    }
}

/// Collect all pane IDs via in-order traversal.
pub fn collect_pane_ids(tree: &LayoutNode) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect_ids_inner(tree, &mut out);
    out
}

fn collect_ids_inner(node: &LayoutNode, out: &mut Vec<PaneId>) {
    match node {
        LayoutNode::Leaf { pane_id } => out.push(pane_id.clone()),
        LayoutNode::Split { children, .. } | LayoutNode::Tabbed { children, .. } => {
            for child in children {
                collect_ids_inner(child, out);
            }
        }
        LayoutNode::Float { pane_id, .. } => out.push(pane_id.clone()),
        _ => {}
    }
}

fn contains_pane(node: &LayoutNode, target: &PaneId) -> bool {
    match node {
        LayoutNode::Leaf { pane_id } => pane_id == target,
        LayoutNode::Split { children, .. } | LayoutNode::Tabbed { children, .. } => {
            children.iter().any(|c| contains_pane(c, target))
        }
        LayoutNode::Float { pane_id, .. } => pane_id == target,
        _ => false,
    }
}
