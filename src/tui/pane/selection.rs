//! Rectangle selection model for pane-editing (Phase 2).
//!
//! A rectangle selection is a user-defined sub-region of the focused
//! pane, stored in normalized coordinates (0.0–1.0) so it survives
//! terminal resize. The selection is converted into recursive split-tree
//! operations (no floating rectangles) — see
//! [`RectangleSelection::convert_to_splits`].
//!
//! ## Workflow
//!
//! 1. `r` in PaneEdit mode enters selection mode (phase = ChoosingAnchor).
//!    The anchor starts at the center of the focused pane.
//! 2. The user moves the anchor (first corner) with arrow keys / hjkl.
//!    Shift+arrows or HJKL move 4x faster.
//! 3. `Enter` confirms the anchor (phase = ChoosingExtent). The opposite
//!    corner (cursor) is now active.
//! 4. The user moves the opposite corner. `Tab` switches the active
//!    corner (anchor ↔ extent).
//! 5. `Enter` confirms the rectangle (phase = Confirming). The module
//!    picker overlay opens to choose the module for the selected region.
//! 6. `Enter` in the module picker converts the rectangle to split-tree
//!    ops and installs the chosen module.
//! 7. `Esc` at any phase cancels (no layout mutation).
//!
//! ## Mouse workflow
//!
//! 1. `r` in PaneEdit mode arms selection mode.
//! 2. Left-click-drag inside the focused pane starts a drag: the click
//!    position becomes the anchor, the drag position becomes the cursor.
//! 3. Mouse-up confirms the rectangle → opens the module picker.
//! 4. Right-click or Escape cancels.
//!
//! ## Layout conversion
//!
//! The selected rectangle (in normalized coords) is converted to nested
//! split-tree operations on the target pane. With all four margins > 0
//! the structure is:
//!
//! ```text
//! Split(H, top)               // top split
//! ├── Leaf(top, Placeholder)
//! └── Split(H, bottom')       // bottom split (on the remaining pane)
//!     ├── Split(V, left)       // left split
//!     │   ├── Leaf(left, Placeholder)
//!     │   └── Split(V, right') // right split (on the remaining pane)
//!     │       ├── Leaf(center, target_module)
//!     │       └── Leaf(right, Placeholder)
//!     └── Leaf(bottom, Placeholder)
//! ```
//!
//! Zero-margin splits are skipped: if `top == 0`, no top split is created;
//! if `left == 0`, no left split; etc. The center pane always gets the
//! chosen module; surrounding panes get `ModuleId::Placeholder` so the
//! user sees they need to choose modules for the surrounding areas.
//!
//! Ratios are computed from the normalized margins and clamped to
//! `[MIN_RATIO, MAX_RATIO]` so the resulting tree is a valid proportional
//! split tree that scales correctly on terminal resize.

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::tui::pane::model::{
    clamp_ratio, Direction, ModuleId, PaneId, PaneNode, SplitAxis, MAX_RATIO, MIN_RATIO,
};

/// Minimum selection size (in terminal cells). A selection below this in
/// either dimension is "too small" and `Enter` refuses to confirm. Matches
/// the pane minimums in [`crate::tui::pane::layout`] so a confirmed
/// selection always yields a usable pane.
pub const MIN_SELECTION_WIDTH: u16 = 10;
pub const MIN_SELECTION_HEIGHT: u16 = 3;

/// Normalized coordinates (0.0 to 1.0) relative to the focused pane's
/// inner content area. Stored normalized so terminal resize requires no
/// mutation — the same coords map to the new pane size on the next
/// render. Converted to terminal cells only for rendering and
/// minimum-size validation (see [`RectangleSelection::to_cell_rect`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub x: f32,
    pub y: f32,
}

impl NormalizedPoint {
    /// Create a new point, clamping both coords to `[0.0, 1.0]`. Never
    /// panics; out-of-range values are silently clamped (defensive —
    /// mouse events can land just outside the focused pane's inner
    /// rect on a resize race).
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}

/// The three phases of a rectangle selection. Stored on
/// [`RectangleSelection::phase`] so the render + input layers can branch
/// on the current state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionPhase {
    /// Moving the selection origin (first corner). The anchor is the
    /// active corner. `Enter` confirms the anchor and transitions to
    /// [`ChoosingExtent`](SelectionPhase::ChoosingExtent).
    ChoosingAnchor,
    /// Expanding/shrinking the opposite corner. The cursor (extent) is
    /// active by default; `Tab` toggles which corner is active. `Enter`
    /// confirms the rectangle and transitions to
    /// [`Confirming`](SelectionPhase::Confirming).
    ChoosingExtent,
    /// Ready to confirm — the module picker overlay is open. The
    /// selection is frozen; the user picks a module or cancels.
    Confirming,
}

/// How the current selection is being driven. `Keyboard` arrows / hjkl
/// move the active corner by a fixed step; `Mouse` drag sets the cursor
/// directly. The input layer uses this to route events (mouse events are
/// only routed to the selection when `input_source == Mouse` OR the
/// selection is armed for mouse via `r`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionInput {
    Keyboard,
    Mouse,
}

/// A user-defined rectangle selection over the focused pane. Stored in
/// normalized coords so it survives terminal resize.
///
/// `anchor` is the first corner (set in [`ChoosingAnchor`] phase).
/// `cursor` is the opposite corner (set in [`ChoosingExtent`] phase).
/// Reversed selections (dragging up-left or down-right) are normalized
/// by [`normalized_rect`](RectangleSelection::normalized_rect) so the
/// split conversion is deterministic regardless of drag direction.
#[derive(Clone, Debug, PartialEq)]
pub struct RectangleSelection {
    pub target_pane: PaneId,
    pub anchor: NormalizedPoint,
    pub cursor: NormalizedPoint,
    pub phase: SelectionPhase,
    pub input_source: SelectionInput,
    /// Which corner is currently active (being moved by arrow keys).
    /// `true` = anchor; `false` = cursor (extent). In
    /// [`ChoosingAnchor`] the anchor is always active. In
    /// [`ChoosingExtent`] `Tab` toggles between the two.
    pub active_is_anchor: bool,
}

impl RectangleSelection {
    /// Create a new selection with a small default rect centered on the
    /// focused pane. Phase = [`ChoosingAnchor`], input_source =
    /// [`Keyboard`]. The anchor is at 40%/40% and the cursor at 60%/60%,
    /// giving a 20%×20% selection that's immediately visible (a 0×0
    /// selection at the center would be invisible and the user would
    /// think nothing happened). The anchor is the active corner.
    ///
    /// [`ChoosingAnchor`]: SelectionPhase::ChoosingAnchor
    /// [`Keyboard`]: SelectionInput::Keyboard
    pub fn new(target_pane: PaneId) -> Self {
        Self {
            target_pane,
            anchor: NormalizedPoint::new(0.4, 0.4),
            cursor: NormalizedPoint::new(0.6, 0.6),
            phase: SelectionPhase::ChoosingAnchor,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: true,
        }
    }

    /// Move the active corner by `step` in `dir`. `fast` (Shift) moves
    /// 4x the step. Clamped to `[0, 1]` so the corner can't leave the
    /// pane. No-op if no corner is active (defensive).
    pub fn move_cursor(&mut self, dir: Direction, step: f32, fast: bool) {
        let s = if fast { 4.0 * step } else { step };
        let target = if self.active_is_anchor {
            &mut self.anchor
        } else {
            &mut self.cursor
        };
        match dir {
            Direction::Left => target.x = (target.x - s).clamp(0.0, 1.0),
            Direction::Right => target.x = (target.x + s).clamp(0.0, 1.0),
            Direction::Up => target.y = (target.y - s).clamp(0.0, 1.0),
            Direction::Down => target.y = (target.y + s).clamp(0.0, 1.0),
        }
    }

    /// Confirm the anchor. Phase becomes [`ChoosingExtent`]; the cursor
    /// (extent) is now the active corner.
    ///
    /// [`ChoosingExtent`]: SelectionPhase::ChoosingExtent
    pub fn confirm_anchor(&mut self) {
        self.phase = SelectionPhase::ChoosingExtent;
        self.active_is_anchor = false;
    }

    /// Toggle the active corner between anchor and cursor (Tab). Only
    /// meaningful in [`ChoosingExtent`] — in [`ChoosingAnchor`] the
    /// anchor is always active.
    ///
    /// [`ChoosingExtent`]: SelectionPhase::ChoosingExtent
    /// [`ChoosingAnchor`]: SelectionPhase::ChoosingAnchor
    pub fn switch_corner(&mut self) {
        self.active_is_anchor = !self.active_is_anchor;
    }

    /// Confirm the rectangle. Phase becomes [`Confirming`] (the module
    /// picker is opened by the caller).
    ///
    /// [`Confirming`]: SelectionPhase::Confirming
    pub fn confirm(&mut self) {
        self.phase = SelectionPhase::Confirming;
    }

    /// Cancel: return to [`ChoosingAnchor`] with the anchor active. The
    /// caller may also drop the selection entirely (Esc) — this method
    /// is for the "cancel but stay in selection mode" case (used by the
    /// mouse right-click handler when we want to keep the selection armed
    /// for another drag).
    ///
    /// [`ChoosingAnchor`]: SelectionPhase::ChoosingAnchor
    pub fn cancel(&mut self) {
        self.phase = SelectionPhase::ChoosingAnchor;
        self.active_is_anchor = true;
    }

    /// The normalized margins of the selected rectangle relative to the
    /// pane. Returns `(top, bottom, left, right)`, each in `[0.0, 1.0]`,
    /// sorted so `top + bottom + selected_height == 1` and
    /// `left + right + selected_width == 1`. Reversed selections
    /// (dragging up-left) are normalized so the anchor is effectively
    /// the top-left corner and the cursor the bottom-right.
    pub fn normalized_rect(&self) -> (f32, f32, f32, f32) {
        let top = self.anchor.y.min(self.cursor.y);
        let bottom = 1.0 - self.anchor.y.max(self.cursor.y);
        let left = self.anchor.x.min(self.cursor.x);
        let right = 1.0 - self.anchor.x.max(self.cursor.x);
        (top, bottom, left, right)
    }

    /// Convert the normalized rect to terminal cells within
    /// `pane_area` (the focused pane's INNER content area — i.e. after
    /// the border has been subtracted). Returns the cell rect, clamped
    /// to non-negative width/height. The returned rect's origin is in
    /// terminal coordinates (i.e. offset by `pane_area.x/y`).
    pub fn to_cell_rect(&self, pane_area: Rect) -> Rect {
        let (top, bottom, left, right) = self.normalized_rect();
        let x = pane_area.x as f32 + left * pane_area.width as f32;
        let y = pane_area.y as f32 + top * pane_area.height as f32;
        let w = (1.0 - left - right) * pane_area.width as f32;
        let h = (1.0 - top - bottom) * pane_area.height as f32;
        Rect::new(
            x.round().max(0.0) as u16,
            y.round().max(0.0) as u16,
            w.round().max(0.0) as u16,
            h.round().max(0.0) as u16,
        )
    }

    /// True if the selected region is large enough to be a pane
    /// (`>= MIN_SELECTION_WIDTH x MIN_SELECTION_HEIGHT` cells). `Enter`
    /// refuses to confirm a too-small selection (the user sees a
    /// "too small" toast). `pane_area` is the focused pane's INNER
    /// content area (after border).
    pub fn is_valid(&self, pane_area: Rect) -> bool {
        let r = self.to_cell_rect(pane_area);
        r.width >= MIN_SELECTION_WIDTH && r.height >= MIN_SELECTION_HEIGHT
    }

    /// A dimension label like `"42x18 cells · 55% × 40%"`. The first
    /// pair is the selected region's size in terminal cells (width x
    /// height); the second pair is the selected region's size as a
    /// percentage of the pane's inner area. Rounded to the nearest
    /// integer so the label stays short.
    pub fn dimensions_label(&self, pane_area: Rect) -> String {
        let r = self.to_cell_rect(pane_area);
        let (top, bottom, left, right) = self.normalized_rect();
        let w_pct = (1.0 - left - right) * 100.0;
        let h_pct = (1.0 - top - bottom) * 100.0;
        format!(
            "{}x{} cells · {:.0}% × {:.0}%",
            r.width, r.height, w_pct, h_pct
        )
    }

    /// Convert the selection to a split subtree that replaces the
    /// target pane. The center pane gets `target_module`; surrounding
    /// panes get [`ModuleId::Placeholder`]. Returns the subtree + the
    /// list of new pane ids (all unique, all from `PaneId(0)` upward).
    ///
    /// The subtree uses `PaneId(0), PaneId(1), ...` for new panes (the
    /// caller is responsible for reassigning fresh ids before
    /// inserting into the workspace — see
    /// [`PaneWorkspace::apply_rectangle_selection`](crate::tui::pane::PaneWorkspace::apply_rectangle_selection)).
    /// The center pane is always `PaneId(0)` (the first leaf created),
    /// so the caller can find it after reassignment by tracking that
    /// placeholder id.
    ///
    /// Ratios are computed from the normalized margins and clamped to
    /// `[MIN_RATIO, MAX_RATIO]`. Zero-margin splits are skipped (if
    /// `top == 0`, no top split is created; etc.). If all four margins
    /// are 0 (the selection covers the whole pane), the returned
    /// subtree is a single center leaf — the caller may handle this as
    /// "just change the pane's module".
    pub fn convert_to_splits(&self, target_module: ModuleId) -> (PaneNode, Vec<PaneId>) {
        let (top, bottom, left, right) = self.normalized_rect();
        let mut next_id = 0u64;
        let mut new_ids: Vec<PaneId> = Vec::new();

        // Allocate a fresh placeholder id (in the local 0..n space).
        let alloc = |ids: &mut Vec<PaneId>, counter: &mut u64| -> PaneId {
            let p = PaneId(*counter);
            *counter += 1;
            ids.push(p);
            p
        };

        // Start with the center leaf (the selected module). The center
        // is always the first leaf created → PaneId(0). The caller
        // finds it after reassignment by tracking this placeholder id.
        let center_id = alloc(&mut new_ids, &mut next_id);
        let mut node = PaneNode::Leaf {
            id: center_id,
            module: target_module,
        };

        // Right split (Vertical): center + right. The center is the
        // first child; right is the second. Ratio (first child share)
        // = center / (center + right) = (1 - left - right) / (1 - left).
        // Skip if right margin is 0.
        if right > 0.0 {
            let right_id = alloc(&mut new_ids, &mut next_id);
            let right_leaf = PaneNode::Leaf {
                id: right_id,
                module: ModuleId::Placeholder,
            };
            let denom = (1.0 - left).max(0.001);
            let ratio = ((1.0 - left - right) / denom).clamp(MIN_RATIO, MAX_RATIO);
            node = PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio: clamp_ratio(ratio),
                first: Box::new(node),
                second: Box::new(right_leaf),
            };
        }

        // Left split (Vertical): left + (center + right). Left is the
        // first child; the rest is the second. Ratio = left / 1 = left.
        // Skip if left margin is 0.
        if left > 0.0 {
            let left_id = alloc(&mut new_ids, &mut next_id);
            let left_leaf = PaneNode::Leaf {
                id: left_id,
                module: ModuleId::Placeholder,
            };
            let ratio = left.clamp(MIN_RATIO, MAX_RATIO);
            node = PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio: clamp_ratio(ratio),
                first: Box::new(left_leaf),
                second: Box::new(node),
            };
        }

        // Bottom split (Horizontal): (left + center + right) + bottom.
        // The center-stuff is the first child; bottom is the second.
        // Ratio (first child share) = (1 - top - bottom) / (1 - top).
        // Skip if bottom margin is 0.
        if bottom > 0.0 {
            let bottom_id = alloc(&mut new_ids, &mut next_id);
            let bottom_leaf = PaneNode::Leaf {
                id: bottom_id,
                module: ModuleId::Placeholder,
            };
            let denom = (1.0 - top).max(0.001);
            let ratio = ((1.0 - top - bottom) / denom).clamp(MIN_RATIO, MAX_RATIO);
            node = PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio: clamp_ratio(ratio),
                first: Box::new(node),
                second: Box::new(bottom_leaf),
            };
        }

        // Top split (Horizontal): top + (everything else). Top is the
        // first child; the rest is the second. Ratio = top / 1 = top.
        // Skip if top margin is 0.
        if top > 0.0 {
            let top_id = alloc(&mut new_ids, &mut next_id);
            let top_leaf = PaneNode::Leaf {
                id: top_id,
                module: ModuleId::Placeholder,
            };
            let ratio = top.clamp(MIN_RATIO, MAX_RATIO);
            node = PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ratio: clamp_ratio(ratio),
                first: Box::new(top_leaf),
                second: Box::new(node),
            };
        }

        (node, new_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(anchor: (f32, f32), cursor: (f32, f32)) -> RectangleSelection {
        RectangleSelection {
            target_pane: PaneId(0),
            anchor: NormalizedPoint::new(anchor.0, anchor.1),
            cursor: NormalizedPoint::new(cursor.0, cursor.1),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        }
    }

    /// `NormalizedPoint::new` clamps to [0, 1].
    #[test]
    fn normalized_point_clamps() {
        assert_eq!(
            NormalizedPoint::new(-1.0, -1.0),
            NormalizedPoint::new(0.0, 0.0)
        );
        assert_eq!(
            NormalizedPoint::new(2.0, 2.0),
            NormalizedPoint::new(1.0, 1.0)
        );
        assert_eq!(
            NormalizedPoint::new(0.5, 0.5),
            NormalizedPoint::new(0.5, 0.5)
        );
    }

    /// `new` initializes the selection with a small default rect
    /// (20%×20%) centered on the pane, with the anchor active.
    #[test]
    fn new_starts_with_default_rect_anchor_active() {
        let s = RectangleSelection::new(PaneId(7));
        assert_eq!(s.target_pane, PaneId(7));
        assert_eq!(s.anchor, NormalizedPoint::new(0.4, 0.4));
        assert_eq!(s.cursor, NormalizedPoint::new(0.6, 0.6));
        assert_eq!(s.phase, SelectionPhase::ChoosingAnchor);
        assert_eq!(s.input_source, SelectionInput::Keyboard);
        assert!(s.active_is_anchor);
    }

    /// Keyboard selection: anchor → extent → confirm. Verify the
    /// phases + that arrow keys move the active corner.
    #[test]
    fn keyboard_workflow_phases() {
        let mut s = RectangleSelection::new(PaneId(0));
        assert_eq!(s.phase, SelectionPhase::ChoosingAnchor);
        // Move anchor right + down.
        s.move_cursor(Direction::Right, 0.02, false);
        s.move_cursor(Direction::Down, 0.02, false);
        assert!(s.anchor.x > 0.4);
        assert!(s.anchor.y > 0.4);
        // Cursor unchanged.
        assert_eq!(s.cursor, NormalizedPoint::new(0.6, 0.6));
        // Confirm anchor → ChoosingExtent, cursor active.
        s.confirm_anchor();
        assert_eq!(s.phase, SelectionPhase::ChoosingExtent);
        assert!(!s.active_is_anchor);
        // Move cursor (extent).
        s.move_cursor(Direction::Left, 0.02, false);
        s.move_cursor(Direction::Up, 0.02, false);
        assert!(s.cursor.x < 0.6);
        assert!(s.cursor.y < 0.6);
        // Anchor unchanged.
        assert!(s.anchor.x > 0.4);
        assert!(s.anchor.y > 0.4);
        // Tab switches active corner.
        s.switch_corner();
        assert!(s.active_is_anchor);
        // Move anchor again.
        let anchor_before = s.anchor;
        s.move_cursor(Direction::Left, 0.02, false);
        assert!(s.anchor.x < anchor_before.x);
        // Confirm → Confirming.
        s.confirm();
        assert_eq!(s.phase, SelectionPhase::Confirming);
    }

    /// Fast movement (Shift) is 4x the step.
    #[test]
    fn fast_move_is_4x() {
        let mut s = RectangleSelection::new(PaneId(0));
        s.move_cursor(Direction::Right, 0.02, true);
        // 4 * 0.02 = 0.08; anchor was 0.4, now 0.48.
        assert!((s.anchor.x - 0.48).abs() < 1e-6);
    }

    /// Dragging in all four directions normalizes (down-right,
    /// down-left, up-right, up-left).
    #[test]
    fn normalized_rect_all_drag_directions() {
        // down-right: anchor top-left, cursor bottom-right.
        let s = sel((0.2, 0.2), (0.8, 0.8));
        let (top, bottom, left, right) = s.normalized_rect();
        assert!((top - 0.2).abs() < 1e-6);
        assert!((bottom - 0.2).abs() < 1e-6);
        assert!((left - 0.2).abs() < 1e-6);
        assert!((right - 0.2).abs() < 1e-6);

        // down-left: anchor top-right, cursor bottom-left.
        let s = sel((0.8, 0.2), (0.2, 0.8));
        let (top, bottom, left, right) = s.normalized_rect();
        assert!((top - 0.2).abs() < 1e-6);
        assert!((bottom - 0.2).abs() < 1e-6);
        assert!((left - 0.2).abs() < 1e-6);
        assert!((right - 0.2).abs() < 1e-6);

        // up-right: anchor bottom-left, cursor top-right.
        let s = sel((0.2, 0.8), (0.8, 0.2));
        let (top, bottom, left, right) = s.normalized_rect();
        assert!((top - 0.2).abs() < 1e-6);
        assert!((bottom - 0.2).abs() < 1e-6);
        assert!((left - 0.2).abs() < 1e-6);
        assert!((right - 0.2).abs() < 1e-6);

        // up-left: anchor bottom-right, cursor top-left.
        let s = sel((0.8, 0.8), (0.2, 0.2));
        let (top, bottom, left, right) = s.normalized_rect();
        assert!((top - 0.2).abs() < 1e-6);
        assert!((bottom - 0.2).abs() < 1e-6);
        assert!((left - 0.2).abs() < 1e-6);
        assert!((right - 0.2).abs() < 1e-6);
    }

    /// Clamping: cursor can't go below 0 or above 1.
    #[test]
    fn move_clamps_to_bounds() {
        let mut s = RectangleSelection::new(PaneId(0));
        // Move far left — should clamp at 0.
        for _ in 0..100 {
            s.move_cursor(Direction::Left, 0.1, false);
        }
        assert_eq!(s.anchor.x, 0.0);
        // Move far right — should clamp at 1.
        for _ in 0..100 {
            s.move_cursor(Direction::Right, 0.1, false);
        }
        assert_eq!(s.anchor.x, 1.0);
        // Up/down.
        for _ in 0..100 {
            s.move_cursor(Direction::Up, 0.1, false);
        }
        assert_eq!(s.anchor.y, 0.0);
        for _ in 0..100 {
            s.move_cursor(Direction::Down, 0.1, false);
        }
        assert_eq!(s.anchor.y, 1.0);
    }

    /// Minimum-size validation: a 1x1 selection is "too small".
    #[test]
    fn is_valid_rejects_too_small() {
        // 1x1 selection at the center of a 100x30 pane.
        let mut s = RectangleSelection::new(PaneId(0));
        // Move anchor slightly to make a tiny selection.
        s.anchor = NormalizedPoint::new(0.5, 0.5);
        s.cursor = NormalizedPoint::new(0.51, 0.51);
        let pane_area = Rect::new(0, 0, 100, 30);
        // 1% of 100 = 1 wide; 1% of 30 = 0.3 high → ~1x0. Too small.
        assert!(!s.is_valid(pane_area), "1x0 selection should be too small");

        // A larger selection (20% x 50%) is valid.
        s.anchor = NormalizedPoint::new(0.3, 0.2);
        s.cursor = NormalizedPoint::new(0.5, 0.7);
        // width = 20% of 100 = 20; height = 50% of 30 = 15. Both >= min.
        assert!(s.is_valid(pane_area), "20x15 selection should be valid");
    }

    /// Coordinate conversion: normalized ↔ cells at multiple pane sizes.
    #[test]
    fn to_cell_rect_at_multiple_sizes() {
        // 50% x 50% selection at the center.
        let s = sel((0.25, 0.25), (0.75, 0.75));
        // At 100x30: x=25, y=7 (rounded from 7.5), w=50, h=15.
        let r = s.to_cell_rect(Rect::new(0, 0, 100, 30));
        assert_eq!(r.x, 25);
        assert_eq!(r.y, 8); // 0.25 * 30 = 7.5 → round to 8
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 15); // 0.5 * 30 = 15

        // At 200x60: x=50, y=15, w=100, h=30.
        let r = s.to_cell_rect(Rect::new(0, 0, 200, 60));
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 15);
        assert_eq!(r.width, 100);
        assert_eq!(r.height, 30);

        // With a non-zero origin: pane at (50, 10) 100x30.
        let r = s.to_cell_rect(Rect::new(50, 10, 100, 30));
        assert_eq!(r.x, 75); // 50 + 25
        assert_eq!(r.y, 18); // 10 + 8
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 15);
    }

    /// Terminal resize during selection: normalized coords don't
    /// change. The same selection maps to different cell rects at
    /// different pane sizes, but the underlying normalized margins are
    /// invariant.
    #[test]
    fn normalized_rect_survives_resize() {
        let s = sel((0.25, 0.25), (0.75, 0.75));
        let (top, bottom, left, right) = s.normalized_rect();
        // At 100x30.
        let r1 = s.to_cell_rect(Rect::new(0, 0, 100, 30));
        // At 200x60 (same proportions, double size).
        let r2 = s.to_cell_rect(Rect::new(0, 0, 200, 60));
        // Normalized margins unchanged.
        let (top2, bottom2, left2, right2) = s.normalized_rect();
        assert_eq!(top, top2);
        assert_eq!(bottom, bottom2);
        assert_eq!(left, left2);
        assert_eq!(right, right2);
        // Cell rect scales with the pane.
        assert_eq!(r2.width, 2 * r1.width);
        assert_eq!(r2.height, 2 * r1.height);
    }

    /// Dimensions label format: "WxH cells · W% × H%".
    #[test]
    fn dimensions_label_format() {
        let s = sel((0.25, 0.25), (0.75, 0.75));
        let label = s.dimensions_label(Rect::new(0, 0, 100, 30));
        // 50% x 50% → 50x15 cells · 50% × 50%.
        assert_eq!(label, "50x15 cells · 50% × 50%");
    }

    /// Conversion to split tree (all margins > 0): verify the nested
    /// structure — top split, bottom split, left split, right split,
    /// center gets the target module, surrounding get Placeholder.
    #[test]
    fn convert_to_splits_full_structure() {
        // Selection with all margins > 0: top=0.1, bottom=0.1, left=0.2, right=0.2.
        let s = sel((0.2, 0.1), (0.8, 0.9));
        let (node, ids) = s.convert_to_splits(ModuleId::Queue);
        // 5 new panes: center + 4 surrounding.
        assert_eq!(ids.len(), 5);
        // All ids unique.
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        // Center is PaneId(0).
        assert!(ids.contains(&PaneId(0)));

        // Structure: Split(H, top, Leaf(top, Ph), Split(H, bottom', Split(V, left, Leaf(left, Ph), Split(V, right', Leaf(center, Queue), Leaf(right, Ph))), Leaf(bottom, Ph)))
        match &node {
            PaneNode::Split {
                axis: SplitAxis::Horizontal,
                first,
                second,
                ..
            } => {
                // First child = top (Placeholder).
                assert!(matches!(
                    first.as_ref(),
                    PaneNode::Leaf {
                        module: ModuleId::Placeholder,
                        ..
                    }
                ));
                // Second child = the bottom split.
                match second.as_ref() {
                    PaneNode::Split {
                        axis: SplitAxis::Horizontal,
                        first,
                        second,
                        ..
                    } => {
                        // First child of bottom split = the left/right split.
                        match first.as_ref() {
                            PaneNode::Split {
                                axis: SplitAxis::Vertical,
                                first,
                                second,
                                ..
                            } => {
                                // First child of left split = left (Placeholder).
                                assert!(matches!(
                                    first.as_ref(),
                                    PaneNode::Leaf {
                                        module: ModuleId::Placeholder,
                                        ..
                                    }
                                ));
                                // Second child of left split = the right split.
                                match second.as_ref() {
                                    PaneNode::Split {
                                        axis: SplitAxis::Vertical,
                                        first,
                                        second,
                                        ..
                                    } => {
                                        // First child of right split = center (Queue).
                                        assert!(matches!(
                                            first.as_ref(),
                                            PaneNode::Leaf {
                                                module: ModuleId::Queue,
                                                ..
                                            }
                                        ));
                                        // Second child of right split = right (Placeholder).
                                        assert!(matches!(
                                            second.as_ref(),
                                            PaneNode::Leaf {
                                                module: ModuleId::Placeholder,
                                                ..
                                            }
                                        ));
                                    }
                                    _ => panic!("expected right Split"),
                                }
                            }
                            _ => panic!("expected left Split"),
                        }
                        // Second child of bottom split = bottom (Placeholder).
                        assert!(matches!(
                            second.as_ref(),
                            PaneNode::Leaf {
                                module: ModuleId::Placeholder,
                                ..
                            }
                        ));
                    }
                    _ => panic!("expected bottom Split"),
                }
            }
            _ => panic!("expected top Split"),
        }
    }

    /// Conversion with zero margins: skip the unnecessary splits.
    #[test]
    fn convert_to_splits_skips_zero_margins() {
        // Only top margin > 0 (selection is the bottom 30% of the
        // pane): should produce just a top split (Placeholder on top,
        // center below).
        let s = sel((0.0, 0.7), (1.0, 1.0));
        let (node, ids) = s.convert_to_splits(ModuleId::Queue);
        // 2 new panes: center + top.
        assert_eq!(ids.len(), 2);
        match &node {
            PaneNode::Split {
                axis: SplitAxis::Horizontal,
                first,
                second,
                ..
            } => {
                assert!(matches!(
                    first.as_ref(),
                    PaneNode::Leaf {
                        module: ModuleId::Placeholder,
                        ..
                    }
                ));
                assert!(matches!(
                    second.as_ref(),
                    PaneNode::Leaf {
                        module: ModuleId::Queue,
                        ..
                    }
                ));
            }
            _ => panic!("expected top Split"),
        }

        // No margins (selection covers the whole pane): just a center
        // leaf, no splits.
        let s = sel((0.0, 0.0), (1.0, 1.0));
        let (node, ids) = s.convert_to_splits(ModuleId::Queue);
        assert_eq!(ids.len(), 1);
        assert!(matches!(
            node,
            PaneNode::Leaf {
                module: ModuleId::Queue,
                ..
            }
        ));

        // Only left + right margins (no top/bottom): a vertical split
        // for left, then a vertical split for right (with center +
        // right).
        let s = sel((0.2, 0.0), (0.8, 1.0));
        let (node, ids) = s.convert_to_splits(ModuleId::Queue);
        // 3 new panes: center + left + right.
        assert_eq!(ids.len(), 3);
        match &node {
            PaneNode::Split {
                axis: SplitAxis::Vertical,
                first,
                second,
                ..
            } => {
                // first = left (Placeholder).
                assert!(matches!(
                    first.as_ref(),
                    PaneNode::Leaf {
                        module: ModuleId::Placeholder,
                        ..
                    }
                ));
                // second = right split (center + right).
                match second.as_ref() {
                    PaneNode::Split {
                        axis: SplitAxis::Vertical,
                        first,
                        second,
                        ..
                    } => {
                        assert!(matches!(
                            first.as_ref(),
                            PaneNode::Leaf {
                                module: ModuleId::Queue,
                                ..
                            }
                        ));
                        assert!(matches!(
                            second.as_ref(),
                            PaneNode::Leaf {
                                module: ModuleId::Placeholder,
                                ..
                            }
                        ));
                    }
                    _ => panic!("expected right Split"),
                }
            }
            _ => panic!("expected left Split"),
        }
    }

    /// `cancel` returns to ChoosingAnchor with the anchor active.
    #[test]
    fn cancel_resets_phase() {
        let mut s = RectangleSelection::new(PaneId(0));
        s.confirm_anchor();
        assert_eq!(s.phase, SelectionPhase::ChoosingExtent);
        s.cancel();
        assert_eq!(s.phase, SelectionPhase::ChoosingAnchor);
        assert!(s.active_is_anchor);
    }
}
