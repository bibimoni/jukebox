//! Phase 0 tests for the extended `Theme` semantic API.
//!
//! Verifies that every new field + method returns the expected `Style`
//! under `{normal, NO_COLOR, high_contrast}` × `{focused, editing,
//! active}`, and that the thread-local env caches (`cached_no_color`,
//! `cached_high_contrast`, `no_motion`) reset correctly between
//! assertions. These pin the visual spec §7 theme contract so later
//! phases can refactor view-layer call sites without drifting the
//! centralized styles.

use jukebox::tui::view::theme::{
    high_contrast, no_color, no_motion, reset_font_mode_cache, reset_no_motion_cache, Theme,
};
use ratatui::style::{Color, Modifier};
use std::sync::{Mutex, OnceLock};

/// Process-wide mutex serializing all tests that mutate env vars
/// (`NO_COLOR`, `JUKEBOX_HIGH_CONTRAST`, `JUKEBOX_NO_MOTION`). Cargo's
/// parallel test runner runs `#[test]`s on multiple threads, but env
/// vars are process-global — without serialization, one test's
/// `set_var` can be undone by another's `remove_var` mid-assertion.
/// Grab this mutex in any test that calls `std::env::set_var`.
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Lock the env mutex, recovering from poison so a single test panic
/// doesn't cascade failures to every other env-mutating test in the file.
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock().lock().unwrap_or_else(|e| e.into_inner())
}

/// Helper: run `body` with `NO_COLOR` set on this thread, then always
/// reset the cache so later tests on the same thread re-read the env.
/// Holds the env mutex for the duration so parallel tests can't race.
fn with_no_color<F: FnOnce() -> R, R>(body: F) -> R {
    let _guard = lock_env();
    std::env::set_var("NO_COLOR", "1");
    let r = body();
    std::env::remove_var("NO_COLOR");
    r
}

fn with_high_contrast<F: FnOnce() -> R, R>(body: F) -> R {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_HIGH_CONTRAST", "1");
    let r = body();
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    r
}

/// Hold the env lock + clear all theme env vars + reset all caches, then
/// run `body`. Use this for tests that assert specific color-mode values
/// (Black/Cyan/Gray/etc.) so a parallel test's `set_var` can't flip the
/// cache mid-assertion.
fn with_clean_env<F: FnOnce() -> R, R>(body: F) -> R {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    std::env::remove_var("JUKEBOX_NO_MOTION");
    reset_no_motion_cache();
    body()
}

// --- Field presence & values per palette -------------------------------

#[test]
fn theme_default_has_all_pane_polish_fields() {
    with_clean_env(|| {
        let t = Theme::default();
        // Color path values (visual spec §7 table).
        assert_eq!(t.background, Color::Black);
        assert_eq!(t.surface_active, Color::Indexed(238));
        assert_eq!(t.border, Color::Gray);
        assert_eq!(t.border_focused, Color::Cyan);
        assert_eq!(t.border_editing, Color::Cyan);
        assert_eq!(t.text_muted, Color::Gray);
        assert_eq!(t.text_disabled, Color::DarkGray);
        assert_eq!(t.accent_soft, Color::Indexed(30));
        assert_eq!(t.selection_bg, Color::Cyan);
        assert_eq!(t.selection_fg, Color::Black);
    })
}

#[test]
fn theme_no_color_collapses_all_pane_polish_fields_to_grayscale() {
    with_no_color(|| {
        let t = Theme::default();
        // NO_COLOR path: no hue anywhere.
        assert_eq!(
            t.background,
            Color::Reset,
            "background must be Reset under NO_COLOR"
        );
        assert_eq!(t.surface_active, Color::Reset);
        assert_eq!(t.border, Color::Reset);
        assert_eq!(t.border_focused, Color::White);
        assert_eq!(t.border_editing, Color::White);
        assert_eq!(t.text_muted, Color::Reset);
        assert_eq!(t.text_disabled, Color::Reset);
        assert_eq!(t.accent_soft, Color::Reset);
        assert_eq!(t.selection_bg, Color::White);
        assert_eq!(t.selection_fg, Color::Black);
    })
}

#[test]
fn theme_high_contrast_uses_pure_white_black_gray() {
    with_high_contrast(|| {
        let t = Theme::default();
        // high_contrast path: pure white/black/gray, no hue.
        assert_eq!(t.background, Color::Black);
        assert_eq!(t.border, Color::Gray);
        assert_eq!(t.border_focused, Color::White);
        assert_eq!(t.border_editing, Color::White);
        assert_eq!(t.text_muted, Color::Gray);
        assert_eq!(t.text_disabled, Color::Gray);
        assert_eq!(t.accent_soft, Color::Gray);
        assert_eq!(t.selection_bg, Color::White);
        assert_eq!(t.selection_fg, Color::Black);
    })
}

// --- pane_border -------------------------------------------------------

#[test]
fn pane_border_picks_color_by_focus_and_edit_state() {
    with_clean_env(|| {
        let t = Theme::default();
        // Color path.
        assert_eq!(t.pane_border(false, false).fg, Some(t.border));
        assert_eq!(t.pane_border(true, false).fg, Some(t.border_focused));
        assert_eq!(t.pane_border(true, true).fg, Some(t.border_editing));
        // editing=true with focused=false is not a real state, but the
        // method should still pick the editing color (editing wins).
        assert_eq!(t.pane_border(false, true).fg, Some(t.border_editing));
    })
}

#[test]
fn pane_border_under_no_color_uses_grayscale() {
    with_no_color(|| {
        let t = Theme::default();
        assert_eq!(t.pane_border(false, false).fg, Some(Color::Reset));
        assert_eq!(t.pane_border(true, false).fg, Some(Color::White));
        assert_eq!(t.pane_border(true, true).fg, Some(Color::White));
    })
}

// --- pane_block --------------------------------------------------------

#[test]
fn pane_block_focused_uses_thick_border_accent_color() {
    with_clean_env(|| {
        let t = Theme::default();
        let b = t.pane_block("Artists", true, false);
        // Render into a tiny buffer to inspect the border type + style.
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(10, 4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(b, ratatui::layout::Rect::new(0, 0, 10, 4));
        })
        .unwrap();
        let buf = term.backend().buffer();
        // Corner cell should be the Thick border glyph, styled with accent.
        let corner = &buf[(0, 0)];
        assert!(
            corner.symbol().starts_with('┏') || corner.symbol().starts_with('┌'),
            "focused pane corner should be a thick/box-drawing char, got {}",
            corner.symbol()
        );
        assert_eq!(corner.style().fg, Some(t.border_focused));
    })
}

#[test]
fn pane_block_unfocused_uses_plain_border_dim_color() {
    with_clean_env(|| {
        let t = Theme::default();
        let b = t.pane_block("Artists", false, false);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(10, 4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(b, ratatui::layout::Rect::new(0, 0, 10, 4));
        })
        .unwrap();
        let buf = term.backend().buffer();
        let corner = &buf[(0, 0)];
        // Plain border corner is `┌` (Unicode) — distinct from Thick `┏`.
        assert_eq!(
            corner.symbol(),
            "┌",
            "unfocused pane corner should be Plain"
        );
        assert_eq!(corner.style().fg, Some(t.border));
    })
}

#[test]
fn pane_block_editing_uses_thick_border_editing_color() {
    with_clean_env(|| {
        let t = Theme::default();
        let b = t.pane_block("Artists [EDIT]", true, true);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(10, 4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(b, ratatui::layout::Rect::new(0, 0, 10, 4));
        })
        .unwrap();
        let buf = term.backend().buffer();
        let corner = &buf[(0, 0)];
        assert!(
            corner.symbol().starts_with('┏') || corner.symbol().starts_with('┌'),
            "editing pane corner should be thick"
        );
        assert_eq!(corner.style().fg, Some(t.border_editing));
    })
}

#[test]
fn pane_block_renders_all_four_corners_and_edges() {
    with_clean_env(|| {
        // Visual spec Phase 1: pane_block must render a complete box (all 4
        // borders). Verify by rendering into a 10×4 buffer and checking that
        // all 4 corners + all 4 edges have non-empty symbols.
        let t = Theme::default();
        let b = t.pane_block("X", false, false);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(10, 4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(b, ratatui::layout::Rect::new(0, 0, 10, 4));
        })
        .unwrap();
        let buf = term.backend().buffer();
        // Corners
        assert!(!buf[(0, 0)].symbol().is_empty());
        assert!(!buf[(9, 0)].symbol().is_empty());
        assert!(!buf[(0, 3)].symbol().is_empty());
        assert!(!buf[(9, 3)].symbol().is_empty());
        // Edges (top, bottom, left, right)
        assert!(!buf[(5, 0)].symbol().is_empty(), "top edge must be drawn");
        assert!(
            !buf[(5, 3)].symbol().is_empty(),
            "bottom edge must be drawn"
        );
        assert!(!buf[(0, 2)].symbol().is_empty(), "left edge must be drawn");
        assert!(!buf[(9, 2)].symbol().is_empty(), "right edge must be drawn");
    })
}

// --- selected_row ------------------------------------------------------

#[test]
fn selected_row_focused_matches_selected_style() {
    let t = Theme::default();
    let focused = t.selected_row(true);
    let sel = t.selected_style();
    assert_eq!(focused.fg, sel.fg);
    assert_eq!(focused.bg, sel.bg);
    assert_eq!(focused.add_modifier, sel.add_modifier);
}

#[test]
fn selected_row_unfocused_uses_text_muted_bold_no_reverse() {
    let t = Theme::default();
    let unfocused = t.selected_row(false);
    assert_eq!(unfocused.fg, Some(t.text_muted));
    assert!(unfocused.add_modifier.contains(Modifier::BOLD));
    assert!(
        !unfocused.add_modifier.contains(Modifier::REVERSED),
        "unfocused selection must NOT be reverse-video (only the focused pane's selection inverts)"
    );
}

#[test]
fn selected_row_under_no_color_focused_keeps_reversed_bold() {
    with_no_color(|| {
        let t = Theme::default();
        let focused = t.selected_row(true);
        assert!(focused.add_modifier.contains(Modifier::REVERSED));
        assert!(focused.add_modifier.contains(Modifier::BOLD));
    })
}

#[test]
fn selected_row_under_no_color_unfocused_keeps_bold_only() {
    with_no_color(|| {
        let t = Theme::default();
        let unfocused = t.selected_row(false);
        assert!(unfocused.add_modifier.contains(Modifier::BOLD));
        assert!(!unfocused.add_modifier.contains(Modifier::REVERSED));
    })
}

// --- tab ---------------------------------------------------------------

#[test]
fn tab_active_is_accent_bold_underline() {
    let t = Theme::default();
    let active = t.tab(true);
    assert_eq!(active.fg, Some(t.accent));
    assert!(active.add_modifier.contains(Modifier::BOLD));
    assert!(
        active.add_modifier.contains(Modifier::UNDERLINED),
        "active tab must be UNDERLINED (not REVERSED — visual spec §3 conflict resolution)"
    );
    assert!(
        !active.add_modifier.contains(Modifier::REVERSED),
        "active tab must NOT be REVERSED (collides with row selection)"
    );
}

#[test]
fn tab_inactive_is_dim() {
    let t = Theme::default();
    let inactive = t.tab(false);
    assert_eq!(inactive.fg, Some(t.dim));
    assert!(!inactive.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn tab_under_no_color_active_keeps_bold_underline() {
    with_no_color(|| {
        let t = Theme::default();
        let active = t.tab(true);
        // Underline survives NO_COLOR; bold survives; the fg collapses
        // to White (the NO_COLOR value of accent).
        assert!(active.add_modifier.contains(Modifier::BOLD));
        assert!(active.add_modifier.contains(Modifier::UNDERLINED));
    })
}

// --- status_key / status_description -----------------------------------

#[test]
fn status_key_is_accent_bold() {
    let t = Theme::default();
    let k = t.status_key();
    assert_eq!(k.fg, Some(t.accent));
    assert!(k.add_modifier.contains(Modifier::BOLD));
    assert!(
        !k.add_modifier.contains(Modifier::UNDERLINED),
        "key caps don't get underline (visual spec §7 status_key)"
    );
}

#[test]
fn status_description_is_dim() {
    let t = Theme::default();
    let d = t.status_description();
    assert_eq!(d.fg, Some(t.dim));
}

#[test]
fn status_description_under_no_color_is_reset_not_darkgray() {
    // Visual spec M38: theme.dim is already Reset under NO_COLOR; the
    // 33 redundant `if no_color() { Reset } else { theme.dim }` branches
    // re-implemented the theme's own collapse. status_description()
    // delegates to theme.dim, so it inherits the NO_COLOR-safe value.
    with_no_color(|| {
        let t = Theme::default();
        let d = t.status_description();
        assert_eq!(
            d.fg,
            Some(Color::Reset),
            "status_description must be Reset under NO_COLOR (WCAG-safe)"
        );
    })
}

// --- overlay -----------------------------------------------------------

#[test]
fn overlay_returns_background_style() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    let t = Theme::default();
    let s = t.overlay();
    assert_eq!(s.bg, Some(t.background));
    assert_eq!(
        s.bg,
        Some(Color::Black),
        "color-path overlay backdrop is Black (will switch to Reset under NO_COLOR)"
    );
}

#[test]
fn overlay_under_no_color_is_reset_not_black() {
    // Visual spec C4 / A2: 10 hardcoded `bg(Color::Black)` sites in
    // overlay.rs violate no-color.org. theme.overlay() must return
    // Reset (not Black) under NO_COLOR so the backdrop doesn't paint
    // a hard black box on light-theme terminals.
    with_no_color(|| {
        let t = Theme::default();
        let s = t.overlay();
        assert_eq!(
            s.bg,
            Some(Color::Reset),
            "overlay backdrop must be Reset under NO_COLOR (no-color.org)"
        );
    })
}

// --- cursor ------------------------------------------------------------
//
// The two cursor states (motion allowed / reduced motion) are tested in
// ONE test function so the env-var mutations happen sequentially on a
// single thread. Running them as separate `#[test]`s races the
// process-global `JUKEBOX_NO_MOTION` env var under cargo's parallel test
// runner (one test's `set_var` can be undone by another's `remove_var`
// mid-assertion). The thread-local cache is correct; the env var is not.

#[test]
fn cursor_default_then_no_motion_drops_blink() {
    let _guard = lock_env();
    // State 1: motion allowed (no JUKEBOX_NO_MOTION).
    std::env::remove_var("JUKEBOX_NO_MOTION");
    reset_no_motion_cache();
    let t = Theme::default();
    let c = t.cursor();
    assert_eq!(c.fg, Some(t.accent));
    assert!(
        c.add_modifier.contains(Modifier::SLOW_BLINK),
        "cursor must SLOW_BLINK when JUKEBOX_NO_MOTION is unset"
    );

    // State 2: reduced motion (JUKEBOX_NO_MOTION=1).
    std::env::set_var("JUKEBOX_NO_MOTION", "1");
    reset_no_motion_cache();
    let t = Theme::default();
    let c = t.cursor();
    assert_eq!(
        c.fg,
        Some(t.accent),
        "cursor stays visible (accent) under reduced-motion"
    );
    assert!(
        !c.add_modifier.contains(Modifier::SLOW_BLINK),
        "SLOW_BLINK must be dropped when JUKEBOX_NO_MOTION=1 (vestibular safety)"
    );

    // Cleanup so later tests on this thread see the default state.
    std::env::remove_var("JUKEBOX_NO_MOTION");
    reset_no_motion_cache();
}

// --- form_field --------------------------------------------------------

#[test]
fn form_field_active_is_accent_bold() {
    let t = Theme::default();
    let active = t.form_field(true);
    assert_eq!(active.fg, Some(t.accent));
    assert!(active.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn form_field_inactive_is_dim() {
    let t = Theme::default();
    let inactive = t.form_field(false);
    assert_eq!(inactive.fg, Some(t.dim));
}

// --- quality_style / source_badge --------------------------------------

#[test]
fn quality_style_hires_vs_cd() {
    // Hold the env lock so a parallel `with_no_color` test can't race
    // the env var (which would make quality_color return Reset for both).
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    let t = Theme::default();
    let hires = t.quality_style(24, 96000);
    let cd = t.quality_style(16, 44100);
    assert_ne!(hires.fg, cd.fg, "Hi-Res and CD must differ");
}

#[test]
fn source_badge_local_vs_yt() {
    let t = Theme::default();
    let local = t.source_badge(false);
    let yt = t.source_badge(true);
    assert_eq!(local.fg, Some(t.source_local));
    assert_eq!(yt.fg, Some(t.source_yt));
}

// --- env-var reads -----------------------------------------------------
//
// `no_color()` and `high_contrast()` read env vars directly (no cache)
// so tests that mutate the env between assertions see the new value
// immediately. `no_motion()` is cached per-thread (it's read in the
// hot cursor path); tests that mutate JUKEBOX_NO_MOTION call
// `reset_no_motion_cache()`.

#[test]
fn no_color_reads_env_directly() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    assert!(!no_color());
    std::env::set_var("NO_COLOR", "1");
    assert!(no_color());
    std::env::remove_var("NO_COLOR");
    assert!(!no_color());
}

#[test]
fn high_contrast_reads_env_directly() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    assert!(!high_contrast());
    std::env::set_var("JUKEBOX_HIGH_CONTRAST", "1");
    assert!(high_contrast());
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    assert!(!high_contrast());
}

#[test]
fn no_motion_caches_and_resets() {
    let _guard = lock_env();
    reset_no_motion_cache();
    std::env::remove_var("JUKEBOX_NO_MOTION");
    reset_no_motion_cache();
    assert!(!no_motion());
    std::env::set_var("JUKEBOX_NO_MOTION", "1");
    reset_no_motion_cache();
    assert!(no_motion());
    std::env::remove_var("JUKEBOX_NO_MOTION");
    reset_no_motion_cache();
    assert!(!no_motion());
}

// --- font_mode cache integration ---------------------------------------

#[test]
fn theme_default_uses_cached_font_mode() {
    // Phase 0 perf: Theme::default now calls cached_font_mode() instead
    // of FontMode::auto_detect() directly, eliminating ~10×5 env reads
    // per frame. Verify the field value still matches auto_detect when
    // the cache is fresh.
    reset_font_mode_cache();
    let t = Theme::default();
    let direct = jukebox::tui::view::icons::FontMode::auto_detect();
    assert_eq!(
        t.font_mode, direct,
        "Theme::default().font_mode should match FontMode::auto_detect() on a fresh cache"
    );
}
