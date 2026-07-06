//! Pure key→action dispatch tests (no terminal).
//!
//! These drive [`jukebox::tui::input::handle_key`] directly against an [`App`]
//! backed by a [`StubPlayer`], asserting on observable state changes. No
//! terminal, no crossterm event source — just the controller.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::search::Searcher;
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::queue::{RepeatMode, ShuffleMode};

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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.now_playing.as_deref(), Some("t1"));
}

#[test]
fn gt_advances_to_next_track() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.now_playing.as_deref(), Some("t1"));
    handle_key(&mut app, key('>'));
    assert_eq!(app.now_playing.as_deref(), Some("t2"));
}

#[test]
fn z_cycles_shuffle_mode() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    assert!(!app.should_quit);
    handle_key(&mut app, key('q'));
    assert!(app.should_quit);
}

#[test]
fn slash_opens_search_overlay_and_esc_closes_it() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), Some(searcher));

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
    assert!(!results.is_empty(), "search overlay results must be non-empty after a matching query");
    assert!(results.iter().any(|id| id == "t1"),
        "results must contain the matched track id t1: {:?}", results);
}

#[test]
fn four_key_opens_search_overlay() {
    let (_d, cat) = cat_with_index();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    assert!(app.overlay.is_none());
    handle_key(&mut app, key('4'));
    assert!(matches!(app.overlay, Some(Overlay::Search { .. })));
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
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
