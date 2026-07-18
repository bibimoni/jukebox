//! Right-side "Now Playing" preview panel.
//!
//! A vertical split of the main content area: the browse columns/YouTube view
//! on the left, this panel on the right (fixed width ~40 cols at ≥140 width).
//! Shows the now-playing track title, artist, album, quality readout, a
//! progress bar with timestamps, the transport state glyph, the source badge
//! (Local/YT), and a "Next:" preview. When nothing is playing, shows a
//! placeholder.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, empty_block, filled_block, is_ascii, pause_glyph, play_glyph,
    sep_dot, stop_glyph, Theme, ASCII_BORDER_SET,
};

/// The panel width in columns. At terminal widths ≥140 the panel is shown
/// alongside the main content; below 140 it's hidden (too narrow for both).
pub const PANEL_WIDTH: u16 = 42;
/// The minimum terminal width at which the panel is shown.
pub const MIN_WIDTH_FOR_PANEL: u16 = 140;

/// True if the now-playing panel should be visible at the given terminal width.
pub fn is_visible(width: u16) -> bool {
    width >= MIN_WIDTH_FOR_PANEL
}

/// Render the now-playing panel into `area`. Called by `layout::draw` when
/// `is_visible(area.width)` and the user hasn't hidden it.
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let nc = crate::tui::view::theme::no_color();
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });
    let accent = Style::default().fg(if nc { Color::Reset } else { theme.accent });
    let text = Style::default().fg(if nc { Color::Reset } else { theme.text });

    let title = if is_ascii() {
        "Now Playing"
    } else {
        "♪ Now Playing"
    };
    let block = if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(theme.accent))
            .title(Span::styled(title, Style::default().fg(theme.accent)))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(theme.accent))
            .title(Span::styled(title, Style::default().fg(theme.accent)))
    };
    let inner = block.inner(area);
    f.render_widget(block, area);

    let np = app.now_playing_view();
    let has_track = app.now_playing.is_some();
    let playing = app.player.is_playing() && has_track;
    let resolving = app.is_resolving();
    let buffering =
        app.pending_play.is_some() || (has_track && !app.player.is_playing() && resolving);

    let mut lines: Vec<Line> = Vec::new();

    if !has_track && !buffering {
        // Nothing playing.
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Nothing playing".to_string(), dim)));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "Press {} on any track",
                if is_ascii() { "Enter" } else { "⏎" }
            ),
            dim,
        )));
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
        return;
    }

    // State glyph.
    let state_glyph = if resolving || buffering {
        let frames: &[&str] = if is_ascii() {
            &["|", "/", "-", "\\"]
        } else {
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        };
        frames[0] // static frame (no tick counter here)
    } else if playing {
        play_glyph()
    } else if has_track {
        pause_glyph()
    } else {
        stop_glyph()
    };
    let state_label = if buffering {
        "BUFFERING"
    } else if playing {
        "PLAYING"
    } else if has_track {
        "PAUSED"
    } else {
        "STOPPED"
    };

    // Title (large, bold).
    let title_text = np.as_ref().map(|v| v.title.clone()).unwrap_or_else(|| {
        if buffering {
            format!("Resolving{}", ellipsis())
        } else {
            // Track is playing but metadata isn't cached (e.g. after
            // restart/resume — the track_cache is empty). Show a clean
            // "Resuming…" label instead of the raw video_id.
            format!("Resuming{}", ellipsis())
        }
    });
    let col_w = inner.width as usize;
    let title_display = if disp_width(&title_text) > col_w.saturating_sub(2) {
        // Truncate to fit.
        let mut t = title_text.clone();
        while disp_width(&t) > col_w.saturating_sub(5) {
            t.pop();
        }
        format!("{t}{}", ellipsis())
    } else {
        title_text
    };

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{state_glyph} "),
            accent.add_modifier(Modifier::BOLD),
        ),
        Span::styled(title_display, text.add_modifier(Modifier::BOLD)),
    ]));

    // Artist.
    let artist_text = np.as_ref().map(|v| v.artist.clone()).unwrap_or_default();
    if !artist_text.is_empty() {
        lines.push(Line::from(Span::styled(format!("  {artist_text}"), dim)));
    }

    // Album.
    let album_text = np
        .as_ref()
        .and_then(|v| v.album.clone())
        .unwrap_or_default();
    if !album_text.is_empty() {
        lines.push(Line::from(Span::styled(format!("  {album_text}"), dim)));
    }

    lines.push(Line::from(""));

    // State label + source badge.
    let src_badge = np
        .as_ref()
        .map(|v| if v.source.is_remote() { "YT" } else { "LOCAL" })
        .unwrap_or("—");
    let state_color = if playing {
        Color::Magenta
    } else if buffering {
        Color::Cyan
    } else if has_track {
        Color::Yellow
    } else {
        theme.dim
    };
    let state_style = Style::default().fg(if nc { Color::Reset } else { state_color });
    lines.push(Line::from(vec![
        Span::styled(format!("[{state_label}]"), state_style),
        Span::raw(" "),
        Span::styled(format!("[{src_badge}]"), dim),
    ]));

    // Quality readout.
    if let Some(v) = &np {
        let q = quality_readout(v, app, &theme);
        lines.push(Line::from(Span::styled(format!("  {q}"), dim)));
    }

    lines.push(Line::from(""));

    // Progress bar.
    let pos = app.player.position().unwrap_or(0.0);
    let dur = app.player.duration().unwrap_or(0.0);
    let bar_w = (inner.width as usize).saturating_sub(2).max(1);
    let progress_bar = if dur > 0.0 {
        let filled = ((pos / dur) * bar_w as f64).round() as usize;
        let filled = filled.min(bar_w);
        let f: String = filled_block().to_string().repeat(filled);
        let e: String = empty_block().to_string().repeat(bar_w - filled);
        format!("{f}{e}")
    } else {
        empty_block().to_string().repeat(bar_w)
    };
    let pct = if dur > 0.0 {
        ((pos / dur) * 100.0) as u32
    } else {
        0
    };
    let pos_str = format_time(pos);
    let dur_str = format_time(dur);
    lines.push(Line::from(Span::styled(progress_bar, accent)));
    lines.push(Line::from(vec![
        Span::styled(format!(" {pos_str} / {dur_str}"), dim),
        Span::raw(" ".repeat(4)),
        Span::styled(format!("{pct}%"), dim),
    ]));

    lines.push(Line::from(""));

    // Transport + volume.
    let vol = app.volume;
    let vol_w = 8usize;
    let vol_filled = ((vol as f64 / 100.0) * vol_w as f64).round() as usize;
    let vol_filled = vol_filled.min(vol_w);
    let vf: String = filled_block().to_string().repeat(vol_filled);
    let ve: String = empty_block().to_string().repeat(vol_w - vol_filled);
    let vol_bar = format!("{vf}{ve}");
    let dash = em_dash();
    lines.push(Line::from(vec![
        Span::styled("  ◀◀  ▶  ▶▶", text),
        Span::raw("  "),
        Span::styled("vol ", dim),
        Span::styled(vol_bar, accent),
        Span::styled(format!(" {}%", vol as u32), dim),
    ]));

    lines.push(Line::from(""));

    // Flags: SHUF · RPT · CONT · PREF · SRC.
    use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
    let shuf = match app.transport.shuffle {
        ShuffleMode::Off => "off",
        ShuffleMode::Smart => "smart",
        ShuffleMode::Random => "on",
    };
    let rpt = match app.transport.repeat {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    };
    let cont = match app.transport.continue_mode {
        ContinueMode::Off => "off",
        ContinueMode::NextAlbum => "album",
        ContinueMode::Radio => "radio",
        ContinueMode::YouTube => "youtube",
    };
    let pref = if matches!(
        app.source_mode,
        crate::mode::SourceMode::Youtube | crate::mode::SourceMode::Mixed
    ) {
        "youtube"
    } else {
        "local"
    };
    let dot = sep_dot();
    lines.push(Line::from(Span::styled(
        format!("SHUF {shuf} {dot} RPT {rpt} {dot} CONT {cont} {dot} PREF {pref}"),
        dim,
    )));

    lines.push(Line::from(""));

    // Next preview.
    let next_id = app.transport.peek_next(app, &app.catalog);
    let next_text = match &next_id {
        Some(id) => {
            let title = if let Some(t) = app.track_by_id_fast(id) {
                t.title.clone()
            } else if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
                rt.title.clone()
            } else {
                format!("Loading{}", ellipsis())
            };
            // Truncate to fit.
            let max_w = col_w.saturating_sub(8);
            if disp_width(&title) > max_w {
                let mut t = title;
                while disp_width(&t) > max_w.saturating_sub(1) {
                    t.pop();
                }
                format!("Next: {t}{}", ellipsis())
            } else {
                format!("Next: {title}")
            }
        }
        None => format!("Queue empty {dash} press e to enqueue"),
    };
    lines.push(Line::from(vec![
        Span::styled("▸ ", accent),
        Span::styled(next_text, dim),
    ]));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Format seconds as M:SS.
fn format_time(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}

/// Quality readout string for the now-playing track.
fn quality_readout(v: &crate::tui::app::NowPlayingView, _app: &App, theme: &Theme) -> String {
    if v.source.is_remote() {
        // YouTube — show codec/abr from fmt, or a generic badge.
        match &v.fmt {
            Some(f) if f.abr > 0 => format!("YT {} {}k", f.codec, f.abr),
            _ => {
                let _ = theme;
                "YT streaming".to_string()
            }
        }
    } else {
        let bd = if v.bit_depth > 0 {
            format!("{}bit", v.bit_depth)
        } else {
            "--".into()
        };
        let sr = if v.sample_rate_hz > 0 {
            format!("{}kHz", v.sample_rate_hz / 1000)
        } else {
            "--".into()
        };
        format!("{bd} / {sr}")
    }
}
