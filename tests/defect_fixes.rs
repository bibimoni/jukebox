//! Regression tests for confirmed defects from the RC-01 black-box review.
//!
//! Each test maps to a DEF-NNN ID in `docs/development/jukebox-release-loop/DEFECTS.md`.
//! Tests are pure key→action dispatch (no terminal) unless noted.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, Playlist, View};
use jukebox::tui::input::handle_key;
use ratatui::{backend::TestBackend, Terminal};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn cat_album() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3)
        .map(|n| {
            serde_json::json!({
                "id": format!("t{n}"),
                "artists": ["40mP"],
                "primary_artist": "40mP",
                "title": format!("Song{n}"),
                "album": "Cosmic",
                "track_number": n,
                "bit_depth": 24,
                "sample_rate_hz": 96000,
                "source_path": format!("lossless/40mP/{n:02}.flac"),
                "symlinked_into_artists": ["40mP"],
            })
        })
        .collect();
    let json = serde_json::json!({
        "version": 1,
        "built_at": "x",
        "source_root": lossless.to_str().unwrap(),
        "tracks": tracks,
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn focus_track_col(app: &mut App) {
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-defect-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

// ---------------------------------------------------------------------------
// DEF-001: Deleting a playlist requires no confirmation (Release Blocker)
// ---------------------------------------------------------------------------

#[test]
fn def001_d_opens_confirm_dialog_not_immediate_delete() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;
    let before = app.playlists.len();

    handle_key(&mut app, key('d'));
    // A confirmation dialog must appear, not immediate deletion.
    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "DEF-001: `d` should open a confirmation dialog, not delete immediately"
    );
    assert_eq!(
        app.playlists.len(),
        before,
        "DEF-001: playlist must still exist until user confirms"
    );
}

#[test]
fn def001_n_cancels_delete() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;
    let before = app.playlists.len();

    handle_key(&mut app, key('d'));
    handle_key(&mut app, key('n'));
    assert!(app.overlay.is_none(), "n should close the dialog");
    assert_eq!(app.playlists.len(), before, "playlist should still exist");
}

#[test]
fn def001_esc_cancels_delete() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;

    handle_key(&mut app, key('d'));
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "Esc should close the dialog");
    assert_eq!(app.playlists.len(), 1, "playlist should still exist");
}

#[test]
fn def001_y_confirms_delete() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;

    handle_key(&mut app, key('d'));
    handle_key(&mut app, key('y'));
    assert!(app.overlay.is_none(), "overlay should close after confirm");
    assert!(app.playlists.is_empty(), "playlist should be deleted");
}

#[test]
fn def001_enter_confirms_delete() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;

    handle_key(&mut app, key('d'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(app.playlists.is_empty(), "Enter should confirm deletion");
}

#[test]
fn def001_confirm_message_contains_playlist_name() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(Playlist {
        name: "My Vibes".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;

    handle_key(&mut app, key('d'));
    if let Some(Overlay::Confirm { message, .. }) = &app.overlay {
        assert!(
            message.contains("My Vibes"),
            "confirm message should include the playlist name, got: {message}"
        );
    } else {
        panic!("expected Confirm overlay");
    }
}

// ---------------------------------------------------------------------------
// DEF-010: Tab in command bar inserts `:` instead of completing
// ---------------------------------------------------------------------------

#[test]
fn def010_tab_does_not_double_colon() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open the command overlay.
    handle_key(&mut app, key(':'));
    // Type "yt".
    handle_key(&mut app, key('y'));
    handle_key(&mut app, key('t'));
    // Press Tab to complete.
    handle_key(&mut app, key_code(KeyCode::Tab));
    // The stored input should NOT have a `:` prefix — the renderer prepends
    // `:` for display. A `:` in the stored input would show as `::yt`.
    let (input, _cursor) = match &app.overlay {
        Some(Overlay::Command { input, cursor }) => (input.clone(), *cursor),
        _ => panic!("expected Command overlay to still be open"),
    };
    assert!(
        !input.starts_with(':'),
        "DEF-010: stored input should not have `:` prefix (renderer adds it), got: {input:?}"
    );
    assert_eq!(input, "yt", "Tab should complete 'yt' to 'yt'");
}

#[test]
fn def010_tab_completes_queue_prefix() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key(':'));
    handle_key(&mut app, key('q'));
    handle_key(&mut app, key_code(KeyCode::Tab));
    let input = match &app.overlay {
        Some(Overlay::Command { input, .. }) => input.clone(),
        _ => panic!("expected Command overlay"),
    };
    // "q" uniquely matches "q" (not "queue" — "q" is its own command).
    assert_eq!(input, "q", "Tab should complete 'q' to 'q' (no `:` prefix)");
}

#[test]
fn def010_tab_common_prefix_no_colon() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key(':'));
    // Type "q" — ambiguous: "q" and "queue" both start with "q"? Actually
    // "q" is an exact match, so Tab picks it. Let's test with "qu" which
    // matches only "queue".
    handle_key(&mut app, key('q'));
    handle_key(&mut app, key('u'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key_code(KeyCode::Tab));
    let input = match &app.overlay {
        Some(Overlay::Command { input, .. }) => input.clone(),
        _ => panic!("expected Command overlay"),
    };
    assert_eq!(
        input, "queue",
        "Tab should complete 'que' to 'queue' (no `:` prefix)"
    );
}

// ---------------------------------------------------------------------------
// DEF-014: Creating a playlist does not prompt for a name
// ---------------------------------------------------------------------------

#[test]
fn def014_new_playlist_opens_text_input() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    // No existing playlists → cursor 0 is "+ new playlist...".
    handle_key(&mut app, key_code(KeyCode::Enter));
    // DEF-014: a text input overlay should appear, not immediate creation.
    assert!(
        matches!(app.overlay, Some(Overlay::TextInput { .. })),
        "DEF-014: '+ new playlist...' should open a text input overlay"
    );
    assert!(
        app.playlists.is_empty(),
        "DEF-014: no playlist should be created until a name is entered"
    );
}

#[test]
fn def014_typed_name_becomes_playlist_name() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Type a custom name.
    for c in "Roadtrip".chars() {
        handle_key(&mut app, key(c));
    }
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Roadtrip");
    assert_eq!(app.playlists[0].track_ids, vec!["t1".to_string()]);
}

#[test]
fn def014_empty_name_falls_back_to_auto_name() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Press Enter immediately (empty name).
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.playlists.len(), 1);
    assert!(
        app.playlists[0].name.starts_with("New Playlist"),
        "empty name should fall back to auto-name, got: {}",
        app.playlists[0].name
    );
}

#[test]
fn def014_esc_cancels_text_input() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "Esc should cancel text input");
    assert!(app.playlists.is_empty(), "no playlist should be created");
}

#[test]
fn def014_backspace_edits_name() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Type "Jazz" then backspace to "Jaz".
    for c in "Jazz".chars() {
        handle_key(&mut app, key(c));
    }
    handle_key(&mut app, key_code(KeyCode::Backspace));
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Jaz");
}

// ---------------------------------------------------------------------------
// DEF-015: `:yt logout` clears credentials with no confirmation
// ---------------------------------------------------------------------------

#[test]
fn def015_yt_logout_opens_confirm_dialog() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open command, type "yt logout", Enter.
    handle_key(&mut app, key(':'));
    for c in "yt logout".chars() {
        handle_key(&mut app, key(c));
    }
    handle_key(&mut app, key_code(KeyCode::Enter));
    // DEF-015: a confirmation dialog should appear, not immediate logout.
    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "DEF-015: `:yt logout` should open a confirmation dialog"
    );
}

#[test]
fn def015_n_cancels_logout() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key(':'));
    for c in "yt logout".chars() {
        handle_key(&mut app, key(c));
    }
    handle_key(&mut app, key_code(KeyCode::Enter));
    handle_key(&mut app, key('n'));
    assert!(app.overlay.is_none(), "n should cancel logout");
    // The yt_status should NOT say "logged out".
    assert!(
        !app.yt_status
            .as_deref()
            .unwrap_or("")
            .contains("logged out"),
        "logout should not have executed"
    );
}

// ---------------------------------------------------------------------------
// DEF-018: Help `End` key does not scroll to bottom
// ---------------------------------------------------------------------------

#[test]
fn def018_end_key_scrolls_help_to_bottom() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open help overlay.
    handle_key(&mut app, key('?'));
    assert_eq!(app.help_scroll, 0, "help should start at top");
    // Press End — should jump to bottom.
    handle_key(&mut app, key_code(KeyCode::End));
    assert!(
        app.help_scroll > 0,
        "DEF-018: End should scroll to bottom, got help_scroll={}",
        app.help_scroll
    );
}

#[test]
fn def018_home_key_scrolls_help_to_top() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('?'));
    // Scroll down first.
    handle_key(&mut app, key_code(KeyCode::PageDown));
    assert!(app.help_scroll > 0, "should have scrolled down");
    // Home should jump back to top.
    handle_key(&mut app, key_code(KeyCode::Home));
    assert_eq!(app.help_scroll, 0, "Home should scroll to top");
}

// ---------------------------------------------------------------------------
// DEF-019: Help separator lines don't extend to right border at 100×30
// ---------------------------------------------------------------------------

#[test]
fn def019_separator_extends_full_width_at_100() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Help);
    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();

    let w = 100u16;
    let h = 30u16;
    // At 100 cols, popup = 90% = 90, inner = 88. Separator rows should have
    // ~88 `─` chars. The old bug had separators of 72 chars.
    let mut found_full_width_sep = false;
    for y in 0..h {
        let line: String = (0..w)
            .map(|x| {
                term.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        let dash_run = line.chars().filter(|c| *c == '\u{2500}').count();
        // Separator rows have >= 85 dashes at 100 width. Border rows have
        // fewer (~58) due to the title + corners.
        if dash_run >= 85 {
            found_full_width_sep = true;
        }
    }
    assert!(
        found_full_width_sep,
        "DEF-019: at 100 width, at least one separator row should have >=85 `─` chars (old bug: 72)"
    );
}

#[test]
fn def019_separator_extends_full_width_at_120() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Help);
    let backend = TestBackend::new(120, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();

    let w = 120u16;
    let h = 24u16;
    // At 120 cols, popup = 90% = 108, inner = 106. Separator rows should have
    // ~106 `─` chars. The top/bottom borders have fewer `─` (they include the
    // title + corners). We verify that at least one CONTENT row (not a border)
    // has >= 100 `─` chars — the old bug had 72.
    let mut found_full_width_sep = false;
    for y in 0..h {
        let line: String = (0..w)
            .map(|x| {
                term.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        let dash_run = line.chars().filter(|c| *c == '\u{2500}').count();
        // Separator rows (inside the popup, not borders) have >= 100 dashes
        // at 120 width. Border rows have ~72 (less due to title/corners).
        if dash_run >= 100 {
            found_full_width_sep = true;
        }
    }
    assert!(
        found_full_width_sep,
        "DEF-019: at 120 width, at least one separator row should have >=100 `─` chars (old bug: 72)"
    );
}

// ---------------------------------------------------------------------------
// DEF-021: `q` (quit) not documented in help overlay
// ---------------------------------------------------------------------------

#[test]
fn def021_q_documented_in_help() {
    // The help overlay text must include `q` as a keybinding for quit.
    // We render at a tall terminal (120×80) so all ~52 help lines are visible
    // without scrolling, then scan the buffer for the `q — quit` entry.
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Help);
    let backend = TestBackend::new(120, 80);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();

    let w = 120u16;
    let h = 80u16;
    let mut found_q = false;
    for y in 0..h {
        let line: String = (0..w)
            .map(|x| {
                term.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        let trimmed = line.trim();
        // The help entry for `q` renders as "│q               quit│" (inside
        // the popup borders). We match a line containing "quit" but not
        // "queue" (the `:queue clear` line contains "queue" but not "quit").
        if trimmed.contains("quit") && !trimmed.contains("queue") {
            found_q = true;
            break;
        }
    }
    assert!(found_q, "DEF-021: help overlay should document `q` as quit");
}
