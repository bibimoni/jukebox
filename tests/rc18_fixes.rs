//! RC18 regression tests for the 7 defects fixed in this batch.
//!
//! - D1: font mode cache (is_ascii) is process-stable — covered by the
//!   existing `def029_*` / `def025_*` suites after they were updated to
//!   call `reset_font_mode_cache`. This file adds a direct same-env /
//!   same-result check.
//! - D4: big bar flags line appends `SRC youtube` when a YT track plays
//!   under MODE=local.
//! - D5: `play_selected()` preserves `focus_col` and `view`, and `j` still
//!   moves the track cursor after Enter.
//! - D6: narrow (non-table) track rows show `Title — Artist` (not
//!   `Title — Album`) — covered by the updated insta snapshots; this
//!   file adds a direct string assertion.
//! - D7: radio / publication overlays clear the full screen so the sidebar
//!   does not bleed through.
//! - D8: `:gen <prompt>` opens the generator with the NL prompt pre-filled.
//! - D14: the big "Now Playing" bar shows the resume hint when stopped with
//!   a saved last-played track.

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::source::{RemoteTrack, StreamFormat, TrackSource};
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, Terminal};
use std::io::Write;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn two_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("A").join("02.flac"), b"x").unwrap();
    // Both tracks share the same album "Adele" so the focused album has 2
    // tracks — `j` on the Tracks column must advance between them.
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["Ada"],"primary_artist":"Ada","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ada"]},
          {"id":"t2","artists":["Ada"],"primary_artist":"Ada","title":"Night Tales","album":"Adele","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/02.flac","symlinked_into_artists":["Ada"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// Spawn a minimal Session backed by a no-op python sidecar so `track_cache`
/// is accessible for render tests that resolve YouTube track metadata.
/// Mirrors the helper in tests/fixer_d.rs.
fn spawn_minimal_session() -> jukebox::yt::session::Session {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("jk-rc18-sidecar-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(b"import sys\nfor line in sys.stdin:\n    pass\n")
        .unwrap();
    let session = jukebox::yt::session::Session::spawn(std::path::Path::new("python3"), &p, None)
        .expect("spawn minimal sidecar");
    let _ = std::fs::remove_file(&p);
    session
}

fn rendered_full(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let mut buf = String::new();
    for y in 0..h {
        for x in 0..w {
            let c = &term.backend().buffer()[(x, y)];
            buf.push(c.symbol().chars().next().unwrap_or(' '));
        }
        buf.push('\n');
    }
    buf
}

/// Render only the big player bar (used for D4 / D14 assertions that target
/// the big bar's contents, not the surrounding chrome).
fn rendered_big_bar(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    let area = ratatui::layout::Rect::new(0, 0, w, h);
    term.draw(|f| jukebox::tui::view::player_bar_big::render_big(f, area, app))
        .unwrap();
    let mut buf = String::new();
    for y in 0..h {
        for x in 0..w {
            let c = &term.backend().buffer()[(x, y)];
            buf.push(c.symbol().chars().next().unwrap_or(' '));
        }
        buf.push('\n');
    }
    buf
}

/// Env-lock so tests that touch `JUKEBOX_FONT_MODE` don't race each other.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    jukebox::tui::view::theme::reset_font_mode_cache();
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// D1: font mode cache is deterministic for a given env
// ---------------------------------------------------------------------------

/// RC18-D1: `is_ascii()` must return the same value across calls in the same
/// process when the env is unchanged. The previous path re-read env vars on
/// every call, which combined with a sloppy PTY driver looked "flaky between
/// launches". The thread-local cache freezes the result.
#[test]
fn rc18_d1_is_ascii_is_stable_across_calls() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    let a = jukebox::tui::view::theme::is_ascii();
    let b = jukebox::tui::view::theme::is_ascii();
    let c = jukebox::tui::view::theme::is_ascii();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    assert!(!a, "unicode env → is_ascii() false");
    assert_eq!(a, b, "D1: is_ascii() must be stable across calls");
    assert_eq!(b, c, "D1: is_ascii() must be stable across calls");
}

/// RC18-D1: when `JUKEBOX_FONT_MODE=ascii`, `is_ascii()` returns true and
/// stays true. After reset + unset, it returns false again.
#[test]
fn rc18_d1_is_ascii_reflects_env_after_reset() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    assert!(
        jukebox::tui::view::theme::is_ascii(),
        "D1: JUKEBOX_FONT_MODE=ascii → is_ascii() true"
    );
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // Without reset the cached ascii value would persist; with reset the
    // next read re-reads the (now unset) env and returns Unicode.
    jukebox::tui::view::theme::reset_font_mode_cache();
    assert!(
        !jukebox::tui::view::theme::is_ascii(),
        "D1: after reset + unset env → is_ascii() false"
    );
}

// ---------------------------------------------------------------------------
// D4: big bar SRC badge
// ---------------------------------------------------------------------------

/// RC18-D4: when a YouTube track is playing while `source_mode=Local`, the
/// big bar's flags line must append `SRC youtube` so the flags don't
/// contradict the actual playing source. Mirrors the mini bar's DEF-013 fix.
#[test]
fn rc18_d4_big_bar_shows_src_youtube_when_yt_track_plays_in_local_mode() {
    let (_d, cat) = two_track_cat();
    let session = spawn_minimal_session();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    // Force a YouTube now-playing track under Local mode.
    app.now_playing = Some(TrackSource::Remote {
        video_id: "v001".to_string(),
    });
    // Seed the YT track cache so now_playing_view resolves a title.
    app.yt_session.as_mut().unwrap().track_cache.insert(
        "v001".to_string(),
        RemoteTrack {
            video_id: "v001".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            album: Some("Hurry Up".into()),
            dur: None,
            fmt: Some(StreamFormat {
                codec: "AAC".into(),
                abr: 256,
                sample_rate: 48000,
                container: "m4a".into(),
                premium: true,
            }),
            isrc: None,
        },
    );
    app.source_mode = jukebox::mode::SourceMode::Local;
    let bar = rendered_big_bar(&app, 100, 10);
    assert!(
        bar.contains("PREF local"),
        "D4: big bar must still show PREF local: {bar}"
    );
    assert!(
        bar.contains("SRC youtube"),
        "D4: big bar must append SRC youtube when a YT track plays under PREF local: {bar}"
    );
}

/// RC18-D4: when the playing source matches the mode, no SRC badge is shown
/// (the existing behavior — no regression).
#[test]
fn rc18_d4_big_bar_no_src_badge_when_sources_match() {
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_big_bar(&app, 100, 10);
    assert!(bar.contains("PREF local"), "D4: PREF local shown: {bar}");
    assert!(
        !bar.contains("SRC"),
        "D4: no SRC badge when playing source matches mode: {bar}"
    );
}

// ---------------------------------------------------------------------------
// D5: focus after play
// ---------------------------------------------------------------------------

/// RC18-D5: `play_selected()` must NOT modify `focus_col` or `view`. The
/// user stays where they were so `j`/`e` keep working on the same column.
#[test]
fn rc18_d5_play_selected_preserves_focus_col_and_view() {
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Land on the Artists view, Tracks column (focus_col=2).
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    app.play_selected();
    assert_eq!(
        app.view,
        View::Artists,
        "D5: play_selected must not change view"
    );
    assert_eq!(
        app.focus_col, 2,
        "D5: play_selected must not change focus_col"
    );
}

/// RC18-D5: after `Enter` (play) on a track, `j` must still advance the track
/// cursor. The report showed `j` no longer moving the cursor after play;
/// this pins the expected behavior.
#[test]
fn rc18_d5_j_moves_track_cursor_after_play() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Artists view, Tracks column (focus_col=2), cursor on track 1.
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    // Enter → play track 1.
    handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.now_playing.as_ref().map(|t| t.id()),
        Some("t1"),
        "D5: Enter should play track t1"
    );
    assert_eq!(app.focus_col, 2, "D5: focus stays on Tracks column");
    // j → move cursor to track 2.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.cursors.track, 1,
        "D5: j must move the track cursor to track 2 after play"
    );
}

// ---------------------------------------------------------------------------
// D6: narrow track rows show Title — Artist
// ---------------------------------------------------------------------------

/// RC18-D6: the narrow (non-table) track row must show `Title — Artist`
/// (matching the player bar / search / generator surfaces), not
/// `Title — Album`. We render the Artists view at a width where the Tracks
/// column is in non-table mode and assert the artist name appears next to
/// the title.
#[test]
fn rc18_d6_narrow_track_row_shows_title_artist_not_album() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0; // Ada
    app.cursors.album = 0; // Adele
    app.cursors.track = 0; // Freedom by Ada
                           // 70×24: narrow path, single-column render. The track row appears in the
                           // focused pane. The artist "Ada" must appear (was "Adele" before the fix
                           // because the old format was Title — Album).
    let buf = rendered_full(&mut app, 70, 24);
    assert!(
        buf.contains("Freedom"),
        "D6: track title must render: {buf}"
    );
    assert!(
        buf.contains("Ada"),
        "D6: artist 'Ada' must appear next to title (not album 'Adele'): {buf}"
    );
}

// ---------------------------------------------------------------------------
// D7: overlays clear full screen so sidebar doesn't bleed
// ---------------------------------------------------------------------------

/// RC18-D7: the radio overlay must clear the full screen so the sidebar
/// rail + Miller columns don't bleed through around the popup. We render
/// with the sidebar visible + a Radio overlay and assert the sidebar's
/// characteristic `VIEWS` header is NOT visible (it would be if the overlay
/// only cleared its own popup rect).
#[test]
fn rc18_d7_radio_overlay_covers_sidebar() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.sidebar_visible = true;
    // Force a width where the sidebar is visible (>= 100 cols) and tall
    // enough for the full chrome. 120×40.
    app.overlay = Some(Overlay::Radio { session: None });
    let buf = rendered_full(&mut app, 120, 40);
    // The sidebar's "VIEWS" section header must NOT be visible because the
    // overlay clears the full screen. (Before the fix, only the popup rect
    // was cleared, so the sidebar showed through on the left edge.)
    assert!(
        !buf.contains("VIEWS"),
        "D7: radio overlay must clear the full screen so the sidebar 'VIEWS' header doesn't bleed through: {buf}"
    );
}

/// RC18-D7: same check for the publication overlay.
#[test]
fn rc18_d7_publication_overlay_covers_sidebar() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.sidebar_visible = true;
    app.overlay = Some(Overlay::Publication {
        state: jukebox::tui::view::publication::PublicationState::default(),
    });
    let buf = rendered_full(&mut app, 120, 40);
    assert!(
        !buf.contains("VIEWS"),
        "D7: publication overlay must clear the full screen so the sidebar doesn't bleed: {buf}"
    );
}

// ---------------------------------------------------------------------------
// D8: `:gen <prompt>` opens the generator with the prompt pre-filled
// ---------------------------------------------------------------------------

/// RC18-D8: `:gen chill vibes` must open the generator overlay with the NL
/// input pre-filled, mirroring `:publish <name>`. The user can immediately
/// press Enter to parse, or edit the prompt.
#[test]
fn rc18_d8_gen_with_prompt_prefills_generator_input() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open the `:` command overlay.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE),
    );
    // Type "gen chill vibes for focusing".
    for c in "gen chill vibes for focusing".chars() {
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    // Enter → dispatches `:gen chill vibes for focusing`.
    handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let overlay = app
        .overlay
        .as_ref()
        .expect("D8: :gen <prompt> must open the generator overlay");
    match overlay {
        Overlay::Generator { state } => {
            assert_eq!(
                state.input, "chill vibes for focusing",
                "D8: generator input must be pre-filled with the prompt (got {:?})",
                state.input
            );
            assert_eq!(
                state.cursor,
                state.input.len(),
                "D8: cursor must sit at the end of the pre-filled prompt"
            );
        }
        other => panic!("D8: expected Generator overlay, got {other:?}"),
    }
}

/// RC18-D8: `:gen` with no argument still opens the generator with empty input
/// (the previous behavior — no regression).
#[test]
fn rc18_d8_gen_no_arg_opens_empty_generator() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE),
    );
    for c in "gen".chars() {
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match app.overlay.as_ref().unwrap() {
        Overlay::Generator { state } => {
            assert!(
                state.input.is_empty(),
                "D8: :gen with no arg must open with empty input (got {:?})",
                state.input
            );
        }
        other => panic!("D8: expected Generator overlay, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// D14: big bar shows the resume hint
// ---------------------------------------------------------------------------

/// RC18-D14: when stopped with a saved last-played track, the big "Now
/// Playing" bar must show the resume hint (`▸ resume: {title} at {M:SS} · R
/// to resume`). The mini bar already rendered it; the big bar didn't, so
/// users with `big_pref=true` (persisted) never saw the offer and `R`
/// looked broken.
#[test]
fn rc18_d14_big_bar_shows_resume_hint_when_stopped() {
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Simulate a launch-restore: last-played = t2 at 5.0s, nothing playing.
    app.last_played_track_id = Some("t2".into());
    app.last_played_position = 5.0;
    app.now_playing = None;
    app.resume_hint = Some("resume: Night Tales at 0:05 · R to resume".into());
    let bar = rendered_big_bar(&app, 100, 10);
    assert!(
        bar.contains("resume:"),
        "D14: big bar must show the resume hint when stopped with a saved track: {bar}"
    );
    assert!(
        bar.contains("R to resume") || bar.contains("R to"),
        "D14: big bar resume hint must tell the user to press R: {bar}"
    );
}

/// RC18-D14: when a track is playing, the big bar must NOT show the resume
/// hint (the offer is consumed). The now-playing title takes the row.
#[test]
fn rc18_d14_big_bar_no_resume_hint_when_playing() {
    let (_d, cat) = two_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app.resume_hint = Some("stale hint that should not render".into());
    let bar = rendered_big_bar(&app, 100, 10);
    assert!(
        !bar.contains("stale hint"),
        "D14: big bar must not show a stale resume hint while playing: {bar}"
    );
    assert!(
        bar.contains("Freedom"),
        "D14: big bar must show the now-playing title while playing: {bar}"
    );
}
