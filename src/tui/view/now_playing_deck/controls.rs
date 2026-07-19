//! Primary controls + volume component for the Now Playing Deck.
//!
//! Renders the transport controls as a **single aligned group** (per
//! spec problem #4: "Playback controls are isolated on the far left"):
//!
//! ```text
//! [←] Previous   [Space] Resume   [→] Next   Vol 70% ███████░░░
//! ```
//!
//! Volume is placed **beside** the primary controls (not at the far
//! right edge of the terminal — spec problem #5). ASCII fallback:
//! `[<]`/`[>]`, `#`/`-`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::spinner::is_buffering;
use crate::tui::view::theme::{empty_block, filled_block, Theme};

/// Render the primary controls + volume group into `area` (1 row).
pub fn render_controls(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_controls_line(app, &theme, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Build the controls line. Exposed for layout composers.
pub fn build_controls_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let key_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.text);
    let dim_style = Style::default().fg(theme.dim);

    // Use `[←]`/`[→]` per the spec diagram (not `◀◀`/`▶▶`).
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

    // Play/pause/resume action label.
    let action = action_label(app);

    let prev_part = format!("{left} Previous");
    let play_part = action.map(|action| format!("[Space] {action}"));
    let next_part = format!("{right} Next");
    let vol_part = build_volume_str(app);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(8);
    spans.push(Span::styled(prev_part.clone(), key_style));
    if let Some(play_part) = &play_part {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(play_part.clone(), key_style));
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled(next_part.clone(), key_style));

    // Volume beside (if room). Use at least 3 cols of gap so the group
    // reads as one block.
    let used = crate::tui::view::theme::disp_width(&prev_part)
        + play_part
            .as_ref()
            .map(|part| 3 + crate::tui::view::theme::disp_width(part))
            .unwrap_or(0)
        + 3
        + crate::tui::view::theme::disp_width(&next_part);
    let vol_w = crate::tui::view::theme::disp_width(&vol_part);
    if width >= used + 3 + vol_w {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(vol_part, dim_style));
    } else if width >= used + 1 + vol_w {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(vol_part, dim_style));
    }
    // Else: drop the volume (it's shown on a second row in the layouts
    // that have height).

    let _ = label_style; // suppress unused
    Line::from(spans)
}

pub fn action_label(app: &App) -> Option<&'static str> {
    if is_buffering(app) {
        None
    } else if app.player.is_playing() {
        Some("Pause")
    } else if app.now_playing.is_some() {
        Some("Play")
    } else if app.resume_hint.is_some() {
        Some("Resume")
    } else {
        Some("Play")
    }
}

/// Build the volume string: `Vol 70% ███████░░░` (or `MUTED` when muted).
pub fn build_volume_str(app: &App) -> String {
    if app.muted {
        return "Vol MUTED".to_string();
    }
    let blocks = 10u32;
    let filled = ((u32::from(app.volume) * blocks + 50) / 100).min(blocks);
    let mut bar = String::with_capacity(blocks as usize);
    for i in 0..blocks {
        bar.push(if i < filled {
            filled_block()
        } else {
            empty_block()
        });
    }
    format!("Vol {}% {}", app.volume, bar)
}
