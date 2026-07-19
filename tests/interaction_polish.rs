//! Phase 6 tests: critical interaction-state bugs.
//!
//! Verifies that the search overlay and playlist picker selected rows
//! are visible under `NO_COLOR=1` (visual spec C1 / C2 / I1 / I2 / V7 /
//! V17), the `Ctrl+w` prefix-key emits a status toast on arm (H6 / I4),
//! and the PaneModulePicker mode keeps the `[EDIT]` badge + status line
//! (H7 / I5).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay};
use jukebox::tui::pane::model::{ModuleId, PaneId, Side, UiMode};
use jukebox::tui::pane::render::render_pane_workspace;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock().lock().unwrap_or_else(|e| e.into_inner())
}

/// Build a 2-artist catalog so overlays + panes have content to render.
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
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song One","album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Song Two","album":"Ghost","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
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

/// Search-overlay selected row must have REVERSED + BOLD modifier
/// (visible under NO_COLOR). Phase 6 visual spec C1 / I1 / V7.
#[test]
fn search_overlay_selected_row_has_reversed_bold() {
    let _guard = lock_env();
    std::env::set_var("NO_COLOR", "1");

    let mut app = build_app();
    // Open the search overlay with both tracks as results, cursor on
    // "Song Two" (the second result).
    app.overlay = Some(Overlay::Search {
        input: "Song".into(),
        results: vec!["t1".into(), "t2".into()],
        cursor: 1,
        scope: jukebox::tui::app::SearchScope::Local,
        submitted: Some("Song".into()),
        searching: false,
    });

    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();

    // Find "Song Two" cells and verify REVERSED + BOLD modifier.
    let mut found_reversed = false;
    let mut found_bold = false;
    for y in 0..24 {
        for x in 0..80 {
            let cell = &buf[(x, y)];
            if cell.symbol().contains('S') {
                // Probe a window to confirm "Song Two" is here.
                let mut probe = String::new();
                for dx in 0..10u16 {
                    if let Some(c) = buf.cell((x + dx, y)) {
                        probe.push(c.symbol().chars().next().unwrap_or(' '));
                    }
                }
                if probe.contains("Song Two") {
                    if cell.modifier.contains(Modifier::REVERSED) {
                        found_reversed = true;
                    }
                    if cell.modifier.contains(Modifier::BOLD) {
                        found_bold = true;
                    }
                }
            }
        }
    }
    assert!(
        found_reversed,
        "C1/I1: search-overlay selected row must have REVERSED modifier (visible under NO_COLOR)"
    );
    assert!(
        found_bold,
        "C1/I1: search-overlay selected row must have BOLD modifier"
    );

    std::env::remove_var("NO_COLOR");
}

/// Playlist-picker selected row must have REVERSED + BOLD modifier
/// (visible under NO_COLOR). Phase 6 visual spec C2 / I2 / V7 / V17.
#[test]
fn playlist_picker_selected_row_has_reversed_bold() {
    let _guard = lock_env();
    std::env::set_var("NO_COLOR", "1");

    let mut app = build_app();
    app.playlists = vec![jukebox::tui::app::Playlist {
        name: "Mix A".into(),
        track_ids: vec![],
    }];
    app.overlay = Some(Overlay::PlaylistPicker {
        track_id: "t1".into(),
        cursor: 0,
    });

    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();

    // Find "Mix A" cells and verify REVERSED + BOLD.
    let mut found_reversed = false;
    let mut found_bold = false;
    for y in 0..24 {
        for x in 0..80 {
            let cell = &buf[(x, y)];
            if cell.symbol().contains('M') {
                let mut probe = String::new();
                for dx in 0..6u16 {
                    if let Some(c) = buf.cell((x + dx, y)) {
                        probe.push(c.symbol().chars().next().unwrap_or(' '));
                    }
                }
                if probe.contains("Mix A") {
                    if cell.modifier.contains(Modifier::REVERSED) {
                        found_reversed = true;
                    }
                    if cell.modifier.contains(Modifier::BOLD) {
                        found_bold = true;
                    }
                }
            }
        }
    }
    assert!(
        found_reversed,
        "C2/I2: playlist-picker selected row must have REVERSED modifier (visible under NO_COLOR)"
    );
    assert!(
        found_bold,
        "C2/I2: playlist-picker selected row must have BOLD modifier"
    );

    std::env::remove_var("NO_COLOR");
}

/// `Ctrl+w` prefix-key arms the prefix and emits a status toast (H6 / I4).
/// Drive the real input layer via `handle_key` so the test exercises the
/// actual code path.
#[test]
fn ctrl_w_prefix_emits_status_toast() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    let ev = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
    jukebox::tui::input::handle_key(&mut app, ev);

    assert!(app.pending_pane_prefix, "Ctrl+w should arm the prefix");
    assert!(
        app.status_toast.is_some(),
        "Ctrl+w should emit a status toast (H6/I4)"
    );
    let toast = app.status_toast.as_ref().unwrap();
    assert!(
        toast.contains("Ctrl+w"),
        "toast should mention Ctrl+w, got: {toast}"
    );
}

/// PaneModulePicker mode keeps the [EDIT] badge + status line. The
/// pane render layer should treat PaneModulePicker as a sub-mode of
/// PaneEdit for badge + status-line visibility. Phase 6 H7 / I5.
#[test]
fn pane_module_picker_keeps_edit_badge_and_status_line() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    // Split into 2 panes + enter edit mode + open the module picker.
    app.pane_workspace
        .split(PaneId(0), Side::Right, ModuleId::Queue)
        .unwrap();
    app.pane_workspace.enter_edit_mode();
    app.pane_workspace.mode = UiMode::PaneModulePicker;

    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_pane_workspace(f, Rect::new(0, 0, 80, 24), &mut app))
        .unwrap();
    let buf = term.backend().buffer();

    // Find "[EDIT]" on the focused pane's top border.
    let mut found_edit_badge = false;
    for y in 0..24 {
        for x in 0..80 {
            let cell = &buf[(x, y)];
            if cell.symbol() == "[" {
                let mut probe = String::new();
                for dx in 0..6u16 {
                    if let Some(c) = buf.cell((x + dx, y)) {
                        probe.push(c.symbol().chars().next().unwrap_or(' '));
                    }
                }
                if probe.contains("[EDIT]") {
                    found_edit_badge = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_edit_badge,
        "H7/I5: [EDIT] badge must remain visible when the module picker is open"
    );

    // The status line at the bottom should also be visible. Find
    // "PICK MODULE" or "PANE EDIT" on the last row.
    let last_row: String = (0..80).map(|x| buf[(x, 23)].symbol()).collect();
    assert!(
        last_row.contains("PICK MODULE") || last_row.contains("PANE EDIT"),
        "H7/I5: status line must remain visible in picker mode, got: {last_row}"
    );
}

/// Rectangle selection border uses Thick + accent + BOLD (no DIM).
/// Phase 1 visual spec M13/M14/V38/V39/I6.
#[test]
fn rectangle_selection_border_is_thick_accent_bold() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    use jukebox::tui::pane::{NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase};
    app.pane_workspace
        .split(PaneId(0), Side::Right, ModuleId::Queue)
        .unwrap();
    app.pane_workspace.enter_edit_mode();
    app.rectangle_selection = Some(RectangleSelection {
        target_pane: PaneId(0),
        anchor: NormalizedPoint { x: 0.2, y: 0.2 },
        cursor: NormalizedPoint { x: 0.8, y: 0.8 },
        phase: SelectionPhase::ChoosingExtent,
        input_source: SelectionInput::Keyboard,
        active_is_anchor: false,
    });

    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_pane_workspace(f, Rect::new(0, 0, 80, 24), &mut app))
        .unwrap();
    let buf = term.backend().buffer();

    // The selection border should be visible somewhere in the focused
    // pane's region with BOLD modifier (no DIM).
    let theme = jukebox::tui::view::theme::Theme::default();
    let mut found_bold_accent = false;
    let mut found_dim = false;
    for y in 0..24 {
        for x in 0..80 {
            let cell = &buf[(x, y)];
            // Look for box-drawing chars styled with the accent color.
            if cell.style().fg == Some(theme.border_focused)
                && (cell.symbol() == "┏"
                    || cell.symbol() == "┓"
                    || cell.symbol() == "┗"
                    || cell.symbol() == "┛"
                    || cell.symbol() == "━"
                    || cell.symbol() == "┃")
            {
                if cell.modifier.contains(Modifier::BOLD) {
                    found_bold_accent = true;
                }
                if cell.modifier.contains(Modifier::DIM) {
                    found_dim = true;
                }
            }
        }
    }
    assert!(
        found_bold_accent,
        "M13/M14/V38/V39/I6: rectangle selection border must be accent + BOLD (visible under NO_COLOR)"
    );
    assert!(
        !found_dim,
        "M13/V38: rectangle selection border must NOT use DIM (washed out under NO_COLOR)"
    );
}

/// `S` in pane edit mode hides the now-playing player bar (the user
/// asked for this: "S shows keymap, not hiding statusline (now playing
/// status line)"). Verify pressing `S` sets `player_bar_state.hidden`
/// and emits a toast. The layout respects `hidden` by giving the bar's
/// rows to the browse area.
#[test]
fn s_key_hides_now_playing_player_bar() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    // Enter pane edit mode first (Ctrl+w, e).
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);

    // Press S — should hide the player bar.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
    );
    assert!(
        app.player_bar_state.hidden,
        "S should hide the now-playing player bar (player_bar_state.hidden = true)"
    );
    assert!(
        app.status_toast.is_some(),
        "S should emit a status toast confirming the toggle"
    );
    let toast = app.status_toast.as_ref().unwrap();
    assert!(
        toast.contains("hidden"),
        "toast should say 'player bar hidden', got: {toast}"
    );

    // Press S again — should restore the player bar.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
    );
    assert!(
        !app.player_bar_state.hidden,
        "S again should restore the now-playing player bar (player_bar_state.hidden = false)"
    );
    let toast = app.status_toast.as_ref().unwrap();
    assert!(
        toast.contains("shown"),
        "toast should say 'player bar shown', got: {toast}"
    );
}

/// The layout respects `player_bar_state.hidden`: when hidden, the
/// player bar's rows go to the browse area. Verify by rendering at a
/// fixed size with `hidden=true` and checking the player bar row is
/// NOT rendered with player-bar content.
#[test]
fn layout_skips_player_bar_when_hidden() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    app.view = jukebox::tui::app::View::Artists;
    app.player_bar_state.hidden = true;

    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();

    // The player bar normally occupies rows 27-28 (100x30: tab(1) +
    // content(Min) + sep(1) + bar(2) + footer(2)). When hidden, those
    // rows should NOT contain player-bar content (now-playing title,
    // progress bar `▰`, etc.). The footer (last 2 rows) is still
    // rendered. We verify the bar region is empty or contains browse
    // content, not player-bar content.
    // A simple check: the `▰` progress-bar glyph should NOT appear
    // anywhere in the buffer when the bar is hidden (it's only drawn
    // by the player bar).
    let body: String = (0..30)
        .flat_map(|y| (0..100).map(move |x| buf[(x, y)].symbol()))
        .collect();
    assert!(
        !body.contains('▰'),
        "player bar should NOT render (no ▰ progress bar) when hidden"
    );

    // Now un-hide and verify the bar renders.
    app.player_bar_state.hidden = false;
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();
    let body2: String = (0..30)
        .flat_map(|y| (0..100).map(move |x| buf[(x, y)].symbol()))
        .collect();
    // The bar should now render. We check for either the progress bar
    // glyph `▰` or the `-` / `─` separator (the bar always renders
    // something). The simplest check: the body should be different
    // from the hidden case (more content in the bar region).
    assert!(
        body != body2 || body2.contains('▰') || body2.contains('─'),
        "player bar should render when not hidden"
    );
}

/// When the user splits a pane and picks a module that another pane
/// already has, a toast warns about the duplicate. The user reported:
/// "i can interact inside it, this behavior is confusing and isn't what
/// i wanted" — two identical panes are confusing because they share
/// global cursor state.
#[test]
fn duplicate_module_warning_on_split() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    // Enter pane edit mode + split right with Placeholder (via `v`).
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
    );
    // Now there are 2 panes: the original (Artists, unfocused) + new
    // (Placeholder, focused). The new pane is focused.
    // Press `1` to change the focused pane to Artists — this creates
    // a duplicate (both panes show Artists). A toast should warn.
    app.status_toast = None; // Clear any prior toast
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
    );
    assert!(
        app.status_toast.is_some(),
        "pressing 1 to create a duplicate Artists pane should emit a warning toast"
    );
    let toast = app.status_toast.as_ref().unwrap();
    assert!(
        toast.contains("another pane") || toast.contains("already"),
        "toast should mention the duplicate, got: {toast}"
    );
}

/// The module picker for a new pane starts the cursor at a module that
/// ISN'T the source pane's module, so the default Enter doesn't create
/// a duplicate.
#[test]
fn module_picker_for_split_skips_source_module() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    // Enter pane edit mode + open split picker (`s`).
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    // The split-direction overlay should be open.
    assert!(matches!(
        app.overlay,
        Some(Overlay::PaneSplitDirection { .. })
    ));
    // Pick right side (`l`).
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );
    // The module picker should be open with pending_split = Some(Right).
    match &app.overlay {
        Some(Overlay::PaneModulePicker {
            ref cursor,
            ref pending_split,
            ..
        }) => {
            assert!(
                pending_split.is_some(),
                "pending_split should be set after picking a side"
            );
            // The cursor should NOT be 0 (Artists) — Artists is the
            // source pane's module, so the picker should skip it.
            let all = jukebox::tui::pane::model::ModuleId::all();
            let source_module = all[0]; // Artists (the default app's focused pane)
            let idx = *cursor;
            let picked_module = all[idx.min(all.len() - 1)];
            assert_ne!(
                picked_module, source_module,
                "module picker for split should NOT default to the source pane's module (Artists), got cursor={idx} → {picked_module:?}"
            );
        }
        _ => panic!("expected PaneModulePicker overlay after picking split side"),
    }
}
