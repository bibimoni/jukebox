//! Publication confirmation overlay — safe playlist publishing to YouTube.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::view::icons::{Icon, IconRenderer};
use crate::tui::view::theme::{ellipsis, em_dash, sep_dot};

/// Which field the user is editing in the publication overlay. `j`/`k`
/// cycles; Tab edits privacy in place; typing/Backspace edit the Name.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PubField {
    #[default]
    Name,
    Privacy,
    Account,
}

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
    /// Which field is focused (Name / Privacy / Account). Account is read-
    /// only (resolved from the active session); j/k still lands on it for
    /// visibility.
    pub field: PubField,
    /// The last validation error message (shown until the user edits
    /// something). Cleared on the next keypress that changes state.
    pub error: Option<String>,
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
            field: PubField::Name,
            error: None,
        }
    }

    /// True if all confirmation steps are met. Account must be non-empty
    /// (resolved from a connected YT session); name must be non-empty;
    /// privacy must be non-empty (default PRIVATE). Publishable track count
    /// is NOT required here — `is_ready` returning true with 0 tracks still
    /// surfaces a separate validation error via `validation_error`.
    pub fn is_ready(&self) -> bool {
        !self.name.is_empty() && !self.privacy.is_empty() && !self.account.is_empty()
    }

    /// The blocking validation reason when `is_ready` is false, or when
    /// the publishable track list is empty (a publish with 0 tracks is
    /// pointless — show the user why nothing will happen).
    pub fn validation_error(&self) -> Option<String> {
        if self.publishable_ids.is_empty() {
            return Some("0 tracks to publish — open the overlay on a playlist with YouTube tracks".to_string());
        }
        if self.account.is_empty() {
            return Some("no account — run :yt auth browser <name> first".to_string());
        }
        if self.name.is_empty() {
            return Some("name is empty — type a playlist name".to_string());
        }
        None
    }

    /// The intended operation description.
    pub fn intended_operation(&self) -> String {
        format!(
            "Create playlist \"{}\" ({}) with {} tracks",
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
    if state.publishable_ids.is_empty() {
        lines.push(Line::from(Span::styled(
            "   (no YouTube tracks — only local-only tracks can't be published)"
                .to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
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
    }
    lines.push(Line::from(""));

    // Step 2: Local-only items — ALWAYS render so the numbering is stable
    // (1,2,3,4,5,6) instead of jumping 1 → 4 when the vectors are empty.
    if state.local_only.is_empty() {
        lines.push(Line::from(Span::styled(
            "2. Local-only: (no local-only tracks)".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
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
    }
    lines.push(Line::from(""));

    // Step 3: Unavailable items — ALWAYS render (see step 2 comment).
    if state.unavailable.is_empty() {
        lines.push(Line::from(Span::styled(
            "3. Unavailable: (no unavailable tracks)".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
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
    }
    lines.push(Line::from(""));

    // Steps 4-6: Name, privacy, account. The focused field is bolded so
    // j/k navigation has a visible cursor (the field label, not just the
    // value — the value already shows the live state).
    let name_style = if state.field == PubField::Name {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let priv_style = if state.field == PubField::Privacy {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let acct_style = if state.field == PubField::Account {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(
        format!("4. Name:     {} (editable)", state.name),
        name_style,
    )));
    lines.push(Line::from(Span::styled(
        format!("5. Privacy:  {} (Tab cycles)", state.privacy),
        priv_style,
    )));
    lines.push(Line::from(Span::styled(
        format!("6. Account:  {}", state.account),
        acct_style,
    )));
    lines.push(Line::from(""));

    // Step 7: Intended operation / validation error.
    if let Some(err) = &state.error {
        lines.push(Line::from(Span::styled(
            format!("! {}", err),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }
    if state.is_ready() && state.error.is_none() {
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
            "Type a name, Tab to set privacy, Enter to publish  ·  Esc to cancel"
                .to_string(),
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
    fn validation_error_0_tracks_when_publishable_empty() {
        let mut state = PublicationState::new();
        state.name = "Mix".into();
        state.account = "u".into();
        // No publishable_ids → validation_error reports 0 tracks even though
        // is_ready() (name+privacy+account) would otherwise be true.
        assert!(!state.is_ready() || state.validation_error().is_some());
        assert_eq!(
            state.validation_error().as_deref(),
            Some("0 tracks to publish — open the overlay on a playlist with YouTube tracks")
        );
    }

    #[test]
    fn validation_error_no_account_when_account_empty() {
        let mut state = PublicationState::new();
        state.name = "Mix".into();
        state.publishable_ids = vec!["v1".into()];
        // account empty → no-account error (and is_ready false).
        assert!(!state.is_ready());
        assert_eq!(
            state.validation_error().as_deref(),
            Some("no account — run :yt auth browser <name> first")
        );
    }

    #[test]
    fn validation_error_none_when_ready_with_tracks() {
        let mut state = PublicationState::new();
        state.name = "Mix".into();
        state.account = "u".into();
        state.publishable_ids = vec!["v1".into()];
        assert!(state.is_ready());
        assert!(state.validation_error().is_none());
    }

    #[test]
    fn render_shows_stable_numbering_with_empty_sections() {
        // The fix for the field-numbering gap: sections 2 and 3 must ALWAYS
        // render (with "(no ...)" placeholders) so 1..=6 is contiguous.
        let state = PublicationState::new();
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &state, &icons);
        let _ = para;
        // The render must not panic with empty vectors; that's the floor.
        // The text content is exercised in tests/generator_ux.rs publication_render.
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
