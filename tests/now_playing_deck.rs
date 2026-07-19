#![allow(clippy::field_reassign_with_default)]

//! Tests for the Now Playing Deck (`src/tui/view/now_playing_deck`).
//!
//! Coverage:
//! - `Breakpoint` selection at the spec's 120/80/60 thresholds × varying
//!   heights.
//! - `Breakpoint::parse` / `as_str` round-trip.
//! - `progress::fmt_time` clamping (NaN, negative, infinity, zero, hour).
//! - `progress::khz` formatting (0, 44100, 48000, 96000, 192000).
//! - `progress::progress(app)` with a custom `Player` that returns NaN /
//!   negative / over-duration / zero-duration / None.
//! - `minimal::render` no-panic sweep at 1x1, 10x2, 20x2, 40x3, 59x10.
//! - `minimal::render` shows the play glyph, title, artist (truncated).
//! - `minimal::render` drops `VOL` below 30 cols, drops duration below 40
//!   cols, keeps artist (per spec: never drop artist/title/elapsed/
//!   play-pause).
//! - `render` (deck dispatch) no-panic sweep over every `Rect` from
//!   (0,0) to (180,40).
//! - `render` delegates to existing renderers at Wide / Medium / Compact
//!   (the title appears in the buffer).
//! - `PlayerTheme` construction under color / NO_COLOR / high-contrast.
//! - `spinner` ASCII / braille frames + `spinner_glyph` modulo.
//! - `DeckGeometry` at Minimal is all zero-sized rects.

use jukebox::catalog::Catalog;
use jukebox::player::{Player, StubPlayer};
use jukebox::tui::app::App;
use jukebox::tui::view::now_playing_deck::progress::{fmt_time, khz, progress};
use jukebox::tui::view::now_playing_deck::spinner::{
    is_buffering, spinner_glyph, SPINNER, SPINNER_ASCII,
};
use jukebox::tui::view::now_playing_deck::theme::PlayerTheme;
use jukebox::tui::view::now_playing_deck::{
    geometry, pick_breakpoint, render, render_for_breakpoint_with_focus, Breakpoint, DeckGeometry,
};
use jukebox::tui::view::theme::Theme;
use ratatui::{backend::TestBackend, layout::Rect, Terminal};
use std::path::Path;
use std::time::Duration;

// ---- env lock ----

/// Env-lock so tests that touch `JUKEBOX_FONT_MODE` / `NO_COLOR` /
/// `JUKEBOX_HIGH_CONTRAST` don't race each other. Mirrors the pattern in
/// `tests/rc18_fixes.rs:104`.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    jukebox::tui::view::theme::reset_font_mode_cache();
    jukebox::tui::view::theme::reset_no_motion_cache();
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- helpers ----

fn one_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
      {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn rendered<F>(w: u16, h: u16, render_fn: F) -> String
where
    F: FnOnce(&mut ratatui::Frame),
{
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_fn(f)).unwrap();
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

fn assert_transparent<F>(w: u16, h: u16, render_fn: F)
where
    F: FnOnce(&mut ratatui::Frame),
{
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_fn(f)).unwrap();
    for y in 0..h {
        for x in 0..w {
            let cell = &term.backend().buffer()[(x, y)];
            assert_eq!(
                cell.bg,
                ratatui::style::Color::Reset,
                "cell ({x},{y}) must retain terminal-default background"
            );
        }
    }
}

/// A `Player` whose `position` / `duration` return whatever the test
/// wants. Used to exercise the `progress` component's clamping logic.
struct FakePlayer {
    pos: Option<f64>,
    dur: Option<f64>,
    playing: bool,
}

impl Default for FakePlayer {
    fn default() -> Self {
        Self {
            pos: Some(0.0),
            dur: Some(180.0),
            playing: false,
        }
    }
}

impl Player for FakePlayer {
    fn load(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.playing = true;
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        self.playing = !self.playing;
        Ok(())
    }
    fn seek(&mut self, secs: f64) -> anyhow::Result<()> {
        self.pos = Some(secs);
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        self.playing = false;
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        self.pos
    }
    fn duration(&self) -> Option<f64> {
        self.dur
    }
    fn is_playing(&self) -> bool {
        self.playing
    }
}

fn app_with_track(player: Box<dyn Player>) -> App {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, player, None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app
}

// ---- Breakpoint selection ----

#[test]
fn breakpoint_pick_at_spec_thresholds() {
    // Wide: >= 120 width AND >= 30 height.
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 120, 30)), Breakpoint::Wide);
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 180, 50)), Breakpoint::Wide);
    // Below 30 height at 130 width → Medium.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 130, 29)),
        Breakpoint::Medium
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 200, 24)),
        Breakpoint::Medium
    );
    // Below 24 height at any width → Compact or Minimal.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 130, 23)),
        Breakpoint::Compact
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 130, 20)),
        Breakpoint::Compact
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 130, 19)),
        Breakpoint::Minimal
    );

    // Medium: 80-119 width AND >= 24 height.
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 80, 24)), Breakpoint::Medium);
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 119, 30)),
        Breakpoint::Medium
    );
    // Below 24 height at 90-129 → Compact or Minimal.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 90, 23)),
        Breakpoint::Compact
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 90, 19)),
        Breakpoint::Minimal
    );

    // Compact: 60-79 width AND >= 20 height.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 60, 20)),
        Breakpoint::Compact
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 79, 30)),
        Breakpoint::Compact
    );
    // Below 20 height at 60-89 → Minimal.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 60, 19)),
        Breakpoint::Minimal
    );
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 89, 1)), Breakpoint::Minimal);

    // Minimal: < 60 width OR < 20 height.
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 59, 30)),
        Breakpoint::Minimal
    );
    assert_eq!(
        pick_breakpoint(Rect::new(0, 0, 40, 20)),
        Breakpoint::Minimal
    );
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 1, 1)), Breakpoint::Minimal);
    assert_eq!(pick_breakpoint(Rect::new(0, 0, 0, 0)), Breakpoint::Minimal);
}

#[test]
fn breakpoint_parse_round_trips() {
    for bp in [
        Breakpoint::Wide,
        Breakpoint::Medium,
        Breakpoint::Compact,
        Breakpoint::Minimal,
    ] {
        assert_eq!(Breakpoint::parse(bp.as_str()), bp, "round-trip {:?}", bp);
    }
    // Unknown / empty falls back to Minimal (default).
    assert_eq!(Breakpoint::parse(""), Breakpoint::Minimal);
    assert_eq!(Breakpoint::parse("garbage"), Breakpoint::Minimal);
    assert_eq!(Breakpoint::default(), Breakpoint::Minimal);
}

#[test]
fn breakpoint_min_width_and_height() {
    assert_eq!(Breakpoint::Wide.min_width(), 120);
    assert_eq!(Breakpoint::Medium.min_width(), 80);
    assert_eq!(Breakpoint::Compact.min_width(), 60);
    assert_eq!(Breakpoint::Minimal.min_width(), 0);
    assert_eq!(Breakpoint::Wide.min_height(), 30);
    assert_eq!(Breakpoint::Medium.min_height(), 24);
    assert_eq!(Breakpoint::Compact.min_height(), 20);
    assert_eq!(Breakpoint::Minimal.min_height(), 1);
}

// ---- progress::fmt_time clamping ----

#[test]
fn fmt_time_zero() {
    assert_eq!(fmt_time(0.0), "0:00");
}

#[test]
fn fmt_time_seconds() {
    assert_eq!(fmt_time(5.0), "0:05");
    assert_eq!(fmt_time(65.0), "1:05");
    assert_eq!(fmt_time(125.0), "2:05");
}

#[test]
fn fmt_time_hour() {
    assert_eq!(fmt_time(3661.0), "1:01:01"); // 1h 1m 1s
    assert_eq!(fmt_time(3600.0), "1:00:00");
}

#[test]
fn fmt_time_negative_clamps_to_zero() {
    assert_eq!(fmt_time(-5.0), "0:00");
    assert_eq!(fmt_time(-0.1), "0:00");
}

#[test]
fn fmt_time_nan_clamps_to_zero() {
    assert_eq!(fmt_time(f64::NAN), "0:00");
}

#[test]
fn fmt_time_infinity_clamps_to_zero() {
    assert_eq!(fmt_time(f64::INFINITY), "0:00");
    assert_eq!(fmt_time(f64::NEG_INFINITY), "0:00");
}

// ---- progress::khz formatting ----

#[test]
fn khz_zero_returns_empty() {
    assert_eq!(khz(0), "");
}

#[test]
fn khz_whole_khz() {
    assert_eq!(khz(96000), "96");
    assert_eq!(khz(48000), "48");
    assert_eq!(khz(192000), "192");
}

#[test]
fn khz_fractional_khz() {
    assert_eq!(khz(44100), "44.1");
    assert_eq!(khz(88200), "88.2");
}

// ---- progress::progress(app) clamping ----

#[test]
fn progress_no_track_returns_zero_and_dashes() {
    let (_d, cat) = one_track_cat();
    let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let (pct, label) = progress(&app);
    assert_eq!(pct, 0);
    assert_eq!(label, "--:-- / --:--");
}

#[test]
fn progress_normal_track() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let (pct, label) = progress(&app);
    // StubPlayer: pos=0, dur=180 → 0%, "0:00 / 3:00".
    assert_eq!(pct, 0);
    assert_eq!(label, "0:00 / 3:00");
}

#[test]
fn progress_zero_duration_no_panic() {
    let mut player = FakePlayer::default();
    player.dur = Some(0.0);
    player.pos = Some(0.0);
    let app = app_with_track(Box::new(player));
    let (pct, _label) = progress(&app);
    // Zero duration → 0% (no divide-by-zero).
    assert_eq!(pct, 0);
}

#[test]
fn progress_none_duration_no_panic() {
    let mut player = FakePlayer::default();
    player.dur = None;
    player.pos = Some(30.0);
    let app = app_with_track(Box::new(player));
    let (pct, label) = progress(&app);
    assert_eq!(pct, 0);
    // Position available but no duration → "M:SS / --:--".
    assert!(
        label.contains("0:30"),
        "label should contain elapsed: {label}"
    );
    assert!(
        label.contains("--:--"),
        "label should contain dashes: {label}"
    );
}

#[test]
fn progress_over_duration_clamps_to_100() {
    let mut player = FakePlayer::default();
    player.dur = Some(180.0);
    player.pos = Some(200.0); // > duration
    let app = app_with_track(Box::new(player));
    let (pct, _label) = progress(&app);
    assert_eq!(pct, 100, "pos > dur must clamp to 100%");
}

#[test]
fn progress_negative_position_clamps_to_zero() {
    let mut player = FakePlayer::default();
    player.dur = Some(180.0);
    player.pos = Some(-5.0);
    let app = app_with_track(Box::new(player));
    let (pct, _label) = progress(&app);
    assert_eq!(pct, 0, "negative pos must clamp to 0%");
}

#[test]
fn progress_nan_position_clamps_to_zero() {
    let mut player = FakePlayer::default();
    player.dur = Some(180.0);
    player.pos = Some(f64::NAN);
    let app = app_with_track(Box::new(player));
    let (pct, _label) = progress(&app);
    assert_eq!(pct, 0, "NaN pos must clamp to 0%");
}

#[test]
fn progress_nan_duration_clamps_to_zero() {
    let mut player = FakePlayer::default();
    player.dur = Some(f64::NAN);
    player.pos = Some(30.0);
    let app = app_with_track(Box::new(player));
    let (pct, _label) = progress(&app);
    assert_eq!(pct, 0, "NaN dur must clamp to 0%");
}

#[test]
fn progress_50_percent() {
    let mut player = FakePlayer::default();
    player.dur = Some(180.0);
    player.pos = Some(90.0);
    let app = app_with_track(Box::new(player));
    let (pct, label) = progress(&app);
    assert_eq!(pct, 50);
    assert_eq!(label, "1:30 / 3:00");
}

// ---- minimal::render no-panic sweep ----

#[test]
fn minimal_render_no_panic_at_1x1() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(1, 1, |f| render(f, f.area(), &app));
}

#[test]
fn minimal_render_no_panic_at_10x2() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(10, 2, |f| render(f, f.area(), &app));
}

#[test]
fn minimal_render_no_panic_at_20x2() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(20, 2, |f| render(f, f.area(), &app));
}

#[test]
fn minimal_render_no_panic_at_40x3() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(40, 3, |f| render(f, f.area(), &app));
}

#[test]
fn minimal_render_no_panic_at_59x10() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(59, 10, |f| render(f, f.area(), &app));
}

#[test]
fn minimal_render_no_panic_at_0x0() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let _ = rendered(0, 0, |f| render(f, f.area(), &app));
}

// ---- minimal::render content ----

#[test]
fn minimal_render_shows_play_glyph_and_title_and_artist() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    // Use a player that reports playing=true to get the ▶ glyph.
    let mut player = FakePlayer::default();
    player.playing = true;
    let app = app_with_track(Box::new(player));
    let bar = rendered(50, 2, |f| render(f, f.area(), &app));
    let play = jukebox::tui::view::theme::play_glyph();
    assert!(bar.contains(play), "play glyph must be present: {bar}");
    assert!(bar.contains("Freedom"), "title must be present: {bar}");
    assert!(bar.contains("Ado"), "artist must be present: {bar}");
}

#[test]
fn minimal_render_keeps_artist_when_truncated() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    // At 20 cols the artist is truncated to "Ado…" or similar but still
    // present (per spec: never drop artist).
    let bar = rendered(20, 2, |f| render(f, f.area(), &app));
    // "Ado" is 3 chars; even at 20 cols it fits without truncation.
    assert!(
        bar.contains("Ado"),
        "artist must be present at 20 cols: {bar}"
    );
}

#[test]
fn minimal_render_keeps_artist_at_10_to_14_cols() {
    // DECK-DEF-001 regression: at 10-14 cols with a realistic 7-char
    // title, the artist must still be visible (truncated), per the
    // spec's 10-19 col band `▶ T… — A…`. The old code starved the
    // artist to 0-1 cols when the title consumed the full budget.
    let app = app_with_track(Box::new(StubPlayer::default()));
    for w in 10..=14u16 {
        let bar = rendered(w, 2, |f| render(f, f.area(), &app));
        assert!(
            bar.contains("Ado") || bar.contains("Ad") || bar.contains("A"),
            "artist must be visible (truncated) at {w} cols: {bar}"
        );
    }
    // At 15+ cols the artist fits fully ("Ado").
    let bar = rendered(15, 2, |f| render(f, f.area(), &app));
    assert!(bar.contains("Ado"), "artist must be full at 15 cols: {bar}");
}

#[test]
fn minimal_render_drops_volume_below_30_cols() {
    let mut app = app_with_track(Box::new(StubPlayer::default()));
    app.volume = 70;
    let bar_29 = rendered(29, 2, |f| render(f, f.area(), &app));
    let bar_30 = rendered(30, 2, |f| render(f, f.area(), &app));
    assert!(
        !bar_29.contains("VOL"),
        "VOL must be hidden below 30 cols: {bar_29}"
    );
    assert!(
        bar_30.contains("VOL"),
        "VOL must be visible at 30 cols: {bar_30}"
    );
}

#[test]
fn minimal_render_drops_duration_below_40_cols() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    // StubPlayer dur=180 → "3:00". At 39 cols the duration is dropped.
    let bar_39 = rendered(39, 2, |f| render(f, f.area(), &app));
    let bar_40 = rendered(40, 2, |f| render(f, f.area(), &app));
    // "3:00" appears in the total-duration slot at >=40 cols.
    assert!(
        !bar_39.contains("3:00"),
        "duration must be hidden below 40 cols: {bar_39}"
    );
    assert!(
        bar_40.contains("3:00"),
        "duration must be visible at 40 cols: {bar_40}"
    );
}

#[test]
fn minimal_render_keeps_elapsed_at_all_widths() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    // StubPlayer pos=0 → "0:00" elapsed. Always shown per spec.
    for w in [10, 20, 30, 40, 50, 59] {
        let bar = rendered(w, 2, |f| render(f, f.area(), &app));
        assert!(
            bar.contains("0:00"),
            "elapsed must be present at {w} cols: {bar}"
        );
    }
}

// ---- render no-panic sweep ----

#[test]
fn render_no_panic_sweep_all_sizes() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    for w in 0..=180u16 {
        for h in 0..=40u16 {
            let _ = rendered(w, h, |f| render(f, f.area(), &app));
        }
    }
}

// ---- render delegates to existing renderers ----

#[test]
fn render_wide_shows_title() {
    let mut app = app_with_track(Box::new(StubPlayer::default()));
    app.player_bar_state.big_pref = true;
    let bar = rendered(140, 32, |f| render(f, f.area(), &app));
    assert!(bar.contains("Freedom"), "Wide deck must show title: {bar}");
}

#[test]
fn render_medium_shows_title() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let bar = rendered(100, 26, |f| render(f, f.area(), &app));
    assert!(
        bar.contains("Freedom"),
        "Medium deck must show title: {bar}"
    );
}

#[test]
fn render_compact_shows_title() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let bar = rendered(70, 22, |f| render(f, f.area(), &app));
    assert!(
        bar.contains("Freedom"),
        "Compact deck must show title: {bar}"
    );
}

// ---- PlayerTheme ----

#[test]
fn player_theme_constructs_under_color() {
    let _guard = lock_env();
    // Ensure NO_COLOR / HIGH_CONTRAST are not set for this thread.
    std::env::remove_var("NO_COLOR");
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    jukebox::tui::view::theme::reset_font_mode_cache();
    jukebox::tui::view::theme::reset_no_motion_cache();
    let theme = Theme::default();
    let p: PlayerTheme = theme.player();
    // In color mode, track_title fg should be Cyan (the theme accent).
    assert_eq!(p.track_title.fg, Some(ratatui::style::Color::Cyan));
}

#[test]
fn player_theme_constructs_under_no_color() {
    let _guard = lock_env();
    std::env::set_var("NO_COLOR", "1");
    jukebox::tui::view::theme::reset_font_mode_cache();
    jukebox::tui::view::theme::reset_no_motion_cache();
    let theme = Theme::default();
    let p: PlayerTheme = theme.player();
    // Under NO_COLOR, accent is White.
    assert_eq!(p.track_title.fg, Some(ratatui::style::Color::White));
    std::env::remove_var("NO_COLOR");
    jukebox::tui::view::theme::reset_font_mode_cache();
}

#[test]
fn player_theme_constructs_under_high_contrast() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_HIGH_CONTRAST", "1");
    jukebox::tui::view::theme::reset_font_mode_cache();
    jukebox::tui::view::theme::reset_no_motion_cache();
    let theme = Theme::default();
    let p: PlayerTheme = theme.player();
    // Under high-contrast, accent is White.
    assert_eq!(p.track_title.fg, Some(ratatui::style::Color::White));
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    jukebox::tui::view::theme::reset_font_mode_cache();
}

#[test]
fn player_theme_has_bold_for_state_styles() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let theme = Theme::default();
    let p: PlayerTheme = theme.player();
    assert!(p
        .track_title
        .add_modifier
        .contains(ratatui::style::Modifier::BOLD));
    assert!(p
        .playback_playing
        .add_modifier
        .contains(ratatui::style::Modifier::BOLD));
    assert!(p
        .playback_paused
        .add_modifier
        .contains(ratatui::style::Modifier::BOLD));
    assert!(p
        .playback_error
        .add_modifier
        .contains(ratatui::style::Modifier::BOLD));
}

// ---- spinner ----

#[test]
fn spinner_ascii_frames_are_ascii() {
    for f in SPINNER_ASCII.iter() {
        assert!(f.is_ascii(), "ASCII spinner frame must be ASCII: {f:?}");
    }
}

#[test]
fn spinner_braille_frames_are_braille() {
    for f in SPINNER.iter() {
        let cp = f.chars().next().unwrap() as u32;
        assert!(
            (0x2800..=0x28FF).contains(&cp),
            "braille spinner frame must be in U+2800..28FF: {f:?} (U+{cp:04X})"
        );
    }
}

#[test]
fn spinner_glyph_picks_ascii_under_ascii_font_mode() {
    let _guard = lock_env();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let app = app_with_track(Box::new(StubPlayer::default()));
    let g = spinner_glyph(&app);
    assert!(
        g.is_ascii(),
        "ASCII font mode must produce ASCII spinner: {g:?}"
    );
    assert!(
        SPINNER_ASCII.contains(&g),
        "ASCII spinner must be a known frame: {g:?}"
    );
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
}

#[test]
fn spinner_glyph_wraps_modulo_frame_count() {
    let _guard = lock_env();
    std::env::remove_var("JUKEBOX_FONT_MODE");
    jukebox::tui::view::theme::reset_font_mode_cache();
    let mut app = app_with_track(Box::new(StubPlayer::default()));
    // Braille spinner has 10 frames. spinner_frame=15 → frame 5.
    app.spinner_frame = 15;
    let g = spinner_glyph(&app);
    assert_eq!(g, SPINNER[5]);
    // spinner_frame=25 → frame 5.
    app.spinner_frame = 25;
    let g = spinner_glyph(&app);
    assert_eq!(g, SPINNER[5]);
}

// ---- is_buffering ----

#[test]
fn is_buffering_false_when_no_pending_play_and_not_resolving() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    assert!(!is_buffering(&app), "normal playback must not be buffering");
}

#[test]
fn is_buffering_true_when_pending_play_set() {
    let mut app = app_with_track(Box::new(StubPlayer::default()));
    app.pending_play = Some("vid123".into());
    assert!(is_buffering(&app), "pending_play must trigger buffering");
}

// ---- DeckGeometry ----

#[test]
fn deck_geometry_minimal_is_all_zero() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let g = geometry(Rect::new(0, 0, 0, 0), &app);
    assert_eq!(
        g,
        DeckGeometry::default(),
        "Minimal deck must have zero geometry"
    );
}

#[test]
fn deck_geometry_wide_matches_visible_controls() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let g = geometry(Rect::new(0, 0, 140, 11), &app);
    assert!(g.progress.width > 0);
    assert!(g.previous.width > 0);
    assert!(g.play_pause.width > 0);
    assert!(g.next.width > 0);
}

#[test]
fn deck_geometry_compact_matches_visible_controls() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let g = geometry(Rect::new(0, 0, 70, 7), &app);
    assert!(g.progress.width > 0);
    assert!(g.previous.width > 0);
    assert!(g.play_pause.width > 0);
    assert!(g.next.width > 0);
}

// ---- render at the spec breakpoints (smoke) ----

#[test]
fn render_at_spec_breakpoints_no_panic() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    for (w, h) in [
        (40, 15),
        (59, 10),
        (60, 20),
        (89, 24),
        (90, 25),
        (129, 30),
        (130, 31),
        (180, 40),
    ] {
        let _ = rendered(w, h, |f| render(f, f.area(), &app));
    }
}

// ---- hi-res wall-clock fix ----

#[test]
fn progress_uses_wall_clock_for_hires_local() {
    // A local track at 192 kHz: StubPlayer.pos=0, but estimated_position
    // uses wall-clock. The deck's `progress` should use the wall-clock
    // estimate, not player.position()=0.
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
      {"id":"hi","artists":["Ado"],"primary_artist":"Ado","title":"HiRes","album":"Adele","bit_depth":24,"sample_rate_hz":192000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["hi".into()], "hi");
    // Simulate 3s of playback via the wall-clock estimate.
    app.play_started_at = Some(std::time::Instant::now() - Duration::from_secs(3));
    app.play_start_offset = 0.0;
    let (pct, _label) = progress(&app);
    // 3s / 180s ≈ 1.67% → rounds to 2%. Without the wall-clock fix the
    // bar would show 0% (StubPlayer.pos=0).
    assert!(
        pct >= 1,
        "hi-res progress must use wall-clock estimate: {pct}%"
    );
}

// ---- transparency-safe redesign ----

#[test]
fn ordinary_cells_keep_terminal_default_background_at_every_breakpoint() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    for (w, h, bp) in [
        (130, 10, Breakpoint::Wide),
        (100, 12, Breakpoint::Medium),
        (70, 7, Breakpoint::Compact),
        (45, 3, Breakpoint::Minimal),
    ] {
        assert_transparent(w, h, |f| {
            render_for_breakpoint_with_focus(f, f.area(), &app, bp, false)
        });
    }
}

#[test]
fn borders_use_a_notched_title_without_overlapping_content() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let output = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, true)
    });
    let mut lines = output.lines();
    let border = lines.next().unwrap_or_default();
    let content = lines.next().unwrap_or_default();
    assert!(border.contains("▶ NOW PLAYING · FOCUSED"), "{output}");
    assert!(border.contains('─') || border.contains('-'), "{output}");
    assert!(
        !content.contains("NOW PLAYING"),
        "title leaked into content: {output}"
    );
}

#[test]
fn focused_and_unfocused_titles_differ_without_background_fill() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let unfocused = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, false)
    });
    let focused = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, true)
    });
    assert!(unfocused
        .lines()
        .next()
        .unwrap_or_default()
        .contains("NOW PLAYING"));
    assert!(!unfocused
        .lines()
        .next()
        .unwrap_or_default()
        .contains("FOCUSED"));
    assert!(focused
        .lines()
        .next()
        .unwrap_or_default()
        .contains("FOCUSED"));
}

#[test]
fn stopped_resume_separates_title_from_action() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.resume_hint = Some("resume: Freedom at 0:05 · R to resume".into());
    let output = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, false)
    });
    assert!(output.contains("Freedom"), "{output}");
    assert!(output.contains("STOPPED"), "{output}");
    assert!(output.contains("[Space] Resume from 0:05"), "{output}");
    assert!(!output.contains("resume: Freedom"), "{output}");
}

#[test]
fn resolving_is_unambiguous_and_not_nothing_playing() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.pending_play = Some("remote".into());
    let output = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, false)
    });
    assert!(output.contains("RESOLVING"), "{output}");
    assert!(output.contains("Finding the stream"), "{output}");
    assert!(!output.contains("nothing playing"), "{output}");
}

#[test]
fn unknown_quality_placeholders_are_never_rendered() {
    let (_d, cat) = one_track_cat();
    let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let output = rendered(100, 12, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Medium, false)
    });
    assert!(!output.contains("--bit"), "{output}");
    assert!(!output.contains("-- kHz"), "{output}");
}

#[test]
fn progress_has_no_separate_percentage_and_is_width_capped() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let output = rendered(180, 10, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Wide, false)
    });
    assert!(!output.contains(" 0% "), "{output}");
    let progress_line = output
        .lines()
        .find(|line| line.contains("0:00") && line.contains("3:00"))
        .unwrap_or_default();
    let bars = progress_line
        .chars()
        .filter(|c| matches!(c, '━' | '─' | '●' | '=' | '-' | '#'))
        .count();
    assert!(bars <= 60, "progress exceeded cap ({bars}): {output}");
}

#[test]
fn compact_handles_cjk_emoji_and_combining_marks_by_display_width() {
    let (_d, mut cat) = one_track_cat();
    cat.tracks[0].title = "夜に駆けるという非常に長い曲名 🎵 e\u{301} encore".into();
    cat.tracks[0].primary_artist = "ヨルシカ".into();
    cat.tracks[0].artists = vec!["ヨルシカ".into()];
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let output = rendered(60, 7, |f| {
        render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Compact, false)
    });
    let title_line = output.lines().nth(1).unwrap_or_default();
    assert!(
        title_line.contains('夜') && title_line.contains('駆') && title_line.contains('る'),
        "{output}"
    );
    assert!(
        title_line.contains('…') || title_line.contains("..."),
        "{output}"
    );
    // `buffer_string` emits the continuation cell of each wide glyph as a
    // space, so its character count (not Unicode width) equals terminal cells.
    assert!(title_line.chars().count() <= 60, "{output}");
}

#[test]
fn deck_never_writes_outside_its_rect_and_tiny_rects_do_not_panic() {
    let app = app_with_track(Box::new(StubPlayer::default()));
    let backend = TestBackend::new(84, 13);
    let mut term = Terminal::new(backend).unwrap();
    let area = Rect::new(7, 3, 70, 7);
    term.draw(|f| render_for_breakpoint_with_focus(f, area, &app, Breakpoint::Compact, false))
        .unwrap();
    for y in 0..13 {
        for x in 0..84 {
            if !area.contains((x, y).into()) {
                assert_eq!(term.backend().buffer()[(x, y)].symbol(), " ");
            }
        }
    }

    let backend = TestBackend::new(1, 1);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_for_breakpoint_with_focus(f, f.area(), &app, Breakpoint::Minimal, false))
        .unwrap();
}
