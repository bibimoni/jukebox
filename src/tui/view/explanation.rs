//! Recommendation explanation overlay — shows provenance-based reasons.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::reco::explanations::Explanation;
use crate::tui::view::icons::{Icon, IconRenderer};

/// Render the explanation overlay.
pub fn render(_area: Rect, explanation: &Explanation, icons: &IconRenderer) -> Paragraph<'static> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("{} Recommendation Explanation", icons.glyph(Icon::Search)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Why this track was recommended:".to_string(),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(format!("  {}", explanation.reason)));

    if let Some(detail) = &explanation.detail {
        lines.push(Line::from(Span::styled(
            format!("  └ {detail}"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc to close".to_string(),
        Style::default().fg(Color::DarkGray),
    )));

    Paragraph::new(lines).wrap(Wrap { trim: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::icons::FontMode;

    #[test]
    fn render_explanation_produces_content() {
        let exp = Explanation {
            reason: "from your liked tracks".into(),
            detail: Some("seeded by track t1".into()),
        };
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &exp, &icons);
        let _ = para;
    }

    #[test]
    fn render_explanation_no_detail() {
        let exp = Explanation {
            reason: "a track you used to love".into(),
            detail: None,
        };
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &exp, &icons);
        let _ = para;
    }
}
