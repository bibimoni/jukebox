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

/// Render the 1-line footer hint bar into `area`.
pub fn render(f: &mut Frame, area: &ratatui::layout::Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });
    let sep = " · ";
    // Scope the hint to the active view so the suggestions stay accurate:
    // - Y view → "/" searches YouTube; otherwise local.
    // - the hint stays the same shape either way (consistency over verbosity).
    let search_hint = if app.view == crate::tui::app::View::Youtube {
        "/ search YT"
    } else {
        "/ search"
    };
    let line = Line::from(vec![
        Span::styled("Enter play", dim),
        Span::raw(sep),
        Span::styled("Space pause", dim),
        Span::raw(sep),
        Span::styled("> < next prev", dim),
        Span::raw(sep),
        Span::styled("M mode", dim),
        Span::raw(sep),
        Span::styled(search_hint, dim),
        Span::raw(sep),
        Span::styled("? help", dim),
        Span::raw(sep),
        Span::styled("q quit", dim),
    ])
    .alignment(Alignment::Left);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        *area,
    );
}

fn no_color() -> bool {
    crate::tui::view::theme::no_color()
}
