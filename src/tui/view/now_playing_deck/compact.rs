//! Compact layout for the Now Playing Deck (60–79 cols, ≥ 20 rows).
//!
//! Condensed single column. Title + artist on one row, progress on
//! the next, then a combined controls/volume/modes row.
//!
//! ## Layout
//!
//! ```text
//! ┌─ ▶ NOW PLAYING ────────────────────────────┐
//! │  [▶ PLAYING] Title — Artist                 │
//! │  0:00  ━━━━━━━━━━━━●──────────  --:--       │
//! │  [Space] Resume  [←/→] Track  Vol 70%      │
//! │  Random · Repeat One · YT Connected         │
//! └─────────────────────────────────────────────┘
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::spinner::{is_buffering, spinner_glyph};
use crate::tui::view::now_playing_deck::{metadata, modes, progress, source_status};
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, is_ascii, pause_glyph, play_glyph, stop_glyph, Theme,
};

/// Render the compact layout into `area`. The caller must ensure
/// `area.width >= 60 && area.height >= 20`.
pub fn render_compact(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let theme = Theme::default();
    let marker = if focused { play_glyph() } else { "" };
    let title = if focused {
        if is_ascii() {
            "NOW PLAYING - FOCUSED"
        } else {
            "NOW PLAYING · FOCUSED"
        }
    } else {
        "NOW PLAYING"
    };
    let outer = theme.pane_block_notched(title, marker, focused, false);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title — artist
            Constraint::Length(1), // progress
            Constraint::Length(1), // controls (compact)
            Constraint::Length(1), // modes (compact)
            Constraint::Length(1), // source + connection
            Constraint::Min(0),    // trailing
        ])
        .split(inner);

    // Row 0: title — artist (single line).
    let title_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.dim);
    let (state_glyph, state_text) = if is_buffering(app) {
        (spinner_glyph(app).to_string(), "RESOLVING")
    } else if app.now_playing.is_none() {
        (stop_glyph().to_string(), "STOPPED")
    } else if app.player.is_playing() {
        (play_glyph().to_string(), "PLAYING")
    } else {
        (pause_glyph().to_string(), "PAUSED")
    };
    let state_prefix = format!("[{state_glyph} {state_text}] ");
    let state_width = disp_width(&state_prefix);
    let row0 = if is_buffering(app) {
        Line::from(vec![
            Span::styled(state_prefix.clone(), theme.status_key()),
            Span::styled(format!("Finding the stream{}", ellipsis()), dim_style),
        ])
    } else {
        match metadata::display_metadata(app) {
            Some(metadata) => {
                let left = if metadata.artist.is_empty() {
                    metadata.primary_title
                } else {
                    format!("{} — {}", metadata.primary_title, metadata.artist)
                };
                let budget = (rows[0].width as usize).saturating_sub(state_width);
                let truncated = crate::tui::view::player_bar::truncate_title(&left, budget);
                Line::from(vec![
                    Span::styled(state_prefix.clone(), theme.status_key()),
                    Span::styled(truncated, title_style),
                ])
            }
            None => {
                let d = em_dash();
                Line::from(vec![
                    Span::styled(state_prefix, theme.status_key()),
                    Span::styled(format!("{d} nothing playing {d}"), dim_style),
                ])
            }
        }
    };
    f.render_widget(Paragraph::new(row0), rows[0]);

    // Row 1: progress.
    progress::render_progress_bar(f, rows[1], app);

    // Row 2: controls (compact form — just the play/pause action + track
    // navigation + volume on one line).
    let key_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let dim_style2 = Style::default().fg(theme.dim);
    let action = if is_buffering(app) {
        None
    } else if app.player.is_playing() {
        Some("Pause")
    } else if app.now_playing.is_some() {
        Some("Play")
    } else if app.resume_hint.is_some() {
        Some("Resume")
    } else {
        Some("Play")
    };
    let left = if crate::tui::view::theme::is_ascii() {
        "[<]"
    } else {
        "[←]"
    };
    let right = if crate::tui::view::theme::is_ascii() {
        "[>]"
    } else {
        "[→]"
    };
    let vol_str = format!("Vol {}%", app.volume);
    let mut control_spans = Vec::new();
    if let Some(action) = action {
        control_spans.push(Span::styled(format!("[Space] {action}"), key_style));
        control_spans.push(Span::raw("   "));
    }
    control_spans.push(Span::styled(format!("{left}/{right} Track"), key_style));
    control_spans.push(Span::raw("   "));
    control_spans.push(Span::styled(vol_str, dim_style2));
    f.render_widget(Paragraph::new(Line::from(control_spans)), rows[2]);

    // Row 3: modes (compact form).
    modes::render_modes_compact(f, rows[3], app);

    // Row 4: source status (subdued secondary metadata).
    source_status::render_source_status(f, rows[4], app);
}
