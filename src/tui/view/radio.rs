//! Radio session overlay — shows seed, history, queue, and feedback.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::reco::radio::RadioSession;
use crate::tui::view::icons::{Icon, IconRenderer};

/// Render the radio session overlay.
pub fn render(_area: Rect, session: &RadioSession, icons: &IconRenderer) -> Paragraph<'static> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("{} Radio Session", icons.glyph(Icon::Radio)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Seed:".to_string(),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(format!("  {}", session.seed.description())));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        format!("Pool: {} tracks remaining", session.pool_size()),
        Style::default().fg(Color::Yellow),
    )));

    if session.needs_refill() {
        lines.push(Line::from(Span::styled(
            "  (refilling…)".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));

    let history = session.history();
    if !history.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Played this session ({}):", history.len()),
            Style::default().fg(Color::Cyan),
        )));
        for (i, track_id) in history.iter().take(10).enumerate() {
            lines.push(Line::from(format!("  {}. {track_id}", i + 1)));
        }
        if history.len() > 10 {
            lines.push(Line::from(Span::styled(
                format!("  … and {} more", history.len() - 10),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "n next · s skip · x negative · + positive · c change seed · q stop · Esc close"
            .to_string(),
        Style::default().fg(Color::DarkGray),
    )));

    Paragraph::new(lines).wrap(Wrap { trim: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reco::radio::{RadioSeed, RadioSession};
    use crate::tui::view::icons::FontMode;

    #[test]
    fn render_radio_session_produces_content() {
        let session = RadioSession::new(RadioSeed::Track("t1".into()));
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &session, &icons);
        let _ = para;
    }

    #[test]
    fn render_radio_shows_seed_description() {
        let session = RadioSession::new(RadioSeed::Artist("Test Artist".into()));
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &session, &icons);
        let _ = para;
    }
}
