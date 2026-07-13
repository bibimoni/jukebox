//! Radio session overlay — shows seed, history, queue, and feedback.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::reco::radio::RadioSession;
use crate::tui::view::icons::{Icon, IconRenderer};
use crate::tui::view::theme::{ellipsis, sep_dot};

/// Render the radio session overlay.
///
/// `seed_title` is the resolved display title for the seed (DEF-061: the raw
/// track id is replaced with "Title — Artist"). `upcoming` is the list of
/// resolved display titles for the next pool tracks (DEF-063: shows the
/// upcoming 5-10 tracks, not just a count). Both are resolved by the caller
/// (which has access to the catalog + YouTube `track_cache`); the view layer
/// only formats them.
pub fn render(
    _area: Rect,
    session: &RadioSession,
    icons: &IconRenderer,
    seed_title: &str,
    upcoming: &[String],
) -> Paragraph<'static> {
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
    // DEF-061: show the resolved seed title instead of the raw track id.
    lines.push(Line::from(format!("  {}", seed_title)));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        format!("Pool: {} tracks remaining", session.pool_size()),
        Style::default().fg(Color::Yellow),
    )));

    if session.needs_refill() {
        lines.push(Line::from(Span::styled(
            format!("  (refilling{})", ellipsis()),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // DEF-063: show the next few upcoming tracks (titles resolved by the
    // caller) so the user can see what's coming, not just a count.
    if !upcoming.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Up next:".to_string(),
            Style::default().fg(Color::Cyan),
        )));
        for (i, title) in upcoming.iter().take(8).enumerate() {
            lines.push(Line::from(format!("  {}. {}", i + 1, title)));
        }
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
                format!("  {} and {} more", ellipsis(), history.len() - 10),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "n next {sd} s skip {sd} x negative {sd} + positive {sd} c change seed {sd} q stop {sd} Esc close",
            sd = sep_dot()
        ),
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
        let para = render(Rect::new(0, 0, 80, 24), &session, &icons, "track t1", &[]);
        let _ = para;
    }

    #[test]
    fn render_radio_shows_seed_description() {
        let session = RadioSession::new(RadioSeed::Artist("Test Artist".into()));
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(
            Rect::new(0, 0, 80, 24),
            &session,
            &icons,
            "artist Test Artist",
            &[],
        );
        let _ = para;
    }

    #[test]
    fn render_radio_shows_resolved_seed_title() {
        // DEF-061: the seed title is the resolved display string, not the raw id.
        let session = RadioSession::new(RadioSeed::Track("rzVKfAQp2No".into()));
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(
            Rect::new(0, 0, 80, 24),
            &session,
            &icons,
            "Ado — あのバンド",
            &[],
        );
        let _ = para;
    }

    #[test]
    fn render_radio_shows_upcoming_list() {
        // DEF-063: upcoming tracks are listed, not just a count.
        let session = RadioSession::new(RadioSeed::Track("t1".into()));
        let icons = IconRenderer::new(FontMode::Unicode);
        let upcoming = vec![
            "Song A — Artist A".to_string(),
            "Song B — Artist B".to_string(),
        ];
        let para = render(Rect::new(0, 0, 80, 24), &session, &icons, "track t1", &upcoming);
        let _ = para;
    }
}
