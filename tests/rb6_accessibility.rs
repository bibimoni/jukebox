//! RB-6 regression tests: required accessibility fallbacks.
//!
//! - NO_COLOR=1: selected row has a visible non-color cue (marker glyph +
//!   REVERSED/BOLD modifier).
//! - TERM=dumb / ASCII locale: FontMode::auto_detect returns Ascii; the event
//!   loop refuses raw mode + alt-screen (no ANSI controls emitted).
//! - yt_status_line: long status/recovery lines don't clip the recovery
//!   message at 80-wide — the recovery keyword stays visible.

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, View, YtList, YtListKind};
use jukebox::tui::view::columns::yt_status_line_pub;
use jukebox::tui::view::icons::FontMode;
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, style::Modifier, Terminal};
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

fn rendered_with_term(app: &mut App, w: u16, h: u16) -> (String, Terminal<TestBackend>) {
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

/// Serializes tests that set/unset env vars so they don't interfere with each
/// other under parallel test execution.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set `JUKEBOX_FONT_MODE` to `mode` (or clear it when `None`) and reset the
/// per-thread font-mode cache so the next `is_ascii()` / `ascii_sanitize`
/// call observes the new mode. Pair with `env_lock()` so parallel tests don't
/// race on the env var.
fn set_font_mode(mode: Option<&str>) {
    match mode {
        Some(v) => std::env::set_var("JUKEBOX_FONT_MODE", v),
        None => std::env::remove_var("JUKEBOX_FONT_MODE"),
    }
    jukebox::tui::view::theme::reset_font_mode_cache();
}

// ---------------------------------------------------------------------------
// NO_COLOR: selected track row has a visible non-color cue
// ---------------------------------------------------------------------------

/// Under NO_COLOR=1, the selected track in the Tracks column must have a
/// visible marker glyph (▸ or >) so the selection is identifiable without
/// color. The marker is a text-visible cue that doesn't depend on REVERSED
/// or color.
#[test]
fn rb6_no_color_track_selection_has_marker_glyph() {
    let _guard = env_lock();
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2; // focus tracks column
    app.cursors.track = 0; // select first track

    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let buf = rendered(&mut app, 80, 24);
    std::env::remove_var("NO_COLOR");
    jukebox::tui::view::theme::reset_font_mode_cache();

    // The selected track row must contain a marker glyph (▸ in Unicode mode,
    // > in ASCII mode). Under NO_COLOR without JUKEBOX_FONT_MODE, font_mode is
    // Unicode, so the marker is ▸ (U+25B8).
    assert!(
        buf.contains('\u{25B8}') || buf.contains('>'),
        "RB-6: NO_COLOR selected track must have a visible marker glyph (▸ or >): {buf}"
    );
}

/// Under NO_COLOR=1, the selected track row must carry the REVERSED modifier
/// (reverse video) as a non-color cue. This is a style-level cue that doesn't
/// depend on color.
#[test]
fn rb6_no_color_track_selection_has_reversed_modifier() {
    let _guard = env_lock();
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.track = 0;

    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let (_, term) = rendered_with_term(&mut app, 80, 24);
    std::env::remove_var("NO_COLOR");
    jukebox::tui::view::theme::reset_font_mode_cache();

    // Find the marker glyph in the buffer and check that cells on the
    // selected row carry the REVERSED modifier.
    let mut found_reversed = false;
    for y in 0..24 {
        for x in 0..80 {
            let cell = &term.backend().buffer()[(x, y)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if (ch == '\u{25B8}' || ch == '>')
                && (cell.modifier.contains(Modifier::REVERSED)
                    || cell.modifier.contains(Modifier::BOLD))
            {
                found_reversed = true;
            }
        }
    }
    assert!(
        found_reversed,
        "RB-6: NO_COLOR selected track marker must carry REVERSED or BOLD modifier"
    );
}

/// Under NO_COLOR=1 + JUKEBOX_FONT_MODE=ascii, the selected track marker must
/// be the ASCII `>` character (not a Unicode glyph).
#[test]
fn rb6_no_color_ascii_mode_track_selection_has_ascii_marker() {
    let _guard = env_lock();
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.track = 0;

    std::env::set_var("NO_COLOR", "1");
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let buf = rendered(&mut app, 80, 24);
    std::env::remove_var("NO_COLOR");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();

    // In ASCII mode, the marker must be `>`, not ▸.
    // Find the track row (contains "Song1") and check it starts with `>`.
    let track_line = buf
        .lines()
        .find(|l| l.contains("Song1"))
        .unwrap_or_else(|| panic!("track row not found: {buf}"));
    assert!(
        track_line.contains('>') && !track_line.contains('\u{25B8}'),
        "RB-6: ASCII mode selected track must use > marker, not ▸: {track_line}"
    );
}

// ---------------------------------------------------------------------------
// TERM=dumb / ASCII locale: FontMode::auto_detect returns Ascii
// ---------------------------------------------------------------------------

/// Under TERM=dumb, FontMode::auto_detect must return Ascii so all glyphs
/// use ASCII labels (no Unicode glyphs that a dumb terminal can't render).
#[test]
fn rb6_font_mode_auto_detect_returns_ascii_for_term_dumb() {
    let _guard = env_lock();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::set_var("TERM", "dumb");
    std::env::remove_var("TERM_FONT");
    let mode = FontMode::auto_detect();
    std::env::remove_var("TERM");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Ascii,
        "RB-6: TERM=dumb must produce FontMode::Ascii (got {mode:?})"
    );
}

/// Under LC_ALL=C (ASCII locale), FontMode::auto_detect must return Ascii.
#[test]
fn rb6_font_mode_auto_detect_returns_ascii_for_lc_all_c() {
    let _guard = env_lock();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("LC_ALL", "C");
    std::env::remove_var("TERM_FONT");
    let mode = FontMode::auto_detect();
    std::env::remove_var("LC_ALL");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Ascii,
        "RB-6: LC_ALL=C must produce FontMode::Ascii (got {mode:?})"
    );
}

/// Under LANG=C (ASCII locale), FontMode::auto_detect must return Ascii.
#[test]
fn rb6_font_mode_auto_detect_returns_ascii_for_lang_c() {
    let _guard = env_lock();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("LANG", "C");
    std::env::remove_var("LC_ALL");
    std::env::remove_var("LC_CTYPE");
    std::env::remove_var("TERM_FONT");
    let mode = FontMode::auto_detect();
    std::env::remove_var("LANG");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Ascii,
        "RB-6: LANG=C must produce FontMode::Ascii (got {mode:?})"
    );
}

/// Under LANG=POSIX, FontMode::auto_detect must return Ascii.
#[test]
fn rb6_font_mode_auto_detect_returns_ascii_for_lang_posix() {
    let _guard = env_lock();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("LANG", "POSIX");
    std::env::remove_var("LC_ALL");
    std::env::remove_var("LC_CTYPE");
    std::env::remove_var("TERM_FONT");
    let mode = FontMode::auto_detect();
    std::env::remove_var("LANG");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Ascii,
        "RB-6: LANG=POSIX must produce FontMode::Ascii (got {mode:?})"
    );
}

/// Under LANG=C.UTF-8, FontMode::auto_detect must return Unicode (C.UTF-8
/// supports Unicode — it is NOT an ASCII locale). This guards against
/// false positives on common modern systems.
#[test]
fn rb6_font_mode_auto_detect_returns_unicode_for_c_utf8() {
    let _guard = env_lock();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("LANG", "C.UTF-8");
    std::env::remove_var("LC_ALL");
    std::env::remove_var("LC_CTYPE");
    std::env::remove_var("TERM_FONT");
    let mode = FontMode::auto_detect();
    std::env::remove_var("LANG");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Unicode,
        "RB-6: LANG=C.UTF-8 must NOT produce FontMode::Ascii (C.UTF-8 supports Unicode) (got {mode:?})"
    );
}

/// JUKEBOX_FONT_MODE overrides TERM=dumb (explicit user choice wins).
#[test]
fn rb6_font_mode_jukebox_override_wins_over_term_dumb() {
    let _guard = env_lock();
    std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
    std::env::set_var("TERM", "dumb");
    let mode = FontMode::auto_detect();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    std::env::remove_var("TERM");
    drop(_guard);
    assert_eq!(
        mode,
        FontMode::Unicode,
        "RB-6: JUKEBOX_FONT_MODE=unicode must override TERM=dumb (got {mode:?})"
    );
}

// ---------------------------------------------------------------------------
// TERM=dumb: event loop refuses raw mode + alt-screen
// ---------------------------------------------------------------------------

/// Under TERM=dumb, terminal_supports_raw_mode() must return false so the
/// event loop does NOT enable raw mode / alt-screen / mouse-capture (which
/// emit ANSI controls a dumb terminal can't render).
#[test]
fn rb6_terminal_supports_raw_mode_false_for_dumb() {
    let _guard = env_lock();
    std::env::set_var("TERM", "dumb");
    let supports = jukebox::tui::event::terminal_supports_raw_mode();
    std::env::remove_var("TERM");
    drop(_guard);
    assert!(
        !supports,
        "RB-6: terminal_supports_raw_mode() must return false for TERM=dumb"
    );
}

/// Under a normal TERM (xterm), terminal_supports_raw_mode() must return true.
#[test]
fn rb6_terminal_supports_raw_mode_true_for_xterm() {
    let _guard = env_lock();
    std::env::set_var("TERM", "xterm-256color");
    let supports = jukebox::tui::event::terminal_supports_raw_mode();
    std::env::remove_var("TERM");
    drop(_guard);
    assert!(
        supports,
        "RB-6: terminal_supports_raw_mode() must return true for TERM=xterm-256color"
    );
}

/// Under no TERM set, terminal_supports_raw_mode() must return true (assume
/// raw-capable — the common case for most terminal emulators).
#[test]
fn rb6_terminal_supports_raw_mode_true_when_term_unset() {
    let _guard = env_lock();
    std::env::remove_var("TERM");
    let supports = jukebox::tui::event::terminal_supports_raw_mode();
    drop(_guard);
    assert!(
        supports,
        "RB-6: terminal_supports_raw_mode() must return true when TERM is unset"
    );
}

// ---------------------------------------------------------------------------
// yt_status_line: long retry_hint not clipped to invisibility at 80 wide
// ---------------------------------------------------------------------------

/// A long yt_error message must not push the recovery guidance off-screen.
/// The yt_status_line for error states with a long yt_error must keep the
/// recovery keyword (e.g. "retry") visible within the first 78 chars so it
/// fits at 80-wide terminals. RB-6/F1: the assertion must hold in BOTH
/// Unicode and ASCII font modes — in ASCII mode the em-dash `—` expands to
/// `--` and the ellipsis `…` expands to `...`, which previously inflated the
/// truncated line to 80 chars (clip at 80x24).
#[test]
fn rb6_yt_status_line_long_error_keeps_recovery_visible() {
    let _guard = env_lock();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::ProviderError;
    app.yt_error = Some(
        "retry failed: HTTPError 500: Internal Server Error — the server returned an unexpected response that indicates a transient infrastructure issue, please try again later"
            .to_string(),
    );

    let check = |mode: &str| {
        let line = yt_status_line_pub(&app, true, true);
        // The recovery keyword must be present in the first 78 chars.
        let head: String = line.chars().take(78).collect();
        assert!(
            head.contains("retry") || head.contains("press R"),
            "RB-6 ({mode}): recovery keyword must be visible in the first 78 chars of a long error status line: {line}"
        );
        // The total line must not exceed 78 chars (fits at 80-wide with 2 border chars).
        assert!(
            line.chars().count() <= 78,
            "RB-6 ({mode}): yt_status_line must be ≤78 chars for 80-wide terminals, got {} chars: {line}",
            line.chars().count()
        );
    };

    // Unicode font mode (default).
    set_font_mode(None);
    check("unicode");
    // ASCII font mode (JUKEBOX_FONT_MODE=ascii / TERM=dumb accessibility target).
    set_font_mode(Some("ascii"));
    check("ascii");
    // Restore.
    set_font_mode(None);
}

/// The ReadyStale status line ("offline — showing cached lists (press R to
/// retry)") must keep the recovery keyword visible at 80 wide. The line must
/// not exceed 78 chars. Asserted in BOTH Unicode and ASCII font modes (F1:
/// ASCII glyph expansion must not inflate the line).
#[test]
fn rb6_yt_status_line_ready_stale_fits_80_wide() {
    let _guard = env_lock();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::ReadyStale;

    let check = |mode: &str| {
        let line = yt_status_line_pub(&app, false, true);
        assert!(
            line.chars().count() <= 78,
            "RB-6 ({mode}): ReadyStale status line must be ≤78 chars, got {}: {line}",
            line.chars().count()
        );
        assert!(
            line.contains("retry") || line.contains("R"),
            "RB-6 ({mode}): ReadyStale status line must contain recovery keyword: {line}"
        );
    };

    set_font_mode(None);
    check("unicode");
    set_font_mode(Some("ascii"));
    check("ascii");
    set_font_mode(None);
}

/// The Unconfigured status line must fit at 80 wide and keep the recovery
/// guidance visible. Asserted in BOTH Unicode and ASCII font modes (F1).
#[test]
fn rb6_yt_status_line_unconfigured_fits_80_wide() {
    let _guard = env_lock();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::Unconfigured;

    let check = |mode: &str| {
        let line = yt_status_line_pub(&app, true, true);
        assert!(
            line.chars().count() <= 78,
            "RB-6 ({mode}): Unconfigured status line must be ≤78 chars, got {}: {line}",
            line.chars().count()
        );
        assert!(
            line.contains(":yt") || line.contains("auth"),
            "RB-6 ({mode}): Unconfigured status line must contain recovery keyword: {line}"
        );
    };

    set_font_mode(None);
    check("unicode");
    set_font_mode(Some("ascii"));
    check("ascii");
    set_font_mode(None);
}

/// When rendered at 80x24, the YouTube view's status line for a ReadyStale
/// state (no tracks) must show the recovery keyword in the rendered buffer.
#[test]
fn rb6_yt_status_line_rendered_recovery_visible_at_80x24() {
    let _guard = env_lock();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::ReadyStale;
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "Liked Songs".into(),
        kind: YtListKind::Account,
        track_ids: vec![],
    }];
    app.cursors.playlist = 0;
    app.focus_col = 0;
    let buf = rendered(&mut app, 80, 24);
    // The recovery keyword must be visible in the rendered buffer.
    assert!(
        buf.contains("retry") || buf.contains("R"),
        "RB-6: ReadyStale recovery keyword must be visible at 80x24: {buf}"
    );
}
