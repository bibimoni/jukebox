//! Playlist generator overlay — NL input, plan display, constraint editing.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::reco::generator::{GeneratedPlaylist, GeneratorConstraints};
use crate::tui::view::icons::{Icon, IconRenderer};
use crate::tui::view::theme::{ellipsis, sep_dot};

/// The state of the generator overlay.
#[derive(Clone, Debug, Default)]
pub struct GeneratorState {
    /// The natural-language input text.
    pub input: String,
    /// The parsed constraints (None until parsed).
    pub constraints: Option<GeneratorConstraints>,
    /// The generated playlist (None until generated).
    pub playlist: Option<GeneratedPlaylist>,
    /// The cursor in the input text.
    pub cursor: usize,
    /// Whether we're in the input phase or the preview phase.
    pub phase: GeneratorPhase,
}

/// Which phase of the generator the user is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GeneratorPhase {
    /// Entering natural-language input.
    #[default]
    Input,
    /// Reviewing the parsed constraints.
    ReviewPlan,
    /// Previewing the generated playlist.
    Preview,
}

impl GeneratorState {
    /// Create a new generator state (input phase).
    pub fn new() -> Self {
        GeneratorState::default()
    }

    /// Parse the input into constraints.
    pub fn parse_input(&mut self) {
        self.constraints = Some(GeneratorConstraints::from_natural_language(&self.input));
        self.phase = GeneratorPhase::ReviewPlan;
    }

    /// Generate a playlist from the constraints.
    pub fn generate(&mut self) {
        self.phase = GeneratorPhase::Preview;
    }

    /// Pin a track (won't be removed on regenerate).
    pub fn pin_track(&mut self, track_id: String) {
        if let Some(p) = &mut self.playlist {
            if !p.pinned.contains(&track_id) {
                p.pinned.push(track_id);
            }
        }
    }

    /// Remove a track from the playlist.
    pub fn remove_track(&mut self, track_id: &str) {
        if let Some(p) = &mut self.playlist {
            p.tracks.retain(|c| c.track_id != track_id);
        }
    }
}

/// Render the generator overlay.
pub fn render(_area: Rect, state: &GeneratorState, icons: &IconRenderer) -> Paragraph<'static> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        format!("{} Playlist Generator", icons.glyph(Icon::Generated)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    match state.phase {
        GeneratorPhase::Input => {
            lines.push(Line::from(Span::styled(
                "Describe the playlist you want:".to_string(),
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("  {}", state.input)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Enter to generate {} Esc to cancel", sep_dot()),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "Examples: \"45-minute energetic running playlist\", \"calm focus mix, 70% discovery\""
                    .to_string(),
                Style::default().fg(Color::DarkGray),
            )));
        }
        GeneratorPhase::ReviewPlan => {
            if let Some(constraints) = &state.constraints {
                lines.push(Line::from(Span::styled(
                    "Generated plan:".to_string(),
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(""));
                for line in constraints.to_plan_string().lines() {
                    lines.push(Line::from(format!("  {line}")));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        "Enter to generate {} e edit constraints {} Esc cancel",
                        sep_dot(),
                        sep_dot()
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        GeneratorPhase::Preview => {
            if let Some(playlist) = &state.playlist {
                lines.push(Line::from(Span::styled(
                    format!("Preview ({} tracks):", playlist.tracks.len()),
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(""));
                for (i, track) in playlist.tracks.iter().take(20).enumerate() {
                    let pin_marker = if playlist.pinned.contains(&track.track_id) {
                        " [pinned]"
                    } else {
                        ""
                    };
                    lines.push(Line::from(format!(
                        "  {}. {}{pin_marker}",
                        i + 1,
                        track.track_id
                    )));
                }
                if playlist.tracks.len() > 20 {
                    lines.push(Line::from(Span::styled(
                        format!("  {} and {} more", ellipsis(), playlist.tracks.len() - 20),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        "Enter save {} p pin {} x remove {} g regenerate {} Esc cancel",
                        sep_dot(),
                        sep_dot(),
                        sep_dot(),
                        sep_dot()
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    Paragraph::new(lines).wrap(Wrap { trim: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::icons::FontMode;

    #[test]
    fn generator_state_new_is_input_phase() {
        let state = GeneratorState::new();
        assert_eq!(state.phase, GeneratorPhase::Input);
        assert!(state.input.is_empty());
    }

    #[test]
    fn parse_input_moves_to_review() {
        let mut state = GeneratorState::new();
        state.input = "45-minute energetic running playlist".into();
        state.parse_input();
        assert_eq!(state.phase, GeneratorPhase::ReviewPlan);
        assert!(state.constraints.is_some());
    }

    #[test]
    fn generate_moves_to_preview() {
        let mut state = GeneratorState::new();
        state.input = "calm mix".into();
        state.parse_input();
        state.generate();
        assert_eq!(state.phase, GeneratorPhase::Preview);
    }

    #[test]
    fn render_input_phase() {
        let state = GeneratorState::new();
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &state, &icons);
        let _ = para;
    }

    #[test]
    fn render_review_phase() {
        let mut state = GeneratorState::new();
        state.input = "calm focus mix".into();
        state.parse_input();
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render(Rect::new(0, 0, 80, 24), &state, &icons);
        let _ = para;
    }
}
