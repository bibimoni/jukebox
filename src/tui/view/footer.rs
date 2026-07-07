//! The always-visible footer hint bar: the 5–6 most-used keys, so basic
//! actions are discoverable without `?` (spec §5.1 cut #2). The full keymap
//! lives behind `?`.

use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::theme::Theme;

/// Render the 1-line footer. Shows a transient YT status/error when set
/// (visible from any view, so auth/setup feedback isn't lost); otherwise the
/// key-hint bar.
pub fn render(f: &mut Frame, area: &ratatui::layout::Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });
    let line = if let Some(e) = &app.yt_error {
        Line::from(Span::styled(
            format!("YT: {e}"),
            Style::default().fg(if no_color() { Color::Reset } else { Color::Yellow }),
        ))
    } else if let Some(s) = &app.yt_status {
        Line::from(Span::styled(
            s.clone(),
            Style::default().fg(if no_color() { Color::Reset } else { theme.accent }),
        ))
    } else {
        hint_line(app, &dim)
    };
    f.render_widget(
        Paragraph::new(line.alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        *area,
    );
}

fn hint_line(app: &App, dim: &Style) -> Line<'static> {
    let sep = " · ";
    let search_hint = if app.view == crate::tui::app::View::Youtube {
        "/ search YT"
    } else {
        "/ search"
    };
    Line::from(vec![
        Span::styled("Enter play", *dim),
        Span::raw(sep),
        Span::styled("Space pause", *dim),
        Span::raw(sep),
        Span::styled("> < next prev", *dim),
        Span::raw(sep),
        Span::styled("M mode", *dim),
        Span::raw(sep),
        Span::styled(search_hint, *dim),
        Span::raw(sep),
        Span::styled("? help", *dim),
        Span::raw(sep),
        Span::styled("q quit", *dim),
    ])
}

fn no_color() -> bool {
    crate::tui::view::theme::no_color()
}
