//! Regression tests for Fixer F defects (Batch F — Lyrics).
//!
//! - F.1 RC11-DEF-009: Lyrics overlay swallows `>` / `<` (global next/prev).
//! - F.2 RC11-DEF-046: Local tracks "not found" with no explanation.
//! - F.3 RC11-DEF-058: No lyrics loading indicator (re-verify — confirmed
//!   the Loading state renders "Loading lyrics…" for the async sidecar path).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, LyricsState, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, Terminal};

// --- Test catalog: 3 local tracks, no sidecar lyrics -----------------------

fn three_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Artist")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("Artist").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3)
        .map(|n| {
            serde_json::json!({
                "id": format!("t{n}"),
                "artists": ["Artist"],
                "primary_artist": "Artist",
                "title": format!("Song{n}"),
                "album": "Album",
                "track_number": n,
                "bit_depth": 16,
                "sample_rate_hz": 44100,
                "source_path": format!("lossless/Artist/{n:02}.flac"),
                "symlinked_into_artists": ["Artist"]
            })
        })
        .collect();
    let json = serde_json::json!({
        "version": 1,
        "built_at": "x",
        "source_root": lossless.to_str().unwrap(),
        "tracks": tracks
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn render_overlay(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    let mut rendered = String::new();
    for y in 0..height {
        for x in 0..width {
            rendered.push(
                terminal.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        rendered.push('\n');
    }
    rendered
}

// --- F.1 RC11-DEF-009: `>` / `<` pass through lyrics overlay ----------------

#[test]
fn def009_gt_advances_track_and_refetches_lyrics() {
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0; // Song1 (t1)
    app.play_selected();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));

    // Open lyrics overlay (local track with no sidecar → NotFound).
    app.toggle_lyrics();
    assert!(matches!(app.overlay, Some(Overlay::Lyrics { .. })));
    let before_gen = app.lyrics_gen;

    // Press `>` while lyrics overlay is open.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE),
    );

    // Track advanced to t2.
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t2"),
        "`>` must advance to the next track while lyrics overlay is open"
    );
    // Lyrics were re-requested for the new track (gen bumped).
    assert_eq!(
        app.lyrics_gen,
        before_gen.wrapping_add(1),
        "lyrics_gen must bump after `>` re-fetch"
    );
    // Overlay stayed open and track_id updated to the new track.
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics { ref track_id, .. }) if track_id == "t2"
        ),
        "overlay must stay open with track_id == t2"
    );
}

#[test]
fn def009_lt_advances_track_and_refetches_lyrics() {
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0; // Song1 (t1)
    app.play_selected();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));

    // Advance to t2 first so prev() has history to go back to t1.
    app.next();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));

    app.toggle_lyrics();
    let before_gen = app.lyrics_gen;

    // Press `<` while lyrics overlay is open → back to t1.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE),
    );

    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t1"),
        "`<` must go to the previous track while lyrics overlay is open"
    );
    assert_eq!(app.lyrics_gen, before_gen.wrapping_add(1));
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics { ref track_id, .. }) if track_id == "t1"
        ),
        "overlay must stay open with track_id == t1"
    );
}

#[test]
fn def009_gt_keeps_overlay_open() {
    // The overlay must NOT close on `>` — it stays open and shows Loading
    // for the new track. This is the core of DEF-009: previously `>` was
    // swallowed by `_ => {}` and the track never advanced.
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    app.play_selected();
    app.toggle_lyrics();

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE),
    );
    assert!(
        app.overlay.is_some(),
        "lyrics overlay must stay open after `>`"
    );
}

// --- F.2 RC11-DEF-046: Local track NotFound shows explanation ---------------

#[test]
fn def046_local_track_not_found_shows_youtube_explanation() {
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Local track with no embedded/sidecar lyrics → NotFound.
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::NotFound,
        scroll: 0,
        track_id: "t1".into(),
        gen: app.lyrics_gen,
    });

    let rendered = render_overlay(&mut app, 100, 30);
    assert!(
        rendered.contains("YouTube"),
        "local track NotFound must mention YouTube: {rendered}"
    );
    assert!(
        rendered.contains("local tracks"),
        "local track NotFound must explain local tracks: {rendered}"
    );
}

#[test]
fn def046_youtube_track_not_found_shows_generic_message() {
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // YouTube track (not in catalog) → generic NotFound message.
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::NotFound,
        scroll: 0,
        track_id: "ytvid123".into(),
        gen: app.lyrics_gen,
    });

    let rendered = render_overlay(&mut app, 100, 30);
    assert!(
        rendered.contains("No lyrics found"),
        "YouTube track NotFound must show generic message: {rendered}"
    );
    // The local-specific message must NOT appear for a YouTube track.
    assert!(
        !rendered.contains("local tracks without a YouTube match"),
        "YouTube track must not show local-track explanation: {rendered}"
    );
}

// --- F.3 RC11-DEF-058: Loading indicator confirmed -------------------------

#[test]
fn def058_loading_state_renders_indicator() {
    // The Loading state is set by request_lyrics before the sidecar responds.
    // This test confirms the "Loading lyrics…" indicator renders. The defect
    // was "Unconfirmed — fixture too fast"; the loading state exists in the
    // code path (request_lyrics sets Loading, then fires the async sidecar;
    // on_tick drains the response and transitions to Available/NotFound).
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::Loading,
        scroll: 0,
        track_id: "t1".into(),
        gen: app.lyrics_gen,
    });

    let rendered = render_overlay(&mut app, 80, 24);
    assert!(
        rendered.contains("Loading lyrics"),
        "Loading state must show 'Loading lyrics' indicator: {rendered}"
    );
}

#[test]
fn def058_request_lyrics_sets_loading_before_async_fetch() {
    // Verify request_lyrics transitions the overlay to Loading for a track
    // that has no local source (the async sidecar path). We use a non-catalog
    // id (simulating a YouTube video_id) with no session — the function sets
    // Loading first, then Error (no session). The Loading state is the
    // indicator the user sees while the sidecar responds (when a session
    // exists). Here we verify the Loading transition happens at all.
    let (_d, cat) = three_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::Idle,
        scroll: 0,
        track_id: "old-track".into(),
        gen: app.lyrics_gen,
    });

    // A local track (t1) with no sidecar → request_lyrics sets Loading and
    // queues the disk read for on_tick (so the user sees a Loading transition
    // before the truthful NotFound). We verify the gen bumps (the request fired)
    // and that Loading is visible before on_tick resolves to NotFound.
    let before_gen = app.lyrics_gen;
    app.request_lyrics("t1");
    assert_eq!(
        app.lyrics_gen,
        before_gen.wrapping_add(1),
        "request_lyrics must bump lyrics_gen"
    );
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics {
                state: LyricsState::Loading,
                ref track_id,
                ..
            }) if track_id == "t1"
        ),
        "local track lyrics must show a Loading transition before resolving"
    );
    // on_tick processes the deferred local read → NotFound (no lyrics for this fixture).
    app.on_tick();
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics {
                state: LyricsState::NotFound,
                ref track_id,
                ..
            }) if track_id == "t1"
        ),
        "local track with no sidecar must end in NotFound after on_tick"
    );
}
