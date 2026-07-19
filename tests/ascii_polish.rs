//! Phase 8 tests: tiny-terminal + ASCII leak fixes.
//!
//! Verifies that the hardcoded Unicode glyphs that previously leaked
//! through `JUKEBOX_FONT_MODE=ascii` now route through the theme's
//! `is_ascii()`-aware helpers (visual spec C5 / A3-A6 / V1 / V2), and
//! that the "terminal too small" message includes the current +
//! required dimensions (M44 / A14).

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::view::theme::{reset_font_mode_cache, Theme};
use ratatui::{backend::TestBackend, Terminal};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock().lock().unwrap_or_else(|e| e.into_inner())
}

/// Build a 2-artist catalog so overlays + panes have content to render.
fn cat_album() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
    std::fs::create_dir_all(lossless.join("DECO")).unwrap();
    std::fs::write(lossless.join("DECO").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song One","album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Song Two","album":"Ghost","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn build_app() -> App {
    let (_d, cat) = cat_album();
    std::mem::forget(_d);
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

// --- Theme::state_label respects ASCII mode (A3) ---------------------

#[test]
fn state_label_uses_unicode_glyphs_in_color_mode() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
    let t = Theme::default();
    assert_eq!(t.state_label(true, false), "[▶]", "color-mode playing");
    assert_eq!(t.state_label(false, true), "[⏸]", "color-mode paused");
    assert_eq!(t.state_label(false, false), "[■]", "color-mode stopped");
}

#[test]
fn state_label_uses_ascii_glyphs_in_ascii_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    reset_font_mode_cache();
    let t = Theme::default();
    assert_eq!(t.state_label(true, false), "[>]", "ascii-mode playing");
    assert_eq!(t.state_label(false, true), "[||]", "ascii-mode paused");
    assert_eq!(t.state_label(false, false), "[#]", "ascii-mode stopped");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
}

// --- Context::label respects ASCII mode (A4) --------------------------

#[test]
fn context_label_uses_unicode_bullet_in_color_mode() {
    use jukebox::tui::context::Context;

    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
    let ctx = Context::Playlist { name: "Mix".into() };
    // Phase 8: Context::label now uses theme::bullet() (`•` in Unicode
    // mode, `*` in ASCII mode) instead of the hardcoded `♫` (which
    // leaked into ASCII-only terminals and was decorative — visual
    // direction says "avoid decorative symbols that do not convey
    // state"). The bullet is more restrained and ASCII-safe.
    assert!(ctx.label().contains('•'), "color-mode: {}", ctx.label());
    let ctx2 = Context::Youtube {
        key: "Mix".into(),
        name: "Mix".into(),
    };
    assert!(
        ctx2.label().contains('•'),
        "color-mode yt: {}",
        ctx2.label()
    );
}

#[test]
fn context_label_uses_ascii_bullet_in_ascii_mode() {
    use jukebox::tui::context::Context;

    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    reset_font_mode_cache();
    let ctx = Context::Playlist { name: "Mix".into() };
    let label = ctx.label();
    assert!(
        !label.contains('♫'),
        "ascii-mode must not contain ♫: {label}"
    );
    // The bullet() helper returns `*` in ASCII mode.
    assert!(
        label.contains('*'),
        "ascii-mode should use `*` as bullet: {label}"
    );
    let ctx2 = Context::Youtube {
        key: "Mix".into(),
        name: "Mix".into(),
    };
    let label2 = ctx2.label();
    assert!(
        !label2.contains('♫'),
        "ascii-mode yt must not contain ♫: {label2}"
    );
    assert!(
        label2.contains('*'),
        "ascii-mode yt should use `*` as bullet: {label2}"
    );
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
}

// --- Minimal playback shell below the browser floor ------------------

#[test]
fn minimal_playback_shell_replaces_resize_only_screen_at_58x20() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    app.view = jukebox::tui::app::View::Artists;

    // Render at 58×20 — below the 60×20 browser minimum but large enough
    // for the borderless Minimal deck required by the responsive design.
    let backend = TestBackend::new(58, 20);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();
    let body: String = (0..20)
        .flat_map(|y| (0..58).map(move |x| buf[(x, y)].symbol()))
        .collect();
    assert!(
        body.contains("Library hidden") && body.contains("nothing playing"),
        "minimal shell should preserve playback while explaining the hidden browser: {body}"
    );
    assert!(
        body.contains("VOL 70%") && body.contains("VIEW: local"),
        "minimal shell should preserve volume and source status: {body}"
    );
}

#[test]
fn too_small_message_does_not_panic_at_extreme_sizes() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    let mut app = build_app();
    app.view = jukebox::tui::app::View::Artists;

    // No panic at any size — the message just renders (or doesn't).
    for &(w, h) in &[(1, 1), (5, 1), (20, 5), (40, 15), (58, 20), (59, 20)] {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
            .unwrap();
    }
}

// --- Now-Playing panel transport respects ASCII mode (A5 / V1) -------
//
// This test verifies the theme helpers used by the Now-Playing transport
// row (`prev_glyph`, `play_glyph`, `next_glyph`) return ASCII glyphs
// under `JUKEBOX_FONT_MODE=ascii`. The integration test (rendering the
// full app + Now-Playing panel) is harder to set up without a real
// catalog with a currently-playing track, but the helper-level test
// pins the contract: if these helpers return ASCII, the transport row
// renders ASCII (it builds the string from the helpers).

#[test]
fn now_playing_transport_helpers_use_ascii_in_ascii_mode() {
    use jukebox::tui::view::theme::{next_glyph, play_glyph, prev_glyph};

    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    reset_font_mode_cache();
    assert_eq!(prev_glyph(), "<<", "ascii-mode prev_glyph");
    assert_eq!(play_glyph(), ">", "ascii-mode play_glyph");
    assert_eq!(next_glyph(), ">>", "ascii-mode next_glyph");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
}

#[test]
fn now_playing_transport_helpers_use_unicode_in_color_mode() {
    use jukebox::tui::view::theme::{next_glyph, play_glyph, prev_glyph};

    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
    assert_eq!(prev_glyph(), "◀◀", "color-mode prev_glyph");
    assert_eq!(play_glyph(), "▶", "color-mode play_glyph");
    assert_eq!(next_glyph(), "▶▶", "color-mode next_glyph");
}

// --- Generator marker respects ASCII mode (A6) -----------------------

#[test]
fn generator_marker_uses_ascii_in_ascii_mode() {
    use jukebox::tui::view::theme::marker_glyph;

    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    reset_font_mode_cache();
    assert_eq!(marker_glyph(), ">", "ascii-mode marker_glyph");
    std::env::remove_var("JUKEBOX_FONT_MODE");
    reset_font_mode_cache();
}
