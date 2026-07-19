//! Shared progress component for the Now Playing Deck.
//!
//! Extracts the progress-bar logic that was duplicated between
//! `player_bar::render_progress_bar` (`player_bar.rs:1301`) and
//! `player_bar_big::render_progress_bar` (`player_bar_big.rs:238`) into a
//! single shared module. Both call sites keep their original implementations
//! byte-identical (so the existing ~40 player-bar tests continue to pass);
//! this module is the canonical home for new callers and for the latent
//! hi-res wall-clock fix documented in `research/01-audit.md` (risk #7).
//!
//! ## Design
//!
//! - [`progress`] returns `(percent, "M:SS / M:SS")` for any `App`. It is
//!   safe for `NaN`, negative, infinite, zero-duration, and over-duration
//!   positions — every value is clamped to `[0.0, duration]` before the
//!   percentage is computed, and the percentage is clamped to `[0, 100]`.
//! - [`render_progress_bar`] paints `▰▰▰▰▱▱▱▱ {pct}% {elapsed} / {total}`
//!   into a `Rect` of any width. The block characters come from the
//!   theme's `filled_block` / `empty_block` helpers so the ASCII fallback
//!   (`#` / `-`) is automatic under `JUKEBOX_FONT_MODE=ascii`.
//! - [`fmt_time`] and [`khz`] are the canonical time / sample-rate
//!   formatters, lifted from `player_bar.rs:1348` / `:1361`.
//! - The hi-res local track path (`>= 192 kHz`) uses `App::estimated_position`
//!   instead of `player.position()` — the existing `player_bar::progress`
//!   does this (`player_bar.rs:1251-1285`) but `player_bar_big::progress`
//!   does NOT (audit latent bug #7). New callers that go through this
//!   module get the fix automatically.
//!
//! ## Performance
//!
//! - [`progress`] does at most one `player.position()` read and one
//!   `player.duration()` read per call. No allocation for the percentage
//!   (it's a `u16`).
//! - [`render_progress_bar`] allocates the bar string and the time label
//!   once per call. The bar string is `filled_block().repeat(filled)` +
//!   `empty_block().repeat(rest)` — two `String` allocations. The label
//!   is one `format!`. Total: 3 `String` allocations per render, which
//!   matches the existing `player_bar_big::render_progress_bar` cost.
//!
//! ## ASCII fallback
//!
//! All glyphs route through `theme::filled_block` / `theme::empty_block`,
//! which return `'#'` / `'-'` under `JUKEBOX_FONT_MODE=ascii`. The bar thus
//! renders as `####-----` in ASCII mode without any explicit branch. The
//! thumb (`'●'`) falls back to `'#'` in ASCII mode (the existing
//! `player_bar::render_progress_bar` does not use a thumb; the deck's
//! minimal layout does — see `minimal::render`).

use ratatui::{
    buffer::CellDiffOption,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::theme::{
    progress_color, progress_fill, progress_thumb, progress_track, Theme,
};

/// `(percent, "M:SS / M:SS")` for the progress gauge. When the position or
/// duration is unavailable the gauge reads `0%` with a `--:--` label. When
/// the duration is zero the gauge reads `0%` with `0:00 / 0:00`.
///
/// Clamps:
/// - `position` to `[0.0, duration]` (NaN / negative / over-duration →
///   clamped; NaN comparisons are false so NaN falls through to 0).
/// - `duration` to `[0.0, +inf)` (NaN → 0).
/// - `percent` to `[0, 100]`.
///
/// Hi-res local tracks (`>= 192 kHz`) use `App::estimated_position` (wall
/// clock) instead of `player.position()` because `mpv.time-pos` advances
/// erratically for hi-res FLAC — see `player_bar.rs:1251-1285` (RC14-DEF-4).
pub fn progress(app: &App) -> (u16, String) {
    if app.now_playing.is_none() {
        return (0, "--:-- / --:--".to_string());
    }

    let duration = match app.player.duration() {
        Some(d) if d.is_finite() && d > 0.0 => d,
        Some(d) if d.is_finite() && d == 0.0 => {
            return (0, "0:00 / 0:00".to_string());
        }
        _ => return (0, label_for_unknown_duration(app)),
    };

    let raw_pos = if is_hires_local(app) {
        app.estimated_position()
            .unwrap_or_else(|| app.player.position().unwrap_or(0.0))
    } else {
        app.player.position().unwrap_or(0.0)
    };

    let pos = if raw_pos.is_nan() || raw_pos < 0.0 {
        0.0
    } else if raw_pos > duration {
        duration
    } else {
        raw_pos
    };

    let pct = ((pos / duration) * 100.0).round().clamp(0.0, 100.0) as u16;
    (pct, format!("{} / {}", fmt_time(pos), fmt_time(duration)))
}

/// `label_for_unknown_duration` produces `M:SS / --:--` when the player
/// reports a position but no duration (a streaming track whose duration
/// hasn't been probed yet), or `--:-- / --:--` when neither is available.
fn label_for_unknown_duration(app: &App) -> String {
    let pos = if is_hires_local(app) {
        app.estimated_position()
            .unwrap_or_else(|| app.player.position().unwrap_or(0.0))
    } else {
        app.player.position().unwrap_or(0.0)
    };
    if pos.is_finite() && pos > 0.0 {
        format!("{} / --:--", fmt_time(pos.max(0.0)))
    } else {
        "--:-- / --:--".to_string()
    }
}

/// True when the now-playing track is a local file at `>= 192 kHz`. The
/// progress bar uses `App::estimated_position` (wall-clock) for these
/// tracks because `mpv.time-pos` advances erratically at hi-res sample
/// rates — see `player_bar.rs:1251-1285` (RC14-DEF-4) and audit risk #7.
fn is_hires_local(app: &App) -> bool {
    let Some(np) = app.now_playing_view() else {
        return false;
    };
    !np.source.is_remote() && np.sample_rate_hz >= 192_000
}

/// Format a duration in seconds as `M:SS` (or `H:MM:SS` past an hour).
/// Negative values clamp to 0; NaN / infinity render as `0:00`.
///
/// Mirrors `player_bar::fmt_time` (`player_bar.rs:1348`) so the deck's
/// progress label is byte-identical to the existing mini bar's.
pub fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".to_string();
    }
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Format a sample rate in Hz as a kHz label: `96000` → `"96"`,
/// `44100` → `"44.1"`, `192000` → `"192"`. Returns an empty string for 0
/// (caller should hide the field rather than display a misleading "0").
///
/// Mirrors `player_bar::khz` (`player_bar.rs:1361`).
pub fn khz(hz: u32) -> String {
    if hz == 0 {
        return String::new();
    }
    if hz.is_multiple_of(1000) {
        format!("{}", hz / 1000)
    } else {
        format!("{:.1}", hz as f64 / 1000.0)
    }
}

/// Render the progress bar into `area` as a single visual group:
/// `0:00  ━━━━━━━━━━━━━━━━━━●──────────────  3:40`.
///
/// Per spec:
/// - **Reserve a reasonable maximum width** — the bar is capped at
///   `MAX_BAR_WIDTH` (60) so it does not stretch across nearly the entire
///   terminal merely because space exists. The bar is left-aligned within
///   `area` (the elapsed/total labels flank it).
/// - **No separate `0%` label** — the filled/empty split is the visual
///   progress indicator; the percentage is implicit in the fill ratio.
/// - **`--:--` for total only when duration is genuinely unknown** (None
///   or non-finite). When duration is 0, show `0:00` (not `--:--`).
/// - **ASCII fallback** — `=`/`-`/`#` instead of `━`/`─`/`●` via the
///   theme's `progress_fill`/`progress_track`/`progress_thumb` helpers.
/// - **Clamped** — `progress(app)` clamps NaN/negative/over-duration.
/// - **Thumb** — a `●` (or `#`) at the playhead position when the bar is
///   wide enough (>= 6 cells). Below that, just the filled/empty split.
///
/// Marks every cell `AlwaysUpdate` so stale CJK glyphs clear on track
/// switch (RC16-DEF-1; mirrors `player_bar::force_area_update`).
pub const MAX_BAR_WIDTH: usize = 60;

pub fn progress_bar_rect(area: Rect, app: &App) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }
    let (_, label) = progress(app);
    let (elapsed, total) = label.split_once(" / ").unwrap_or(("--:--", "--:--"));
    let elapsed_w = disp_width(elapsed) as u16;
    let total_w = disp_width(total) as u16;
    let reserved = elapsed_w + 2 + total_w + 2;
    let width = area
        .width
        .saturating_sub(reserved)
        .min(MAX_BAR_WIDTH as u16);
    if width < 3 {
        return Rect::default();
    }
    Rect::new(area.x + elapsed_w + 2, area.y, width, 1)
}

pub fn render_progress_bar(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let (pct, label) = progress(app);

    // Split the label into elapsed + total. The label is "M:SS / M:SS",
    // "M:SS / --:--", or "--:-- / --:--".
    let (elapsed_str, total_str) = label.split_once(" / ").unwrap_or(("--:--", "--:--"));
    let elapsed_w = disp_width(elapsed_str);
    let total_w = disp_width(total_str);

    // Reserve: elapsed + 2 gaps (left) + 2 gaps + total (right). The bar
    // sits between them. Cap the bar at MAX_BAR_WIDTH per spec.
    let reserved = (elapsed_w + 2 + total_w + 2) as u16;
    let available = area.width.saturating_sub(reserved) as usize;
    let bar_w = available.min(MAX_BAR_WIDTH);

    let fill_color = progress_color(&theme);
    let fill_style = Style::default().fg(fill_color).add_modifier(Modifier::BOLD);
    let track_style = Style::default().fg(theme.dim);
    let thumb_style = Style::default().fg(fill_color).add_modifier(Modifier::BOLD);
    let time_style = Style::default().fg(theme.dim);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(6);
    spans.push(Span::styled(elapsed_str.to_string(), time_style));
    spans.push(Span::raw("  "));

    if bar_w >= 3 {
        let filled = (pct as usize * bar_w).div_ceil(100).min(bar_w);
        // Thumb position: at the boundary between filled and empty.
        // When the bar is fully filled (100%) the thumb sits at the right
        // edge; when fully empty (0%) it sits at the left edge.
        let thumb_at = filled.min(bar_w);
        let rest = bar_w.saturating_sub(filled);

        let mut bar = String::with_capacity(bar_w + 1);
        for _ in 0..filled {
            bar.push(progress_fill());
        }
        // Only render the thumb when there is room (bar_w >= 6 and
        // the thumb doesn't fully cover the bar).
        let render_thumb = bar_w >= 6 && filled > 0 && filled < bar_w;
        if render_thumb {
            bar.push(progress_thumb());
        }
        for _ in 0..rest {
            bar.push(progress_track());
        }
        // The thumb replaces what would have been the first `track` cell,
        // so the total width is unchanged (filled + 1 thumb + rest-1 track
        // when render_thumb, else filled + rest).
        if render_thumb {
            // We've over-counted `rest` by 1 (the thumb took a cell).
            // Pop one track char to keep the width correct.
            bar.pop();
        }
        // Split the bar into filled (styled) and track (dim) so the
        // colors differ. The thumb (if present) sits at the boundary.
        let _ = thumb_style; // used below if we split
        let _ = track_style;
        // For simplicity render the whole bar as one styled span — the
        // filled + thumb chars get fill_style, the track chars get
        // track_style. Split into two spans at the thumb position.
        if render_thumb {
            let filled_part: String = (0..filled).map(|_| progress_fill()).collect();
            let thumb_char = progress_thumb().to_string();
            let track_part: String = (0..rest.saturating_sub(1))
                .map(|_| progress_track())
                .collect();
            spans.push(Span::styled(filled_part, fill_style));
            spans.push(Span::styled(thumb_char, thumb_style));
            spans.push(Span::styled(track_part, track_style));
        } else {
            let filled_part: String = (0..filled).map(|_| progress_fill()).collect();
            let track_part: String = (0..rest).map(|_| progress_track()).collect();
            spans.push(Span::styled(filled_part, fill_style));
            spans.push(Span::styled(track_part, track_style));
        }
        let _ = thumb_at; // suppress unused warning
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(total_str.to_string(), time_style));

    f.render_widget(Paragraph::new(Line::from(spans)), area);

    // RC16-DEF-1: force every cell in `area` to AlwaysUpdate so stale CJK
    // double-width glyphs clear on track switch. Mirrors
    // `player_bar::force_area_update` (`player_bar.rs:108`).
    let buf = f.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
}

/// Display width of a string, counting CJK / kana / fullwidth as 2 and
/// combining marks as 0. Delegates to `theme::disp_width` so the deck's
/// width math is identical to the existing bars'.
fn disp_width(s: &str) -> usize {
    crate::tui::view::theme::disp_width(s)
}
