//! The always-visible footer hint bar: the 5–6 most-used keys, so basic
//! actions are discoverable without `?` (spec §5.1 cut #2). The full keymap
//! lives behind `?`.

use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::theme::Theme;

/// Render the 1-line footer. Derives the YT provider label from `app.yt_state`
/// (the truthful state machine) rather than the old freeform `yt_status`/
/// `yt_error` strings that could claim "connected" before any data fetch
/// verified the credential (the "connected but empty" bug, yt-recon §8/§10).
///
/// Priority: error detail (yt_error) > state label (yt_state) > key-hint bar.
/// The state label is ALWAYS shown for non-Ready states (so the user knows the
/// provider's status); `yt_error` adds detail when there's a specific error
/// message. Ready is silent (falls through to the hint bar) so the footer
/// isn't noisy when everything works.
pub fn render(f: &mut Frame, area: &ratatui::layout::Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });
    let line = footer_line(app, &theme, &dim, area.width);
    f.render_widget(
        Paragraph::new(line.alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        *area,
    );
}

/// Build the footer line: a YT provider status (from `yt_state`) when the
/// provider isn't silently Ready, else the key-hint bar. Exposed for unit
/// testing the derived label without rendering. `width` is the footer area
/// width so the hint bar can collapse low-priority hints on narrow terminals.
pub fn footer_line(app: &App, theme: &Theme, dim: &Style, width: u16) -> Line<'static> {
    use crate::yt::state::YtState;
    // Ready with a transient non-state message (e.g. "upgraded to AAC 256k"):
    // show that message (accent-colored), like the old footer did.
    if app.yt_state == YtState::Ready {
        if let Some(msg) = &app.yt_status {
            let color = if no_color() {
                Color::Reset
            } else {
                theme.accent
            };
            return Line::from(Span::styled(msg.clone(), Style::default().fg(color)));
        }
        return hint_line(app, dim, width);
    }
    // Non-Ready: derive from the state machine. The label already embeds the
    // recovery action (e.g. "not configured — run :yt auth browser").
    let label = app.yt_state.human_label();
    let icon = app.yt_state.icon();
    // Color: red for hard errors, yellow for degraded/retry states, accent for
    // transient/informational states. Reset under NO_COLOR (accessibility: the
    // icon + label distinguish states without color).
    let color = if no_color() {
        Color::Reset
    } else {
        match app.yt_state {
            YtState::AuthExpired | YtState::ProviderError | YtState::Failed => Color::Red,
            YtState::RateLimited | YtState::ReadyStale => Color::Yellow,
            // Transient + authed-not-synced + unconfigured + signed-out:
            // accent (informational, not an error).
            _ => theme.accent,
        }
    };
    let style = Style::default().fg(color);
    // Compose: "[icon] YT: label" + optional " — detail" from yt_error.
    // The icon is ASCII-safe (accessibility: distinguishable without color).
    let prefix = match icon {
        Some(ic) => format!("{ic} YT: {label}"),
        None => format!("YT: {label}"),
    };
    let detail = app
        .yt_error
        .as_deref()
        .map(|e| format!(" — {e}"))
        .unwrap_or_default();
    Line::from(Span::styled(format!("{prefix}{detail}"), style))
}

/// The key-hint bar, ordered by priority so the most discoverable keys survive
/// narrow terminals. Priority: `Enter play` > `q quit` > `? help` >
/// `> < next prev` > `M mode` > `/ search`. Below 60 cols only the top 3 are
/// shown so `Enter play · q quit · ? help` always fits.
fn hint_line(app: &App, dim: &Style, width: u16) -> Line<'static> {
    let sep = " · ";
    let search_hint = if app.view == crate::tui::app::View::Youtube {
        "/ search YT"
    } else {
        "/ search"
    };
    let mut parts: Vec<String> = vec![
        "Enter play".to_string(),
        "q quit".to_string(),
        "? help".to_string(),
    ];
    if width >= 60 {
        parts.push("> < next prev".to_string());
        parts.push("M mode".to_string());
        parts.push(search_hint.to_string());
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, s) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(sep));
        }
        spans.push(Span::styled(s.clone(), *dim));
    }
    Line::from(spans)
}

fn no_color() -> bool {
    crate::tui::view::theme::no_color()
}
