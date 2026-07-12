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
