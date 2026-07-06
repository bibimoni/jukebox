//! Persistent bottom player bar.
//!
//! Renders the now-playing line (play glyph + title — artist · album), the
//! transport glyphs, a progress [`Gauge`] with an `M:SS / M:SS` label, the
//! hi-fi quality readout (`24-bit / 96 kHz`, plus `· bit-perfect` when the
//! output sample-rate is being switched to match the track), a block-bar
//! volume meter, and the shuffle/repeat mode flags as plain text (no emoji —
//! monochrome-safe).
//!
//! Layout: at `area.height >= 2` the info occupies the top row and the gauge
//! the row beneath; the info content is composed as a single [`Line`] of
//! [`Span`]s so it stays on one visual line at wide widths (≥ ~100 cols) and
//! only wraps below that. At `area.height == 1` only the info line is drawn.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::catalog::Track;
use crate::tui::app::App;
use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
use crate::tui::view::theme::{quality_color, Theme};

/// Render the player bar into `area` using state from `app`.
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    // Split into an info row + a gauge row. When we only have one row, drop
    // the gauge so the info always fits.
    let rows = Layout::vertical(if area.height >= 2 {
        vec![Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Fill(1)]
    })
    .split(area);

    let info_area = rows[0];
    let gauge_area = rows.get(1).copied();

    let line = build_info_line(app, info_area.width as usize);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        info_area,
    );

    if let Some(g) = gauge_area {
        let (pct, label) = progress(app);
        f.render_widget(
            Gauge::default()
                .block(Block::default().borders(Borders::NONE))
                .gauge_style(Style::default().fg(Color::DarkGray))
                .percent(pct)
                .label(label),
            g,
        );
    }
}

/// Compose the single info [`Line`]: play glyph, title — artist · album on the
/// left; transport, quality, volume, and mode flags toward the right. The
/// pieces are joined with a thin separator and the line is left-aligned; if it
/// overflows the area width it simply wraps (Paragraph with height 1 shows
/// only the first visual row, which is what we want).
fn build_info_line(app: &App, _width: usize) -> Line<'static> {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let text = Style::default().fg(theme.text);

    let playing = app.player.is_playing();
    // Play/pause glyph: show ⏸ while playing, ▶ while paused/stopped. Both are
    // geometric/control symbols (not emoji), so they render in monochrome.
    let play_glyph = if playing { "⏸" } else { "▶" };

    let mut spans: Vec<Span<'static>> = Vec::new();

    spans.push(Span::styled(format!("{play_glyph} "), text));

    // Now-playing: title — artist · album (or a dim placeholder).
    match now_playing_track(app) {
        Some(t) => {
            spans.push(Span::styled(t.title.clone(), text));
            spans.push(Span::styled(" — ", dim));
            spans.push(Span::styled(t.primary_artist.clone(), text));
            if let Some(album) = &t.album {
                if !album.is_empty() {
                    spans.push(Span::styled(" · ", dim));
                    spans.push(Span::styled(album.clone(), dim));
                }
            }
        }
        None => {
            spans.push(Span::styled("— nothing playing —", dim));
        }
    }

    spans.push(Span::raw("   "));

    // Transport: ◀◀ ⏸/▶ ▶▶. Plain-text-friendly glyphs.
    let transport = format!("◀◀ {play_glyph} ▶▶");
    spans.push(Span::styled(transport, dim));

    spans.push(Span::raw("   "));

    // Quality readout: `24-bit / 96 kHz` (+ `· bit-perfect`).
    if let Some(t) = now_playing_track(app) {
        let q_color = quality_color(t.bit_depth, t.sample_rate_hz);
        let q_text = format!("{}-bit / {} kHz", t.bit_depth, khz(t.sample_rate_hz));
        spans.push(Span::styled(q_text, Style::default().fg(q_color)));
        if app.switch_sample_rate {
            spans.push(Span::styled(" · bit-perfect", Style::default().fg(q_color)));
        }
    } else {
        spans.push(Span::styled("--bit / -- kHz", dim));
    }

    spans.push(Span::raw("   "));

    // Volume: `vol ▰▰▰▱ 64%`. 4 blocks; filled = round(volume/25).
    spans.push(Span::styled("vol ", dim));
    let blocks = 4u32;
    let filled = ((u32::from(app.volume) * blocks + 50) / 100).min(blocks);
    let mut vol_bar = String::new();
    for i in 0..blocks {
        vol_bar.push(if i < filled { '▰' } else { '▱' });
    }
    let vol_pct = if app.muted { 0 } else { app.volume };
    spans.push(Span::styled(
        format!("{vol_bar} {vol_pct}%"),
        Style::default().fg(if app.muted { theme.dim } else { theme.text }),
    ));

    spans.push(Span::raw("   "));

    // Mode flags as plain text (monochrome-safe): `SHUF smart` / `RPT all`.
    let shuf = match app.transport.shuffle {
        ShuffleMode::Off => "off",
        ShuffleMode::Smart => "smart",
        ShuffleMode::Random => "random",
    };
    let rpt = match app.transport.repeat {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    };
    // Continue mode: what plays when the current context ends (repeat off).
    // off = stop; next = continue to the next album by the same artist;
    // radio = continue with the whole library (shuffled), never stops.
    let cont = match app.transport.continue_mode {
        ContinueMode::Off => "off",
        ContinueMode::NextAlbum => "next",
        ContinueMode::Radio => "radio",
    };
    spans.push(Span::styled(format!("SHUF {shuf}"), dim));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(format!("RPT {rpt}"), dim));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(format!("CONT {cont}"), dim));

    Line::from(spans).alignment(Alignment::Left)
}

/// `(percent, "M:SS / M:SS")` for the progress gauge. When position/duration
/// are unavailable the gauge reads 0% with a `--:--` label.
fn progress(app: &App) -> (u16, String) {
    let pos = app.player.position();
    let dur = app.player.duration();
    match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => {
            let pct = ((p / d) * 100.0).clamp(0.0, 100.0) as u16;
            (pct, format!("{} / {}", fmt_time(p), fmt_time(d)))
        }
        _ => (0, "--:-- / --:--".to_string()),
    }
}

/// `M:SS` (or `H:MM:SS` past an hour). Truncates toward zero.
fn fmt_time(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `96000 -> "96"`, `44100 -> "44.1"`.
fn khz(sample_rate_hz: u32) -> String {
    if sample_rate_hz.is_multiple_of(1000) {
        format!("{}", sample_rate_hz / 1000)
    } else {
        format!("{:.1}", sample_rate_hz as f64 / 1000.0)
    }
}

/// Find the currently-playing [`Track`] by `app.now_playing` id.
fn now_playing_track(app: &App) -> Option<&Track> {
    let id = app.now_playing.as_ref()?;
    app.catalog.tracks.iter().find(|t| &t.id == id)
}
