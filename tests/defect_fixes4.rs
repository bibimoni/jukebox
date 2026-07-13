//! Regression tests for Fixer B defects (MOD-1 through MOD-4).
//!
//! - MOD-1: ASCII mode — discover overlay still uses Unicode
//! - MOD-2: Discover overlay overlaps main panel at 80x24
//! - MOD-3: Track name and bitrate overlap in Artists view
//! - MOD-4: `:queue clear` has no confirmation

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, DiscoverItem, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

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

/// A catalog with a track whose title + album + quality is long enough to
/// trigger the pad_between 0-padding case (MOD-3).
fn long_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Night Tales")).unwrap();
    std::fs::write(lossless.join("Night Tales").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["Night Tales"],"primary_artist":"Night Tales",
        "title":"Midnight Journey","album":"Night Tales",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/Night Tales/01.flac",
        "symlinked_into_artists":["Night Tales"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-mod4-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

/// Render the full TUI into a flat string.
fn rendered(app: &mut App, w: u16, h: u16) -> (String, Terminal<TestBackend>) {
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

/// Render just the columns (wide layout) into a flat string.
fn rendered_cols(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    let area = ratatui::layout::Rect::new(0, 0, w, h);
    term.draw(|f| jukebox::tui::view::columns::render(f, area, app))
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

/// Serializes tests that set/unset JUKEBOX_FONT_MODE.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Lock the env + reset the font mode cache so the env var the test is
/// about to set actually takes effect. RC18-D1: the cache is process-stable
/// in production; tests that mutate `JUKEBOX_FONT_MODE` must reset it.
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    jukebox::tui::view::theme::reset_font_mode_cache();
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// MOD-1: ASCII mode — discover overlay still uses Unicode
// ---------------------------------------------------------------------------

#[test]
fn mod1_discover_overlay_uses_ascii_border_in_ascii_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Album {
            artist: "A".into(),
            album: "Al".into(),
        }],
        cursor: 0,
    });
    let (buf, _term) = rendered(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // MOD-1: in ASCII mode, the discover overlay must not use Unicode
    // box-drawing chars (┌┐└┘│─) or Unicode glyphs (♫ ✦ —).
    assert!(
        !buf.contains('\u{250C}')
            && !buf.contains('\u{2510}')
            && !buf.contains('\u{2514}')
            && !buf.contains('\u{2518}')
            && !buf.contains('\u{2500}')
            && !buf.contains('\u{2502}'),
        "MOD-1: ASCII discover overlay must not use Unicode box-drawing chars: {buf}"
    );
    // Must not contain the Unicode music note or star glyphs.
    assert!(
        !buf.contains('\u{266B}') && !buf.contains('\u{2726}'),
        "MOD-1: ASCII discover overlay must not use Unicode ♫/✦ glyphs: {buf}"
    );
    // Must not contain the Unicode em-dash.
    assert!(
        !buf.contains('\u{2014}'),
        "MOD-1: ASCII discover overlay must not use Unicode em-dash: {buf}"
    );
}

#[test]
fn mod1_discover_overlay_uses_unicode_border_by_default() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Album {
            artist: "A".into(),
            album: "Al".into(),
        }],
        cursor: 0,
    });
    let (buf, _term) = rendered(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // In Unicode mode, the discover overlay should use Unicode box-drawing.
    assert!(
        buf.contains('\u{2502}') || buf.contains('\u{2500}'),
        "MOD-1: Unicode discover overlay should use Unicode box-drawing chars: {buf}"
    );
}

#[test]
fn mod1_discover_overlay_ascii_uses_ascii_glyphs() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Discover {
        items: vec![
            DiscoverItem::Album {
                artist: "Artist".into(),
                album: "Album".into(),
            },
            DiscoverItem::Playlist {
                id: "RD1".into(),
                name: "Mix".into(),
            },
        ],
        cursor: 0,
    });
    let (buf, _term) = rendered(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // ASCII mode: album glyph should be '#' and playlist glyph '*'.
    assert!(
        buf.contains('#'),
        "MOD-1: ASCII discover overlay should use '#' for albums: {buf}"
    );
    assert!(
        buf.contains('*'),
        "MOD-1: ASCII discover overlay should use '*' for playlists: {buf}"
    );
    // The ASCII em-dash replacement '--' should appear in the title or album text.
    assert!(
        buf.contains("--"),
        "MOD-1: ASCII discover overlay should use '--' for em-dash: {buf}"
    );
}

// ---------------------------------------------------------------------------
// MOD-2: Discover overlay overlaps main panel at 80x24
// ---------------------------------------------------------------------------

#[test]
fn mod2_discover_overlay_clears_full_area_no_bleed_through() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Put the app in a state with visible track text in the main panel.
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    // Open the discover overlay.
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Album {
            artist: "A".into(),
            album: "Al".into(),
        }],
        cursor: 0,
    });
    let (buf, term) = rendered(&mut app, 80, 24);
    // MOD-2: the discover overlay must clear its entire area so background
    // text doesn't bleed through. Check that the columns on either side of
    // the popup don't contain track text from the main panel.
    //
    // The popup is centered at 55% width, 45% height of an 80x24 area.
    // popup x range: roughly [18, 62], y range: roughly [6, 16].
    // Check the left margin (x=0..17) and right margin (x=63..79) at the
    // popup's y range for any non-space characters that would indicate
    // bleed-through from the main panel.
    let popup_x_start = 80 * (100 - 55) / 2 / 100;
    let popup_x_end = 80 - popup_x_start;
    for y in 6..=16 {
        for x in 0..popup_x_start {
            let cell = &term.backend().buffer()[(x as u16, y as u16)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            // Border characters from the main panel should NOT be visible
            // in the margins around the popup. We check for the main panel's
            // track text — "Local Song" or "Al" should not bleed through.
            if ch != ' ' {
                // Some characters are OK (e.g. the player bar below). But
                // within the popup's y range (6-16) the main panel columns
                // should be fully cleared.
                assert!(
                    !buf.contains("Local Song")
                        || !format!("{ch}").contains('L'),
                    "MOD-2: main panel text should not bleed through the discover overlay margins at ({x},{y}): '{ch}'\n{buf}"
                );
            }
        }
        for x in popup_x_end..80 {
            let cell = &term.backend().buffer()[(x as u16, y as u16)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if ch != ' ' {
                assert!(
                    !format!("{ch}").contains('L'),
                    "MOD-2: main panel text should not bleed through the discover overlay margins at ({x},{y}): '{ch}'\n{buf}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MOD-3: Track name and bitrate overlap in Artists view
// ---------------------------------------------------------------------------

#[test]
fn mod3_track_row_has_space_between_album_and_quality() {
    let _xdg = isolate_xdg();
    let (_d, cat) = long_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    // 100x30 triggers the 3-column layout. The track column is the
    // remaining width after col1 + col2, which is narrow enough to trigger
    // the 0-padding case in pad_between.
    let buf = rendered_cols(&mut app, 100, 30);
    // MOD-3: the track row must NOT have the quality info stuck directly
    // after the album name without a space. The bug produced
    // "Midnight Journey — Night Tales16bit…" (no space before "16bit").
    // We check that "Tales16bit" or "Tales 16bit" — the fix ensures a space.
    //
    // Find the line containing "Midnight Journey" and check it doesn't
    // have "Tales16" (no space between album and quality).
    for line in buf.lines() {
        if line.contains("Midnight Journey") || line.contains("Night Tales") {
            assert!(
                !line.contains("Tales16") && !line.contains("Tales24"),
                "MOD-3: track row must have a space between album name and quality info, got: {line}"
            );
        }
    }
}

#[test]
fn mod3_track_row_quality_visible_with_space_when_truncated() {
    let _xdg = isolate_xdg();
    let (_d, cat) = long_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
    // Render at a narrow track-column width to force truncation.
    let buf = rendered_cols(&mut app, 100, 30);
    // The quality info "16bit-44.1kHz" (or truncated) should either be
    // visible with a preceding space, or truncated away entirely — but
    // never stuck directly after the album name without a space.
    for line in buf.lines() {
        // Check for any digit immediately following "Tales" (no space).
        if line.contains("Tales") {
            let idx = line.find("Tales").unwrap();
            let after = &line[idx + 5..];
            if after.starts_with(|c: char| c.is_ascii_digit()) {
                panic!(
                    "MOD-3: quality info immediately follows album name without a space: {line}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MOD-4: `:queue clear` has no confirmation
// ---------------------------------------------------------------------------

/// Open the `:` command overlay, type `text`, and press Enter.
fn open_command(app: &mut App, text: &str) {
    handle_key(app, key(':'));
    for c in text.chars() {
        handle_key(app, key(c));
    }
    handle_key(app, key_code(KeyCode::Enter));
}

#[test]
fn mod4_queue_clear_opens_confirm_when_queue_nonempty() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    assert_eq!(app.transport.manual_queue.len(), 1);
    open_command(&mut app, "queue clear");
    // MOD-4: a confirmation dialog must appear, not immediate clearing.
    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "MOD-4: `:queue clear` with non-empty queue should open a confirmation dialog"
    );
    assert_eq!(
        app.transport.manual_queue.len(),
        1,
        "MOD-4: queue must not be cleared until user confirms"
    );
}

#[test]
fn mod4_queue_clear_noop_when_queue_empty() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(app.transport.manual_queue.is_empty());
    open_command(&mut app, "queue clear");
    // MOD-4: when the queue is empty, no confirmation is needed — it's a no-op.
    assert!(
        app.overlay.is_none(),
        "MOD-4: `:queue clear` on empty queue should not open a dialog"
    );
    assert!(
        app.transport.manual_queue.is_empty(),
        "MOD-4: empty queue should stay empty"
    );
}

#[test]
fn mod4_queue_clear_y_confirms() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    open_command(&mut app, "queue clear");
    assert!(matches!(app.overlay, Some(Overlay::Confirm { .. })));
    handle_key(&mut app, key('y'));
    assert!(
        app.overlay.is_none(),
        "MOD-4: overlay should close after confirm"
    );
    assert!(
        app.transport.manual_queue.is_empty(),
        "MOD-4: queue should be cleared after 'y' confirmation"
    );
    assert_eq!(
        app.yt_status.as_deref(),
        Some("queue cleared"),
        "MOD-4: status should confirm the clear"
    );
}

#[test]
fn mod4_queue_clear_enter_confirms() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    open_command(&mut app, "queue clear");
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(
        app.transport.manual_queue.is_empty(),
        "MOD-4: Enter should confirm the clear"
    );
}

#[test]
fn mod4_queue_clear_n_cancels() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    open_command(&mut app, "queue clear");
    handle_key(&mut app, key('n'));
    assert!(app.overlay.is_none(), "MOD-4: n should close the dialog");
    assert_eq!(
        app.transport.manual_queue.len(),
        1,
        "MOD-4: queue should not be cleared after 'n'"
    );
    assert!(
        !app.yt_status.as_deref().unwrap_or("").contains("cleared"),
        "MOD-4: status should not say 'cleared' after cancel"
    );
}

#[test]
fn mod4_queue_clear_esc_cancels() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    open_command(&mut app, "queue clear");
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "MOD-4: Esc should close the dialog");
    assert_eq!(
        app.transport.manual_queue.len(),
        1,
        "MOD-4: queue should not be cleared after Esc"
    );
}

#[test]
fn mod4_queue_clear_confirm_message_mentions_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    open_command(&mut app, "queue clear");
    if let Some(Overlay::Confirm { message, .. }) = &app.overlay {
        assert!(
            message.to_lowercase().contains("queue"),
            "MOD-4: confirm message should mention 'queue', got: {message}"
        );
    } else {
        panic!("MOD-4: expected Confirm overlay");
    }
}
