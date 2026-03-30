//! Directional focus navigation.

use malt_protocol::common::{FocusDir, PaneId, ResolvedPane};

/// Find the next pane to focus given a direction.
///
/// For Up/Down/Left/Right: filters to visible candidates in the correct
/// direction, then picks the nearest by Euclidean distance from center to center.
///
/// For Next/Prev: cycles through visible pane IDs in order, wrapping around.
pub fn focus_direction(
    panes: &[ResolvedPane],
    focused: PaneId,
    dir: FocusDir,
) -> Option<PaneId> {
    let visible: Vec<&ResolvedPane> = panes.iter().filter(|p| p.visible).collect();

    let current = visible.iter().find(|p| p.pane_id == focused)?;

    match dir {
        FocusDir::Next | FocusDir::Prev => {
            let ids: Vec<PaneId> = visible.iter().map(|p| p.pane_id.clone()).collect();
            if ids.len() <= 1 {
                return None;
            }
            let idx = ids.iter().position(|id| *id == focused)?;
            let next_idx = match dir {
                FocusDir::Next => (idx + 1) % ids.len(),
                FocusDir::Prev => (idx + ids.len() - 1) % ids.len(),
                _ => unreachable!(),
            };
            Some(ids[next_idx].clone())
        }

        _ => {
            let cx = current.x as f64 + current.width as f64 / 2.0;
            let cy = current.y as f64 + current.height as f64 / 2.0;

            let candidates: Vec<&ResolvedPane> = visible
                .iter()
                .filter(|p| p.pane_id != focused)
                .filter(|p| {
                    let pcx = p.x as f64 + p.width as f64 / 2.0;
                    let pcy = p.y as f64 + p.height as f64 / 2.0;
                    match dir {
                        FocusDir::Up => pcy < cy,
                        FocusDir::Down => pcy > cy,
                        FocusDir::Left => pcx < cx,
                        FocusDir::Right => pcx > cx,
                        _ => false,
                    }
                })
                .copied()
                .collect();

            candidates
                .iter()
                .min_by(|a, b| {
                    let da = dist(cx, cy, a);
                    let db = dist(cx, cy, b);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|p| p.pane_id.clone())
        }
    }
}

fn dist(cx: f64, cy: f64, pane: &ResolvedPane) -> f64 {
    let px = pane.x as f64 + pane.width as f64 / 2.0;
    let py = pane.y as f64 + pane.height as f64 / 2.0;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}
