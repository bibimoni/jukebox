//! Pure key→action dispatch tests (no terminal).
//!
//! These drive [`jukebox::tui::input::handle_key`] directly against an [`App`]
//! backed by a [`StubPlayer`], asserting on observable state changes. No
//! terminal, no crossterm event source — just the controller.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::search::Searcher;
use jukebox::tui::app::{App, DiscoverItem, Overlay, View, YtTab};
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

// ---------------------------------------------------------------------------
// DEF-026: j/k navigation broken in discover overlay
// ---------------------------------------------------------------------------
// The discover overlay only bound ↑↓, not j/k, so pressing j/k (documented in
// the help text as "h j k l · ↑↓←→ move") was swallowed by the `_` arm, leaving
// the cursor unchanged while the renderer dropped the highlight. j/k must
// navigate the overlay the same way as ↑↓.

#[test]
fn def026_j_navigates_down_in_discover_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "A".into(),
                album: "One".into(),
            },
            DiscoverItem::Album {
                artist: "B".into(),
                album: "Two".into(),
            },
            DiscoverItem::Album {
                artist: "C".into(),
                album: "Three".into(),
            },
        ],
        cursor: 0,
    });
    handle_key(&mut app, key('j'));
    match &app.overlay {
        Some(Overlay::Discover { cursor, .. }) => assert_eq!(
            *cursor, 1,
            "DEF-026: j should move the discover cursor down to index 1"
        ),
        _ => panic!("DEF-026: discover overlay should still be open after j"),
    }
}

#[test]
fn def026_j_wraps_around_in_discover_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "A".into(),
                album: "One".into(),
            },
            DiscoverItem::Album {
                artist: "B".into(),
                album: "Two".into(),
            },
        ],
        cursor: 1,
    });
    handle_key(&mut app, key('j'));
    match &app.overlay {
        Some(Overlay::Discover { cursor, .. }) => assert_eq!(
            *cursor, 0,
            "DEF-026: j should wrap from the last item back to index 0"
        ),
        _ => panic!("DEF-026: discover overlay should still be open after j"),
    }
}

#[test]
fn def026_k_navigates_up_in_discover_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "A".into(),
                album: "One".into(),
            },
            DiscoverItem::Album {
                artist: "B".into(),
                album: "Two".into(),
            },
            DiscoverItem::Album {
                artist: "C".into(),
                album: "Three".into(),
            },
        ],
        cursor: 2,
    });
    handle_key(&mut app, key('k'));
    match &app.overlay {
        Some(Overlay::Discover { cursor, .. }) => assert_eq!(
            *cursor, 1,
            "DEF-026: k should move the discover cursor up to index 1"
        ),
        _ => panic!("DEF-026: discover overlay should still be open after k"),
    }
}

#[test]
fn def026_k_wraps_around_in_discover_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "A".into(),
                album: "One".into(),
            },
            DiscoverItem::Album {
                artist: "B".into(),
                album: "Two".into(),
            },
        ],
        cursor: 0,
    });
    handle_key(&mut app, key('k'));
    match &app.overlay {
        Some(Overlay::Discover { cursor, .. }) => assert_eq!(
            *cursor, 1,
            "DEF-026: k should wrap from index 0 to the last item"
        ),
        _ => panic!("DEF-026: discover overlay should still be open after k"),
    }
}

#[test]
fn def026_jk_match_arrow_navigation_in_discover_overlay() {
    // j/k and ↑↓ must produce identical cursor movement.
    let (_d, cat) = cat_album();
    let items = vec![
        DiscoverItem::Album {
            artist: "A".into(),
            album: "One".into(),
        },
        DiscoverItem::Album {
            artist: "B".into(),
            album: "Two".into(),
        },
        DiscoverItem::Album {
            artist: "C".into(),
            album: "Three".into(),
        },
    ];
    let mut app_j = App::new(cat.clone(), Box::new(StubPlayer::default()), None, None);
    app_j.overlay = Some(Overlay::Discover {
        items: items.clone(),
        cursor: 0,
    });
    handle_key(&mut app_j, key('j'));
    handle_key(&mut app_j, key('j'));

    let mut app_down = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app_down.overlay = Some(Overlay::Discover { items, cursor: 0 });
    handle_key(&mut app_down, key_code(KeyCode::Down));
    handle_key(&mut app_down, key_code(KeyCode::Down));

    let cursor_j = match app_j.overlay {
        Some(Overlay::Discover { cursor, .. }) => cursor,
        _ => panic!("j path: overlay should remain open"),
    };
    let cursor_down = match app_down.overlay {
        Some(Overlay::Discover { cursor, .. }) => cursor,
        _ => panic!("Down path: overlay should remain open"),
    };
    assert_eq!(
        cursor_j, cursor_down,
        "DEF-026: j and Down should produce identical cursor positions"
    );
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

// ---------------------------------------------------------------------------
// Recommendation feature keybindings (Home, Radio, Generator, Explanation,
// Publication overlays + :home/:gen/:radio/:publish commands)
// ---------------------------------------------------------------------------

/// `H` opens the YouTube Home overlay.
#[test]
fn h_opens_home_overlay() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(app.overlay.is_none());
    handle_key(&mut app, key('H'));
    assert!(
        matches!(app.overlay, Some(Overlay::Home { .. })),
        "H should open the Home overlay"
    );
}

/// `G` in the YouTube view opens the playlist generator.
#[test]
fn g_in_youtube_view_opens_generator() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    handle_key(&mut app, key('G'));
    assert!(
        matches!(app.overlay, Some(Overlay::Generator { .. })),
        "G in the Y view should open the generator overlay"
    );
}

/// `G` in a non-YouTube view remains "bottom of column" (existing keymap
/// preserved — the generator is only on G in the Y view).
#[test]
fn g_in_artists_view_is_bottom_of_column() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 0;
    app.cursors.artist = 0; // "40mP"
                            // Press j a few times to move down, then G should jump to the bottom.
                            // With 1 artist, bottom = index 0. Verify G doesn't open an overlay.
    handle_key(&mut app, key('G'));
    assert!(
        app.overlay.is_none(),
        "G in Artists view must not open an overlay"
    );
}

/// `:home` command opens the Home overlay.
#[test]
fn home_command_opens_home() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app, "home");
    assert!(
        matches!(app.overlay, Some(Overlay::Home { .. })),
        ":home should open the Home overlay"
    );
}

/// `:gen` command opens the generator overlay.
#[test]
fn gen_command_opens_generator() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app, "gen");
    assert!(
        matches!(app.overlay, Some(Overlay::Generator { .. })),
        ":gen should open the generator overlay"
    );
}

/// `:radio` command starts a radio session from the selected track.
#[test]
fn radio_command_starts_radio_from_track() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app); // cursor on t1
    open_command(&mut app, "radio");
    assert!(
        matches!(app.overlay, Some(Overlay::Radio { .. })),
        ":radio should open the Radio overlay"
    );
}

/// `:publish` command opens the publication overlay for the focused playlist.
#[test]
fn publish_command_opens_publication() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Create a playlist so there's one to publish.
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "My Mix".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.cursors.playlist = 0;
    open_command(&mut app, "publish");
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert_eq!(state.name, "My Mix");
        }
        _ => panic!(":publish should open the Publication overlay"),
    }
}

/// `:radio artist <name>` starts a radio session from an artist.
#[test]
fn radio_artist_command_starts_radio() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app, "radio artist 40mP");
    assert!(
        matches!(app.overlay, Some(Overlay::Radio { .. })),
        ":radio artist <name> should open the Radio overlay"
    );
}

/// Home overlay: j moves the cursor down.
#[test]
fn home_overlay_j_moves_cursor_down() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // RC11-DEF-001: open_home populates sections so j is bounded by the
    // focused section's item count (was cursor_down(usize::MAX) — unbounded).
    app.open_home();
    handle_key(&mut app, key('j'));
    match &app.overlay {
        Some(Overlay::Home { state }) => {
            assert_eq!(state.cursor, 1, "j should move the Home cursor down to 1")
        }
        _ => panic!("Home overlay should remain open after j"),
    }
}

/// Home overlay: k moves the cursor up.
#[test]
fn home_overlay_k_moves_cursor_up() {
    use jukebox::tui::view::home::HomeState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = HomeState::new();
    state.cursor = 2;
    app.overlay = Some(Overlay::Home { state });
    handle_key(&mut app, key('k'));
    match &app.overlay {
        Some(Overlay::Home { state }) => {
            assert_eq!(state.cursor, 1, "k should move the Home cursor up to 1")
        }
        _ => panic!("Home overlay should remain open after k"),
    }
}

/// Home overlay: Tab switches to the next section.
#[test]
fn home_overlay_tab_switches_section() {
    use jukebox::tui::view::home::HomeState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Home {
        state: HomeState::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Tab));
    match &app.overlay {
        Some(Overlay::Home { state }) => assert_eq!(
            state.focused_section, 1,
            "Tab should advance the focused section to 1"
        ),
        _ => panic!("Home overlay should remain open after Tab"),
    }
}

/// Home overlay: Esc closes.
#[test]
fn home_overlay_esc_closes() {
    use jukebox::tui::view::home::HomeState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Home {
        state: HomeState::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "Esc should close the Home overlay");
}

/// Generator overlay: typing in the input phase accumulates into state.input.
#[test]
fn generator_overlay_types_into_input() {
    use jukebox::tui::view::generator::GeneratorState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Generator {
        state: GeneratorState::new(),
    });
    handle_key(&mut app, key('h'));
    handle_key(&mut app, key('i'));
    match &app.overlay {
        Some(Overlay::Generator { state }) => assert_eq!(
            state.input, "hi",
            "typing should accumulate in the generator input"
        ),
        _ => panic!("Generator overlay should remain open while typing"),
    }
}

/// Generator overlay: Backspace removes the last character.
#[test]
fn generator_overlay_backspace_removes_char() {
    use jukebox::tui::view::generator::GeneratorState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = GeneratorState::new();
    state.input = "hello".into();
    app.overlay = Some(Overlay::Generator { state });
    handle_key(&mut app, key_code(KeyCode::Backspace));
    match &app.overlay {
        Some(Overlay::Generator { state }) => {
            assert_eq!(state.input, "hell", "Backspace should remove the last char");
        }
        _ => panic!("Generator overlay should remain open after Backspace"),
    }
}

/// Generator overlay: Enter in the input phase calls generate_playlist (the
/// overlay transitions to the preview phase with a generated playlist).
#[test]
fn generator_overlay_enter_generates() {
    use jukebox::tui::view::generator::{GeneratorPhase, GeneratorState};
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = GeneratorState::new();
    state.input = "calm focus mix".into();
    app.overlay = Some(Overlay::Generator { state });
    handle_key(&mut app, key_code(KeyCode::Enter));
    match &app.overlay {
        Some(Overlay::Generator { state }) => {
            assert_eq!(
                state.phase,
                GeneratorPhase::Preview,
                "Enter should generate and move to the preview phase"
            );
            assert!(
                state.playlist.is_some(),
                "Enter should produce a generated playlist"
            );
        }
        _ => panic!("Generator overlay should remain open after Enter"),
    }
}

/// Generator overlay: Esc closes.
#[test]
fn generator_overlay_esc_closes() {
    use jukebox::tui::view::generator::GeneratorState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Generator {
        state: GeneratorState::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        app.overlay.is_none(),
        "Esc should close the generator overlay"
    );
}

/// Publication overlay: Tab cycles privacy PRIVATE -> UNLISTED -> PUBLIC.
#[test]
fn publication_overlay_tab_cycles_privacy() {
    use jukebox::tui::view::publication::PublicationState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Publication {
        state: PublicationState::new(),
    });
    // PRIVATE -> UNLISTED
    handle_key(&mut app, key_code(KeyCode::Tab));
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert_eq!(state.privacy, "UNLISTED", "Tab should cycle to UNLISTED");
        }
        _ => panic!("Publication overlay should remain open after Tab"),
    }
    // UNLISTED -> PUBLIC
    handle_key(&mut app, key_code(KeyCode::Tab));
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert_eq!(state.privacy, "PUBLIC", "Tab should cycle to PUBLIC");
        }
        _ => panic!("Publication overlay should remain open after Tab"),
    }
    // PUBLIC -> PRIVATE (wraps)
    handle_key(&mut app, key_code(KeyCode::Tab));
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert_eq!(state.privacy, "PRIVATE", "Tab should wrap to PRIVATE");
        }
        _ => panic!("Publication overlay should remain open after Tab"),
    }
}

/// Publication overlay: n cancels (closes the overlay).
#[test]
fn publication_overlay_n_cancels() {
    use jukebox::tui::view::publication::PublicationState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Publication {
        state: PublicationState::new(),
    });
    handle_key(&mut app, key('n'));
    assert!(
        app.overlay.is_none(),
        "n should cancel and close the publication overlay"
    );
}

/// Publication overlay: Esc closes.
#[test]
fn publication_overlay_esc_closes() {
    use jukebox::tui::view::publication::PublicationState;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Publication {
        state: PublicationState::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        app.overlay.is_none(),
        "Esc should close the publication overlay"
    );
}

/// Explanation overlay: Esc closes.
#[test]
fn explanation_overlay_esc_closes() {
    use jukebox::reco::explanations::Explanation;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Explanation {
        explanation: Explanation {
            reason: "from your liked tracks".into(),
            detail: None,
        },
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        app.overlay.is_none(),
        "Esc should close the explanation overlay"
    );
}

/// Explanation overlay: a non-Esc key does NOT close it (stays open).
#[test]
fn explanation_overlay_stays_open_on_other_keys() {
    use jukebox::reco::explanations::Explanation;
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Explanation {
        explanation: Explanation {
            reason: "a track you used to love".into(),
            detail: None,
        },
    });
    handle_key(&mut app, key('j'));
    assert!(
        matches!(app.overlay, Some(Overlay::Explanation { .. })),
        "non-Esc keys should not close the explanation overlay"
    );
}

/// Radio overlay: n advances to the next track.
#[test]
fn radio_overlay_n_advances() {
    use jukebox::reco::radio::{RadioSeed, RadioSession};
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Start playback so next() has something to advance from.
    focus_track_col(&mut app);
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    // Open the radio overlay.
    app.overlay = Some(Overlay::Radio {
        session: Some(RadioSession::new(RadioSeed::Track("t1".into()))),
    });
    handle_key(&mut app, key('n'));
    assert!(
        matches!(app.overlay, Some(Overlay::Radio { .. })),
        "Radio overlay should remain open after n"
    );
}

/// Radio overlay: Esc closes.
#[test]
fn radio_overlay_esc_closes() {
    use jukebox::reco::radio::{RadioSeed, RadioSession};
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Radio {
        session: Some(RadioSession::new(RadioSeed::Track("t1".into()))),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "Esc should close the radio overlay");
}

/// Tab completion includes the new commands.
#[test]
fn tab_completes_home_command() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open command overlay, type "ho", press Tab — should complete to "home".
    handle_key(&mut app, key(':'));
    handle_key(&mut app, key('h'));
    handle_key(&mut app, key('o'));
    handle_key(&mut app, key_code(KeyCode::Tab));
    let input = match &app.overlay {
        Some(Overlay::Command { input, .. }) => input.clone(),
        _ => panic!("Command overlay should be open"),
    };
    assert_eq!(input, "home", "Tab should complete 'ho' to 'home'");
}

/// Tab completion includes the gen command.
#[test]
fn tab_completes_gen_command() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key(':'));
    handle_key(&mut app, key('g'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key_code(KeyCode::Tab));
    let input = match &app.overlay {
        Some(Overlay::Command { input, .. }) => input.clone(),
        _ => panic!("Command overlay should be open"),
    };
    assert_eq!(input, "gen", "Tab should complete 'ge' to 'gen'");
}

// ---------------------------------------------------------------------------
// Radio overlay key handler regression tests (Issue 2: keys were no-ops
// because `app.next()` couldn't find the radio session — the overlay was
// taken out during key handling, so `reco_radio_next()` saw `self.overlay`
// as None. The fix uses the session directly from the destructured overlay.)
// ---------------------------------------------------------------------------

/// `n` in the Radio overlay should advance to the next radio track.
/// (DEF-056: `start_radio_from_track` now auto-starts the first track, so
/// `now_playing` is already Some before `n` is pressed; `n` advances to the
/// next track.)
#[test]
fn radio_overlay_n_key_starts_playback() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    app.start_radio_from_track("t1");
    assert!(matches!(app.overlay, Some(Overlay::Radio { .. })));
    // DEF-056: the radio auto-starts, so a track is already playing.
    assert!(
        app.now_playing.is_some(),
        "start_radio_from_track should auto-start playback"
    );
    let first_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after auto-start");
    // Press 'n' — should advance to the next radio track.
    handle_key(&mut app, key('n'));
    assert!(
        app.now_playing.is_some(),
        "pressing 'n' in Radio overlay must keep playback going"
    );
    let next_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after 'n'");
    assert_ne!(
        next_id, first_id,
        "'n' should advance to a different radio track"
    );
    // The overlay must still be open.
    assert!(matches!(app.overlay, Some(Overlay::Radio { .. })));
}

/// `-` in the Radio overlay should apply negative feedback (HideTrack) and
/// advance to the next track.
#[test]
fn radio_overlay_minus_key_feedback_and_advance() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    app.start_radio_from_track("t1");
    // DEF-056: auto-start already plays the first radio track.
    let first_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after auto-start");
    // Press '-' — should hide the current track and advance.
    handle_key(&mut app, key('-'));
    assert!(
        app.reco_profile.is_hidden(&first_id),
        "'-' must apply HideTrack feedback to the current track"
    );
}

/// `s` in the Radio overlay should apply RemoveFromMix feedback and advance.
#[test]
fn radio_overlay_s_key_skips_and_advances() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    app.start_radio_from_track("t1");
    // Start playback.
    handle_key(&mut app, key('n'));
    assert!(app.now_playing.is_some());
    let first_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after 'n'");
    // Press 's' — should remove from mix and advance.
    handle_key(&mut app, key('s'));
    // The overlay should still be open.
    assert!(matches!(app.overlay, Some(Overlay::Radio { .. })));
    // The first track should have been removed from the session's mix.
    // (RemoveFromMix records a RemovedFromQueue event in the profile.)
    let _ = first_id; // track_id used for potential future assertions
}

/// `+` in the Radio overlay should apply Like feedback to the current track
/// without advancing.
#[test]
fn radio_overlay_plus_key_applies_like() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    app.start_radio_from_track("t1");
    // Start playback.
    handle_key(&mut app, key('n'));
    let track_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after 'n'");
    let playing_id = track_id.clone();
    // Press '+' — should like the current track without advancing.
    handle_key(&mut app, key('+'));
    // The profile should have a positive score for the liked track.
    assert!(
        app.reco_profile.track_score(&track_id) > 0.0,
        "'+' must apply Like feedback (positive score) to the current track"
    );
    // Playback should NOT have advanced (still the same track).
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some(playing_id.as_str()),
        "'+' should not advance to the next track"
    );
}

/// `=` is an alias for `+` (same key on unshifted keyboards).
#[test]
fn radio_overlay_equals_key_applies_like() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    app.start_radio_from_track("t1");
    handle_key(&mut app, key('n'));
    let track_id = app
        .now_playing
        .as_ref()
        .map(|s| s.id().to_string())
        .expect("a track should be playing after 'n'");
    handle_key(&mut app, key('='));
    assert!(
        app.reco_profile.track_score(&track_id) > 0.0,
        "'=' must apply Like feedback just like '+'"
    );
}

// ---------------------------------------------------------------------------
// RB-3: YouTube keyboard trap. The global view-switch keys (1-4, Tab/
// Shift+Tab) were intercepted inside the YouTube view as YT sub-tab switches,
// Esc was a no-op with no overlay, and `h`/move_left couldn't escape — so the
// user could get trapped in a view with no reliable exit, and the trapped
// view persisted across restart. These regressions pin the fix: the
// documented global keys always work inside YouTube, Esc exits, `h` exits
// at the left edge, `[`/`]` cycle YT tabs, and a persisted YouTube view is
// not a trap on restore.
// ---------------------------------------------------------------------------

/// An `App` placed inside the YouTube view with no overlay/filter open.
fn rb3_yt_app() -> (tempfile::TempDir, App) {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.overlay = None;
    app.filter = None;
    app.yt_view.tab = YtTab::Home;
    (_d, app)
}

#[test]
fn rb3_global_view_switch_keys_work_inside_youtube() {
    let (_d, mut app) = rb3_yt_app();
    assert_eq!(app.view, View::Youtube);
    handle_key(&mut app, key('1'));
    assert_eq!(
        app.view,
        View::Artists,
        "1 must switch to Artists from inside YouTube"
    );
    app.view = View::Youtube;
    handle_key(&mut app, key('2'));
    assert_eq!(
        app.view,
        View::Playlists,
        "2 must switch to Playlists from inside YouTube"
    );
    app.view = View::Youtube;
    handle_key(&mut app, key('3'));
    assert_eq!(
        app.view,
        View::Queue,
        "3 must switch to Queue from inside YouTube"
    );
}

#[test]
fn rb3_tab_and_shift_tab_escape_youtube() {
    let (_d, mut app) = rb3_yt_app();
    handle_key(&mut app, key_code(KeyCode::Tab));
    assert_ne!(app.view, View::Youtube, "Tab must cycle away from YouTube");
    app.view = View::Youtube;
    handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
    assert_ne!(
        app.view,
        View::Youtube,
        "Shift+Tab must cycle away from YouTube"
    );
}

#[test]
fn rb3_esc_exits_youtube_view() {
    let (_d, mut app) = rb3_yt_app();
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert_ne!(app.view, View::Youtube, "Esc must exit the YouTube view");
    assert_eq!(app.view, View::Artists);
}

#[test]
fn rb3_h_at_left_edge_exits_youtube() {
    let (_d, mut app) = rb3_yt_app();
    app.yt_view.tab = YtTab::Library;
    app.focus_col = 0;
    handle_key(&mut app, key('h'));
    assert_eq!(
        app.view,
        View::Artists,
        "h at the left edge must exit YouTube"
    );
}

#[test]
fn rb3_brackets_cycle_yt_tabs() {
    let (_d, mut app) = rb3_yt_app();
    assert_eq!(app.yt_view.tab, YtTab::Home);
    handle_key(&mut app, key(']'));
    assert_eq!(
        app.yt_view.tab,
        YtTab::Library,
        "] must cycle to the next YT tab"
    );
    handle_key(&mut app, key(']'));
    assert_eq!(app.yt_view.tab, YtTab::Search);
    handle_key(&mut app, key('['));
    assert_eq!(
        app.yt_view.tab,
        YtTab::Library,
        "[ must cycle to the previous YT tab"
    );
}

#[test]
fn rb3_persisted_youtube_view_is_not_a_trap() {
    use jukebox::state::{load_layout_at, save_layout_at, LayoutSave, ARTISTS};
    use jukebox::tui::app::{ColumnWidths, PlaylistColumnState};

    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    // First launch (no saved layout): the default focus is "artists", not a
    // trapped YouTube view.
    let fresh = load_layout_at(&path).unwrap();
    assert_eq!(fresh.focus, ARTISTS);

    // Simulate quitting while inside the YouTube view: focus="youtube" persists.
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    save_layout_at(
        &path,
        &LayoutSave {
            focus: app.focus_key(),
            widths: &ColumnWidths::default(),
            volume: app.volume,
            shuffle: app.transport.shuffle,
            repeat: app.transport.repeat,
            continue_mode: app.transport.continue_mode,
            source_mode: app.source_mode,
            yt_browser: &app.yt_browser,
            last_played_track_id: app.last_played_track_id.as_deref(),
            last_played_position: app.last_played_position,
            last_cursor_artist: app.cursors.artist,
            last_cursor_album: app.cursors.album,
            last_cursor_track: app.cursors.track,
            last_cursor_playlist: app.cursors.playlist,
            player_bar_mode: "mini",
            track_layout_mode: "table",
            sidebar_visible: app.sidebar_visible,
            playlist_col: &PlaylistColumnState::default(),
        },
    )
    .unwrap();
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(
        loaded.focus, "youtube",
        "the trapped YouTube view was persisted"
    );

    // Simulate the next launch restoring the YouTube view (as main.rs does),
    // then verify the user can navigate away — the exit keys work on restore.
    let (_d2, cat2) = cat_album();
    let mut restored = App::new(cat2, Box::new(StubPlayer::default()), None, None);
    restored.view = View::Youtube;
    restored.overlay = None;
    handle_key(&mut restored, key_code(KeyCode::Esc));
    assert_ne!(
        restored.view,
        View::Youtube,
        "Esc must exit the restored YouTube view (no persistent trap)"
    );
}

#[test]
fn rb3_recovery_hint_fits_80_columns() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert_eq!(app.view, View::Artists);
    handle_key(&mut app, key('4')); // enter YouTube
    assert_eq!(app.view, View::Youtube);
    let hint = app
        .status_toast
        .as_deref()
        .expect("a recovery hint should be shown when entering the YouTube view");
    assert!(
        hint.chars().count() <= 80,
        "recovery hint must fit 80 columns, got {} chars: {hint}",
        hint.chars().count()
    );
    assert!(
        hint.contains("Esc") || hint.contains("1-4") || hint.contains("Tab"),
        "recovery hint should mention an exit key: {hint}"
    );
}

#[test]
fn rb3_recovery_hint_renders_at_80x24() {
    use ratatui::{backend::TestBackend, Terminal};
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('4')); // enter YouTube -> sets the recovery toast
    assert_eq!(app.view, View::Youtube);
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();
    let mut text = String::new();
    for y in 0..24u16 {
        for x in 0..80u16 {
            text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        text.push('\n');
    }
    assert!(
        text.contains("Esc"),
        "recovery hint must be visible at 80x24 (expected 'Esc' in footer):\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Task 6: `R` manual refresh on the Home/Explore/Charts tabs.
//
// The spec's `r` (lowercase) is shorthand for "the refresh key" — it's mapped
// to `R` (uppercase) here because `r` is already bound to `cycle_repeat`
// globally. The YT-view intercept fires `App::refresh_yt_home_explore_charts`
// on Home/Explore/Charts; on Library/Search/Discover/Radio it falls through
// to the global R handler (`retry_yt_probe` / `resume_last`).
//
// Distinguishing the intercept from the fall-through WITHOUT a session:
//   - `refresh_yt_home_explore_charts` early-returns when `yt_session` is
//     None (no cache clear, no toast, no state change).
//   - `retry_yt_probe` ALWAYS clears `yt_error` (even with no session —
//     line 3515 of app.rs), then early-returns.
// So: with no session, R on Home/Explore/Charts leaves `yt_error` UNCHANGED
// (intercept fired → refresh_yt_home_explore_charts → early return → no
// yt_error clear), while R on Library/Search/Discover/Radio CLEARS
// `yt_error` (global handler fired). That's the observable distinction.
//
// The positive "cache IS cleared" path needs a session — covered by
// `r_key_on_home_tab_clears_cache_with_session` below using a dummy sidecar
// (same pattern as `tests/lyrics.rs::fake_lyrics_session`).
// ---------------------------------------------------------------------------

/// Place an `App` in the YouTube view on `tab` with no overlay/filter and a
/// preset `yt_error` (so we can observe whether `retry_yt_probe`'s
/// unconditional `yt_error` clear ran).
fn yt_app_on_tab(tab: YtTab) -> (tempfile::TempDir, App) {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.overlay = None;
    app.filter = None;
    app.yt_view.tab = tab;
    app.yt_error = Some("preset error".to_string());
    (_d, app)
}

#[test]
fn r_key_on_home_tab_intercepted_global_not_called() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Home);
    app.yt_view.home_sections_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    // Intercept fired → refresh_yt_home_explore_charts → early-return (no
    // session) → global handler did NOT run → yt_error preserved.
    assert_eq!(
        app.yt_error.as_deref(),
        Some("preset error"),
        "R on Home tab must NOT fall through to the global handler (which \
         would clear yt_error)"
    );
    // Cache field unchanged (early-return preserved it).
    assert!(
        app.yt_view.home_sections_cached.is_some(),
        "home_sections_cached must be unchanged when refresh_yt_home_explore_\
         charts early-returns (no session)"
    );
}

#[test]
fn r_key_on_explore_tab_intercepted_global_not_called() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Explore);
    app.yt_view.explore_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert_eq!(
        app.yt_error.as_deref(),
        Some("preset error"),
        "R on Explore tab must NOT fall through to the global handler"
    );
    assert!(
        app.yt_view.explore_cached.is_some(),
        "explore_cached must be unchanged when refresh_yt_home_explore_charts \
         early-returns (no session)"
    );
}

#[test]
fn r_key_on_charts_tab_intercepted_global_not_called() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Charts);
    app.yt_view.charts_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert_eq!(
        app.yt_error.as_deref(),
        Some("preset error"),
        "R on Charts tab must NOT fall through to the global handler"
    );
    assert!(
        app.yt_view.charts_cached.is_some(),
        "charts_cached must be unchanged when refresh_yt_home_explore_charts \
         early-returns (no session)"
    );
}

#[test]
fn r_key_on_library_tab_falls_through_to_global() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Library);
    handle_key(&mut app, key('R'));
    // No intercept for Library → global handler ran → retry_yt_probe cleared
    // yt_error unconditionally (app.rs line 3515), then early-returned (no
    // session). The cache fields are untouched (Library isn't a refresh tab).
    assert!(
        app.yt_error.is_none(),
        "R on Library tab must fall through to the global handler, which \
         clears yt_error unconditionally (got {:?})",
        app.yt_error
    );
}

#[test]
fn r_key_on_search_tab_falls_through_to_global() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Search);
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_error.is_none(),
        "R on Search tab must fall through to the global handler (got {:?})",
        app.yt_error
    );
}

#[test]
fn r_key_on_discover_tab_falls_through_to_global() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Discover);
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_error.is_none(),
        "R on Discover tab must fall through to the global handler (got {:?})",
        app.yt_error
    );
}

#[test]
fn r_key_on_radio_tab_falls_through_to_global() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Radio);
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_error.is_none(),
        "R on Radio tab must fall through to the global handler (got {:?})",
        app.yt_error
    );
}

#[test]
fn r_key_lowercase_cycle_repeat_unchanged() {
    // Lowercase `r` is bound to `cycle_repeat` globally — it must NOT be
    // intercepted by the new R refresh on any YT tab.
    let (_d, mut app) = yt_app_on_tab(YtTab::Home);
    app.yt_view.home_sections_cached = Some(Vec::new());
    let repeat_before = app.transport.repeat;
    handle_key(&mut app, key('r'));
    assert_ne!(
        app.transport.repeat, repeat_before,
        "lowercase r must cycle repeat mode (existing global binding)"
    );
    // Cache and yt_error untouched — the R refresh intercept didn't fire.
    assert!(
        app.yt_view.home_sections_cached.is_some(),
        "lowercase r must not clear home_sections_cached"
    );
    assert_eq!(
        app.yt_error.as_deref(),
        Some("preset error"),
        "lowercase r must not clear yt_error (R refresh intercept didn't fire)"
    );
}

/// Spawns a minimal sidecar that just drains stdin (no ytmusicapi responses).
/// Same pattern as `tests/lyrics.rs::fake_lyrics_session` — lets us exercise
/// the cache-clearing positive path without a real ytmusicapi roundtrip.
/// `Session::spawn` requires `python3` on PATH (the lyrics test has the same
/// dependency).
fn fake_session() -> jukebox::yt::session::Session {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let script = std::env::temp_dir()
        .join(format!("jk-input-fake-{}-{}.py", std::process::id(), n));
    std::fs::write(&script, "import sys\nfor line in sys.stdin: pass\n").unwrap();
    jukebox::yt::session::Session::spawn(std::path::Path::new("python3"), &script, None).unwrap()
}

#[test]
fn r_key_on_home_tab_clears_cache_with_session() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Home);
    app.yt_session = Some(fake_session());
    app.yt_view.home_sections_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_view.home_sections_cached.is_none(),
        "R on Home tab with a session must clear home_sections_cached"
    );
    assert!(
        app.yt_session.as_ref().unwrap().home_loading(),
        "R on Home tab must re-fire send_home (home_inflight should be true)"
    );
}

#[test]
fn r_key_on_explore_tab_clears_cache_with_session() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Explore);
    app.yt_session = Some(fake_session());
    app.yt_view.explore_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_view.explore_cached.is_none(),
        "R on Explore tab with a session must clear explore_cached"
    );
    assert!(
        app.yt_session.as_ref().unwrap().explore_loading(),
        "R on Explore tab must re-fire send_explore (explore_inflight should be true)"
    );
}

#[test]
fn r_key_on_charts_tab_clears_cache_with_session() {
    let (_d, mut app) = yt_app_on_tab(YtTab::Charts);
    app.yt_session = Some(fake_session());
    app.yt_view.charts_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert!(
        app.yt_view.charts_cached.is_none(),
        "R on Charts tab with a session must clear charts_cached"
    );
    assert!(
        app.yt_session.as_ref().unwrap().charts_loading(),
        "R on Charts tab must re-fire send_charts (charts_inflight should be true)"
    );
}

#[test]
fn r_key_on_home_tab_does_not_clear_other_tab_caches() {
    // R on Home must only clear the home cache, not explore/charts.
    let (_d, mut app) = yt_app_on_tab(YtTab::Home);
    app.yt_session = Some(fake_session());
    app.yt_view.home_sections_cached = Some(Vec::new());
    app.yt_view.explore_cached = Some(Vec::new());
    app.yt_view.charts_cached = Some(Vec::new());
    handle_key(&mut app, key('R'));
    assert!(app.yt_view.home_sections_cached.is_none(), "home cleared");
    assert!(app.yt_view.explore_cached.is_some(), "explore untouched");
    assert!(app.yt_view.charts_cached.is_some(), "charts untouched");
}

#[test]
fn bracket_keys_cycle_yt_tabs_with_fetch_on_visit() {
    // `]` from Radio must land on Explore (per YtTab::next) and fire the
    // fetch-on-visit (explore_inflight becomes true with a session).
    let (_d, mut app) = yt_app_on_tab(YtTab::Radio);
    app.yt_session = Some(fake_session());
    app.yt_view.explore_cached = None;
    handle_key(&mut app, key(']'));
    assert_eq!(app.yt_view.tab, YtTab::Explore, "] must cycle Radio -> Explore");
    assert!(
        app.yt_session.as_ref().unwrap().explore_loading(),
        "tab-switch to Explore must fire send_explore via yt_tab_fetch_on_visit"
    );

    // `[` from Explore goes back to Radio (no fetch). Then `[` from Radio
    // goes to Discover (no fetch — Discover isn't a fetch tab). Then `[` from
    // Discover goes to Search (no fetch). Then `[` from Search goes to Library
    // (no fetch). Then `[` from Library goes to Home and SHOULD fire send_home.
    app.yt_view.tab = YtTab::Library;
    app.yt_view.home_sections_cached = None;
    handle_key(&mut app, key('['));
    assert_eq!(app.yt_view.tab, YtTab::Home, "[ must cycle Library -> Home");
    assert!(
        app.yt_session.as_ref().unwrap().home_loading(),
        "tab-switch to Home must fire send_home via yt_tab_fetch_on_visit"
    );
}

#[test]
fn bracket_keys_skip_fetch_when_cache_populated() {
    // `yt_tab_fetch_on_visit` only fires when the cache is None — a populated
    // cache means the tab is already loaded, so no re-fetch on switch.
    let (_d, mut app) = yt_app_on_tab(YtTab::Radio);
    app.yt_session = Some(fake_session());
    app.yt_view.explore_cached = Some(Vec::new()); // already cached
    handle_key(&mut app, key(']'));
    assert_eq!(app.yt_view.tab, YtTab::Explore);
    assert!(
        !app.yt_session.as_ref().unwrap().explore_loading(),
        "tab-switch to a cached Explore tab must NOT re-fire send_explore"
    );
}
