//! Layout resolution: walk the split tree and compute a `Rect` per leaf.
//!
//! Pure: no mutation, no panics on tiny terminals. A child whose computed
//! size is below the minimum usable size is clamped to 0×0 (and skipped by
//! the render layer), but the stored ratio is never mutated — the next
//! render at a larger terminal size restores the correct layout.

use ratatui::layout::Rect;

use crate::tui::pane::model::{PaneNode, PaneNode::Split, SplitAxis};

/// A leaf pane resolved to a terminal `Rect`. The render layer iterates
/// this and dispatches each pane's module renderer with its rect.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedPane {
    pub pane_id: crate::tui::pane::PaneId,
    pub module_id: crate::tui::pane::ModuleId,
    pub rect: Rect,
}

/// Minimum usable pane size (in terminal cells). A leaf whose computed
/// `Rect` is below this in either dimension is skipped during render
/// (its module is not called) to avoid underflow and confusing half-rendered
/// content. The pane stays in the tree; a larger terminal restores it.
pub const MIN_PANE_WIDTH: u16 = 10;
pub const MIN_PANE_HEIGHT: u16 = 3;

/// Walk the split tree and compute a `Rect` for each leaf. Leaves appear
/// in left-to-right / top-to-bottom order (the order the render layer
/// paints them; also the order `cycle_focus` traverses).
///
/// On a tiny terminal, children whose computed size is below
/// `MIN_PANE_WIDTH`/`MIN_PANE_HEIGHT` are clamped to 0×0 (and skipped by
/// the render layer). The stored ratio is never mutated — terminal
/// resize requires no layout work.
pub fn resolve_rects(root: &PaneNode, area: Rect) -> Vec<ResolvedPane> {
    let mut out = Vec::new();
    resolve_into(root, area, &mut out);
    out
}

fn resolve_into(node: &PaneNode, area: Rect, out: &mut Vec<ResolvedPane>) {
    match node {
        Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_area(area, *axis, *ratio);
            resolve_into(first, a, out);
            resolve_into(second, b, out);
        }
        PaneNode::Leaf { id, module } => {
            out.push(ResolvedPane {
                pane_id: *id,
                module_id: *module,
                rect: area,
            });
        }
    }
}

/// Split `area` along `axis` at `ratio` (the first child's share). Returns
/// the two child rects. Clamps each child to be at least 0 (never
/// negative); the render layer skips children below
/// `MIN_PANE_WIDTH`/`MIN_PANE_HEIGHT`.
///
/// The ratio is clamped to `[0.0, 1.0]` defensively (the model already
/// clamps to `[MIN_RATIO, MAX_RATIO]` on mutation, but a corrupted DTO
/// could have an out-of-range value — never panic).
fn split_area(area: Rect, axis: SplitAxis, ratio: f32) -> (Rect, Rect) {
    let r = ratio.clamp(0.0, 1.0);
    match axis {
        SplitAxis::Vertical => {
            // Left/right children: divide the width.
            let total = area.width as f32;
            let first_w = (total * r).round() as u16;
            let second_w = area.width.saturating_sub(first_w);
            let first = Rect::new(area.x, area.y, first_w, area.height);
            let second = Rect::new(
                area.x.saturating_add(first_w),
                area.y,
                second_w,
                area.height,
            );
            (first, second)
        }
        SplitAxis::Horizontal => {
            // Top/bottom children: divide the height.
            let total = area.height as f32;
            let first_h = (total * r).round() as u16;
            let second_h = area.height.saturating_sub(first_h);
            let first = Rect::new(area.x, area.y, area.width, first_h);
            let second = Rect::new(area.x, area.y.saturating_add(first_h), area.width, second_h);
            (first, second)
        }
    }
}

/// True if `r` is large enough to render a pane into (>= MIN_PANE_WIDTH
/// and >= MIN_PANE_HEIGHT). The render layer skips panes whose rects
/// fail this check.
pub fn is_usable(r: Rect) -> bool {
    r.width >= MIN_PANE_WIDTH && r.height >= MIN_PANE_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::pane::model::{ModuleId, PaneId};

    fn leaf(id: u64, module: ModuleId) -> PaneNode {
        PaneNode::Leaf {
            id: PaneId(id),
            module,
        }
    }

    fn split(axis: SplitAxis, ratio: f32, first: PaneNode, second: PaneNode) -> PaneNode {
        PaneNode::Split {
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// A single leaf resolves to the full area.
    #[test]
    fn single_leaf_resolves_to_full_area() {
        let root = leaf(0, ModuleId::Artists);
        let area = Rect::new(0, 0, 100, 30);
        let panes = resolve_rects(&root, area);
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].rect, area);
    }

    /// A vertical split divides the width by the ratio.
    #[test]
    fn vertical_split_divides_width() {
        let root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let panes = resolve_rects(&root, Rect::new(0, 0, 100, 30));
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].rect, Rect::new(0, 0, 50, 30));
        assert_eq!(panes[1].rect, Rect::new(50, 0, 50, 30));
    }

    /// A horizontal split divides the height by the ratio.
    #[test]
    fn horizontal_split_divides_height() {
        let root = split(
            SplitAxis::Horizontal,
            0.3,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let panes = resolve_rects(&root, Rect::new(0, 0, 100, 30));
        assert_eq!(panes.len(), 2);
        // 30 * 0.3 = 9
        assert_eq!(panes[0].rect, Rect::new(0, 0, 100, 9));
        assert_eq!(panes[1].rect, Rect::new(0, 9, 100, 21));
    }

    /// The same proportional layout resolves correctly at multiple
    /// terminal sizes (resize needs no layout mutation).
    #[test]
    fn same_layout_at_multiple_sizes() {
        let root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(1, ModuleId::Queue),
                leaf(2, ModuleId::Youtube),
            ),
        );
        for (w, h) in [(100, 30), (120, 40), (80, 24), (160, 50)] {
            let panes = resolve_rects(&root, Rect::new(0, 0, w, h));
            assert_eq!(panes.len(), 3, "at {w}x{h}");
            // Pane 0 (left, half width).
            assert_eq!(panes[0].rect.width, w / 2);
            assert_eq!(panes[0].rect.height, h);
            // Pane 1 (top-right, half of right half = quarter width, half height).
            assert_eq!(panes[1].rect.width, w - w / 2);
            assert_eq!(panes[1].rect.height, h / 2);
            // Pane 2 (bottom-right).
            assert_eq!(panes[2].rect.width, w - w / 2);
            assert_eq!(panes[2].rect.height, h - h / 2);
        }
    }

    /// Tiny terminal: no panic, leaves are clamped to 0×0 (skipped by the
    /// render layer via `is_usable`).
    #[test]
    fn tiny_terminal_no_panic() {
        let root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        // 0×0 area.
        let panes = resolve_rects(&root, Rect::new(0, 0, 0, 0));
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].rect, Rect::new(0, 0, 0, 0));
        assert_eq!(panes[1].rect, Rect::new(0, 0, 0, 0));
        // 1×1 area.
        let panes = resolve_rects(&root, Rect::new(0, 0, 1, 1));
        assert_eq!(panes.len(), 2);
        // Neither pane is usable.
        assert!(!is_usable(panes[0].rect));
        assert!(!is_usable(panes[1].rect));
    }

    /// `is_usable` enforces the minimum pane size.
    #[test]
    fn is_usable_thresholds() {
        assert!(is_usable(Rect::new(0, 0, MIN_PANE_WIDTH, MIN_PANE_HEIGHT)));
        assert!(!is_usable(Rect::new(
            0,
            0,
            MIN_PANE_WIDTH - 1,
            MIN_PANE_HEIGHT
        )));
        assert!(!is_usable(Rect::new(
            0,
            0,
            MIN_PANE_WIDTH,
            MIN_PANE_HEIGHT - 1
        )));
        assert!(!is_usable(Rect::new(0, 0, 0, 0)));
    }

    /// A 2×2 grid (nested splits) resolves correctly.
    #[test]
    fn two_by_two_grid() {
        // Split(V, 0.5, Split(H, 0.5, A, B), Split(H, 0.5, C, D))
        let root = split(
            SplitAxis::Vertical,
            0.5,
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(0, ModuleId::Artists),
                leaf(1, ModuleId::Queue),
            ),
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(2, ModuleId::Youtube),
                leaf(3, ModuleId::Playlists),
            ),
        );
        let panes = resolve_rects(&root, Rect::new(0, 0, 100, 40));
        assert_eq!(panes.len(), 4);
        // Top-left.
        assert_eq!(panes[0].rect, Rect::new(0, 0, 50, 20));
        // Bottom-left.
        assert_eq!(panes[1].rect, Rect::new(0, 20, 50, 20));
        // Top-right.
        assert_eq!(panes[2].rect, Rect::new(50, 0, 50, 20));
        // Bottom-right.
        assert_eq!(panes[3].rect, Rect::new(50, 20, 50, 20));
    }

    /// A corrupted DTO with an out-of-range ratio is clamped, not panicked.
    #[test]
    fn out_of_range_ratio_clamped() {
        let root = split(
            SplitAxis::Vertical,
            2.0, // out of range
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let panes = resolve_rects(&root, Rect::new(0, 0, 100, 30));
        // First child gets all the width; second gets 0. No panic.
        assert_eq!(panes[0].rect.width, 100);
        assert_eq!(panes[1].rect.width, 0);
    }
}
