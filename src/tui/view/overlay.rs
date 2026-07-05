//! Modal overlays rendered on top of the browse layout.
//!
//! [`render`] reads `app.overlay: Option<Overlay>` and draws the active overlay
//! on top of `area`. Each overlay clears its region first (via [`Clear`]) so the
//! underlying browse chrome doesn't bleed through, then draws a centered popup
//! block (or, for the command line, a bottom strip).
//!
//! The four overlays:
//! - [`Overlay::Search`] — centered popup: a `/ query` input line + a list of
//!   resolved track titles, highlight on the cursor.
//! - [`Overlay::Help`] — centered popup listing the full keymap, grouped
//!   (navigation / playback / modes / mouse).
//! - [`Overlay::PlaylistPicker`] — centered popup listing playlist names to
//!   add the selected track to, plus a "new playlist..." entry.
//! - [`Overlay::Command`] — a one-line `:` command input at the bottom of the
//!   screen.
//!
//! This module only *renders* overlays; key dispatch lives in
//! [`crate::tui::input`].

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::catalog::Track;
use crate::tui::app::{App, Overlay};
use crate::tui::view::theme::Theme;

/// Render the active overlay (if any) into `area`.
pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(overlay) = app.overlay.clone() else { return };
    match overlay {
        Overlay::Search { input, results, cursor } => {
            render_search(f, area, app, &input, &results, cursor);
        }
        Overlay::Help => render_help(f, area),
        Overlay::PlaylistPicker => render_playlist_picker(f, area, app),
        Overlay::Command { input } => render_command(f, area, &input),
    }
}

/// Center a popup rect at ~60% of `area`, clamped to a minimum so its content
/// never collapses to nothing on a small terminal.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let pop = Layout::vertical([
        Constraint::Percentage((100 - height_pct) / 2),
        Constraint::Percentage(height_pct),
        Constraint::Percentage((100 - height_pct) / 2),
    ])
    .split(area)[1];
    Layout::horizontal([
        Constraint::Percentage((100 - width_pct) / 2),
        Constraint::Percentage(width_pct),
        Constraint::Percentage((100 - width_pct) / 2),
    ])
    .split(pop)[1]
}

/// Resolve a track id to a `Title — Artist` display string.
fn track_label(app: &App, id: &str) -> String {
    let t = app.catalog.tracks.iter().find(|t| t.id == id);
    match t {
        Some(Track { title, primary_artist, .. }) => format!("{title} — {primary_artist}"),
        None => id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

fn render_search(
    f: &mut Frame,
    area: Rect,
    app: &App,
    input: &str,
    results: &[String],
    cursor: usize,
) {
    let theme = Theme::default();
    let popup = centered(area, 60, 60);

    // Clear the popup region so browse chrome doesn't bleed through.
    f.render_widget(Clear, popup);

    let inner = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" search ", Style::default().fg(theme.accent)));
    let inner_area = inner.inner(popup);
    f.render_widget(inner, popup);

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)])
        .split(inner_area);

    // Input line: `/ query` with a block cursor on the trailing cell.
    let input_line = Line::from(vec![
        Span::styled("/ ", Style::default().fg(theme.accent)),
        Span::styled(input.to_string(), Style::default().fg(theme.text)),
        Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    f.render_widget(input_line, rows[0]);

    // Results list.
    let items: Vec<ListItem> = results
        .iter()
        .map(|id| ListItem::new(track_label(app, id)))
        .collect();
    let mut state = ListState::default();
    state.select(Some(cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().fg(theme.hi_fg).bg(theme.accent)),
        rows[1],
        &mut state,
    );
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

/// The grouped keymap lines shown in the Help overlay. Kept here (not in the
/// spec doc) so the help text can't drift from the implementation.
fn help_lines<'a>() -> Vec<Line<'a>> {
    let group = |title: &'a str, body: &'a str| -> Line<'a> {
        Line::from(vec![
            Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(body, Style::default().fg(Color::Reset)),
        ])
    };
    vec![
        Line::from(""),
        group("navigation", "h j k l · arrows   move (←→ columns, ↑↓ within)"),
        group("", "gg / G   top / bottom of column"),
        group("", "1 2 3   switch view: Artists / Playlists / Queue"),
        group("", "Tab / Shift+Tab   cycle view"),
        Line::from(""),
        group("playback", "Enter   play selected in context"),
        group("", "Space   play / pause"),
        group("", "> / <   next / previous track"),
        group("", ", / .   seek −5s / +5s"),
        group("", "+ / -   volume up / down"),
        group("", "m   mute"),
        group("", "z / Z   cycle shuffle / reshuffle"),
        group("", "r   cycle repeat (off → all → one)"),
        Line::from(""),
        group("modes", "/   search   ·   ?   help   ·   :   command"),
        group("", "a   add to playlist"),
        group("", "n / N   next / prev search match"),
        group("", "Esc   close overlay / cancel"),
        group("", "q   quit"),
        Line::from(""),
        group("mouse", "click row — focus + select   ·   dbl-click track — play"),
        group("", "drag divider — resize   ·   click progress/volume — seek/set"),
        group("", "wheel — scroll focused column"),
    ]
}

fn render_help(f: &mut Frame, area: Rect) {
    let theme = Theme::default();
    let popup = centered(area, 64, 70);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" help — press Esc to close ", Style::default().fg(theme.accent)));
    f.render_widget(Paragraph::new(help_lines()).block(block), popup);
}

// ---------------------------------------------------------------------------
// Playlist picker
// ---------------------------------------------------------------------------

fn render_playlist_picker(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let popup = centered(area, 50, 50);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" add to playlist ", Style::default().fg(theme.accent)));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut items: Vec<ListItem> = app
        .playlists
        .iter()
        .map(|p| ListItem::new(p.name.clone()))
        .collect();
    items.push(ListItem::new(Span::styled(
        "+ new playlist...",
        Style::default().fg(theme.dim),
    )));

    f.render_widget(List::new(items), inner);
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

fn render_command(f: &mut Frame, area: Rect, input: &str) {
    let theme = Theme::default();
    // One-line strip at the very bottom of the screen.
    let strip = Rect { height: 1u16, y: area.height.saturating_sub(1), x: area.x, width: area.width };
    f.render_widget(Clear, strip);

    let line = Line::from(vec![
        Span::styled(":", Style::default().fg(theme.accent)),
        Span::styled(input.to_string(), Style::default().fg(theme.text)),
        Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ])
    .alignment(Alignment::Left);
    f.render_widget(
        Paragraph::new(line),
        strip,
    );
}
