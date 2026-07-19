//! Source + connection status component for the Now Playing Deck.
//!
//! Renders connectivity as **secondary metadata** (spec problem #11:
//! "The connection status is more prominent than important track
//! metadata"):
//!
//! ```text
//! YOUTUBE · ● Connected
//! YOUTUBE · ○ Disconnected
//! YOUTUBE · ⠹ Connecting
//! ```
//!
//! The styling is subdued (dim) so it never visually competes with the
//! track title, artist, progress, or controls. The big centered
//! `[ok] YT connected` status message is removed (spec calls this out).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::spinner::spinner_glyph;
use crate::tui::view::theme::{is_ascii, sep_dot, Theme};

/// Render the source + connection status into `area` (1 row).
pub fn render_source_status(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_source_status_line(app, &theme, area.width as usize);
    if let Some(line) = line {
        f.render_widget(
            Paragraph::new(line),
            Rect::new(area.x, area.y, area.width, 1),
        );
    }
}

/// Build the source + connection status line. Returns `None` when no
/// source is relevant (e.g. local-only mode with no track playing and
/// no YouTube session). Exposed for layout composers.
pub fn build_source_status_line(app: &App, theme: &Theme, width: usize) -> Option<Line<'static>> {
    let dim_style = Style::default().fg(theme.dim);
    let connected_style = Style::default().fg(theme.success);
    let disconnected_style = Style::default().fg(theme.dim);
    let connecting_style = Style::default()
        .fg(theme.warning)
        .add_modifier(Modifier::BOLD);
    let error_style = Style::default()
        .fg(theme.error)
        .add_modifier(Modifier::BOLD);

    let sd = sep_dot();
    let _ = sd;

    // Source name: "YOUTUBE" or "LOCAL".
    let now_remote = app
        .now_playing
        .as_ref()
        .map(|ts| ts.is_remote())
        .unwrap_or(false);
    let source_name = if now_remote || app.source_mode != crate::mode::SourceMode::Local {
        "YOUTUBE"
    } else {
        "LOCAL"
    };

    // Connection state (only relevant for YouTube).
    use crate::yt::state::YtState;
    let conn = if app.source_mode == crate::mode::SourceMode::Local && !now_remote {
        // Local: no connection state. Just "LOCAL".
        return Some(Line::from(vec![Span::styled("LOCAL", dim_style)]));
    } else {
        let disconnected = if is_ascii() {
            "o Disconnected".to_string()
        } else {
            "○ Disconnected".to_string()
        };
        let connecting = format!("{} Connecting", spinner_glyph(app));
        let connected = if is_ascii() {
            "* Connected".to_string()
        } else {
            "● Connected".to_string()
        };
        match app.yt_state {
            YtState::Unconfigured | YtState::SignedOut => (disconnected, disconnected_style),
            YtState::Authenticating | YtState::AuthenticatedNotSynced | YtState::Synchronizing => {
                (connecting, connecting_style)
            }
            YtState::Ready => {
                if app.yt_error.is_some() {
                    ("! Error".to_string(), error_style)
                } else {
                    (connected, connected_style)
                }
            }
            YtState::RateLimited => (
                format!("{} Rate-limited", spinner_glyph(app)),
                connecting_style,
            ),
            YtState::AuthExpired | YtState::ProviderError | YtState::Failed => {
                (disconnected, disconnected_style)
            }
            YtState::ReadyStale => ("~ Stale".to_string(), connecting_style),
        }
    };
    let (conn_label, conn_style) = conn;

    let line = format!("{source_name} {sd} {conn_label}");
    let line_w = crate::tui::view::theme::disp_width(&line);
    if width < line_w {
        // Too narrow — just show the source name.
        return Some(Line::from(vec![Span::styled(
            source_name.to_string(),
            dim_style,
        )]));
    }
    Some(Line::from(vec![
        Span::styled(source_name.to_string(), dim_style),
        Span::raw(format!(" {sd} ")),
        Span::styled(conn_label, conn_style),
    ]))
}
