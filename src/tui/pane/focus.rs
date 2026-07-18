//! Spatial (directional) focus movement + cycling.
//!
//! `move_focus_directional` picks the next pane in a given direction
//! using the algorithm from the spec:
//!
//! 1. Consider panes located in the requested direction from the focused pane.
//! 2. Prefer panes whose perpendicular spans overlap.
//! 3. Choose the nearest candidate by edge distance.
//! 4. Use center distance as a deterministic tie-breaker.
//!
//! `cycle_focus` is a fallback that walks leaves in tree order.

use crate::tui::pane::layout::ResolvedPane;
use crate::tui::pane::model::{Direction, PaneId};

/// Move focus in a direction. Returns the new focused pane id, or `None`
/// if no candidate exists (the focused pane already spans the workspace
/// in that direction).
pub fn move_focus_directional(
    panes: &[ResolvedPane],
    focused: PaneId,
    dir: Direction,
) -> Option<PaneId> {
    let cur = panes.iter().find(|p| p.pane_id == focused)?;
    let cur_rect = cur.rect;

    // Step 1: filter to panes in the requested direction.
    let candidates: Vec<&ResolvedPane> = panes
        .iter()
        .filter(|p| p.pane_id != focused && is_in_direction(cur_rect, p.rect, dir))
        .collect();

    // Step 2: prefer panes whose perpendicular spans overlap.
    let perpendicular_overlap: Vec<&ResolvedPane> = candidates
        .iter()
        .copied()
        .filter(|p| perpendicular_overlap(cur_rect, p.rect, dir))
        .collect();

    let pool = if !perpendicular_overlap.is_empty() {
        perpendicular_overlap
    } else {
        candidates
    };

    if pool.is_empty() {
        return None;
    }

    // Step 3 + 4: pick the nearest by edge distance, tie-break by center
    // distance. Sort stably so the deterministic order is preserved.
    let mut scored: Vec<(f32, f32, &ResolvedPane)> = pool
        .iter()
        .map(|p| {
            let edge = edge_distance(cur_rect, p.rect, dir);
            let center = center_distance(cur_rect, p.rect);
            (edge, center, *p)
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    scored.first().map(|(_, _, p)| p.pane_id)
}

/// Cycle focus to the next/previous leaf in tree order (left-to-right,
/// top-to-bottom). Returns `None` if there's only one pane.
pub fn cycle_focus(panes: &[ResolvedPane], focused: PaneId, forward: bool) -> Option<PaneId> {
    if panes.len() <= 1 {
        return None;
    }
    let idx = panes.iter().position(|p| p.pane_id == focused)?;
    let next = if forward {
        (idx + 1) % panes.len()
    } else {
        idx.checked_sub(1).unwrap_or(panes.len() - 1)
    };
    Some(panes[next].pane_id)
}

/// True if `other` is in the `dir` direction from `cur`. "In the direction"
/// means the other pane's extent in the parallel axis is strictly beyond
/// the current pane's extent (no overlap on the parallel axis counted as
/// a directional relationship — we just need the other pane to be on the
/// correct side).
fn is_in_direction(
    cur: ratatui::layout::Rect,
    other: ratatui::layout::Rect,
    dir: Direction,
) -> bool {
    match dir {
        Direction::Left => other.right() <= cur.x,
        Direction::Right => other.x >= cur.right(),
        Direction::Up => other.bottom() <= cur.y,
        Direction::Down => other.y >= cur.bottom(),
    }
}

/// True if the perpendicular spans of `cur` and `other` overlap. "Perpendicular"
/// is the axis perpendicular to `dir` (vertical span for left/right, horizontal
/// span for up/down).
fn perpendicular_overlap(
    cur: ratatui::layout::Rect,
    other: ratatui::layout::Rect,
    dir: Direction,
) -> bool {
    match dir {
        Direction::Left | Direction::Right => {
            // Perpendicular = vertical. Overlap in y.
            spans_overlap(cur.y, cur.height, other.y, other.height)
        }
        Direction::Up | Direction::Down => {
            // Perpendicular = horizontal. Overlap in x.
            spans_overlap(cur.x, cur.width, other.x, other.width)
        }
    }
}

/// True if two 1D spans [a_start, a_start+a_len) and [b_start, b_start+b_len)
/// overlap (interior or boundary). Zero-length spans never overlap.
fn spans_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    if a_len == 0 || b_len == 0 {
        return false;
    }
    let a_end = a_start.saturating_add(a_len);
    let b_end = b_start.saturating_add(b_len);
    a_start < b_end && b_start < a_end
}

/// Distance from `cur`'s edge (in the `dir` direction) to `other`'s nearest
/// edge. Always non-negative. Zero when the panes are adjacent.
fn edge_distance(cur: ratatui::layout::Rect, other: ratatui::layout::Rect, dir: Direction) -> f32 {
    match dir {
        Direction::Left => (cur.x.saturating_sub(other.right())) as f32,
        Direction::Right => (other.x.saturating_sub(cur.right())) as f32,
        Direction::Up => (cur.y.saturating_sub(other.bottom())) as f32,
        Direction::Down => (other.y.saturating_sub(cur.bottom())) as f32,
    }
}

/// Euclidean distance between the centers of the two rects. Used as a
/// deterministic tie-breaker when edge distances are equal.
fn center_distance(a: ratatui::layout::Rect, b: ratatui::layout::Rect) -> f32 {
    let acx = a.x as f32 + a.width as f32 / 2.0;
    let acy = a.y as f32 + a.height as f32 / 2.0;
    let bcx = b.x as f32 + b.width as f32 / 2.0;
    let bcy = b.y as f32 + b.height as f32 / 2.0;
    ((acx - bcx).powi(2) + (acy - bcy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::pane::model::ModuleId;
    use ratatui::layout::Rect;

    fn pane(id: u64, x: u16, y: u16, w: u16, h: u16) -> ResolvedPane {
        ResolvedPane {
            pane_id: PaneId(id),
            module_id: ModuleId::Artists,
            rect: Rect::new(x, y, w, h),
        }
    }

    /// Two panes side by side: moving right from left pane picks the right.
    #[test]
    fn move_right_to_adjacent() {
        let panes = [pane(0, 0, 0, 50, 30), pane(1, 50, 0, 50, 30)];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Right),
            Some(PaneId(1))
        );
        assert_eq!(
            move_focus_directional(&panes, PaneId(1), Direction::Left),
            Some(PaneId(0))
        );
    }

    /// Two panes stacked: moving down from top picks the bottom.
    #[test]
    fn move_down_to_adjacent() {
        let panes = [pane(0, 0, 0, 100, 15), pane(1, 0, 15, 100, 15)];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Down),
            Some(PaneId(1))
        );
        assert_eq!(
            move_focus_directional(&panes, PaneId(1), Direction::Up),
            Some(PaneId(0))
        );
    }

    /// Moving toward a direction with no pane returns None.
    #[test]
    fn move_no_candidate_returns_none() {
        let panes = [pane(0, 0, 0, 50, 30), pane(1, 50, 0, 50, 30)];
        // Left pane has nothing to its left.
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Left),
            None
        );
        // Right pane has nothing to its right.
        assert_eq!(
            move_focus_directional(&panes, PaneId(1), Direction::Right),
            None
        );
    }

    /// Perpendicular overlap preferred: in a 2x2 grid, moving right from
    /// the top-left picks the top-right (perpendicular overlap), not the
    /// bottom-right (which is also to the right but has no y overlap).
    #[test]
    fn prefers_perpendicular_overlap() {
        // 2x2 grid:
        //   0 | 1
        //   --+--
        //   2 | 3
        let panes = [
            pane(0, 0, 0, 50, 15),
            pane(1, 50, 0, 50, 15),
            pane(2, 0, 15, 50, 15),
            pane(3, 50, 15, 50, 15),
        ];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Right),
            Some(PaneId(1))
        );
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Down),
            Some(PaneId(2))
        );
    }

    /// Tie-break by center distance: when two candidates have the same
    /// edge distance, the one whose center is closer wins.
    #[test]
    fn tie_break_by_center_distance() {
        // Focused pane at (50,0) 50x50. Two right candidates with the same
        // edge distance (both touch the focused pane's right edge): one
        // centered near the top, one near the bottom.
        // cur center y = 25.
        // candidate A: y=0..10 (center 5)  → center distance ~20
        // candidate B: y=20..30 (center 25) → center distance ~0
        let panes = [
            pane(0, 50, 0, 50, 50),
            pane(1, 100, 0, 50, 10),  // far center
            pane(2, 100, 20, 50, 10), // near center
        ];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Right),
            Some(PaneId(2))
        );
    }

    /// Cycle forward wraps around.
    #[test]
    fn cycle_forward_wraps() {
        let panes = [
            pane(0, 0, 0, 10, 10),
            pane(1, 10, 0, 10, 10),
            pane(2, 20, 0, 10, 10),
        ];
        assert_eq!(cycle_focus(&panes, PaneId(0), true), Some(PaneId(1)));
        assert_eq!(cycle_focus(&panes, PaneId(1), true), Some(PaneId(2)));
        assert_eq!(cycle_focus(&panes, PaneId(2), true), Some(PaneId(0)));
    }

    /// Cycle backward wraps around.
    #[test]
    fn cycle_backward_wraps() {
        let panes = [
            pane(0, 0, 0, 10, 10),
            pane(1, 10, 0, 10, 10),
            pane(2, 20, 0, 10, 10),
        ];
        assert_eq!(cycle_focus(&panes, PaneId(0), false), Some(PaneId(2)));
        assert_eq!(cycle_focus(&panes, PaneId(2), false), Some(PaneId(1)));
    }

    /// Cycle with one pane returns None.
    #[test]
    fn cycle_single_returns_none() {
        let panes = [pane(0, 0, 0, 10, 10)];
        assert_eq!(cycle_focus(&panes, PaneId(0), true), None);
    }

    /// Diagonal arrangement: moving right picks the closest rightward pane
    /// even when there's no perpendicular overlap (fallback to all
    /// candidates in that direction).
    #[test]
    fn fallback_when_no_perpendicular_overlap() {
        // Pane 0 at top-left. Pane 1 at bottom-right (no y overlap with 0).
        // Moving right from 0 should still pick 1 (it's the only rightward pane).
        let panes = [pane(0, 0, 0, 50, 15), pane(1, 50, 20, 50, 15)];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Right),
            Some(PaneId(1))
        );
    }

    /// Unknown focused pane returns None.
    #[test]
    fn unknown_focused_returns_none() {
        let panes = [pane(0, 0, 0, 10, 10), pane(1, 10, 0, 10, 10)];
        assert_eq!(
            move_focus_directional(&panes, PaneId(99), Direction::Right),
            None
        );
    }

    /// Three panes side by side: moving right from the leftmost picks the
    /// middle, not the rightmost (edge distance wins).
    #[test]
    fn three_side_by_side_picks_nearest() {
        let panes = [
            pane(0, 0, 0, 30, 30),
            pane(1, 30, 0, 30, 30),
            pane(2, 60, 0, 30, 30),
        ];
        assert_eq!(
            move_focus_directional(&panes, PaneId(0), Direction::Right),
            Some(PaneId(1))
        );
        assert_eq!(
            move_focus_directional(&panes, PaneId(2), Direction::Left),
            Some(PaneId(1))
        );
    }
}
