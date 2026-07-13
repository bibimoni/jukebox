//! Regression tests for Fixer A defects from the RC-02 revalidation.
//!
//! - DEF-023: YouTube tracks not visible in queue view
//! - DEF-024: 80x24 playlist selection invisible (narrow layout)
//! - DEF-025: ASCII help dialog still uses Unicode

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::source::RemoteTrack;
use jukebox::tui::app::{App, Overlay, Playlist, View, YtList, YtListKind};
use jukebox::tui::view::columns;
use jukebox::tui::view::layout::draw;
use jukebox::tui::view::overlay;
use ratatui::{
    backend::TestBackend,
    style::{Color, Modifier},
    Terminal,
};
use std::io::Write;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn two_artist_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
    std::fs::create_dir_all(lossless.join("DECO")).unwrap();
    std::fs::write(lossless.join("DECO").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1","album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Ghost Rule","album":"Ghost","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
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

/// Render `columns::render` (wide layout) into a flat string.
fn rendered_cols(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    let area = ratatui::layout::Rect::new(0, 0, w, h);
    term.draw(|f| columns::render(f, area, app)).unwrap();
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

/// Render `layout::draw` (full TUI) and return the buffer string + the terminal
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

/// True if the cell at (x, y) carries the selection style (accent bg in color
/// mode, or REVERSED modifier in any mode — selected_style always sets
/// REVERSED).
fn cell_has_selection_style(term: &Terminal<TestBackend>, x: u16, y: u16) -> bool {
    let cell = &term.backend().buffer()[(x, y)];
    cell.modifier.contains(Modifier::REVERSED) || cell.bg == Color::Cyan
}

// ---------------------------------------------------------------------------
// DEF-023: YouTube tracks not visible in queue view
// ---------------------------------------------------------------------------

#[test]
fn def023_youtube_track_in_queue_visible_as_loading_without_session() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Queue;
    // A YouTube video id enqueued — no yt_session, so metadata can't resolve.
    // DEF-023: previously this row was DROPPED (track_by_id_fast returns None
    // and filter_map skipped it). Now it must render as "Loading...".
    app.transport.manual_queue.push("v001".into());
    let buf = rendered_cols(&mut app, 120, 30);
    assert!(
        buf.contains("Loading"),
        "DEF-023: YouTube track in queue must be visible (Loading...), not dropped: {buf}"
    );
    assert!(
        !buf.contains("Queue is empty"),
        "DEF-023: queue with a YouTube track must not show the empty hint: {buf}"
    );
}

#[test]
fn def023_youtube_track_in_queue_resolves_title_with_session() {
    let (_d, cat) = two_artist_cat();
    let session = spawn_minimal_session();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.view = View::Queue;
    // Cache a YouTube track so track_for(video_id) resolves.
    app.yt_session.as_mut().unwrap().track_cache.insert(
        "v001".to_string(),
        RemoteTrack {
            video_id: "v001".into(),
            title: "YT Song".into(),
            artist: "YT Artist".into(),
            album: None,
            dur: None,
            fmt: None,
            isrc: None,
        },
    );
    app.transport.manual_queue.push("v001".into());
    let buf = rendered_cols(&mut app, 120, 30);
    assert!(
        buf.contains("YT Song"),
        "DEF-023: YouTube track in queue must resolve its title from the session cache: {buf}"
    );
    assert!(
        !buf.contains("Queue is empty"),
        "DEF-023: queue with a resolved YouTube track must not show the empty hint: {buf}"
    );
}

#[test]
fn def023_mixed_queue_shows_both_local_and_youtube() {
    let (_d, cat) = two_artist_cat();
    let session = spawn_minimal_session();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.view = View::Queue;
    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.yt_session.as_mut().unwrap().track_cache.insert(
        "v001".to_string(),
        RemoteTrack {
            video_id: "v001".into(),
            title: "YT Song".into(),
            artist: "YT Artist".into(),
            album: None,
            dur: None,
            fmt: None,
            isrc: None,
        },
    );
    // Local track + YouTube track in the same queue.
    app.transport.manual_queue.push("t1".into());
    app.transport.manual_queue.push("v001".into());
    let buf = rendered_cols(&mut app, 120, 30);
    assert!(
        buf.contains("Song1"),
        "DEF-023: local track must still render in mixed queue: {buf}"
    );
    assert!(
        buf.contains("YT Song"),
        "DEF-023: YouTube track must render in mixed queue: {buf}"
    );
}

// ---------------------------------------------------------------------------
// DEF-024: 80x24 playlist selection invisible (narrow layout)
// ---------------------------------------------------------------------------

#[test]
fn def024_narrow_playlists_selected_item_has_selection_style() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Playlists;
    app.playlists = vec![
        Playlist {
            name: "Alpha".into(),
            track_ids: vec![],
        },
        Playlist {
            name: "Beta".into(),
            track_ids: vec![],
        },
    ];
    app.cursors.playlist = 1; // select "Beta"
    app.focus_col = 0;
    // 80x24 triggers the narrow render path (width < MIN_WIDTH=101).
    let (buf, term) = rendered_draw(&mut app, 80, 24);
    let pos =
        find_substr(&term, 80, 24, "Beta").unwrap_or_else(|| panic!("Beta not rendered: {buf}"));
    assert!(
        cell_has_selection_style(&term, pos.0, pos.1),
        "DEF-024: selected playlist 'Beta' must carry the selection style at 80x24: {buf}"
    );
    // The unselected "Alpha" must NOT carry the selection style.
    let alpha_pos =
        find_substr(&term, 80, 24, "Alpha").unwrap_or_else(|| panic!("Alpha not rendered: {buf}"));
    assert!(
        !cell_has_selection_style(&term, alpha_pos.0, alpha_pos.1),
        "DEF-024: unselected playlist 'Alpha' must NOT carry the selection style: {buf}"
    );
}

#[test]
fn def024_narrow_albums_selected_item_has_selection_style() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 1; // Albums pane focused (narrow shows Albums breadcrumb)
    app.cursors.artist = 0; // 40mP
    app.cursors.album = 0; // Cosmic
    let (buf, term) = rendered_draw(&mut app, 80, 24);
    let pos = find_substr(&term, 80, 24, "Cosmic")
        .unwrap_or_else(|| panic!("Cosmic not rendered: {buf}"));
    assert!(
        cell_has_selection_style(&term, pos.0, pos.1),
        "DEF-024: selected album 'Cosmic' must carry the selection style at 80x24 (focus_col=1): {buf}"
    );
}

#[test]
fn def024_narrow_youtube_list_selected_item_has_selection_style() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_lists = vec![
        YtList {
            id: "PL1".into(),
            name: "Liked".into(),
            kind: YtListKind::Account,
            track_ids: vec![],
        },
        YtList {
            id: "RD1".into(),
            name: "Focus".into(),
            kind: YtListKind::Suggested,
            track_ids: vec![],
        },
    ];
    app.cursors.playlist = 0; // select "Liked"
    app.focus_col = 0;
    let (buf, term) = rendered_draw(&mut app, 80, 24);
    let pos =
        find_substr(&term, 80, 24, "Liked").unwrap_or_else(|| panic!("Liked not rendered: {buf}"));
    assert!(
        cell_has_selection_style(&term, pos.0, pos.1),
        "DEF-024: selected YT list 'Liked' must carry the selection style at 80x24: {buf}"
    );
}

// ---------------------------------------------------------------------------
// DEF-025: ASCII help dialog still uses Unicode
// ---------------------------------------------------------------------------

/// Serializes tests that set/unset JUKEBOX_FONT_MODE so they don't interfere
/// with each other under parallel test execution.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn def025_help_lines_ascii_mode_uses_no_unicode_borders_or_arrows() {
    let lines = overlay::help_lines(80, true);
    let mut all = String::new();
    for l in &lines {
        for s in &l.spans {
            all.push_str(&s.content);
        }
        all.push('\n');
    }
    // Separators must be ASCII '-' not Unicode '─'.
    assert!(
        !all.contains('\u{2500}'),
        "DEF-025: ASCII help_lines must not use Unicode horizontal line (U+2500): {all:?}"
    );
    // Arrows must be ASCII, not Unicode ↑↓←→.
    assert!(
        !all.contains('\u{2191}')
            && !all.contains('\u{2193}')
            && !all.contains('\u{2190}')
            && !all.contains('\u{2192}'),
        "DEF-025: ASCII help_lines must not use Unicode arrows: {all:?}"
    );
    // Em-dash / middle-dot replaced in ASCII mode.
    assert!(
        !all.contains('\u{2014}') && !all.contains('\u{00B7}'),
        "DEF-025: ASCII help_lines must not use Unicode em-dash / middle dot: {all:?}"
    );
    // Separators must contain ASCII '-' repeated.
    assert!(
        all.contains("----"),
        "DEF-025: ASCII help_lines separator must use '-' chars: {all:?}"
    );
}

#[test]
fn def025_help_lines_unicode_mode_uses_unicode_separator() {
    let lines = overlay::help_lines(80, false);
    let mut all = String::new();
    for l in &lines {
        for s in &l.spans {
            all.push_str(&s.content);
        }
        all.push('\n');
    }
    assert!(
        all.contains('\u{2500}'),
        "DEF-025: Unicode help_lines must use the U+2500 separator: {all:?}"
    );
}

#[test]
fn def025_help_overlay_renders_ascii_border_in_ascii_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Help);
    let (buf, term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // The help popup border must use ASCII '+' corners, '-' horizontal, '|'
    // vertical — not Unicode box-drawing (U+250C etc).
    assert!(
        !buf.contains('\u{250C}')
            && !buf.contains('\u{2510}')
            && !buf.contains('\u{2514}')
            && !buf.contains('\u{2518}')
            && !buf.contains('\u{2500}')
            && !buf.contains('\u{2502}'),
        "DEF-025: ASCII help overlay must not use Unicode box-drawing chars: {buf}"
    );
    // Must contain ASCII border chars somewhere.
    let border_cell = &term.backend().buffer()[(0, 1)];
    assert!(
        border_cell.symbol().contains('+') || buf.contains('+'),
        "DEF-025: ASCII help overlay must use '+' corner chars: {buf}"
    );
}

#[test]
fn def025_help_overlay_renders_unicode_border_by_default() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Help);
    let (buf, _term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    assert!(
        buf.contains('\u{250C}') || buf.contains('\u{2502}') || buf.contains('\u{2500}'),
        "DEF-025: Unicode help overlay must use Unicode box-drawing chars: {buf}"
    );
}

// ---------------------------------------------------------------------------
// DEF-029: ASCII mode Unicode leaks in dynamic UI
// ---------------------------------------------------------------------------

/// Lock the env var, recovering from a poisoned mutex (a prior test may have
/// panicked while holding the lock).
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    // RC18-D1: reset the process-stable font mode cache so the env var
    // the test is about to set actually takes effect on the next
    // `is_ascii()` / `cached_font_mode()` call.
    jukebox::tui::view::theme::reset_font_mode_cache();
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn def029_theme_arrow_helpers_return_ascii_in_ascii_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    use jukebox::tui::view::theme;
    assert_eq!(theme::right_arrow(), "->");
    assert_eq!(theme::left_arrow(), "<-");
    assert_eq!(theme::up_arrow(), "^");
    assert_eq!(theme::down_arrow(), "v");
    assert_eq!(theme::bullet(), "*");
    assert_eq!(theme::em_dash(), "--");
    assert_eq!(theme::sep_dot(), "*");
    assert_eq!(theme::ellipsis(), "...");
    std::env::remove_var("JUKEBOX_FONT_MODE");
}

#[test]
fn def029_theme_arrow_helpers_return_unicode_in_unicode_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    use jukebox::tui::view::theme;
    assert_eq!(theme::right_arrow(), "\u{2192}");
    assert_eq!(theme::left_arrow(), "\u{2190}");
    assert_eq!(theme::up_arrow(), "\u{2191}");
    assert_eq!(theme::down_arrow(), "\u{2193}");
    assert_eq!(theme::bullet(), "\u{2022}");
    assert_eq!(theme::em_dash(), "\u{2014}");
    assert_eq!(theme::sep_dot(), "\u{00B7}");
    assert_eq!(theme::ellipsis(), "\u{2026}");
    std::env::remove_var("JUKEBOX_FONT_MODE");
}

#[test]
fn def029_ascii_sanitize_replaces_all_known_unicode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    use jukebox::tui::view::theme;
    let input = "test — · … → ← ↑ ↓ ▸ ▶ ♫ ✦ ≡ ⏸ ■ • end";
    let sanitized = theme::ascii_sanitize(input);
    assert!(
        sanitized.is_ascii(),
        "DEF-029: ascii_sanitize must produce pure ASCII: {sanitized:?}"
    );
    assert_eq!(sanitized, "test -- * ... -> <- ^ v > > # * # || # * end");
    std::env::remove_var("JUKEBOX_FONT_MODE");
}

#[test]
fn def029_ascii_sanitize_noop_in_unicode_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    use jukebox::tui::view::theme;
    let input = "test — · … → end";
    let sanitized = theme::ascii_sanitize(input);
    assert_eq!(
        sanitized, input,
        "DEF-029: ascii_sanitize must be noop in Unicode mode"
    );
    std::env::remove_var("JUKEBOX_FONT_MODE");
}

#[test]
fn def029_columns_ascii_mode_no_unicode_in_breadcrumbs() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 1; // Albums pane — narrow breadcrumb shows "Albums * 40mP <- Artists"
    app.cursors.artist = 0;
    app.cursors.album = 0;
    // 80x24 triggers the narrow render path with breadcrumbs.
    let (buf, _term) = rendered_draw(&mut app, 80, 24);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // No Unicode arrows or middle dots in the rendered output.
    assert!(
        !buf.contains('\u{2192}') && !buf.contains('\u{2190}') && !buf.contains('\u{00B7}'),
        "DEF-029: ASCII narrow layout must not contain Unicode arrows/middle-dot: {buf}"
    );
    // Must contain ASCII breadcrumb equivalents.
    assert!(
        buf.contains("<-"),
        "DEF-029: ASCII breadcrumb must use '<-' for left arrow: {buf}"
    );
}

#[test]
fn def029_columns_ascii_mode_no_unicode_in_empty_state_hints() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    let buf = rendered_cols(&mut app, 120, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // The "no artists" hint must not contain Unicode em-dash.
    assert!(
        !buf.contains('\u{2014}'),
        "DEF-029: ASCII empty-state hint must not contain Unicode em-dash: {buf}"
    );
    assert!(
        buf.contains("--"),
        "DEF-029: ASCII empty-state hint must use '--' for em-dash: {buf}"
    );
}

#[test]
fn def029_layout_ascii_mode_no_unicode_in_navigation_hints() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 0;
    // 80x24 triggers the narrow render path with navigation hints.
    let (buf, _term) = rendered_draw(&mut app, 80, 24);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // Navigation hints must not contain Unicode arrows or middle dots.
    assert!(
        !buf.contains('\u{2192}') && !buf.contains('\u{2190}') && !buf.contains('\u{00B7}'),
        "DEF-029: ASCII narrow navigation hints must not contain Unicode arrows/middle-dot: {buf}"
    );
    // Must contain ASCII arrow equivalents.
    assert!(
        buf.contains("->"),
        "DEF-029: ASCII navigation hint must use '->' for right arrow: {buf}"
    );
}

#[test]
fn def029_footer_ascii_mode_no_unicode_in_search_hints() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Search {
        input: "test".into(),
        results: vec![],
        cursor: 0,
        scope: jukebox::tui::app::SearchScope::Local,
        submitted: None,
        searching: false,
    });
    let (buf, _term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // The footer search hint must not contain Unicode middle dot.
    assert!(
        !buf.contains('\u{00B7}'),
        "DEF-029: ASCII footer search hint must not contain Unicode middle dot: {buf}"
    );
}

#[test]
fn def029_search_overlay_ascii_mode_no_unicode_arrows() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Search {
        input: "test".into(),
        results: vec![],
        cursor: 0,
        scope: jukebox::tui::app::SearchScope::Youtube,
        submitted: None,
        searching: false,
    });
    let (buf, _term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // Search overlay hints must not contain Unicode arrows, middle dots, or ellipsis.
    assert!(
        !buf.contains('\u{2192}')
            && !buf.contains('\u{2190}')
            && !buf.contains('\u{00B7}')
            && !buf.contains('\u{2026}'),
        "DEF-029: ASCII search overlay must not contain Unicode arrows/middle-dot/ellipsis: {buf}"
    );
}

#[test]
fn def029_diagnostics_overlay_ascii_mode_no_unicode_em_dash() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Diagnostics);
    let (buf, _term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    // Diagnostics overlay title must not contain Unicode em-dash.
    assert!(
        !buf.contains('\u{2014}'),
        "DEF-029: ASCII diagnostics overlay must not contain Unicode em-dash: {buf}"
    );
}

#[test]
fn def029_confirm_overlay_ascii_mode_no_unicode_middle_dot() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Confirm {
        message: "Delete playlist?".into(),
        action: jukebox::tui::app::ConfirmAction::DeletePlaylist,
    });
    let (buf, _term) = rendered_draw(&mut app, 100, 30);
    std::env::remove_var("JUKEBOX_FONT_MODE");
    assert!(
        !buf.contains('\u{00B7}'),
        "DEF-029: ASCII confirm overlay must not contain Unicode middle dot: {buf}"
    );
}

// ---------------------------------------------------------------------------
// Minimal fake sidecar (for DEF-023 session-backed tests)
// ---------------------------------------------------------------------------

/// Spawn a Session backed by a minimal python sidecar that reads stdin and
/// does nothing. We only need the Session struct so `track_cache` is
/// accessible; no sidecar responses are required for the render tests.
fn spawn_minimal_session() -> jukebox::yt::session::Session {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("jk-fixA-sidecar-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(b"import sys\nfor line in sys.stdin:\n    pass\n")
        .unwrap();
    writeln!(f).unwrap();
    let session = jukebox::yt::session::Session::spawn(std::path::Path::new("python3"), &p, None)
        .expect("spawn minimal sidecar");
    // Best-effort cleanup; Session drop kills the child.
    let _ = std::fs::remove_file(&p);
    session
}
