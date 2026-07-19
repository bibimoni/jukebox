//! Diagnostics overlay renderer: a scrollable list of recent diagnostic
//! messages (provider errors, respawn notices, sidecar failures) captured
//! by [`crate::diagnostics::Diagnostics`]. The overlay is intended to be
//! opened from the `:` command mode (`:diag`) and closed with `ESC`; this
//! module only renders — the toggle wiring lives in the input/layout layers.
//!
//! The buffer is bounded by [`crate::diagnostics::Diagnostics`]; here we
//! render every retained message (oldest at top, newest at bottom) in a dim
//! style so errors stay readable without competing with the focused pane.

use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::diagnostics::Diagnostics;
use crate::tui::view::theme::{em_dash, is_ascii, Theme, ASCII_BORDER_SET};

/// Render the diagnostics overlay into `area`: a bordered box titled
/// "diagnostics — Esc to close" listing the buffered messages (newest last),
/// dim style. The caller is responsible for sizing `area` (typically a
/// centered rect over the screen); this function clears the area underneath
/// so the overlay doesn't blend with the pane behind it. An empty buffer
/// renders a single "no diagnostics yet" placeholder line.
///
/// Phase 2 (visual spec H17 / V16): the diagnostics border + title now
/// use the accent color (matching every other overlay). Previously the
/// border had no `border_style` call, so it rendered in the terminal
/// default color — making diagnostics the lone outlier.
pub fn render(f: &mut Frame, area: Rect, diag: &Diagnostics) {
    let theme = Theme::default();
    let dim = theme.status_description();
    let accent = theme.status_key();

    // Clear the area so the overlay reads as a popup, not a blend with the
    // pane behind it (mirrors the Command overlay in `overlay::render_command`).
    f.render_widget(Clear, area);

    let title = format!("diagnostics {} Esc to close", em_dash());
    let block = if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(accent)
            .title(Span::styled(title, accent))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_style(accent)
            .title(Span::styled(title, accent))
    };

    let msgs = diag.messages();
    let body: Vec<Line> = if msgs.is_empty() {
        vec![Line::from(Span::styled("no diagnostics yet", dim))]
    } else {
        // M-2: render newest-first so the latest error (the recovery
        // guidance the user needs) is at the top and always visible —
        // oldest-first clips the latest off-screen when the buffer has more
        // messages than rows (especially at 80x24). A header note makes the
        // order explicit.
        let mut lines: Vec<Line> = msgs
            .iter()
            .rev()
            .map(|m| Line::from(Span::styled(m.clone(), dim)))
            .collect();
        lines.insert(0, Line::from(Span::styled("(newest first)", dim)));
        lines
    };

    f.render_widget(
        Paragraph::new(body)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        area,
    );
}
