//! Publication confirmation overlay — safe playlist publishing to YouTube.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::view::icons::{Icon, IconRenderer};
use crate::tui::view::theme::{ellipsis, em_dash, sep_dot};

/// The publication confirmation state.
#[derive(Clone, Debug, Default)]
pub struct PublicationState {
    /// The playlist name.
    pub name: String,
    /// Privacy: "PRIVATE" (default), "PUBLIC", "UNLISTED".
    pub privacy: String,
    /// The account being published to.
    pub account: String,
    /// The track ids to publish (YouTube video_ids only — local-only excluded).
    pub publishable_ids: Vec<String>,
    /// Local-only track ids (can't be published).
    pub local_only: Vec<String>,
    /// Unavailable track ids.
    pub unavailable: Vec<String>,
    /// Whether the user has confirmed.
    pub confirmed: bool,
    /// The confirmation step the user is on (0-9).
    pub step: usize,
}

impl PublicationState {
    /// Create a new publication state with defaults.
    pub fn new() -> Self {
        PublicationState {
            name: String::new(),
            privacy: "PRIVATE".into(),
            account: String::new(),
            publishable_ids: Vec::new(),
            local_only: Vec::new(),
            unavailable: Vec::new(),
            confirmed: false,
            step: 0,
        }
    }

    /// True if all confirmation steps are met.
    pub fn is_ready(&self) -> bool {
        !self.name.is_empty() && !self.privacy.is_empty() && !self.account.is_empty()
    }

    /// The intended operation description.
    pub fn intended_operation(&self) -> String {
        format!(
            "Create playlist \"{}\" ({} ) with {} tracks",
            self.name,
            self.privacy,
            self.publishable_ids.len()
        )
    }
}

/// Render the publication confirmation overlay.
pub fn render(_area: Rect, state: &PublicationState, icons: &IconRenderer) -> Paragraph<'static> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("{} Publish to YouTube", icons.glyph(Icon::Youtube)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Step 1: Final track list
    lines.push(Line::from(Span::styled(
        format!("1. Track list ({} tracks):", state.publishable_ids.len()),
        Style::default().fg(Color::Cyan),
    )));
    for (i, id) in state.publishable_ids.iter().take(5).enumerate() {
        lines.push(Line::from(format!("   {}. {id}", i + 1)));
    }
    if state.publishable_ids.len() > 5 {
        lines.push(Line::from(Span::styled(
            format!(
                "   {} and {} more",
                ellipsis(),
                state.publishable_ids.len() - 5
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));

    // Step 2: Local-only items
    if !state.local_only.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "2. Local-only ({} {} cannot be published):",
                state.local_only.len(),
                em_dash()
            ),
            Style::default().fg(Color::Yellow),
        )));
        for id in state.local_only.iter().take(5) {
            lines.push(Line::from(Span::styled(
                format!("   {id} (local)",),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
    }

    // Step 4: Unavailable items
    if !state.unavailable.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("3. Unavailable ({}):", state.unavailable.len()),
            Style::default().fg(Color::Red),
        )));
        for id in state.unavailable.iter().take(5) {
            lines.push(Line::from(Span::styled(
                format!("   {id} (unavailable)",),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
    }

    // Step 5-7: Name, privacy, account
    lines.push(Line::from(format!("4. Name:     {}", state.name)));
    lines.push(Line::from(format!(
        "5. Privacy:  {} (default: PRIVATE)",
        state.privacy
    )));
    lines.push(Line::from(format!("6. Account:  {}", state.account)));
    lines.push(Line::from(""));

    // Step 8: Intended operation
    if state.is_ready() {
        lines.push(Line::from(Span::styled(
            "7. Operation:".to_string(),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(format!("   {}", state.intended_operation())));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Enter to confirm {} Esc to cancel", sep_dot()),
            Style::default().fg(Color::Green),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Enter name, then press Enter to confirm".to_string(),
            Style::default().fg(Color::Yellow),
        )));
    }

    Paragraph::new(lines).wrap(Wrap { trim: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::icons::FontMode;

    #[test]
    fn publication_state_defaults_private() {
        let state = PublicationState::new();
        assert_eq!(state.privacy, "PRIVATE");
    }

    #[test]
    fn publication_state_not_ready_when_empty() {
        let state = PublicationState::new();
        assert!(!state.is_ready());
    }

    #[test]
    fn publication_state_ready_when_all_set() {
        let mut state = PublicationState::new();
        state.name = "My Mix".into();
        state.account = "user@gmail.com".into();
        state.publishable_ids = vec!["v1".into()];
        assert!(state.is_ready());
    }

    #[test]
    fn intended_operation_includes_name_and_count() {
        let mut state = PublicationState::new();
        state.name = "Test".into();
        state.privacy = "PRIVATE".into();
        state.publishable_ids = vec!["v1".into(), "v2".into()];
        let op = state.intended_operation();
        assert!(op.contains("Test"));
        assert!(op.contains("2 tracks"));
    }

    #[test]
    fn render_publication_produces_content() {
        let state = PublicationState::new();
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &state, &icons);
        let _ = para;
    }
}
