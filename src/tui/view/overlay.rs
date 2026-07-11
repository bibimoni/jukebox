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
    let Some(overlay) = app.overlay.clone() else {
        return;
    };
    match overlay {
        Overlay::Search {
            input,
            results,
            cursor,
            scope,
            submitted,
            searching,
        } => {
            render_search(
                f, area, app, &input, &results, cursor, scope, &submitted, searching,
            );
        }
        Overlay::Help => render_help(f, area, app.help_scroll),
        Overlay::PlaylistPicker { track_id, cursor } => {
            render_playlist_picker(f, area, app, &track_id, cursor)
        }
        Overlay::Command { input, cursor } => render_command(f, area, &input, cursor),
        Overlay::YtAuth { input } => render_yt_auth(f, area, &input),
        Overlay::Discover { items, cursor } => render_discover(f, area, &items, cursor),
        Overlay::Lyrics {
            content,
            state,
            scroll,
            ..
        } => render_lyrics_overlay(f, area, app, content.as_ref(), &state, scroll),
        Overlay::Diagnostics => {
            crate::tui::view::diagnostics::render(f, area, &app.diagnostics);
        }
    }
}

/// The discover overlay (`S`): a centered list of suggested albums / YT
/// playlists. `Enter` plays the selection (wired in input.rs).
fn render_discover(
    f: &mut Frame,
    area: Rect,
    items: &[crate::tui::app::DiscoverItem],
    cursor: usize,
) {
    let theme = Theme::default();
    let popup = centered(area, 55, 45);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            " discover — press Enter to play ",
            Style::default().fg(theme.accent),
        ));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (glyph, text) = match d {
                crate::tui::app::DiscoverItem::Album { artist, album } => {
                    ("♫", format!("{artist} — {album}"))
                }
                crate::tui::app::DiscoverItem::Playlist { name, .. } => ("✦", name.clone()),
            };
            let style = if i == cursor {
                Style::default().fg(theme.hi_fg).bg(theme.accent)
            } else {
                Style::default().fg(theme.text)
            };
            Line::from(Span::styled(format!("{glyph} {text}"), style))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// The YouTube cookie-paste overlay (spec §5.7). A centered popup with the
/// paste instructions and the accumulating cookie text; `Enter` saves.
fn render_yt_auth(f: &mut Frame, area: Rect, input: &str) {
    let theme = Theme::default();
    let popup = centered(area, 70, 40);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            " YouTube auth ",
            Style::default().fg(theme.accent),
        ));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Paste your YouTube cookies (Premium recommended).",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "Export from a logged-in youtube.com tab with a",
            Style::default().fg(theme.dim),
        )),
        Line::from(Span::styled(
            "\"Get cookies.txt\" browser extension, then paste:",
            Style::default().fg(theme.dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.accent)),
            Span::styled(input.to_string(), Style::default().fg(theme.text)),
            Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: save & connect    ·    Esc: cancel",
            Style::default().fg(theme.dim),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
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

/// Resolve a track id to a `Title — Artist` display string. Catalog tracks
/// (Local/Mixed local results) resolve from the catalog; YouTube video ids
/// resolve from the session's `track_cache` (populated by search). Falls back
/// to the raw id only when neither has the metadata yet.
fn track_label(app: &App, id: &str) -> String {
    if let Some(Track {
        title,
        primary_artist,
        ..
    }) = app.catalog.tracks.iter().find(|t| t.id == id)
    {
        return format!("{title} — {primary_artist}");
    }
    if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
        if rt.artist.is_empty() {
            return rt.title.clone();
        }
        return format!("{} — {}", rt.title, rt.artist);
    }
    id.to_string()
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_search(
    f: &mut Frame,
    area: Rect,
    app: &App,
    input: &str,
    results: &[String],
    cursor: usize,
    scope: crate::tui::app::SearchScope,
    submitted: &Option<String>,
    searching: bool,
) {
    let theme = Theme::default();
    let popup = centered(area, 60, 60);

    // Clear the popup region so browse chrome doesn't bleed through.
    f.render_widget(Clear, popup);

    let title = format!(" search · {} ", scope.as_str());
    let inner = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(title, Style::default().fg(theme.accent)));
    let inner_area = inner.inner(popup);
    f.render_widget(inner, popup);

    // Row 0: query input. Row 1: a one-line status/hint (Youtube scope only).
    // Row 2: the results list. The status row is reserved for Youtube scope so
    // the results list doesn't jump when the query state changes; for Local
    // scope (instant) row 1 is blank and results fill from row 2.
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner_area);

    // Input line: `/ query` with a block cursor on the trailing cell.
    let input_line = Line::from(vec![
        Span::styled("/ ", Style::default().fg(theme.accent)),
        Span::styled(input.to_string(), Style::default().fg(theme.text)),
        Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    f.render_widget(input_line, rows[0]);

    // Status/hint line (Youtube scope). Local scope has no roundtrip, so it
    // needs no "searching…" indicator — leave the row blank for stable layout.
    let dim = Style::default().fg(theme.dim);
    let status: Line = match scope {
        crate::tui::app::SearchScope::Local => Line::from(Span::styled(
            "Tab → youtube   ·   Enter plays selection",
            dim,
        )),
        crate::tui::app::SearchScope::Youtube => {
            if searching {
                Line::from(Span::styled("searching…   (Tab → local · Esc cancel)", dim))
            } else if input.trim().is_empty() {
                Line::from(Span::styled(
                    "type a query, then Enter to search   ·   Tab → local",
                    dim,
                ))
            } else if results.is_empty() && submitted.as_deref() == Some(input) {
                // A search ran and returned nothing.
                Line::from(Span::styled(
                    "no results — edit the query or Tab → local",
                    dim,
                ))
            } else if submitted.as_deref() == Some(input) && !results.is_empty() {
                Line::from(Span::styled(
                    "↑↓ select   ·   Enter plays   ·   Tab → local",
                    dim,
                ))
            } else {
                Line::from(Span::styled(
                    "Enter to search YouTube   ·   Tab → local",
                    dim,
                ))
            }
        }
    };
    f.render_widget(status, rows[1]);

    // Results list.
    let items: Vec<ListItem> = results
        .iter()
        .map(|id| ListItem::new(track_label(app, id)))
        .collect();
    let mut state = ListState::default();
    state.select(Some(cursor));
    f.render_stateful_widget(
        List::new(items).highlight_style(Style::default().fg(theme.hi_fg).bg(theme.accent)),
        rows[2],
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
        group(
            "navigation",
            "h j k l · arrows   move (←→ columns, ↑↓ within)",
        ),
        group("", "gg / G   top / bottom of column"),
        group(
            "",
            "1 2 3 4   switch view: Artists / Playlists / Queue / YouTube",
        ),
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
        group("", "c   cycle continue (mode-dependent)"),
        group("", "M   cycle source mode (Local / YouTube / Mixed)"),
        Line::from(""),
        group(
            "discover",
            "s   instant random track   ·   S   discover overlay",
        ),
        group(
            "",
            "f   filter focused column   ·   Enter on filter jumps to match",
        ),
        Line::from(""),
        group(
            "modes",
            "/   search (scoped to view)   ·   ?   help   ·   :   command",
        ),
        group("", "a   add to playlist"),
        group("", "L   lyrics for the playing track (synced/plain)"),
        group("", "D   diagnostics overlay (recent provider errors)"),
        group(
            "",
            "e   enqueue (play next)   ·   x   remove from queue   ·   d   delete playlist",
        ),
        group(
            "",
            "R   retry YouTube connection (after error / rate-limit)",
        ),
        group(
            "",
            ":yt auth  paste cookies  ·  :yt auth browser <chrome|firefox|safari|edge|brave>",
        ),
        group("", ":yt logout / :yt setup   clear cookies / install deps"),
        group("", ":queue clear   empty the play-next queue"),
        group("", "↑ / ↓   move search-result / discover selection"),
        group("", "Esc   close overlay / cancel"),
        group("", "q   quit"),
        Line::from(""),
        group("mouse", "click row — focus + select"),
        group("", "click progress — seek"),
        group("", "wheel — scroll focused column"),
    ]
}

fn render_help(f: &mut Frame, area: Rect, scroll: u16) {
    let theme = Theme::default();
    let popup = centered(area, 64, 86);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            " help — j/k scroll · Esc to close ",
            Style::default().fg(theme.accent),
        ));
    f.render_widget(
        Paragraph::new(help_lines())
            .scroll((scroll, 0))
            .block(block),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Playlist picker
// ---------------------------------------------------------------------------

fn render_playlist_picker(f: &mut Frame, area: Rect, app: &App, track_id: &str, cursor: usize) {
    let theme = Theme::default();
    let popup = centered(area, 50, 50);
    f.render_widget(Clear, popup);

    let title = format!(
        " add \"{}\" to playlist · Enter confirm ",
        track_label(app, track_id)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(title, Style::default().fg(theme.accent)));
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

    let mut state = ListState::default();
    state.select(Some(cursor.min(items.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items).highlight_style(Style::default().fg(theme.hi_fg).bg(theme.accent)),
        inner,
        &mut state,
    );
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

fn render_command(f: &mut Frame, area: Rect, input: &str, cursor: usize) {
    let theme = Theme::default();
    // One-line strip at the very bottom of the screen.
    let strip = Rect {
        height: 1u16,
        y: area.height.saturating_sub(1),
        x: area.x,
        width: area.width,
    };
    f.render_widget(Clear, strip);

    // Show the input with the block cursor `▏` at the cursor position.
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];
    let after = &input[cursor..];

    let line = Line::from(vec![
        Span::styled(":", Style::default().fg(theme.accent)),
        Span::styled(before.to_string(), Style::default().fg(theme.text)),
        Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        Span::styled(after.to_string(), Style::default().fg(theme.text)),
    ])
    .alignment(Alignment::Left);
    f.render_widget(Paragraph::new(line), strip);
}

/// Lyrics overlay (`L`): shows timestamped or plain lyrics with a scrollable
/// viewport. The `state` controls the truthful lifecycle label (loading /
/// unavailable / error); `content` is the parsed lyrics once loaded.
fn render_lyrics_overlay(
    f: &mut Frame,
    area: Rect,
    _app: &crate::tui::app::App,
    content: Option<&crate::lyrics::Lyrics>,
    state: &crate::tui::app::LyricsState,
    scroll: u16,
) {
    use crate::tui::app::LyricsState;
    use ratatui::widgets::{Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation};

    let theme = crate::tui::view::theme::Theme::default();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Lyrics ", Style::default().fg(theme.accent)))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = match state {
        LyricsState::Idle => "Press any key or wait — fetching lyrics…".into(),
        LyricsState::Loading => "Loading lyrics…".into(),
        LyricsState::NotFound => "Lyrics unavailable for this track.".into(),
        LyricsState::Error(msg) => format!("Lyrics error: {msg}"),
        LyricsState::Available(_synced) => {
            if let Some(lyrics) = content {
                if lyrics.lines.is_empty() {
                    "Lyrics are empty.".into()
                } else {
                    lyrics
                        .lines
                        .iter()
                        .map(|l| l.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            } else {
                "Lyrics unavailable.".into()
            }
        }
    };

    let para = Paragraph::new(text)
        .scroll((scroll, 0))
        .style(Style::default().fg(theme.text));
    f.render_widget(para, inner);

    // Scrollbar on the right edge.
    let total = content.map(|l| l.lines.len() as u16).unwrap_or(1);
    if total > inner.height {
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            inner,
            &mut ratatui::widgets::ScrollbarState::new(total as usize).position(scroll as usize),
        );
    }
}
