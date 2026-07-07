//! Miller columns + view-switcher rail.
//!
//! Renders the left rail (A/P/Q// switcher with the active `View` highlighted)
//! and the main browse area split into columns per the active view:
//!
//! - **Artists**: col1 = artists, col2 = albums of the focused artist, col3 =
//!   tracks of the focused album (`# Title Album Quality` rows, `▶` on the
//!   now-playing track, highlight on `cursors.track`).
//! - **Playlists**: col1 = playlist names, col2 = tracks of the focused
//!   playlist (same row format). col3 collapses.
//! - **Queue**: a single column listing `transport.manual_queue` ids resolved
//!   to titles.
//!
//! The column matching `app.focus_col` gets the accent focus border; the
//! others get a dim unfocused border. Track rows use [`pad_between`] so CJK /
//! wide titles still align against the right-anchored quality tag.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::app::{App, View};
use crate::tui::view::theme::{pad_between, Theme};

/// Render the rail + columns into `area` using state from `app`.
pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = Theme::default();
    let split = Layout::horizontal([Constraint::Length(app.column_widths.rail), Constraint::Min(1)])
        .split(area);
    let rail_area = split[0];
    let main_area = split[1];

    render_rail(f, rail_area, app, &theme);

    match app.view {
        View::Artists => render_artists(f, main_area, app, &theme),
        View::Playlists => render_playlists(f, main_area, app, &theme),
        View::Queue => render_queue(f, main_area, app, &theme),
    }
}

/// A titled border whose color reflects focus: accent when `focused`, dim
/// otherwise. Used to frame every Miller column.
fn border<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let color = if focused { theme.accent } else { theme.dim };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color)))
}

// --- Rail -------------------------------------------------------------------

/// The left switcher rail. Four single-letter rows (A/P/Q//) highlight the
/// active `View` with the accent color.
fn render_rail(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let accent = Style::default().fg(theme.accent);
    let dim = Style::default().fg(theme.dim);

    let rows = [
        ('A', View::Artists),
        ('P', View::Playlists),
        ('Q', View::Queue),
    ];

    let lines: Vec<Line> = rows
        .iter()
        .map(|(g, v)| {
            let style = if app.view == *v { accent } else { dim };
            Line::from(Span::styled(g.to_string(), style))
        })
        .chain(std::iter::once(Line::from(Span::styled("/", dim))))
        .collect();

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::NONE)),
        area,
    );
}

// --- Artists view -----------------------------------------------------------

fn render_artists(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let cw = &app.column_widths;
    let cols = Layout::horizontal([
        Constraint::Length(cw.col1),
        Constraint::Length(cw.col2),
        Constraint::Min(cw.col3),
    ])
    .split(area);

    let artist_area = cols[0];
    let album_area = cols[1];
    let track_area = cols[2];

    // col1: artists.
    let items: Vec<ListItem> = app
        .artists
        .iter()
        .map(|a| ListItem::new(a.clone()))
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.cursors.artist));
    f.render_stateful_widget(
        List::new(items)
            .block(border("Artists", app.focus_col == 0, theme))
            .highlight_style(Style::default().fg(theme.accent)),
        artist_area,
        &mut state,
    );

    // col2: albums for the focused artist.
    let artist = app
        .artists
        .get(app.cursors.artist)
        .cloned()
        .unwrap_or_default();
    let albums = app
        .albums_by_artist
        .get(&artist)
        .cloned()
        .unwrap_or_default();
    let album_items: Vec<ListItem> = albums
        .iter()
        .map(|a| ListItem::new(a.title.clone()))
        .collect();
    let mut album_state = ListState::default();
    album_state.select(Some(app.cursors.album));
    f.render_stateful_widget(
        List::new(album_items)
            .block(border("Albums", app.focus_col == 1, theme))
            .highlight_style(Style::default().fg(theme.accent)),
        album_area,
        &mut album_state,
    );

    // col3: tracks for the focused album — the FULL album across all
    // primary_artists, not just the focused artist's copy (collaboration
    // albums have tracks under several primary_artists; the album is a
    // cohesive object). See `App::tracks_for_album`.
    let focused_album = albums.get(app.cursors.album).cloned();
    let track_ids: Vec<String> = match &focused_album {
        Some(a) => app.tracks_for_album(&a.title),
        None => vec![],
    };
    let track_lines = track_rows(app, &track_ids, track_area.width.saturating_sub(2) as usize, theme);
    f.render_widget(
        Paragraph::new(track_lines).block(border("Tracks", app.focus_col == 2, theme)),
        track_area,
    );
}

// --- Playlists view ---------------------------------------------------------

fn render_playlists(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let cw = &app.column_widths;
    let cols = Layout::horizontal([Constraint::Length(cw.col1), Constraint::Min(cw.col2)]).split(area);

    // col1: playlist names.
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .map(|p| ListItem::new(p.name.clone()))
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.cursors.playlist));
    f.render_stateful_widget(
        List::new(items)
            .block(border("Playlists", app.focus_col == 0, theme))
            .highlight_style(Style::default().fg(theme.accent)),
        cols[0],
        &mut state,
    );

    // col2: tracks of the focused playlist.
    let ids = app
        .playlists
        .get(app.cursors.playlist)
        .map(|p| p.track_ids.clone())
        .unwrap_or_default();
    let lines = track_rows(app, &ids, cols[1].width.saturating_sub(2) as usize, theme);
    f.render_widget(
        Paragraph::new(lines).block(border("Tracks", app.focus_col == 1, theme)),
        cols[1],
    );
}

// --- Queue view -------------------------------------------------------------

fn render_queue(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let ids = app.transport.manual_queue.clone();
    let lines = track_rows(app, &ids, area.width.saturating_sub(2) as usize, theme);
    f.render_widget(
        Paragraph::new(lines).block(border("Queue", app.focus_col == 0, theme)),
        area,
    );
}

// --- Track rows -------------------------------------------------------------

/// Build the track-column rows: `# Title Album Quality` with the right side
/// (album + quality) right-anchored via [`pad_between`] so wide/CJK titles keep
/// alignment. The now-playing track is prefixed with `▶`; the row under
/// `cursors.track` is rendered with the accent color (selection highlight).
fn track_rows(app: &App, ids: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim);
    let accent = Style::default().fg(theme.accent);

    ids.iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let t = app.catalog.tracks.iter().find(|t| &t.id == id)?;
            let np = app.now_playing.as_ref().map(|s| s.id()) == Some(id.as_str());
            let glyph = if np { "▶" } else { " " };
            let num = format!("{:>2}", i + 1);
            let album = t.album.as_deref().unwrap_or("");
            let left = format!("{glyph} {num} {} — {album}", t.title);
            let quality = t.quality_label();
            let line = pad_between(&left, &quality, width);
            let selected = i == app.cursors.track;
            let style = if selected || np { accent } else { dim };
            Some(Line::from(Span::styled(line, style)))
        })
        .collect()
}

