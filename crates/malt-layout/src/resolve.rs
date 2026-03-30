//! Layout resolution — tree to concrete screen positions.

use crate::{LayoutConfig, Rect};
use malt_protocol::common::{
    Direction, LayoutNode, PaneId, ResolvedPane, SplitSize, TabContext,
};

/// Walk the layout tree and produce a flat list of resolved pane positions.
///
/// Pure function — no mutation of inputs. Call on every render with the current
/// terminal size to get absolute screen positions for each pane.
pub fn compute_resolved_panes(
    tree: &LayoutNode,
    terminal: Rect,
    focused: PaneId,
    config: &LayoutConfig,
) -> Vec<ResolvedPane> {
    let usable = terminal.inset(config.outer_gap);
    let mut panes = Vec::new();
    let mut float_z = 1u32;
    resolve_node(tree, usable, &focused, config, &mut panes, &mut float_z, 0);
    panes
}

fn resolve_node(
    node: &LayoutNode,
    rect: Rect,
    focused: &PaneId,
    config: &LayoutConfig,
    out: &mut Vec<ResolvedPane>,
    float_z: &mut u32,
    base_z: u32,
) {
    match node {
        LayoutNode::Leaf { pane_id } => {
            out.push(ResolvedPane {
                pane_id: pane_id.clone(),
                x: rect.x,
                y: rect.y,
                width: rect.w,
                height: rect.h,
                focused: *pane_id == *focused,
                visible: true,
                z_order: base_z,
                tab_context: None,
                _unknown: Vec::new(),
            });
        }

        LayoutNode::Split {
            direction,
            sizes,
            children,
            ..
        } => {
            resolve_split(direction, sizes, children, rect, focused, config, out, float_z, base_z);
        }

        LayoutNode::Tabbed {
            active, children, ..
        } => {
            let active_idx = *active as usize;
            for (i, child) in children.iter().enumerate() {
                if i == active_idx {
                    resolve_node(child, rect, focused, config, out, float_z, base_z);
                } else {
                    resolve_inactive_tab(child, rect, focused, out, i);
                }
            }
        }

        LayoutNode::Float {
            pane_id,
            x,
            y,
            width,
            height,
        } => {
            out.push(ResolvedPane {
                pane_id: pane_id.clone(),
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                focused: *pane_id == *focused,
                visible: true,
                z_order: *float_z,
                tab_context: None,
                _unknown: Vec::new(),
            });
            *float_z += 1;
        }

        // Forward-compatible: ignore unknown variants
        _ => {}
    }
}

fn resolve_split(
    direction: &Direction,
    sizes: &[SplitSize],
    children: &[LayoutNode],
    rect: Rect,
    focused: &PaneId,
    config: &LayoutConfig,
    out: &mut Vec<ResolvedPane>,
    float_z: &mut u32,
    base_z: u32,
) {
    if children.is_empty() {
        return;
    }

    let n = children.len();
    let total_gaps = if n > 1 {
        (n - 1) as u16 * config.inner_gap
    } else {
        0
    };

    // Determine axis size
    let axis_size = match direction {
        Direction::Vertical => rect.w,
        Direction::Horizontal => rect.h,
    };

    let available = axis_size.saturating_sub(total_gaps);

    // Minimum size along the split axis
    let min_size = match direction {
        Direction::Vertical => config.min_cols,
        Direction::Horizontal => config.min_rows,
    };

    // First pass: sum fixed sizes, count ratio children, sum ratios
    let mut fixed_total: u16 = 0;
    let mut ratio_sum: f32 = 0.0;
    for size in sizes.iter().take(n) {
        match size {
            SplitSize::Fixed { value } => {
                fixed_total += value;
            }
            SplitSize::Ratio { value } => {
                ratio_sum += value;
            }
            _ => {
                // Min/Max/Unknown: treat as Ratio(1.0) for allocation purposes
                ratio_sum += 1.0;
            }
        }
    }

    let remaining = available.saturating_sub(fixed_total);

    // Second pass: compute child sizes
    let mut child_sizes = Vec::with_capacity(n);
    for i in 0..n {
        let size_spec = sizes.get(i);
        let sz = match size_spec {
            Some(SplitSize::Fixed { value }) => *value,
            Some(SplitSize::Ratio { value }) => {
                if ratio_sum > 0.0 {
                    ((value / ratio_sum) * remaining as f32).round() as u16
                } else {
                    remaining / n as u16
                }
            }
            _ => {
                // Default: equal share of remaining
                if ratio_sum > 0.0 {
                    ((1.0 / ratio_sum) * remaining as f32).round() as u16
                } else {
                    remaining / n as u16
                }
            }
        };
        child_sizes.push(sz.max(min_size));
    }

    // Build sub-rects and recurse
    let mut offset = match direction {
        Direction::Vertical => rect.x,
        Direction::Horizontal => rect.y,
    };

    for (i, child) in children.iter().enumerate() {
        let sz = child_sizes[i];
        let child_rect = match direction {
            Direction::Vertical => Rect::new(offset, rect.y, sz, rect.h),
            Direction::Horizontal => Rect::new(rect.x, offset, rect.w, sz),
        };

        resolve_node(child, child_rect, focused, config, out, float_z, base_z);

        offset += sz;
        if i < n - 1 {
            offset += config.inner_gap;
        }
    }
}

/// Resolve an inactive tab child — marks all leaves as invisible with tab context.
fn resolve_inactive_tab(
    node: &LayoutNode,
    rect: Rect,
    focused: &PaneId,
    out: &mut Vec<ResolvedPane>,
    tab_index: usize,
) {
    match node {
        LayoutNode::Leaf { pane_id } => {
            out.push(ResolvedPane {
                pane_id: pane_id.clone(),
                x: rect.x,
                y: rect.y,
                width: rect.w,
                height: rect.h,
                focused: *pane_id == *focused,
                visible: false,
                z_order: 0,
                tab_context: Some(TabContext {
                    label: format!("Pane {}", pane_id.0),
                    is_active: false,
                    tab_index: tab_index as u16,
                    _unknown: Vec::new(),
                }),
                _unknown: Vec::new(),
            });
        }
        LayoutNode::Split { children, .. } | LayoutNode::Tabbed { children, .. } => {
            for child in children {
                resolve_inactive_tab(child, rect, focused, out, tab_index);
            }
        }
        LayoutNode::Float {
            pane_id,
            x,
            y,
            width,
            height,
        } => {
            out.push(ResolvedPane {
                pane_id: pane_id.clone(),
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                focused: *pane_id == *focused,
                visible: false,
                z_order: 0,
                tab_context: Some(TabContext {
                    label: format!("Pane {}", pane_id.0),
                    is_active: false,
                    tab_index: tab_index as u16,
                    _unknown: Vec::new(),
                }),
                _unknown: Vec::new(),
            });
        }
        _ => {}
    }
}
