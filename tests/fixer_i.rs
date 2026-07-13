//! Regression tests for Fixer I defects (RC-15 remaining blockers).
//!
//! - RC15-DEF-1: Help `G` key overshoots past content (popup blank).
//! - RC15-DEF-2 (ACC-01): No-color mode Artists column selection invisible.
//! - RC15-DEF-3 (D-1): Publish overlay shows raw video IDs not track titles.
//! - RC15-DEF-4 (D1): `:yt setup` zero feedback (blocks 30s, nothing visible).
//! - RC15-DEF-5 (ACC-02): 80x24 status bar long title hard-cut, no ellipsis.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use jukebox::tui::view::overlay::help_lines;
use ratatui::{
    backend::TestBackend,
    style::{Modifier, Style},
    Terminal,
};

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

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-fixi-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

/// Render the full TUI into a flat string.
fn rendered(app: &mut App, w: u16, h: u16) -> String {
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

/// Render the TUI and collect (char, Style) for each cell in a given row.
fn row_cells(app: &mut App, w: u16, h: u16, y: u16) -> Vec<(char, Style)> {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    (0..w)
        .map(|x| {
            let cell = &term.backend().buffer()[(x, y)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            (ch, cell.style())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// RC15-DEF-1: Help G key overshoots past content
// ---------------------------------------------------------------------------

/// `G` should scroll to the last PAGE of content, not past it. After pressing
/// `G`, the help popup must show content (not blank lines). The render-side
/// clamp in `render_help` ensures `scroll <= lines.len() - inner_height`.
#[test]
fn rc15_def1_help_g_key_shows_content_not_blank() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open help.
    handle_key(&mut app, key('?'));
    assert_eq!(app.help_scroll, 0, "help should start at top");
    // Press G — should jump to the last page.
    handle_key(&mut app, key('G'));
    let max_scroll = help_lines(0, false).len().saturating_sub(1) as u16;
    assert_eq!(
        app.help_scroll, max_scroll,
        "G should set scroll to max_scroll (last content page), not past it"
    );
    // Render at 100x30 and verify the help popup has non-blank content rows
    // (not all spaces). The old bug scrolled past all content, leaving the
    // popup entirely blank.
    let buf = rendered(&mut app, 100, 30);
    let mut content_chars = 0usize;
    for (i, line) in buf.lines().enumerate() {
        if (1..=27).contains(&i) {
            for c in line.chars().skip(5).take(90) {
                if !c.is_whitespace() {
                    content_chars += 1;
                }
            }
        }
    }
    assert!(
        content_chars > 50,
        "RC15-DEF-1: after G, help popup must show content (not blank). \
         Found {content_chars} non-space chars in popup region.\n{buf}"
    );
}

/// `j` pressed 100 times should never leave the popup blank — the scroll
/// cap + render-side clamp ensure content is always visible.
#[test]
fn rc15_def1_help_j_100x_content_always_visible() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('?'));
    for _ in 0..100 {
        handle_key(&mut app, key('j'));
    }
    // Render and verify content is visible.
    let buf = rendered(&mut app, 100, 30);
    let mut content_chars = 0usize;
    for (i, line) in buf.lines().enumerate() {
        if (1..=27).contains(&i) {
            for c in line.chars().skip(5).take(90) {
                if !c.is_whitespace() {
                    content_chars += 1;
                }
            }
        }
    }
    assert!(
        content_chars > 50,
        "RC15-DEF-1: after 100x j, help popup must still show content. \
         Found {content_chars} non-space chars.\n{buf}"
    );
}

/// `End` should mirror `G` — jump to the last page, not past content.
#[test]
fn rc15_def1_help_end_shows_content_not_blank() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('?'));
    handle_key(&mut app, key_code(KeyCode::End));
    let buf = rendered(&mut app, 100, 30);
    let mut content_chars = 0usize;
    for (i, line) in buf.lines().enumerate() {
        if (1..=27).contains(&i) {
            for c in line.chars().skip(5).take(90) {
                if !c.is_whitespace() {
                    content_chars += 1;
                }
            }
        }
    }
    assert!(
        content_chars > 50,
        "RC15-DEF-1: after End, help popup must show content (not blank). \
         Found {content_chars} non-space chars.\n{buf}"
    );
}

// ---------------------------------------------------------------------------
// RC15-DEF-2 (ACC-01): No-color Artists column selection invisible
// ---------------------------------------------------------------------------

/// In no-color mode, the selected artist in the Artists column must have the
/// REVERSED modifier (reverse video) applied to its text. The old bug: a
/// stray `\x1b[;m` reset cancelled the highlight before the text was drawn,
/// making the selection invisible. Fix: render the Artists column via
/// Paragraph+per-line styles (like the Tracks column) instead of
/// List+highlight_style, avoiding the stray reset.
#[test]
fn rc15_def2_no_color_artists_selection_has_reversed_modifier() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.artists = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
    app.cursors.artist = 1; // Select "Beta"

    // Set NO_COLOR so the theme collapses to grayscale + modifiers.
    std::env::set_var("NO_COLOR", "1");

    // Render at 100x30 (wide layout, not narrow). The selected artist "Beta"
    // is at index 1 → row y=3 (y=0 tab bar, y=1 border, y=2 Alpha, y=3 Beta).
    let cells = row_cells(&mut app, 100, 30, 3);

    std::env::remove_var("NO_COLOR");

    // The selected row uses Paragraph+Span::styled with selected_style()
    // which applies REVERSED|BOLD under NO_COLOR. The marker glyph `▸`
    // (U+25B8) precedes the name. Find the `▸` cell and verify it (and the
    // following "Beta" cells) have the REVERSED modifier.
    let marker_idx = cells
        .iter()
        .position(|(c, _)| *c == '\u{25B8}')
        .unwrap_or_else(|| {
            let row_str: String = cells.iter().map(|(c, _)| *c).collect();
            panic!("RC15-DEF-2: marker glyph ▸ not found in row: {row_str}")
        });

    // Check the marker cell + the next 5 cells ("▸ Beta") for REVERSED.
    let end = (marker_idx + 6).min(cells.len());
    let has_reversed = cells[marker_idx..end]
        .iter()
        .any(|(_, style)| style.add_modifier.contains(Modifier::REVERSED));

    assert!(
        has_reversed,
        "RC15-DEF-2 (ACC-01): in NO_COLOR mode, the selected artist 'Beta' must have \
         REVERSED modifier on the ▸ marker or the name cells."
    );
}

/// In no-color mode, the selected artist must be visually distinct from
/// unselected artists. The `▸` marker glyph provides a non-color cue even
/// if reverse video fails.
#[test]
fn rc15_def2_no_color_artists_selection_has_marker_glyph() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.artists = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
    app.cursors.artist = 1;

    std::env::set_var("NO_COLOR", "1");
    // "Beta" is at index 1 → row y=3 (y=0 tabs, y=1 border, y=2 Alpha, y=3 Beta).
    let cells = row_cells(&mut app, 100, 30, 3);
    std::env::remove_var("NO_COLOR");

    let row_str: String = cells.iter().map(|(c, _)| *c).collect();
    // The selected row should start with the marker glyph `▸` (U+25B8) or
    // `>` in ASCII mode. Under NO_COLOR (not ASCII), it's `▸`.
    assert!(
        row_str.contains('\u{25B8}'),
        "RC15-DEF-2: selected artist row should have the ▸ marker glyph. Row: {row_str}"
    );
    // Check row y=4 (Gamma, unselected) — no marker.
    let cells_unselected = row_cells(&mut app, 100, 30, 4);
    let unselected_str: String = cells_unselected.iter().map(|(c, _)| *c).collect();
    assert!(
        !unselected_str.contains('\u{25B8}'),
        "RC15-DEF-2: unselected artist should NOT have the marker. Row: {unselected_str}"
    );
}

// ---------------------------------------------------------------------------
// RC15-DEF-3 (D-1): Publish overlay shows raw video IDs not track titles
// ---------------------------------------------------------------------------

/// `open_publication` must populate `publishable_titles` with "Title — Artist"
/// display strings, not raw video IDs. Local catalog ids resolve via
/// `track_by_id_fast`; YouTube video ids resolve via the session's
/// `track_cache` (or fall back to the raw id when uncached).
#[test]
fn rc15_def3_publication_publishable_titles_resolved() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Create a local playlist with a local track (t1, in catalog) and a
    // YouTube video id (v001, not in catalog → publishable).
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "My Mix".into(),
        track_ids: vec!["t1".into(), "v001".into()],
    });
    app.open_publication("My Mix");
    let state = match &app.overlay {
        Some(Overlay::Publication { state }) => state,
        _ => panic!("expected Publication overlay"),
    };
    // t1 is in the catalog → local_only with resolved title.
    assert_eq!(state.local_only.len(), 1);
    assert_eq!(state.local_only[0], "t1");
    assert_eq!(
        state.local_only_titles.len(),
        1,
        "local_only_titles should be populated"
    );
    // "Local Song" is the title of t1, "A" is the primary_artist.
    assert!(
        state.local_only_titles[0].contains("Local Song"),
        "RC15-DEF-3: local_only title should contain 'Local Song', got: {:?}",
        state.local_only_titles[0]
    );
    assert!(
        state.local_only_titles[0].contains("A"),
        "RC15-DEF-3: local_only title should contain artist 'A', got: {:?}",
        state.local_only_titles[0]
    );

    // v001 is not in the catalog → publishable. No yt_session, so the
    // title falls back to the raw id "v001".
    assert_eq!(state.publishable_ids.len(), 1);
    assert_eq!(state.publishable_ids[0], "v001");
    assert_eq!(
        state.publishable_titles.len(),
        1,
        "publishable_titles should be populated"
    );
    assert_eq!(
        state.publishable_titles[0], "v001",
        "RC15-DEF-3: uncached YouTube id falls back to raw id"
    );
}

/// The publish overlay render must show titles (not raw IDs) when titles are
/// available. Render the overlay and verify "Local Song" appears (for the
/// local-only track), not just "t1".
#[test]
fn rc15_def3_publish_overlay_render_shows_titles() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "My Mix".into(),
        track_ids: vec!["t1".into()],
    });
    app.open_publication("My Mix");
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("Local Song"),
        "RC15-DEF-3: publish overlay should show 'Local Song' (title), not 't1' (raw id). \
         Got:\n{buf}"
    );
}

// ---------------------------------------------------------------------------
// RC15-DEF-4 (D1): :yt setup zero feedback
// ---------------------------------------------------------------------------

/// `:yt setup` must immediately set `yt_status` to a "Setting up…" toast
/// AND spawn the blocking install on a background thread (so the TUI stays
/// responsive). The old code blocked synchronously — no render happened
/// before the 30s install, so the user saw nothing.
#[test]
fn rc15_def4_yt_setup_sets_status_toast_immediately() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Before: yt_status is None.
    assert!(app.yt_status.is_none(), "fixture: no status before setup");
    assert!(
        app.yt_setup_handle.is_none(),
        "fixture: no setup handle before setup"
    );
    // Call yt_setup — must return immediately (non-blocking).
    app.yt_setup();
    // After: yt_status must be set to a "Setting up…" toast.
    let status = app.yt_status.as_deref().unwrap_or("");
    assert!(
        status.contains("Setting up") || status.contains("setting up"),
        "RC15-DEF-4: yt_setup must immediately set a 'Setting up…' toast. Got: {status:?}"
    );
    // The setup handle must be set (background thread spawned).
    assert!(
        app.yt_setup_handle.is_some(),
        "RC15-DEF-4: yt_setup must spawn the install on a background thread"
    );
    // Clean up: wait for the thread to finish (don't leave it running).
    if let Some(handle) = app.yt_setup_handle.take() {
        let _ = handle.join();
    }
}

/// The "Setting up…" toast must be visible in the footer render immediately
/// after `yt_setup()` returns (before the thread finishes).
#[test]
fn rc15_def4_yt_setup_toast_visible_in_render() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_setup();
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("Setting up") || buf.contains("setting up"),
        "RC15-DEF-4: footer must show 'Setting up…' immediately after yt_setup. \
         Got:\n{buf}"
    );
    // Clean up.
    if let Some(handle) = app.yt_setup_handle.take() {
        let _ = handle.join();
    }
}

// ---------------------------------------------------------------------------
// RC15-DEF-5 (ACC-02): 80x24 status bar long title hard-cut, no ellipsis
// ---------------------------------------------------------------------------

/// A catalog fixture with a long-titled track for testing truncation.
fn long_title_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let long_title = "A Very Long Track Title That Should Be Truncated When Displayed In Narrow Terminals And Miller Columns";
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":long_title,
        "album":"Al","bit_depth":16,"sample_rate_hz":44100,
        "source_path":"lossless/A/01.flac","symlinked_into_artists":["A"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// At 80x24, playing a long-titled track must show "…" at the cut point in
/// the compact player bar (not a hard mid-word cut). The old bug: the compact
/// bar pushed the title as a raw span with no truncation, so it was hard-cut
/// at the terminal width with no ellipsis.
#[test]
fn rc15_def5_80x24_long_title_shows_ellipsis() {
    let _xdg = isolate_xdg();
    let (_d, cat) = long_title_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Play the long-titled track.
    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert!(
        app.now_playing.is_some(),
        "fixture: track should be playing"
    );

    // Render at 80x24 (compact bar, height 1).
    let buf = rendered(&mut app, 80, 24);

    // The player bar is the last row before the footer. Find the row
    // containing "[PLAYING]" (the compact bar's state label).
    let bar_line = buf
        .lines()
        .find(|l| l.contains("[PLAYING]"))
        .map(|l| l.to_string())
        .unwrap_or_else(|| {
            // If not found, try [STOPPED] or [PAUSED].
            buf.lines()
                .find(|l| {
                    l.contains("[STOPPED]") || l.contains("[PAUSED]") || l.contains("[BUFFERING]")
                })
                .map(|l| l.to_string())
                .unwrap_or_default()
        });

    // The bar must contain the ellipsis character `…` (U+2026) or `...`
    // (ASCII mode). Under default test env, font_mode is Unicode, so `…`.
    assert!(
        bar_line.contains('\u{2026}') || bar_line.contains("..."),
        "RC15-DEF-5 (ACC-02): at 80x24, long title must be truncated with ellipsis (…), \
         not hard-cut. Bar line: {bar_line}"
    );
}

/// At 100x30 the full bar already truncates correctly — this test verifies
/// the fix doesn't regress the wide path.
#[test]
fn rc15_def5_100x30_long_title_still_truncates() {
    let _xdg = isolate_xdg();
    let (_d, cat) = long_title_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let buf = rendered(&mut app, 100, 30);
    let bar_line = buf
        .lines()
        .find(|l| l.contains("[PLAYING]"))
        .map(|l| l.to_string())
        .unwrap_or_default();
    assert!(
        bar_line.contains('\u{2026}') || bar_line.contains("..."),
        "RC15-DEF-5: 100x30 should still truncate long titles with ellipsis. \
         Bar line: {bar_line}"
    );
}
