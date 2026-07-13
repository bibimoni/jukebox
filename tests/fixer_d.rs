//! Regression tests for Fixer D defects (MOD-4, MOD-5).
//!
//! - MOD-4: YouTube tracks panel empty at 80x24 — the narrow render path
//!   shows only the playlist-list pane when focus_col=0, so tracks of the
//!   selected playlist are invisible until the user drills in with `l`.
//! - MOD-5: No-color discover overlay has no selection indicator — the
//!   discover overlay used fg(hi_fg).bg(accent) for the selected row, which
//!   under NO_COLOR carries no REVERSED/BOLD modifier, leaving the selected
//!   item with no non-color cue (the main views use selected_style()).

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::source::RemoteTrack;
use jukebox::tui::app::{App, DiscoverItem, Overlay, View, YtList, YtListKind};
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, style::Modifier, Terminal};
use std::io::Write;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn one_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":"Local Song","album":"Al",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac",
        "symlinked_into_artists":["A"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// Render `layout::draw` (full TUI) and return the buffer string + terminal
/// so the caller can inspect cell styles.
fn rendered_draw(app: &mut App, w: u16, h: u16) -> (String, Terminal<TestBackend>) {
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
    (buf, term)
}

/// Find the first (x, y) of a substring in the rendered buffer, scanning
/// row-major. Returns None if not found.
fn find_substr(term: &Terminal<TestBackend>, w: u16, h: u16, needle: &str) -> Option<(u16, u16)> {
    let chars: Vec<char> = needle.chars().collect();
    for y in 0..h {
        for x in 0..w {
            let mut ok = true;
            for (i, c) in chars.iter().enumerate() {
                let xx = x as usize + i;
                if xx >= w as usize {
                    ok = false;
                    break;
                }
                let cell = &term.backend().buffer()[(xx as u16, y)];
                if cell.symbol().chars().next().unwrap_or(' ') != *c {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some((x, y));
            }
        }
    }
    None
}

/// True if the cell at (x, y) carries the REVERSED modifier (the non-color
/// selection cue used by selected_style() under NO_COLOR).
fn cell_has_reversed(term: &Terminal<TestBackend>, x: u16, y: u16) -> bool {
    let cell = &term.backend().buffer()[(x, y)];
    cell.modifier.contains(Modifier::REVERSED)
}

/// Spawn a minimal Session backed by a no-op python sidecar so `track_cache`
/// is accessible for render tests that resolve YouTube track metadata.
fn spawn_minimal_session() -> jukebox::yt::session::Session {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("jk-fixD-sidecar-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(b"import sys\nfor line in sys.stdin:\n    pass\n")
        .unwrap();
    let session = jukebox::yt::session::Session::spawn(std::path::Path::new("python3"), &p, None)
        .expect("spawn minimal sidecar");
    let _ = std::fs::remove_file(&p);
    session
}

// ---------------------------------------------------------------------------
// MOD-4: YouTube tracks panel empty at 80x24
// ---------------------------------------------------------------------------

#[test]
fn mod4_youtube_tracks_visible_at_80x24_when_playlist_selected() {
    // At 80x24 the narrow render path is used (width < MIN_WIDTH=100). The
    // YouTube focus_col=0 branch must show a preview of the selected list's
    // tracks below the list (mirroring the Artists narrow path's album
    // preview), so tracks are visible without drilling in with `l`.
    let (_d, cat) = one_track_cat();
    let session = spawn_minimal_session();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::Ready;
    // Cache a YouTube track so its title resolves in yt_track_rows.
    app.yt_session.as_mut().unwrap().track_cache.insert(
        "v001".to_string(),
        RemoteTrack {
            video_id: "v001".into(),
            title: "DiscoTrack".into(),
            artist: "DiscoArtist".into(),
            album: None,
            dur: None,
            fmt: None,
            isrc: None,
        },
    );
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "Liked Songs".into(),
        kind: YtListKind::Account,
        track_ids: vec!["v001".into()],
    }];
    app.cursors.playlist = 0;
    app.focus_col = 0; // list pane focused — tracks pane is NOT drilled into
    let (buf, _term) = rendered_draw(&mut app, 80, 24);
    assert!(
        buf.contains("DiscoTrack"),
        "MOD-4: YouTube tracks must be visible at 80x24 when a playlist is selected \
         (focus_col=0): {buf}"
    );
}

#[test]
fn mod4_youtube_tracks_preview_visible_when_no_metadata_at_80x24() {
    // Even when track metadata isn't cached yet, the preview must show the
    // "Loading…" placeholder (not be empty) so the user knows tracks exist.
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::Ready;
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "Liked Songs".into(),
        kind: YtListKind::Account,
        track_ids: vec!["vidXYZ".into()],
    }];
    app.cursors.playlist = 0;
    app.focus_col = 0;
    let (buf, _term) = rendered_draw(&mut app, 80, 24);
    assert!(
        buf.contains("Loading"),
        "MOD-4: YouTube track preview must show 'Loading' placeholder at 80x24 \
         when a playlist with tracks is selected: {buf}"
    );
}

#[test]
fn mod4_youtube_list_still_visible_at_80x24_with_track_preview() {
    // The track preview must not displace the playlist list — the selected
    // list name must still render alongside the track preview.
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::Ready;
    app.yt_lists = vec![
        YtList {
            id: "PL1".into(),
            name: "Liked Songs".into(),
            kind: YtListKind::Account,
            track_ids: vec!["vidXYZ".into()],
        },
        YtList {
            id: "RD1".into(),
            name: "Focus Flow".into(),
            kind: YtListKind::Suggested,
            track_ids: vec![],
        },
    ];
    app.cursors.playlist = 0;
    app.focus_col = 0;
    let (buf, _term) = rendered_draw(&mut app, 80, 24);
    assert!(
        buf.contains("Liked Songs"),
        "MOD-4: playlist list must still render at 80x24 with track preview: {buf}"
    );
    assert!(
        buf.contains("Loading"),
        "MOD-4: track preview must render at 80x24: {buf}"
    );
}

// ---------------------------------------------------------------------------
// MOD-5: No-color discover overlay has no selection indicator
// ---------------------------------------------------------------------------

/// Serializes tests that set/unset NO_COLOR so they don't interfere with each
/// other under parallel test execution. Acquired via `env_lock()` which
/// recovers from a poisoned mutex (a prior test's assertion panic) so an
/// unrelated failure doesn't cascade.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn mod5_discover_overlay_selected_item_has_reversed_under_no_color() {
    let _guard = env_lock();
    std::env::set_var("NO_COLOR", "1");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "Artist1".into(),
                album: "Album1".into(),
            },
            DiscoverItem::Album {
                artist: "Artist2".into(),
                album: "Album2".into(),
            },
        ],
        cursor: 0,
    });
    let (buf, term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("NO_COLOR");
    let pos = find_substr(&term, 100, 30, "Album1")
        .unwrap_or_else(|| panic!("Album1 not rendered: {buf}"));
    assert!(
        cell_has_reversed(&term, pos.0, pos.1),
        "MOD-5: selected discover item 'Album1' must carry REVERSED modifier \
         under NO_COLOR (consistent with selected_style()): {buf}"
    );
    let pos2 = find_substr(&term, 100, 30, "Album2")
        .unwrap_or_else(|| panic!("Album2 not rendered: {buf}"));
    assert!(
        !cell_has_reversed(&term, pos2.0, pos2.1),
        "MOD-5: unselected discover item 'Album2' must NOT carry REVERSED: {buf}"
    );
}

#[test]
fn mod5_discover_overlay_selected_item_has_bold_in_color_mode() {
    // In color mode selected_style() adds BOLD; the old discover style did
    // not. This catches the inconsistency without touching env vars.
    let _guard = env_lock();
    std::env::remove_var("NO_COLOR");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "Artist1".into(),
                album: "Album1".into(),
            },
            DiscoverItem::Album {
                artist: "Artist2".into(),
                album: "Album2".into(),
            },
        ],
        cursor: 0,
    });
    let (buf, term) = rendered_draw(&mut app, 100, 30);
    let pos = find_substr(&term, 100, 30, "Album1")
        .unwrap_or_else(|| panic!("Album1 not rendered: {buf}"));
    let cell = &term.backend().buffer()[(pos.0, pos.1)];
    assert!(
        cell.modifier.contains(Modifier::BOLD),
        "MOD-5: selected discover item must carry BOLD modifier (selected_style \
         adds BOLD in color mode): {buf}"
    );
    let pos2 = find_substr(&term, 100, 30, "Album2")
        .unwrap_or_else(|| panic!("Album2 not rendered: {buf}"));
    let cell2 = &term.backend().buffer()[(pos2.0, pos2.1)];
    assert!(
        !cell2.modifier.contains(Modifier::BOLD),
        "MOD-5: unselected discover item must NOT carry BOLD: {buf}"
    );
}

// ---------------------------------------------------------------------------
// RC11 Batch D — Playback + player bar + pause/restore
// ---------------------------------------------------------------------------

/// Two-track catalog for resume/progress tests (t1 short, t2 short, same album).
fn two_track_cat_d() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("A").join("02.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["Ada"],"primary_artist":"Ada","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ada"]},
          {"id":"t2","artists":["Bop"],"primary_artist":"Bop","title":"Night Tales","album":"Beep","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/02.flac","symlinked_into_artists":["Bop"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// RC11-DEF-041: after stopping (now_playing = None), the progress bar must
/// reset to `0% --:-- / --:--` — not keep the last track's `33% 0:02 / 0:06`.
/// StubPlayer keeps pos/dur after `stop()`, so without the `now_playing.is_none()`
/// guard in `progress()` the stale position bleeds through.
#[test]
fn def041_progress_resets_to_zero_when_stopped() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    // While playing: progress is non-zero (StubPlayer reports pos advancing).
    app.player.seek(2.0).unwrap();
    app.on_tick();
    let (buf, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        !buf.contains("--:-- / --:--") || buf.contains("[PLAYING]"),
        "DEF-041: while playing, progress should show a time (not all --:--): {buf}"
    );
    // Stop: now_playing cleared → progress must reset to 0% / --:--.
    app.player.stop().unwrap();
    app.now_playing = None;
    let (buf, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf.contains("--:-- / --:--"),
        "DEF-041: stopped bar must show --:-- / --:-- (not stale position): {buf}"
    );
    assert!(
        buf.contains("0%"),
        "DEF-041: stopped bar must show 0%: {buf}"
    );
}

/// RC11-DEF-022: when muted, the player bar must show a "MUTED" text label
/// alongside the volume bar so mute isn't ambiguous with volume-0.
#[test]
fn def022_muted_label_visible_when_muted() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app.volume = 70;
    app.muted = false;
    let (buf_unmuted, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        !buf_unmuted.contains("MUTED"),
        "DEF-022: MUTED label must NOT show when unmuted: {buf_unmuted}"
    );
    app.muted = true;
    let (buf_muted, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf_muted.contains("MUTED"),
        "DEF-022: MUTED label must show alongside volume bar when muted: {buf_muted}"
    );
}

/// RC11-DEF-043: pressing `e` (enqueue) sets a toast rendered in the player
/// bar regardless of yt_state (the old `yt_status` toast was gated on Ready,
/// so local-only users never saw it). The toast reads "Added to queue".
#[test]
fn def043_enqueue_toast_visible_in_player_bar() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // No YT session → yt_state stays Unconfigured (not Ready), so the old
    // yt_status toast would be hidden. The dedicated toast must still show.
    app.view = View::Artists;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    app.enqueue_selected();
    assert_eq!(app.toast.as_deref(), Some("Added to queue"));
    let (buf, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf.contains("Added to queue"),
        "DEF-043: enqueue toast must render in the player bar even without YT (yt_state != Ready): {buf}"
    );
}

/// RC11-DEF-015: when a YouTube track is pending (cold miss, `pending_play`
/// set, now_playing still None), the player bar must show `[BUFFERING]` +
/// "Buffering…" — not `[STOPPED] — nothing playing —`.
#[test]
fn def015_buffering_label_when_pending_resolve() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Simulate a cold-miss: a pending YouTube pick whose URL hasn't landed.
    // now_playing stays None (the URL hasn't loaded yet); pending_play is set.
    app.now_playing = None;
    app.pending_play = Some("vidXYZ".into());
    // is_resolving() reads the session; with no session it's false, but
    // is_buffering() also fires on pending_play alone (cold-start case).
    let (buf, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf.contains("[BUFFERING]"),
        "DEF-015: cold-miss pick must show [BUFFERING] (not [STOPPED]): {buf}"
    );
    assert!(
        buf.contains("Buffering"),
        "DEF-015: bar must show 'Buffering' text: {buf}"
    );
    assert!(
        !buf.contains("nothing playing"),
        "DEF-015: buffering must NOT show 'nothing playing': {buf}"
    );
}

/// RC11-DEF-014: `resume_last()` plays the saved last-played track and, when
/// the backend can seek (StubPlayer), resumes at the saved position via
/// `load_at` (StubPlayer.load_at = load + seek_to). The resume hint clears
/// on play.
#[test]
fn def014_resume_last_seeks_to_saved_position() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Simulate a launch-restore state: last-played = t2 at 5.0s.
    app.last_played_track_id = Some("t2".into());
    app.last_played_position = 5.0;
    app.resume_hint = Some("resume: Night Tales at 0:05 · Enter to resume".into());
    // Resume → plays t2 at 5.0s.
    app.resume_last();
    assert_eq!(
        app.now_playing.as_ref().map(|t| t.id()),
        Some("t2"),
        "DEF-014: resume_last must play the saved track t2"
    );
    // StubPlayer.load_at = load (pos=0, dur=180) + seek_to(5.0) → pos=5.0.
    assert_eq!(
        app.player.position(),
        Some(5.0),
        "DEF-014: resume must seek to the saved position (5.0s) on a seekable backend"
    );
    // Hint clears on play.
    assert!(
        app.resume_hint.is_none(),
        "DEF-014: resume hint must clear after resume plays"
    );
}

/// RC11-DEF-014: the `R` key, when a resume hint is showing, must resume the
/// last track (not retry the YT probe). Verifies the input.rs wiring.
#[test]
fn def014_r_key_resumes_when_hint_showing() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.last_played_track_id = Some("t2".into());
    app.last_played_position = 3.0;
    app.resume_hint = Some("resume: Night Tales at 0:03 · Enter to resume".into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.now_playing.as_ref().map(|t| t.id()),
        Some("t2"),
        "DEF-014: R must resume the last track when the hint is showing"
    );
    assert_eq!(
        app.player.position(),
        Some(3.0),
        "DEF-014: R-resume must seek to the saved position"
    );
}

/// RC11-DEF-014: when stopped with a saved last-played track, the player bar
/// shows the "resume" hint so the user knows they can pick up where they left
/// off. The hint is cleared on the first play.
#[test]
fn def014_resume_hint_shown_when_stopped() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.now_playing = None;
    app.resume_hint = Some("resume: Night Tales at 0:05 · Enter to resume".into());
    let (buf, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf.contains("resume:"),
        "DEF-014: resume hint must render in the player bar when stopped: {buf}"
    );
    assert!(
        buf.contains("Enter") || buf.contains("R to resume") || buf.contains("R to"),
        "DEF-014: hint must tell the user how to resume: {buf}"
    );
}

/// RC11-DEF-014: state.db round-trip for the last-played track + position +
/// cursors (the persistence that powers resume across restarts).
#[test]
fn def014_last_played_state_round_trip() {
    use jukebox::state::{load_layout_at, save_layout_at, LayoutSave};
    use jukebox::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let widths = jukebox::tui::app::ColumnWidths {
        rail: 4,
        col1: 24,
        col2: 28,
        col3: 48,
    };
    save_layout_at(
        &path,
        &LayoutSave {
            focus: "artists",
            widths: &widths,
            volume: 70,
            shuffle: ShuffleMode::Off,
            repeat: RepeatMode::Off,
            continue_mode: ContinueMode::Off,
            source_mode: jukebox::mode::SourceMode::Local,
            yt_browser: "",
            last_played_track_id: Some("t2"),
            last_played_position: 7.0,
            last_cursor_artist: 1,
            last_cursor_album: 0,
            last_cursor_track: 1,
            last_cursor_playlist: 0,
            player_bar_mode: "mini",
            track_layout_mode: "table",
            sidebar_visible: true,
            playlist_col: &jukebox::tui::app::PlaylistColumnState::default(),
        },
    )
    .unwrap();
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(loaded.last_played_track_id.as_deref(), Some("t2"));
    assert!((loaded.last_played_position - 7.0).abs() < f64::EPSILON);
    assert_eq!(loaded.last_cursor_artist, 1);
    assert_eq!(loaded.last_cursor_track, 1);
}

/// RC11-DEF-042: on auto-advance (`next()` after a track ends), the player
/// bar must reflect the NEW track immediately — not keep showing the old
/// track's title/position. The event loop draws right after `on_tick`, so a
/// track switch (which sets `now_playing` to the new track) shows up on the
/// very next render. This test guards that: after `next()` from t1, the bar
/// contains t2's title and NOT t1's.
#[test]
fn def042_track_switch_reflects_new_track_in_bar() {
    let (_d, cat) = two_track_cat_d();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Play t1 of a 2-track context.
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    let (buf_t1, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf_t1.contains("Freedom"),
        "DEF-042: t1 (Freedom) should be playing first: {buf_t1}"
    );
    // Auto-advance (on_track_ended → next). The bar must now show t2.
    app.next();
    let (buf_t2, _t) = rendered_draw(&mut app, 100, 30);
    assert!(
        buf_t2.contains("Night Tales"),
        "DEF-042: after next(), bar must show the new track (Night Tales): {buf_t2}"
    );
    // And the old track's progress must not linger as a stale non-zero value.
    // After the switch the new track starts at 0 (StubPlayer.load resets pos).
    assert_eq!(
        app.player.position(),
        Some(0.0),
        "DEF-042: new track must start at position 0 (no stale position from the old track)"
    );
}
