//! Diagnostics overlay renderer: a scrollable list of recent diagnostic
//! messages (provider errors, respawn notices, sidecar failures) captured
//! by [`crate::diagnostics::Diagnostics`]. The overlay is intended to be
//! opened from the `:` command mode (`:diag`) and closed with `Esc`; this
//! module only renders — the toggle wiring lives in the input/layout layers.
//!
//! The buffer is bounded by [`crate::diagnostics::Diagnostics`]; here we
//! render every retained message (oldest at top, newest at bottom) in a dim
//! style so errors stay readable without competing with the focused pane.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::diagnostics::Diagnostics;
use crate::tui::view::theme::Theme;

/// Render the diagnostics overlay into `area`: a bordered box titled
/// "diagnostics — Esc to close" listing the buffered messages (newest last),
/// dim style. The caller is responsible for sizing `area` (typically a
/// centered rect over the screen); this function clears the area underneath
/// so the overlay doesn't blend with the pane behind it. An empty buffer
/// renders a single "no diagnostics yet" placeholder line.
pub fn render(f: &mut Frame, area: Rect, diag: &Diagnostics) {
    let theme = Theme::default();
    let dim = Style::default().fg(if crate::tui::view::theme::no_color() {
        Color::Reset
    } else {
        theme.dim
    });

    // Clear the area so the overlay reads as a popup, not a blend with the
    // pane behind it (mirrors the Command overlay in `overlay::render_command`).
    f.render_widget(Clear, area);

    let title_style = Style::default().add_modifier(Modifier::BOLD);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("diagnostics — Esc to close", title_style));

    let msgs = diag.messages();
    let body: Vec<Line> = if msgs.is_empty() {
        vec![Line::from(Span::styled("no diagnostics yet", dim))]
    } else {
        msgs.iter()
            .map(|m| Line::from(Span::styled(m.clone(), dim)))
            .collect()
    };

    f.render_widget(
        Paragraph::new(body)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        area,
    );
}
