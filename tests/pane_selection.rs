//! Integration tests for the rectangle-selection pane workflow (Phase 2).
//!
//! Drives `tui::input::handle_key` against a real `App` (no terminal)
//! to verify the end-to-end rectangle-selection workflow: enter
//! rectangle mode, move the anchor + cursor, confirm, pick a module,
//! and verify the resulting split tree. Also tests mouse drag,Esc
//! cancellation, minimum-size validation, terminal-resize invariance,
//! and PaneId uniqueness after conversion.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::input::{handle_key, handle_mouse_in_area};
use jukebox::tui::pane::model::{ModuleId, PaneId, UiMode};
use jukebox::tui::pane::selection::{
    NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase, MIN_SELECTION_HEIGHT,
    MIN_SELECTION_WIDTH,
};
use jukebox::tui::pane::{PaneNode, PaneWorkspace, SplitAxis};
use ratatui::layout::Rect;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a 2-artist catalog so Artists / Albums / Tracks columns have
/// real content to render. Mirrors `tests/pane.rs::cat_album`.
fn cat_album() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
    std::fs::create_dir_all(lossless.join("DECO")).unwrap();
    std::fs::write(lossless.join("DECO").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1","album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Ghost Rule","album":"Ghost","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn build_app() -> App {
    let (_d, cat) = cat_album();
    // Leak the tempdir so the catalog's source paths stay valid for the
    // app's lifetime (the test process exits soon enough).
    std::mem::forget(_d);
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

fn enter_edit_mode(app: &mut App) {
    handle_key(
        app,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    handle_key(app, key('e'));
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
}

fn enter_rectangle_selection(app: &mut App) {
    handle_key(app, key('r'));
    assert!(app.rectangle_selection.is_some());
}

/// Move the active corner repeatedly to build a selection of the
/// desired size. `dx`/`dy` are signed step counts (positive = right/down).
fn move_corner(app: &mut App, dx: i32, dy: i32) {
    if dx > 0 {
        for _ in 0..dx {
            handle_key(app, key_code(KeyCode::Right));
        }
    } else if dx < 0 {
        for _ in 0..-dx {
            handle_key(app, key_code(KeyCode::Left));
        }
    }
    if dy > 0 {
        for _ in 0..dy {
            handle_key(app, key_code(KeyCode::Down));
        }
    } else if dy < 0 {
        for _ in 0..-dy {
            handle_key(app, key_code(KeyCode::Up));
        }
    }
}

/// Collect all leaf ids in tree order.
fn leaf_ids(node: &PaneNode) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect_leaf_ids(node, &mut out);
    out
}

fn collect_leaf_ids(node: &PaneNode, out: &mut Vec<PaneId>) {
    match node {
        PaneNode::Leaf { id, .. } => out.push(*id),
        PaneNode::Split { first, second, .. } => {
            collect_leaf_ids(first, out);
            collect_leaf_ids(second, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 1 regression: ensure existing pane tests aren't broken by the
// new `r` binding in PaneEdit mode.
// ---------------------------------------------------------------------------

/// `r` in Normal mode still cycles repeat (it's a global key). The
/// pane-edit `r` (rectangle selection) shouldn't shadow it in Normal.
#[test]
fn r_in_normal_still_cycles_repeat() {
    let mut app = build_app();
    let repeat_before = app.transport.repeat;
    handle_key(&mut app, key('r'));
    assert_ne!(
        app.transport.repeat, repeat_before,
        "`r` should cycle repeat in Normal mode"
    );
    assert!(
        app.rectangle_selection.is_none(),
        "rectangle selection should not activate in Normal mode"
    );
}

// ---------------------------------------------------------------------------
// Keyboard workflow
// ---------------------------------------------------------------------------

/// `r` in PaneEdit mode enters rectangle selection. The selection
/// starts at the center of the focused pane, in ChoosingAnchor phase,
/// with the anchor active.
#[test]
fn r_enters_rectangle_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.target_pane, app.pane_workspace.focused_pane);
    assert_eq!(sel.phase, SelectionPhase::ChoosingAnchor);
    assert_eq!(sel.input_source, SelectionInput::Keyboard);
    assert_eq!(sel.anchor, NormalizedPoint::new(0.4, 0.4));
    assert_eq!(sel.cursor, NormalizedPoint::new(0.6, 0.6));
    assert!(sel.active_is_anchor);
}

/// Arrow keys move the active corner in ChoosingAnchor phase. The
/// cursor (extent) stays put at the center.
#[test]
fn arrows_move_anchor_in_choosing_anchor() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
    move_corner(&mut app, 5, 3);
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(sel.anchor.x > anchor_before.x, "anchor.x should increase");
    assert!(sel.anchor.y > anchor_before.y, "anchor.y should increase");
    // Cursor unchanged.
    assert_eq!(sel.cursor, NormalizedPoint::new(0.6, 0.6));
    // Still in ChoosingAnchor.
    assert_eq!(sel.phase, SelectionPhase::ChoosingAnchor);
}

/// `h/j/k/l` move the active corner (mirrors arrow keys).
#[test]
fn hjkl_moves_active_corner() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    handle_key(&mut app, key('j'));
    handle_key(&mut app, key('l'));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(sel.anchor.y > 0.4, "j should move anchor down from 0.4");
    assert!(sel.anchor.x > 0.4, "l should move anchor right from 0.4");
    handle_key(&mut app, key('h'));
    handle_key(&mut app, key('k'));
    let sel = app.rectangle_selection.as_ref().unwrap();
    // After h+k, anchor should have moved back (but not necessarily to
    // exactly 0.4 — we moved right+down by 0.02 each, then left+up by
    // 0.02 each, so we end up at ~0.4 with float rounding).
    assert!(sel.anchor.x <= 0.4 + 0.001);
    assert!(sel.anchor.y <= 0.4 + 0.001);
}

/// Shift+arrows move the active corner 4x faster than plain arrows.
#[test]
fn shift_arrows_move_4x_faster() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // One Shift+Right = 4 * 0.02 = 0.08. Anchor starts at 0.4.
    handle_key(&mut app, shift(KeyCode::Right));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(
        (sel.anchor.x - 0.48).abs() < 1e-6,
        "Shift+Right should move by 0.08 from 0.4 to 0.48, got {}",
        sel.anchor.x
    );
}

/// `H/J/K/L` move the active corner 4x faster (uppercase = Shift).
#[test]
fn uppercase_hjkl_moves_4x_faster() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    handle_key(&mut app, key('L'));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(
        (sel.anchor.x - 0.48).abs() < 1e-6,
        "L should move by 0.08 from 0.4 to 0.48, got {}",
        sel.anchor.x
    );
}

/// Enter in ChoosingAnchor phase transitions to ChoosingExtent; the
/// cursor (extent) is now the active corner.
#[test]
fn enter_transitions_to_choosing_extent() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 5, 5);
    handle_key(&mut app, key_code(KeyCode::Enter));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.phase, SelectionPhase::ChoosingExtent);
    assert!(
        !sel.active_is_anchor,
        "cursor should be active after confirm_anchor"
    );
}

/// Tab in ChoosingExtent switches the active corner.
#[test]
fn tab_switches_active_corner() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
                                                    // By default, cursor is active.
    let cursor_before = app.rectangle_selection.as_ref().unwrap().cursor;
    handle_key(&mut app, key('j')); // move cursor down
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(sel.cursor.y > cursor_before.y, "cursor should have moved");
    // Tab switches to anchor.
    handle_key(&mut app, key_code(KeyCode::Tab));
    let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
    handle_key(&mut app, key('j')); // now moves anchor
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert!(
        sel.anchor.y > anchor_before.y,
        "anchor should have moved after Tab"
    );
}

/// Enter in ChoosingExtent with a valid selection transitions to
/// Confirming AND opens the module picker overlay.
#[test]
fn enter_in_choosing_extent_opens_picker_when_valid() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Anchor starts at (0.4, 0.4). Move it to (0.1, 0.1) by 15 left + 15 up.
    for _ in 0..15 {
        handle_key(&mut app, key_code(KeyCode::Left));
        handle_key(&mut app, key_code(KeyCode::Up));
    }
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
                                                    // Cursor starts at (0.6, 0.6). Move it to (0.9, 0.9) by 15 right + 15 down.
    for _ in 0..15 {
        handle_key(&mut app, key_code(KeyCode::Right));
        handle_key(&mut app, key_code(KeyCode::Down));
    }
    // The selection is now ~(0.1..0.9, 0.1..0.9) — 80%×80% — well above minimum.
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(
        matches!(
            app.overlay,
            Some(jukebox::tui::app::Overlay::PaneModulePicker { .. })
        ),
        "module picker should be open"
    );
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.phase, SelectionPhase::Confirming);
}

/// Enter in ChoosingExtent with a too-small selection shows a toast
/// and does NOT open the picker.
#[test]
fn enter_in_choosing_extent_toast_when_too_small() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Anchor starts at (0.4, 0.4). Confirm anchor → cursor active.
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
                                                    // Move the cursor very close to the anchor (1 step = 0.02).
    handle_key(&mut app, key_code(KeyCode::Left)); // cursor.x 0.6→0.58
                                                   // Selection is now 0.4..0.58 = 0.18 wide, 0.4..0.6 = 0.2 tall —
                                                   // that's still valid at most sizes. Make it tiny: move cursor to
                                                   // nearly the same point as anchor.
    for _ in 0..9 {
        handle_key(&mut app, key_code(KeyCode::Left));
        handle_key(&mut app, key_code(KeyCode::Up));
    }
    // Now cursor is at ~(0.4, 0.42) — selection ~0.02×0.02 — too small.
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Should NOT open the picker.
    assert!(
        app.overlay.is_none(),
        "picker should not open for a too-small selection"
    );
    // Should show a status toast.
    assert!(
        app.status_toast.is_some(),
        "should show a 'too small' toast"
    );
    // Phase should still be ChoosingExtent (the confirm was refused).
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(
        sel.phase,
        SelectionPhase::ChoosingExtent,
        "should stay in ChoosingExtent after a refused confirm"
    );
}

/// Esc cancels the selection (clears `app.rectangle_selection`).
#[test]
fn esc_cancels_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        app.rectangle_selection.is_none(),
        "Esc should clear the selection"
    );
    // Should still be in PaneEdit mode (Esc doesn't exit edit mode when
    // a selection is active).
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
}

/// Esc during selection doesn't mutate the tree.
#[test]
fn esc_does_not_mutate_tree() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 5, 5);
    let tree_before = app.pane_workspace.root.clone();
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        tree_before.leaf_count(),
        "tree should be unchanged after cancel"
    );
}

/// Playback keys (Space, >, <, q) keep working during rectangle
/// selection.
#[test]
fn playback_keys_work_during_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Space toggles pause (no panic).
    handle_key(&mut app, key(' '));
    // q quits.
    handle_key(&mut app, key('q'));
    assert!(
        app.should_quit,
        "q should quit even during rectangle selection"
    );
}

/// `r` during selection resets the selection (back to center,
/// ChoosingAnchor).
#[test]
fn r_resets_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 10, 10);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
                                                    // Press r again — should reset.
    handle_key(&mut app, key('r'));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.phase, SelectionPhase::ChoosingAnchor);
    assert_eq!(sel.anchor, NormalizedPoint::new(0.4, 0.4));
    assert_eq!(sel.cursor, NormalizedPoint::new(0.6, 0.6));
}

/// Split/close keys (`s`, `v`, `x`, `d`, `m`) are swallowed during
/// rectangle selection (the user must Esc out first).
#[test]
fn split_close_keys_swallowed_during_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    let leaf_count_before = app.pane_workspace.root.leaf_count();
    // Press each of s/v/x/d/m — none should split/close/open overlay.
    for c in ['s', 'v', 'x', 'd', 'm'] {
        handle_key(&mut app, key(c));
    }
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        leaf_count_before,
        "split/close keys should be swallowed during selection"
    );
    assert!(
        app.overlay.is_none(),
        "no overlay should open during selection"
    );
}

// ---------------------------------------------------------------------------
// Clamping
// ---------------------------------------------------------------------------

/// The active corner can't go below 0 or above 1 (clamped).
#[test]
fn cursor_clamps_to_pane_bounds() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Move far left.
    for _ in 0..200 {
        handle_key(&mut app, key_code(KeyCode::Left));
    }
    assert_eq!(
        app.rectangle_selection.as_ref().unwrap().anchor.x,
        0.0,
        "anchor.x should clamp at 0"
    );
    // Move far right.
    for _ in 0..200 {
        handle_key(&mut app, key_code(KeyCode::Right));
    }
    assert_eq!(
        app.rectangle_selection.as_ref().unwrap().anchor.x,
        1.0,
        "anchor.x should clamp at 1"
    );
    // Up/down.
    for _ in 0..200 {
        handle_key(&mut app, key_code(KeyCode::Up));
    }
    assert_eq!(app.rectangle_selection.as_ref().unwrap().anchor.y, 0.0);
    for _ in 0..200 {
        handle_key(&mut app, key_code(KeyCode::Down));
    }
    assert_eq!(app.rectangle_selection.as_ref().unwrap().anchor.y, 1.0);
}

// ---------------------------------------------------------------------------
// Normalization (dragging in all 4 directions)
// ---------------------------------------------------------------------------

/// Direct unit-test-level verification that dragging in all 4
/// directions produces the same normalized rect. (The selection model
/// normalizes reversed selections so the anchor is effectively the
/// top-left and the cursor the bottom-right.)
#[test]
fn normalized_rect_all_drag_directions() {
    let expect = (0.2f32, 0.2f32, 0.2f32, 0.2f32);
    // down-right
    let s = rect_selection((0.2, 0.2), (0.8, 0.8));
    assert_rect_approx(s.normalized_rect(), expect);
    // down-left
    let s = rect_selection((0.8, 0.2), (0.2, 0.8));
    assert_rect_approx(s.normalized_rect(), expect);
    // up-right
    let s = rect_selection((0.2, 0.8), (0.8, 0.2));
    assert_rect_approx(s.normalized_rect(), expect);
    // up-left
    let s = rect_selection((0.8, 0.8), (0.2, 0.2));
    assert_rect_approx(s.normalized_rect(), expect);
}

fn assert_rect_approx(actual: (f32, f32, f32, f32), expected: (f32, f32, f32, f32)) {
    let eps = 1e-5;
    assert!(
        (actual.0 - expected.0).abs() < eps
            && (actual.1 - expected.1).abs() < eps
            && (actual.2 - expected.2).abs() < eps
            && (actual.3 - expected.3).abs() < eps,
        "rect mismatch: got {actual:?}, expected {expected:?}"
    );
}

fn rect_selection(anchor: (f32, f32), cursor: (f32, f32)) -> RectangleSelection {
    RectangleSelection {
        target_pane: PaneId(0),
        anchor: NormalizedPoint::new(anchor.0, anchor.1),
        cursor: NormalizedPoint::new(cursor.0, cursor.1),
        phase: SelectionPhase::ChoosingExtent,
        input_source: SelectionInput::Keyboard,
        active_is_anchor: false,
    }
}

// ---------------------------------------------------------------------------
// Coordinate conversion + resize invariance
// ---------------------------------------------------------------------------

/// `to_cell_rect` at multiple pane sizes — the same normalized rect
/// maps to the expected cell rect.
#[test]
fn to_cell_rect_at_multiple_sizes() {
    let s = rect_selection((0.25, 0.25), (0.75, 0.75));
    // 50% x 50% at 100x30 → 50x15 at (25, 8).
    let r = s.to_cell_rect(Rect::new(0, 0, 100, 30));
    assert_eq!(r.x, 25);
    assert_eq!(r.y, 8);
    assert_eq!(r.width, 50);
    assert_eq!(r.height, 15);
    // At 200x60 → 100x30 at (50, 15).
    let r = s.to_cell_rect(Rect::new(0, 0, 200, 60));
    assert_eq!(r.x, 50);
    assert_eq!(r.y, 15);
    assert_eq!(r.width, 100);
    assert_eq!(r.height, 30);
}

/// Terminal resize during selection: normalized coords don't change.
/// The same selection maps to different cell rects at different pane
/// sizes, but the underlying normalized margins are invariant.
#[test]
fn normalized_coords_survive_resize() {
    let s = rect_selection((0.25, 0.25), (0.75, 0.75));
    let (top, bottom, left, right) = s.normalized_rect();
    // Compute the cell rect at one size, then at a larger size. The
    // normalized margins should be identical.
    let _r1 = s.to_cell_rect(Rect::new(0, 0, 100, 30));
    let _r2 = s.to_cell_rect(Rect::new(0, 0, 200, 60));
    let (top2, bottom2, left2, right2) = s.normalized_rect();
    assert_eq!(top, top2);
    assert_eq!(bottom, bottom2);
    assert_eq!(left, left2);
    assert_eq!(right, right2);
}

// ---------------------------------------------------------------------------
// Conversion: full structure + zero margins + id uniqueness
// ---------------------------------------------------------------------------

/// Conversion to a split tree with all margins > 0: verify the nested
/// structure (top split → bottom split → left split → right split →
/// center) and that the center gets the chosen module, surrounding get
/// Placeholder.
#[test]
fn convert_to_splits_full_structure() {
    let s = rect_selection((0.2, 0.1), (0.8, 0.9));
    let (node, ids) = s.convert_to_splits(ModuleId::Queue);
    // 5 new panes: center + 4 surrounding.
    assert_eq!(ids.len(), 5);
    // Center is PaneId(0) (the first leaf created).
    assert!(ids.contains(&PaneId(0)));
    // Verify the nested structure: Split(H, top, Leaf(top, Ph), Split(H, bottom, Split(V, left, Leaf(left, Ph), Split(V, right, Leaf(center, Queue), Leaf(right, Ph))), Leaf(bottom, Ph)))
    match &node {
        PaneNode::Split {
            axis: SplitAxis::Horizontal,
            first,
            second,
            ..
        } => {
            // first = top (Placeholder).
            assert!(matches!(
                first.as_ref(),
                PaneNode::Leaf {
                    module: ModuleId::Placeholder,
                    ..
                }
            ));
            // second = bottom split.
            match second.as_ref() {
                PaneNode::Split {
                    axis: SplitAxis::Horizontal,
                    first,
                    second,
                    ..
                } => {
                    // first = the left/right split.
                    match first.as_ref() {
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
                            // second = right split.
                            match second.as_ref() {
                                PaneNode::Split {
                                    axis: SplitAxis::Vertical,
                                    first,
                                    second,
                                    ..
                                } => {
                                    // first = center (Queue).
                                    assert!(matches!(
                                        first.as_ref(),
                                        PaneNode::Leaf {
                                            module: ModuleId::Queue,
                                            ..
                                        }
                                    ));
                                    // second = right (Placeholder).
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
                    // second = bottom (Placeholder).
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

/// Conversion with zero margins: only the necessary splits are created.
#[test]
fn convert_to_splits_skips_zero_margins() {
    // Only top margin > 0 (selection at the bottom of the pane):
    // Split(H, top, Leaf(top, Ph), Leaf(center, Module)).
    let s = rect_selection((0.0, 0.7), (1.0, 1.0));
    let (node, ids) = s.convert_to_splits(ModuleId::Queue);
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

    // No margins (selection covers the whole pane): just a center leaf.
    let s = rect_selection((0.0, 0.0), (1.0, 1.0));
    let (node, ids) = s.convert_to_splits(ModuleId::Queue);
    assert_eq!(ids.len(), 1);
    assert!(matches!(
        node,
        PaneNode::Leaf {
            module: ModuleId::Queue,
            ..
        }
    ));
}

/// No duplicate PaneId after conversion: the workspace's
/// `apply_rectangle_selection` reassigns all new ids to fresh ones, so
/// the resulting tree has all-unique leaf ids.
#[test]
fn no_duplicate_pane_id_after_conversion() {
    let mut ws = PaneWorkspace::new();
    // Start: single root leaf (PaneId(0)).
    assert_eq!(ws.next_id, 1);
    // Build a rectangle selection covering the center 50% of the pane.
    // top=0.25, bottom=0.25, left=0.25, right=0.25 → 5 new panes.
    let sel = rect_selection((0.25, 0.25), (0.75, 0.75));
    let ok = ws.apply_rectangle_selection(&sel, ModuleId::Queue);
    assert!(ok);
    // The tree now has 5 leaves.
    let ids = leaf_ids(&ws.root);
    assert_eq!(ids.len(), 5);
    // All ids unique.
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate ids found: {ids:?}");
    // The new ids are 1, 2, 3, 4, 5 (next_id was 1; after allocating 5
    // fresh ids, next_id is 6).
    assert_eq!(ws.next_id, 6);
    // The original PaneId(0) is gone (the target pane was replaced).
    assert!(
        !ids.contains(&PaneId(0)),
        "PaneId(0) should be gone after conversion"
    );
    // The focused pane is one of the new ids (the center).
    assert!(ids.contains(&ws.focused_pane));
}

/// The center pane is focused after conversion (so the user's focus
/// stays on the selected region).
#[test]
fn center_pane_focused_after_conversion() {
    let mut ws = PaneWorkspace::new();
    let sel = rect_selection((0.25, 0.25), (0.75, 0.75));
    ws.apply_rectangle_selection(&sel, ModuleId::Queue);
    // Find the focused leaf — it should have the Queue module.
    let panes = jukebox::tui::pane::layout::resolve_rects(&ws.root, Rect::new(0, 0, 100, 30));
    let focused = panes.iter().find(|p| p.pane_id == ws.focused_pane).unwrap();
    assert_eq!(
        focused.module_id,
        ModuleId::Queue,
        "the focused (center) pane should have the chosen module"
    );
}

/// Surrounding panes get Placeholder after conversion.
#[test]
fn surrounding_panes_get_placeholder_after_conversion() {
    let mut ws = PaneWorkspace::new();
    let sel = rect_selection((0.25, 0.25), (0.75, 0.75));
    ws.apply_rectangle_selection(&sel, ModuleId::Queue);
    let panes = jukebox::tui::pane::layout::resolve_rects(&ws.root, Rect::new(0, 0, 100, 30));
    // Exactly one Queue pane (the center); the other 4 are Placeholder.
    let queue_count = panes
        .iter()
        .filter(|p| p.module_id == ModuleId::Queue)
        .count();
    let placeholder_count = panes
        .iter()
        .filter(|p| p.module_id == ModuleId::Placeholder)
        .count();
    assert_eq!(queue_count, 1, "exactly one Queue pane (the center)");
    assert_eq!(
        placeholder_count, 4,
        "the 4 surrounding panes are Placeholder"
    );
}

/// Cancellation (Esc) doesn't mutate the tree.
#[test]
fn cancellation_does_not_mutate_tree() {
    let mut app = build_app();
    let tree_before = app.pane_workspace.root.clone();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 5, 5);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
    handle_key(&mut app, key_code(KeyCode::Esc)); // cancel
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        tree_before.leaf_count(),
        "tree should be unchanged after cancel"
    );
    // The selection is cleared.
    assert!(app.rectangle_selection.is_none());
}

// ---------------------------------------------------------------------------
// End-to-end: rectangle selection → module picker → conversion
// ---------------------------------------------------------------------------

/// End-to-end: enter rectangle mode, pick a region, confirm, pick a
/// module in the picker, and verify the tree is converted to nested
/// splits with the chosen module in the center.
#[test]
fn end_to_end_rectangle_to_split_tree() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Build a 60%-wide/tall selection. The responsive Now Playing deck
    // leaves a shorter pane at 80x24, so this must exceed the existing
    // three-cell minimum after normalized coordinates are resolved.
    move_corner(&mut app, 15, 15);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → ChoosingExtent
                                                    // Move cursor to (0.2, 0.2).
    move_corner(&mut app, -15, -15);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → Confirming + picker
    assert!(matches!(
        app.overlay,
        Some(jukebox::tui::app::Overlay::PaneModulePicker { .. })
    ));
    // Pick the Queue module (cursor 0 = Artists; we want Queue = index 2).
    handle_key(&mut app, key('j'));
    handle_key(&mut app, key('j'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // The overlay is closed; we're back in PaneEdit mode.
    assert!(app.overlay.is_none());
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    // The selection is cleared.
    assert!(app.rectangle_selection.is_none());
    // The tree now has 5 leaves (center + 4 surrounding).
    assert_eq!(app.pane_workspace.root.leaf_count(), 5);
    // The focused pane is the center (Queue module).
    let panes = jukebox::tui::pane::layout::resolve_rects(
        &app.pane_workspace.root,
        Rect::new(0, 0, 100, 30),
    );
    let focused = panes
        .iter()
        .find(|p| p.pane_id == app.pane_workspace.focused_pane)
        .unwrap();
    assert_eq!(focused.module_id, ModuleId::Queue);
}

/// Esc in the module picker cancels the rectangle selection (no tree
/// mutation).
#[test]
fn esc_in_picker_cancels_rectangle_selection() {
    let mut app = build_app();
    let tree_before = app.pane_workspace.root.clone();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 10, 10);
    handle_key(&mut app, key_code(KeyCode::Enter));
    move_corner(&mut app, -10, -10);
    handle_key(&mut app, key_code(KeyCode::Enter)); // → Confirming + picker
    handle_key(&mut app, key_code(KeyCode::Esc)); // cancel picker
    assert!(app.overlay.is_none());
    assert!(
        app.rectangle_selection.is_none(),
        "Esc in picker should clear the rectangle selection"
    );
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        tree_before.leaf_count(),
        "tree should be unchanged after picker cancel"
    );
}

// ---------------------------------------------------------------------------
// Mouse workflow
// ---------------------------------------------------------------------------

/// Build a `MouseEvent` for testing.
fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Mouse drag inside the focused pane starts a selection, updates the
/// cursor on drag, and confirms on mouse-up.
#[test]
fn mouse_drag_starts_updates_confirms() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    // Compute the actual focused pane inner rect (the mouse handler
    // uses `app.pane_content_area()`, which derives from
    // `crossterm::terminal::size()` — in tests this falls back to
    // 80x24, NOT the area we pass to `handle_mouse_in_area`). We use
    // the same computation to derive the expected normalized coords.
    let inner = jukebox::tui::pane::input::focused_pane_inner_rect(&app)
        .expect("focused pane should have an inner rect");
    assert!(
        inner.width > 10 && inner.height > 5,
        "inner rect too small for the test: {inner:?}"
    );
    // Pick a click position near the top-left of the inner rect.
    let click_x = inner.x + 5;
    let click_y = inner.y + 2;
    // And a drag position near the bottom-right.
    let drag_x = inner.right() - 5;
    let drag_y = inner.bottom() - 2;
    let area = app.pane_content_area();

    // Mouse-down at (click_x, click_y) → anchor + cursor there.
    handle_mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), click_x, click_y),
        area,
    );
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.phase, SelectionPhase::ChoosingExtent);
    assert_eq!(sel.input_source, SelectionInput::Mouse);
    let expected_x = (click_x as f32 - inner.x as f32) / inner.width as f32;
    let expected_y = (click_y as f32 - inner.y as f32) / inner.height as f32;
    assert!(
        (sel.anchor.x - expected_x).abs() < 1e-3,
        "anchor.x = {}, expected {}",
        sel.anchor.x,
        expected_x
    );
    assert!(
        (sel.anchor.y - expected_y).abs() < 1e-3,
        "anchor.y = {}, expected {}",
        sel.anchor.y,
        expected_y
    );

    // Drag to (drag_x, drag_y) → cursor updates.
    handle_mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Drag(MouseButton::Left), drag_x, drag_y),
        area,
    );
    let sel = app.rectangle_selection.as_ref().unwrap();
    let drag_nx = (drag_x as f32 - inner.x as f32) / inner.width as f32;
    let drag_ny = (drag_y as f32 - inner.y as f32) / inner.height as f32;
    assert!(
        (sel.cursor.x - drag_nx).abs() < 1e-3,
        "cursor.x = {}, expected {}",
        sel.cursor.x,
        drag_nx
    );
    assert!(
        (sel.cursor.y - drag_ny).abs() < 1e-3,
        "cursor.y = {}, expected {}",
        sel.cursor.y,
        drag_ny
    );
    // Anchor unchanged.
    assert!((sel.anchor.x - expected_x).abs() < 1e-3);

    // Mouse-up → confirm + open picker (the selection is large enough).
    handle_mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Up(MouseButton::Left), drag_x, drag_y),
        area,
    );
    assert!(matches!(
        app.overlay,
        Some(jukebox::tui::app::Overlay::PaneModulePicker { .. })
    ));
    let sel = app.rectangle_selection.as_ref().unwrap();
    assert_eq!(sel.phase, SelectionPhase::Confirming);
}

/// Right-click during selection cancels (clears the selection).
#[test]
fn right_click_cancels_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    let area = app.pane_content_area();
    let inner = jukebox::tui::pane::input::focused_pane_inner_rect(&app)
        .expect("focused pane should have an inner rect");
    handle_mouse_in_area(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + inner.width / 2,
            inner.y + inner.height / 2,
        ),
        area,
    );
    assert!(
        app.rectangle_selection.is_none(),
        "right-click should cancel the selection"
    );
}

/// Mouse events outside the focused pane don't affect the selection
/// (they fall through to the normal mouse handler).
#[test]
fn mouse_outside_focused_pane_ignored_by_selection() {
    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    let sel_before = app.rectangle_selection.clone();
    // Mouse-click at (10000, 10000) — way outside the pane.
    handle_mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 10000, 10000),
        Rect::new(0, 0, 100, 30),
    );
    // The selection is unchanged (the click was outside the focused
    // pane, so it fell through to the normal mouse handler).
    assert_eq!(
        app.rectangle_selection, sel_before,
        "selection should be unchanged by an out-of-pane click"
    );
}

// ---------------------------------------------------------------------------
// Rendering smoke test
// ---------------------------------------------------------------------------

/// Rendering the pane workspace with an active rectangle selection
/// doesn't panic at various terminal sizes.
#[test]
fn render_with_selection_no_panic_at_multiple_sizes() {
    use jukebox::tui::pane::render::render_pane_workspace;
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 10, 10);
    handle_key(&mut app, key_code(KeyCode::Enter));
    move_corner(&mut app, -10, -10);

    for (w, h) in [
        (120, 40),
        (100, 30),
        (80, 24),
        (60, 20),
        (40, 12),
        (20, 6),
        (1, 1),
    ] {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, w, h);
        // No panic at any size.
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
    }
}

/// Rendering the selection shows the dimension label in the buffer.
#[test]
fn render_shows_dimension_label() {
    use jukebox::tui::pane::render::render_pane_workspace;
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = build_app();
    enter_edit_mode(&mut app);
    enter_rectangle_selection(&mut app);
    move_corner(&mut app, 10, 10);
    handle_key(&mut app, key_code(KeyCode::Enter));
    move_corner(&mut app, -10, -10);

    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 100, 30);
    term.draw(|f| render_pane_workspace(f, area, &mut app))
        .unwrap();
    // The buffer should contain "cells" (part of the dimension label).
    let buf = term.backend().buffer();
    let mut found = false;
    for y in 0..30 {
        for x in 0..100 {
            if buf[(x, y)].symbol().contains('c') {
                // Look for "cells" starting at this position.
                let mut word = String::new();
                for dx in 0..6 {
                    if x + dx < 100 {
                        word.push_str(buf[(x + dx, y)].symbol());
                    }
                }
                if word.contains("cells") {
                    found = true;
                    break;
                }
            }
        }
        if found {
            break;
        }
    }
    assert!(
        found,
        "dimension label 'cells' should be visible in the buffer"
    );
}

// ---------------------------------------------------------------------------
// Minimum-size validation
// ---------------------------------------------------------------------------

/// Minimum-size constants match the spec (10x3).
#[test]
fn min_selection_constants_match_spec() {
    assert_eq!(MIN_SELECTION_WIDTH, 10);
    assert_eq!(MIN_SELECTION_HEIGHT, 3);
}

/// A 1x1 selection at the center of a 100x30 pane is too small.
#[test]
fn tiny_selection_is_too_small() {
    let s = rect_selection((0.5, 0.5), (0.51, 0.51));
    assert!(!s.is_valid(Rect::new(0, 0, 100, 30)));
}

/// A selection exactly at the minimum (10x3) is valid.
#[test]
fn min_size_selection_is_valid() {
    // 10/100 = 0.1 wide, 3/30 = 0.1 high.
    let s = rect_selection((0.45, 0.45), (0.55, 0.55));
    // 10% of 100 = 10 wide; 10% of 30 = 3 high. Exactly at the minimum.
    let r = s.to_cell_rect(Rect::new(0, 0, 100, 30));
    assert!(r.width >= MIN_SELECTION_WIDTH);
    assert!(r.height >= MIN_SELECTION_HEIGHT);
    assert!(s.is_valid(Rect::new(0, 0, 100, 30)));
}
