//! Up Next component for the Now Playing Deck.
//!
//! Renders the up-next preview as **secondary metadata** (spec: visual
//! hierarchy places Up Next at position 10 — below modes, above audio
//! metadata):
//!
//! ```text
//! Up next: Nothing queued
//! Up next: Track title — Artist
//! ```
//!
//! In the wide layout, the Up Next area may show several queue entries
//! (owned by `wide.rs`, not this module).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::theme::{ellipsis, Theme};

/// Render the up-next line into `area` (1 row).
pub fn render_up_next(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_up_next_line(app, &theme, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Build the up-next line. Exposed for layout composers.
pub fn build_up_next_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let key_style = Style::default().fg(theme.text_muted);
    let value_style = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);

    // "Up next: " prefix (8 cols).
    let prefix = "Up next: ";
    let prefix_w = crate::tui::view::theme::disp_width(prefix);

    let next = match app.transport.peek_next(app, &app.catalog) {
        Some(id) => {
            // Resolve the title — local catalog first, then YT cache.
            if let Some(t) = app.track_by_id_fast(&id) {
                format!("{} — {}", t.title, t.primary_artist)
            } else if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(&id)) {
                format!("{} — {}", rt.title, rt.artist)
            } else {
                format!("Loading{}", ellipsis())
            }
        }
        None => "Nothing queued".to_string(),
    };

    let next_w = crate::tui::view::theme::disp_width(&next);
    let total_w = prefix_w + next_w;

    if total_w <= width {
        Line::from(vec![
            Span::styled(prefix.to_string(), key_style),
            Span::styled(next, value_style),
        ])
    } else {
        // Truncate the value. Keep the prefix intact (it's the key).
        let budget = width.saturating_sub(prefix_w);
        let truncated = crate::tui::view::player_bar::truncate_title(&next, budget);
        Line::from(vec![
            Span::styled(prefix.to_string(), key_style),
            Span::styled(truncated, value_style),
        ])
    }
}
