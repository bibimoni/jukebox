//! Integration tests for the modular pane-editing system.
//!
//! Drives `tui::input::handle_key` against a real `App` (no terminal) to
//! verify the end-to-end pane workflow: enter edit mode, split, navigate,
//! resize, close, change module, exit. Also verifies that global
//! keybindings (playback, quit) keep working in PaneEdit mode and that
//! a fresh app behaves identically to today.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::input::handle_key;
use jukebox::tui::pane::model::{ModuleId, PaneId, Side, UiMode};
use jukebox::tui::pane::persistence::PaneWorkspaceDto;

/// Build a 2-artist catalog so Artists / Albums / Tracks columns have
/// real content to render.
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

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// `Ctrl+w, e` enters pane edit mode.
#[test]
fn ctrl_w_e_enters_pane_edit_mode() {
    let mut app = build_app();
    assert_eq!(app.pane_workspace.mode, UiMode::Normal);
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
}

/// `Esc` exits pane edit mode without changing the layout.
#[test]
fn esc_exits_pane_edit_mode() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    let before = app.pane_workspace.root.clone();
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert_eq!(app.pane_workspace.mode, UiMode::Normal);
    assert_eq!(app.pane_workspace.root.leaf_count(), before.leaf_count());
}

/// `Ctrl+w, e` again exits pane edit mode (toggles).
#[test]
fn ctrl_w_e_toggles_edit_mode() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    assert_eq!(app.pane_workspace.mode, UiMode::Normal);
}

/// `v` in PaneEdit splits the focused pane vertically with the new pane
/// on the right (Placeholder module). The new pane is focused.
#[test]
fn v_splits_vertically_new_pane_focused() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        2,
        "should have 2 panes"
    );
    // The new pane (id 1) is focused.
    assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
    // The new pane is the SECOND child (right) of a Vertical split.
    match &app.pane_workspace.root {
        jukebox::tui::pane::PaneNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            assert_eq!(*axis, jukebox::tui::pane::SplitAxis::Vertical);
            assert!(first.is_leaf_with_id(PaneId(0)));
            assert!(second.is_leaf_with_id(PaneId(1)));
        }
        _ => panic!("expected Split"),
    }
}

/// `x` in PaneEdit splits horizontally with new pane on the bottom.
#[test]
fn x_splits_horizontally_new_pane_on_bottom() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('x'));
    match &app.pane_workspace.root {
        jukebox::tui::PaneNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            assert_eq!(*axis, jukebox::tui::SplitAxis::Horizontal);
            assert!(first.is_leaf_with_id(PaneId(0)));
            assert!(second.is_leaf_with_id(PaneId(1)));
        }
        _ => panic!("expected Split"),
    }
}

/// `d` in PaneEdit closes the focused pane.
#[test]
fn d_closes_focused_pane() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    assert_eq!(app.pane_workspace.root.leaf_count(), 2);
    // Focused is the new pane (id 1). Close it.
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.pane_workspace.root.leaf_count(),
        1,
        "pane should be closed"
    );
    // Focus moves back to the surviving pane (id 0).
    assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
}

/// `d` on the last (root) pane is a no-op with a status toast.
#[test]
fn d_on_root_pane_is_noop() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    let before = app.pane_workspace.root.clone();
    handle_key(&mut app, key('d'));
    assert_eq!(app.pane_workspace.root.leaf_count(), before.leaf_count());
    // A status toast is shown.
    assert!(
        app.status_toast.is_some(),
        "should show a 'can't close' toast"
    );
}

/// `h/j/k/l` in PaneEdit moves pane focus spatially.
#[test]
fn hjkl_moves_pane_focus() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    // Split right (new pane focused), then move left.
    handle_key(&mut app, key('v'));
    assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
    handle_key(&mut app, key('h'));
    assert_eq!(
        app.pane_workspace.focused_pane,
        PaneId(0),
        "h should move focus left"
    );
    handle_key(&mut app, key('l'));
    assert_eq!(
        app.pane_workspace.focused_pane,
        PaneId(1),
        "l should move focus right"
    );
}

/// `Tab` cycles pane focus forward, `Shift+Tab` backward.
#[test]
fn tab_cycles_pane_focus() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    // 2 panes: 0 (left), 1 (right). Focused is 1.
    handle_key(&mut app, key_code(KeyCode::Tab));
    assert_eq!(
        app.pane_workspace.focused_pane,
        PaneId(0),
        "Tab should cycle to 0"
    );
    handle_key(&mut app, key_code(KeyCode::Tab));
    assert_eq!(
        app.pane_workspace.focused_pane,
        PaneId(1),
        "Tab should cycle back to 1"
    );
    handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
    assert_eq!(
        app.pane_workspace.focused_pane,
        PaneId(0),
        "Shift+Tab should cycle backward"
    );
}

/// `1`/`2`/`3`/`4` in PaneEdit changes the focused pane's module.
#[test]
fn number_keys_change_focused_module() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    // Focused pane is the new one (Placeholder). Change it to Queue.
    handle_key(&mut app, key('3'));
    // Verify the focused pane's module is now Queue.
    let panes = jukebox::tui::pane::layout::resolve_rects(
        &app.pane_workspace.root,
        ratatui::layout::Rect::new(0, 0, 100, 30),
    );
    let focused = panes
        .iter()
        .find(|p| p.pane_id == app.pane_workspace.focused_pane)
        .unwrap();
    assert_eq!(focused.module_id, ModuleId::Queue);
}

/// `H`/`L` in PaneEdit resizes a vertical split.
/// - Focused on the right pane: `H` grows it leftward (decrease ratio);
///   `L` is a no-op (nothing to the right to absorb).
/// - Focused on the left pane: `L` grows it rightward (increase ratio);
///   `H` is a no-op (nothing to the left).
#[test]
fn hl_resizes_vertical_split() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    // Focused is pane 1 (right). H grows leftward (decrease ratio).
    let ratio_before = match &app.pane_workspace.root {
        jukebox::tui::PaneNode::Split { ratio, .. } => *ratio,
        _ => panic!("expected Split"),
    };
    handle_key(&mut app, key('H'));
    let ratio_after = match &app.pane_workspace.root {
        jukebox::tui::PaneNode::Split { ratio, .. } => *ratio,
        _ => panic!("expected Split"),
    };
    assert!(
        ratio_after < ratio_before,
        "H should decrease ratio: {ratio_before} -> {ratio_after}"
    );
    // Move focus to the left pane (id 0). L grows rightward (increase ratio).
    handle_key(&mut app, key('h'));
    assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
    handle_key(&mut app, key('L'));
    let ratio_final = match &app.pane_workspace.root {
        jukebox::tui::PaneNode::Split { ratio, .. } => *ratio,
        _ => panic!("expected Split"),
    };
    assert!(
        ratio_final > ratio_after,
        "L should increase ratio: {ratio_after} -> {ratio_final}"
    );
}

/// Global playback keys keep working in PaneEdit mode.
#[test]
fn playback_keys_work_in_pane_edit_mode() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    // Space toggles pause (no panic, no crash).
    handle_key(&mut app, key(' ')); // Space toggles pause
                                    // `>` advances to next track.
    handle_key(&mut app, key('>'));
    // `+` volume up.
    handle_key(&mut app, key('+'));
    // q quits.
    handle_key(&mut app, key('q'));
    assert!(app.should_quit, "q should quit even in PaneEdit mode");
}

/// `Ctrl+w, h` in Normal mode moves pane focus without entering edit mode.
#[test]
fn ctrl_w_h_in_normal_moves_focus() {
    let mut app = build_app();
    // First split (so there are 2 panes to move between).
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('v'));
    handle_key(&mut app, key_code(KeyCode::Esc));
    // Now in Normal mode with 2 panes, focused on 1.
    assert_eq!(app.pane_workspace.mode, UiMode::Normal);
    assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
    // Ctrl+w, h moves focus to pane 0.
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('h'));
    assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
    // Still in Normal mode.
    assert_eq!(app.pane_workspace.mode, UiMode::Normal);
}

/// A fresh app (no split, no edit mode) is in the inactive state — the
/// legacy per-view renderer would run, not the pane workspace.
#[test]
fn fresh_app_is_inactive() {
    let app = build_app();
    assert!(!app.pane_workspace.is_active());
    assert_eq!(app.pane_workspace.root.leaf_count(), 1);
    assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
}

/// `s` opens the split-direction picker overlay.
#[test]
fn s_opens_split_direction_overlay() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('s'));
    assert!(matches!(
        app.overlay,
        Some(jukebox::tui::app::Overlay::PaneSplitDirection { .. })
    ));
}

/// `m` opens the module picker overlay.
#[test]
fn m_opens_module_picker_overlay() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('m'));
    assert!(matches!(
        app.overlay,
        Some(jukebox::tui::app::Overlay::PaneModulePicker { .. })
    ));
}

/// Split-direction picker: `h` selects Left, then module picker opens.
/// `j/k` navigates modules, Enter confirms and the split happens.
#[test]
fn split_direction_then_module_picker_workflow() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('s'));
    // Pick Left.
    handle_key(&mut app, key('h'));
    // Now in module picker. Pick the first module (Artists, cursor 0).
    handle_key(&mut app, key_code(KeyCode::Enter));
    // The split happened: 2 panes, focused on the new (left) pane.
    assert_eq!(app.pane_workspace.root.leaf_count(), 2);
    // New pane is the FIRST child of the new split (split left).
    match &app.pane_workspace.root {
        jukebox::tui::PaneNode::Split { axis, first, .. } => {
            assert_eq!(*axis, jukebox::tui::SplitAxis::Vertical);
            assert_eq!(first.leaf_count(), 1);
            // First child is the new pane (id 1, Artists).
            assert!(first.is_leaf_with_id(PaneId(1)));
        }
        _ => panic!("expected Split"),
    }
}

/// Module picker: `j/k` navigates, Enter changes the focused pane's module.
#[test]
fn module_picker_changes_module() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('m'));
    // Cursor starts at 0 (Artists). Press j twice to reach Queue (index 2).
    handle_key(&mut app, key('j'));
    handle_key(&mut app, key('j'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Focused pane's module is now Queue.
    let panes = jukebox::tui::pane::layout::resolve_rects(
        &app.pane_workspace.root,
        ratatui::layout::Rect::new(0, 0, 100, 30),
    );
    let focused = panes
        .iter()
        .find(|p| p.pane_id == app.pane_workspace.focused_pane)
        .unwrap();
    assert_eq!(focused.module_id, ModuleId::Queue);
}

/// Esc in the split-direction picker cancels (no split, back to PaneEdit).
#[test]
fn esc_in_split_direction_picker_cancels() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('s'));
    let before = app.pane_workspace.root.clone();
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert_eq!(
        app.pane_workspace.mode,
        UiMode::PaneEdit,
        "should return to PaneEdit"
    );
    assert!(app.overlay.is_none(), "overlay should be closed");
    assert_eq!(app.pane_workspace.root.leaf_count(), before.leaf_count());
}

/// Esc in the module picker cancels (no module change).
#[test]
fn esc_in_module_picker_cancels() {
    let mut app = build_app();
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('m'));
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    assert!(app.overlay.is_none());
}

/// PaneWorkspace DTO round-trips through JSON.
#[test]
fn pane_workspace_dto_round_trip() {
    let mut ws = jukebox::tui::PaneWorkspace::new();
    let _ = ws.split(PaneId(0), Side::Right, ModuleId::Queue);
    let _ = ws.split(PaneId(1), Side::Bottom, ModuleId::Youtube);
    let dto = PaneWorkspaceDto::from(&ws);
    let json = serde_json::to_string(&dto).unwrap();
    let restored: PaneWorkspaceDto = serde_json::from_str(&json).unwrap();
    let ws2: jukebox::tui::PaneWorkspace = restored.into();
    assert_eq!(ws.root.leaf_count(), ws2.root.leaf_count());
    assert_eq!(ws.focused_pane, ws2.focused_pane);
    assert_eq!(ws.next_id, ws2.next_id);
}

/// Render the pane workspace at multiple terminal sizes — no panic.
#[test]
fn render_at_multiple_sizes_no_panic() {
    use jukebox::tui::pane::render::render_pane_workspace;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    let mut app = build_app();
    app.pane_workspace
        .split(PaneId(0), Side::Right, ModuleId::Queue)
        .unwrap();
    app.pane_workspace
        .split(PaneId(0), Side::Bottom, ModuleId::Youtube)
        .unwrap();
    app.pane_workspace.enter_edit_mode();

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

/// A split with a Placeholder module renders the "Press m to choose a
/// module" hint without panicking.
#[test]
fn placeholder_module_renders() {
    use jukebox::tui::pane::render::render_pane_workspace;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    let mut app = build_app();
    app.pane_workspace
        .split(PaneId(0), Side::Right, ModuleId::Placeholder)
        .unwrap();
    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 100, 30);
    term.draw(|f| render_pane_workspace(f, area, &mut app))
        .unwrap();
    // No panic; the placeholder module rendered into the right pane.
}

/// `Ctrl+w` is not reserved and doesn't conflict with existing keys.
/// After `Ctrl+w`, a non-pane key falls through to normal dispatch.
#[test]
fn ctrl_w_plus_unknown_key_falls_through() {
    let mut app = build_app();
    // Ctrl+w arms the prefix; 'z' (cycle shuffle) is not a pane command,
    // so it falls through to the global handler.
    let shuffle_before = app.transport.shuffle;
    handle_key(&mut app, ctrl('w'));
    handle_key(&mut app, key('z'));
    // 'z' should have cycled shuffle (the global handler ran).
    assert_ne!(
        app.transport.shuffle, shuffle_before,
        "z should fall through to global"
    );
    // The prefix should be cleared.
    assert!(!app.pending_pane_prefix);
}
