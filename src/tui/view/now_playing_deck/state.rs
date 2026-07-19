//! Playback state component for the Now Playing Deck.
//!
//! Renders the playback state as a **dedicated row** separate from the
//! track title. Per spec:
//!
//! - Playing:    `▶ PLAYING  [Space] Pause`
//! - Paused:     `Ⅱ PAUSED  [Space] Play`
//! - Stopped:    `■ STOPPED  [Space] Resume from {M:SS}` (when resume
//!   hint available) OR `■ STOPPED` (no resume)
//! - Resolving:  `⠹ RESOLVING  Finding the stream…`
//! - Error:      `! PLAYBACK ERROR  [R] Retry  [D] Diagnostics`
//!
//! **Never** prefix the track title with `resume:` — the resume action
//! is a state-row affordance, not a title decoration (spec problem #3).
//!
//! ## Transparency
//!
//! No `bg` is set. Hierarchy comes from the glyph + BOLD + the explicit
//! label text (3 non-color cues for playing, 2 for the others).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::spinner::{is_buffering, spinner_glyph};
use crate::tui::view::theme::{ellipsis, pause_glyph, play_glyph, stop_glyph, Theme};

/// Parse the persisted, user-facing resume hint created by `main.rs` /
/// `App::on_tick`: `resume: {title} at {M:SS} · R to resume`.
/// Returns the actual title and position so metadata and state can render
/// them separately (the title must never include the `resume:` action).
pub fn resume_parts(hint: &str) -> (String, Option<String>) {
    let clean = hint.strip_prefix("resume: ").unwrap_or(hint);
    let clean = clean.split(" · ").next().unwrap_or(clean);
    if let Some((title, position)) = clean.rsplit_once(" at ") {
        return (title.to_string(), Some(position.to_string()));
    }
    (clean.to_string(), None)
}

/// Render the playback state row into `area` (1 row tall). The row
/// always starts with the state glyph + label so the state survives
/// truncation at narrow widths. When width permits, the row includes
/// the action hint (`[Space] Pause`, `[Space] Resume from {M:SS}`,
/// `[R] Retry  [D] Diagnostics`, etc.).
pub fn render_state(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_state_line(app, &theme, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Build the state line. Exposed for callers that compose a multi-row
/// layout (the wide/medium/compact layouts use this directly).
pub fn build_state_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let (glyph, label, style) = state_pieces(app, theme);
    let glyph_w = crate::tui::view::theme::disp_width(glyph);
    let label_w = crate::tui::view::theme::disp_width(label);

    // "▶ PLAYING" prefix
    let _prefix = format!("{glyph} {label}");
    let prefix_w = glyph_w + 1 + label_w;

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(glyph.to_string(), style),
        Span::raw(" "),
        Span::styled(label.to_string(), style),
    ];

    // Action hint (if room)
    let action = state_action(app, theme);
    if let Some((action_text, action_style)) = action {
        let action_w = crate::tui::view::theme::disp_width(&action_text);
        // Need: prefix_w + 2 (gap) + action_w
        if width >= prefix_w + 2 + action_w {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(action_text, action_style));
        }
    }

    Line::from(spans)
}

/// Pick the (glyph, label, style) triple for the current playback state.
fn state_pieces(app: &App, theme: &Theme) -> (&'static str, &'static str, Style) {
    let bold = Modifier::BOLD;
    if is_buffering(app) {
        return (
            spinner_glyph(app),
            "RESOLVING",
            Style::default().fg(theme.accent).add_modifier(bold),
        );
    }
    if app.player.is_playing() {
        return (
            play_glyph(),
            "PLAYING",
            Style::default().fg(theme.accent).add_modifier(bold),
        );
    }
    if app.now_playing.is_some() {
        return (
            pause_glyph(),
            "PAUSED",
            Style::default().fg(theme.warning).add_modifier(bold),
        );
    }
    // Stopped.
    (stop_glyph(), "STOPPED", Style::default().fg(theme.dim))
}

/// The action hint for the current state, if any. Returns `(text, style)`.
fn state_action(app: &App, theme: &Theme) -> Option<(String, Style)> {
    let key_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.dim);

    if is_buffering(app) {
        // No key hint — show a textual "Finding the stream…" indicator.
        return Some((format!("Finding the stream{}", ellipsis()), dim_style));
    }
    if app.player.is_playing() {
        return Some(("[Space] Pause".to_string(), key_style));
    }
    if app.now_playing.is_some() {
        return Some(("[Space] Play".to_string(), key_style));
    }
    // Stopped with resume?
    if app.resume_hint.is_some() {
        let (_, position) = resume_parts(app.resume_hint.as_deref().unwrap_or_default());
        if let Some(position) = position {
            return Some((format!("[Space] Resume from {position}"), key_style));
        }
        return Some(("[Space] Resume".to_string(), key_style));
    }
    None
}
