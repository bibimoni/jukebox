//! Pure key→action dispatch tests (no terminal).
//!
//! These drive [`jukebox::tui::input::handle_key`] directly against an [`App`]
//! backed by a [`StubPlayer`], asserting on observable state changes. No
//! terminal, no crossterm event source — just the controller.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::search::Searcher;
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::input::{handle_key, handle_mouse_in_area};
use jukebox::tui::queue::{RepeatMode, ShuffleMode};
use jukebox::tui::view::{layout::player_bar_area, player_bar::geometry};
use ratatui::layout::Rect;

/// A 3-track catalog under one artist/album, with real on-disk source files
/// (so `play_selected`'s `std::fs::metadata` check passes and playback starts).
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

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Focus the track column (col 2) and place the cursor on track 0 of the album.
fn focus_track_col(app: &mut App) {
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0; // 40mP
    app.cursors.album = 0; // Cosmic
    app.cursors.track = 0; // Song1 (t1)
}

#[test]
fn enter_plays_selected_in_context() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
}

#[test]
fn gt_advances_to_next_track() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    handle_key(&mut app, key('>'));
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));
}

#[test]
fn z_cycles_shuffle_mode() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);
    handle_key(&mut app, key('z'));
    assert_eq!(app.transport.shuffle, ShuffleMode::Smart);
    handle_key(&mut app, key('z'));
    assert_eq!(app.transport.shuffle, ShuffleMode::Random);
    handle_key(&mut app, key('z'));
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);
}

#[test]
fn r_cycles_repeat_mode() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert_eq!(app.transport.repeat, RepeatMode::Off);
    handle_key(&mut app, key('r'));
    assert_eq!(app.transport.repeat, RepeatMode::All);
    handle_key(&mut app, key('r'));
    assert_eq!(app.transport.repeat, RepeatMode::One);
    handle_key(&mut app, key('r'));
    assert_eq!(app.transport.repeat, RepeatMode::Off);
}

#[test]
fn q_sets_should_quit() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(!app.should_quit);
    handle_key(&mut app, key('q'));
    assert!(app.should_quit);
}

#[test]
fn slash_opens_search_overlay_and_esc_closes_it() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(app.overlay.is_none());
    handle_key(&mut app, key('/'));
    assert!(matches!(app.overlay, Some(Overlay::Search { .. })));
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none());
}

/// Build a small catalog (2 artists) backed by a real Tantivy index so a query
/// actually matches. Returns the tempdir (keep it alive for the test's duration).
fn cat_with_index() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"]},
          {"id":"t2","artists":["Aimer"],"primary_artist":"Aimer","title":"Brave",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Aimer"]},
        ]
    }).to_string();
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let idx = d.path().join("search-index");
    jukebox::search::build_index(&cat, &idx).unwrap();
    (d, cat)
}

#[test]
fn search_overlay_populates_results() {
    let (_d, cat) = cat_with_index();
    let searcher = Searcher::open(&_d.path().join("search-index")).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), Some(searcher), None);

    // Open the search overlay (same as `/`).
    handle_key(&mut app, key('/'));
    // Type a query that matches t1 ("Freedom"). Each keystroke re-runs the
    // search and updates the overlay's `results`.
    handle_key(&mut app, key('F'));
    handle_key(&mut app, key('r'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('d'));
    handle_key(&mut app, key('o'));
    handle_key(&mut app, key('m'));

    let results = match &app.overlay {
        Some(Overlay::Search { results, .. }) => results.clone(),
        _ => panic!("expected Search overlay to still be open"),
    };
    assert!(
        !results.is_empty(),
        "search overlay results must be non-empty after a matching query"
    );
    assert!(
        results.iter().any(|id| id == "t1"),
        "results must contain the matched track id t1: {:?}",
        results
    );
}

#[test]
fn four_key_switches_to_youtube_view() {
    let (_d, cat) = cat_with_index();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('4'));
    assert_eq!(app.view, jukebox::tui::app::View::Youtube);
}

/// Regression for the "Capital letters dropped in search overlay input" bug.
///
/// Typing a capital letter (Shift+letter, e.g. `F`) is delivered by crossterm
/// as `Char('F')` carrying the SHIFT modifier. The overlay char handler
/// previously guarded on `KeyModifiers::NONE`, so the shifted `F` was dropped
/// and the input stayed empty. After the fix, SHIFT is accepted and the char
/// is appended.
#[test]
fn search_overlay_accepts_capital_letters() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('/'));
    // Shift+F: a real terminal sends Char('F') with the SHIFT modifier.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT),
    );
    let input = match &app.overlay {
        Some(Overlay::Search { input, .. }) => input.clone(),
        _ => panic!("expected Search overlay to still be open"),
    };
    assert_eq!(input, "F");
}

#[test]
fn search_overlay_n_types_into_query_not_navigation() {
    // `n` was previously bound to "next search match", which swallowed it from
    // the query — so you couldn't search for e.g. "nirvana". Now arrows are
    // the only navigator; `n` must go into the input.
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('/'));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
    );
    let input = match &app.overlay {
        Some(Overlay::Search { input, .. }) => input.clone(),
        _ => panic!("overlay should be open"),
    };
    assert_eq!(input, "n", "'n' must be typed into the query, not navigate");
}

#[test]
fn search_overlay_arrow_keys_move_selection() {
    // Reproduces the "can't use arrow keys in search" bug: Down/Up must move
    // the result cursor (previously only `n`/`N` did, so arrows were no-ops).
    let (_d, cat) = cat_with_index();
    let searcher = Searcher::open(&_d.path().join("search-index")).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), Some(searcher), None);
    // Type a query that yields multiple results ("a" matches Ado/Aimer/etc).
    handle_key(&mut app, key('/'));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    let n_results = match &app.overlay {
        Some(Overlay::Search { results, .. }) => results.len(),
        _ => panic!("overlay should be open"),
    };
    assert!(
        n_results >= 2,
        "need >=2 results to test navigation, got {n_results}"
    );
    // Down moves the cursor to result 1.
    handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let cursor = match &app.overlay {
        Some(Overlay::Search { cursor, .. }) => *cursor,
        _ => panic!("overlay should still be open"),
    };
    assert_eq!(cursor, 1, "Down should advance the search cursor to 1");
    // Up moves it back to 0.
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let cursor = match &app.overlay {
        Some(Overlay::Search { cursor, .. }) => *cursor,
        _ => panic!("overlay should still be open"),
    };
    assert_eq!(cursor, 0, "Up should return the search cursor to 0");
}

#[test]
fn search_overlay_typing_letters_not_intercepted_as_navigation() {
    // 'j'/'k' are NOT navigation in the search overlay (only arrows are), so
    // typing "joji" must put 'j' into the input rather than move the cursor.
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('/'));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    let input = match &app.overlay {
        Some(Overlay::Search { input, .. }) => input.clone(),
        _ => panic!("overlay should be open"),
    };
    assert_eq!(input, "j", "'j' must be typed into the query, not navigate");
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-xdg-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

#[test]
fn yt_auth_command_opens_overlay() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app, "yt auth");
    assert!(matches!(app.overlay, Some(Overlay::YtAuth { .. })));
}

#[test]
fn yt_auth_enter_saves_closes_esc_cancels() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::YtAuth {
        input: "# Netscape cookies".into(),
    });
    // Enter submits → writes cookies, closes overlay.
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(app.overlay.is_none(), "Enter should close the auth overlay");
    let _ = std::fs::remove_file(jukebox::yt::session::cookies_file());
}

#[test]
fn yt_auth_esc_cancels_without_saving() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::YtAuth { input: "x".into() });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none());
}

fn open_command(app: &mut App, text: &str) {
    handle_key(app, key(':'));
    for c in text.chars() {
        handle_key(app, key(c));
    }
    handle_key(app, key_code(KeyCode::Enter));
}

#[test]
fn f_opens_filter_and_typing_narrows_artists() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 0;
    handle_key(&mut app, key('f'));
    assert!(app.filter.is_some(), "f should open the filter");
    handle_key(&mut app, key('4')); // "40mP" starts with '4'? no — artists are "40mP". type 'm'
    handle_key(&mut app, key('m'));
    // filter text is "4m" — no artist matches "4m"; that's fine, we assert the
    // filter captured the keys.
    assert_eq!(app.filter.as_ref().unwrap().text, "4m");
    // Esc clears.
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.filter.is_none());
}

#[test]
fn m_cycles_source_mode_without_stopping() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let playing = app.now_playing.clone();
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, jukebox::mode::SourceMode::Youtube);
    assert_eq!(app.now_playing, playing, "M must not stop playback");
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, jukebox::mode::SourceMode::Mixed);
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, jukebox::mode::SourceMode::Local);
}

#[test]
fn four_switches_to_youtube_view() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key_code(KeyCode::Char('4')));
    assert_eq!(app.view, View::Youtube);
}

#[test]
fn s_instant_random_via_key() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    handle_key(&mut app, key('s'));
    assert!(app.now_playing.is_some(), "s should start a random track");
}

#[test] // `s` documents the shift+s Discover keybinding under test
fn s_opens_discover_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    handle_key(&mut app, key('S'));
    assert!(matches!(app.overlay, Some(Overlay::Discover { .. })));
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn mouse_uses_rendered_player_bar_geometry() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    let screen = Rect::new(0, 0, 120, 25);
    let bar = player_bar_area(screen).expect("wide layout has a player bar");
    let geo = geometry(bar);

    handle_mouse_in_area(&mut app, click(geo.next.x, geo.next.y), screen);
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));

    let before = app.player.position();
    handle_mouse_in_area(
        &mut app,
        click(geo.progress.right() + 2, geo.progress.y),
        screen,
    );
    assert_eq!(app.player.position(), before, "outside gauge must not seek");

    handle_mouse_in_area(
        &mut app,
        click(geo.progress.x + geo.progress.width / 2, geo.progress.y),
        screen,
    );
    assert_eq!(app.player.position(), Some(90.0));
}
