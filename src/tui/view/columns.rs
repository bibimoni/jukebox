//! Miller columns + view-switcher rail.
//!
//! Renders the left rail (1/2/3/4 switcher — matching the view-switch keys —
//! with the active `View` highlighted) and the main browse area split into
//! columns per the active view:
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
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::app::{App, View};
use crate::tui::view::theme::{pad_between, Theme};

/// Render the rail + columns into `area` using state from `app`.
pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = Theme::default();
    let split = Layout::horizontal([
        Constraint::Length(app.column_widths.rail),
        Constraint::Min(1),
    ])
    .split(area);
    let rail_area = split[0];
    let main_area = split[1];

    render_rail(f, rail_area, app, &theme);

    match app.view {
        View::Artists => render_artists(f, main_area, app, &theme),
        View::Playlists => render_playlists(f, main_area, app, &theme),
        View::Queue => render_queue(f, main_area, app, &theme),
        View::Youtube => render_youtube(f, main_area, app, &theme),
    }
}

/// A titled border whose color reflects focus: accent when `focused`, dim
/// otherwise. Used to frame every Miller column.
///
/// In addition to color, the focused column gets a `Thick` (double-line) border
/// and unfocused a `Plain` one, so focus is still visible under `NO_COLOR`
/// (where both colors collapse to `Reset`).
fn border<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let color = if focused { theme.accent } else { theme.dim };
    let bt = if focused {
        BorderType::Thick
    } else {
        BorderType::Plain
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(bt)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color)))
}

/// The column title with an inline filter prompt appended when the filter is
/// active on this column: `Artists` → `Artists (filter: ade▏)`.
fn filtered_title(base: &str, app: &App, col: usize) -> String {
    if let Some(f) = &app.filter {
        if f.col == col && !f.text.is_empty() {
            return format!("{base} (filter: {}▏)", f.text);
        }
    }
    base.to_string()
}

// --- Rail -------------------------------------------------------------------

/// A dim, centered one-line message used for empty-state and filter-no-match
/// hints inside a column border. The caller attaches the column `block`.
fn dim_centered(msg: String, theme: &Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        msg,
        Style::default().fg(theme.dim),
    )))
    .alignment(Alignment::Center)
}

/// The filter text when the filter is active on `col`, else `None`.
fn filter_text_on(app: &App, col: usize) -> Option<&str> {
    app.filter.as_ref().and_then(|f| {
        if f.col == col && !f.text.is_empty() {
            Some(f.text.as_str())
        } else {
            None
        }
    })
}

/// The left switcher rail. `1`/`2`/`3`/`4` rows highlight the active `View`
/// with the accent color — the glyphs match the actual view-switch keys
/// (the old `A`/`P`/`Q`/`Y` glyphs were mnemonic but didn't match the keys).
fn render_rail(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let accent = Style::default().fg(theme.accent);
    let dim = Style::default().fg(theme.dim);

    let rows = [
        ('1', View::Artists),
        ('2', View::Playlists),
        ('3', View::Queue),
        ('4', View::Youtube),
    ];

    let lines: Vec<Line> = rows
        .iter()
        .map(|(g, v)| {
            let style = if app.view == *v { accent } else { dim };
            Line::from(Span::styled(g.to_string(), style))
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::NONE)),
        area,
    );
}

// --- YouTube view ----------------------------------------------------------

/// Narrow fallback (spec §5.6): rail + a single focused pane with a breadcrumb
/// title. `h`/`l` drills in/out (focus_col changes which column is shown).
pub fn render_narrow(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = Theme::default();
    let split = Layout::horizontal([
        Constraint::Length(app.column_widths.rail),
        Constraint::Min(1),
    ])
    .split(area);
    render_rail(f, split[0], app, &theme);
    let pane = split[1];

    let (title, lines): (String, Vec<Line>) = match app.view {
        View::Artists => match app.focus_col {
            0 => (
                "Artists".into(),
                app.artists
                    .iter()
                    .map(|a| Line::from(Span::styled(a.clone(), Style::default().fg(theme.text))))
                    .collect(),
            ),
            1 => {
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
                (
                    format!("Albums · {artist} ← Artists"),
                    albums
                        .iter()
                        .map(|a| {
                            Line::from(Span::styled(
                                a.title.clone(),
                                Style::default().fg(theme.text),
                            ))
                        })
                        .collect(),
                )
            }
            _ => {
                let artist = app
                    .artists
                    .get(app.cursors.artist)
                    .cloned()
                    .unwrap_or_default();
                let album = app
                    .albums_by_artist
                    .get(&artist)
                    .and_then(|a| a.get(app.cursors.album))
                    .map(|a| a.title.clone())
                    .unwrap_or_default();
                let ids = app.tracks_for_album(&album);
                (
                    format!("Tracks · {album} ← Albums · {artist}"),
                    track_rows(app, &ids, pane.width.saturating_sub(2) as usize, &theme),
                )
            }
        },
        View::Playlists => match app.focus_col {
            0 => (
                "Playlists".into(),
                app.playlists
                    .iter()
                    .map(|p| {
                        Line::from(Span::styled(
                            p.name.clone(),
                            Style::default().fg(theme.text),
                        ))
                    })
                    .collect(),
            ),
            _ => {
                let name = app
                    .playlists
                    .get(app.cursors.playlist)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let ids = app
                    .playlists
                    .get(app.cursors.playlist)
                    .map(|p| p.track_ids.clone())
                    .unwrap_or_default();
                (
                    format!("Tracks · {name} ← Playlists"),
                    track_rows(app, &ids, pane.width.saturating_sub(2) as usize, &theme),
                )
            }
        },
        View::Youtube => match app.focus_col {
            0 => (
                "YouTube".into(),
                app.yt_lists
                    .iter()
                    .map(|l| {
                        let g = if l.kind == crate::tui::app::YtListKind::Account {
                            "♫"
                        } else {
                            "✦"
                        };
                        Line::from(Span::styled(
                            format!("{g} {}", l.name),
                            Style::default().fg(theme.text),
                        ))
                    })
                    .collect(),
            ),
            _ => {
                let name = app
                    .yt_lists
                    .get(app.cursors.playlist)
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                let ids = app
                    .yt_lists
                    .get(app.cursors.playlist)
                    .map(|l| l.track_ids.clone())
                    .unwrap_or_default();
                (
                    format!("Tracks · {name} ← YouTube"),
                    yt_track_rows(app, &ids, pane.width.saturating_sub(2) as usize, &theme),
                )
            }
        },
        View::Queue => (
            "Queue".into(),
            track_rows(
                app,
                &app.transport.manual_queue.clone(),
                pane.width.saturating_sub(2) as usize,
                &theme,
            ),
        ),
    };

    f.render_widget(
        Paragraph::new(lines).block(border(&title, true, &theme)),
        pane,
    );
}

/// Render the Y view: col1 = YT lists (account ♫ + suggested ✦), col2 = the
/// focused list's tracks. Below the tracks, a "Suggested / Up Next" pane
/// lists the other suggested lists so short track lists don't waste space.
fn render_youtube(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let cw = &app.column_widths;
    // Split off a 3-row Up-Next pane at the bottom when there are suggested
    // lists to show; otherwise use the whole area for the 2-col browse.
    let has_suggestions = app
        .yt_lists
        .iter()
        .any(|l| l.kind == crate::tui::app::YtListKind::Suggested);
    let split = if has_suggestions && area.height > 8 {
        Layout::vertical([Constraint::Min(4), Constraint::Length(3)]).split(area)
    } else {
        Layout::vertical([Constraint::Min(1)]).split(area)
    };
    let browse_area = split[0];
    let upnext_area = split.get(1).copied();

    let cols = Layout::horizontal([Constraint::Length(cw.col1), Constraint::Min(cw.col2)])
        .split(browse_area);
    let dim = Style::default().fg(theme.dim);

    // col1: YT list names (♫ account, ✦ suggested), narrowed by the filter.
    // When a filter is active and excludes every list, show a "no matches"
    // hint instead of a bare empty list.
    let yt_title = filtered_title("YouTube", app, 0);
    let col1_block = border(&yt_title, app.focus_col == 0, theme);
    let items: Vec<ListItem> = app
        .yt_lists
        .iter()
        .filter(|l| app.filter_matches(&l.name))
        .map(|l| {
            let glyph = if l.kind == crate::tui::app::YtListKind::Account {
                "♫"
            } else {
                "✦"
            };
            ListItem::new(format!("{glyph} {}", l.name))
        })
        .collect();
    if items.is_empty() && !app.yt_lists.is_empty() && filter_text_on(app, 0).is_some() {
        let text = filter_text_on(app, 0).unwrap_or("");
        f.render_widget(
            dim_centered(format!("no matches for '{text}'"), theme).block(col1_block),
            cols[0],
        );
    } else {
        let mut state = ListState::default();
        state.select(Some(
            app.cursors.playlist.min(items.len().saturating_sub(1)),
        ));
        f.render_stateful_widget(
            List::new(items)
                .block(col1_block)
                .highlight_style(Style::default().fg(theme.accent)),
            cols[0],
            &mut state,
        );
    }

    // col2: tracks of the focused list, or a status line derived from the
    // truthful `yt_state` machine (M2) — NOT the old `yt_session.is_none()` +
    // `yt_error` pair that could show "not configured" when the sidecar was
    // actually up but the probe failed. Each state gets a distinct, actionable
    // message so the user knows what to do next.
    let ids = app
        .yt_lists
        .get(app.cursors.playlist)
        .map(|l| l.track_ids.clone())
        .unwrap_or_default();
    let body = yt_status_line(app, ids.is_empty());
    if ids.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(body, dim))).block(border(
                "Tracks",
                app.focus_col == 1,
                theme,
            )),
            cols[1],
        );
    } else {
        let lines = yt_track_rows(app, &ids, cols[1].width.saturating_sub(2) as usize, theme);
        f.render_widget(
            Paragraph::new(lines).block(border("Tracks", app.focus_col == 1, theme)),
            cols[1],
        );
    }

    // Up-Next pane: the other suggested lists, one per line as `▶ name →`.
    if let Some(up) = upnext_area {
        let sugg: Vec<&crate::tui::app::YtList> = app
            .yt_lists
            .iter()
            .filter(|l| l.kind == crate::tui::app::YtListKind::Suggested)
            .collect();
        let lines: Vec<Line> = sugg
            .iter()
            .map(|l| Line::from(Span::styled(format!("▶ {} →", l.name), dim)))
            .collect();
        f.render_widget(
            Paragraph::new(lines).block(border("Suggested / Up Next", false, theme)),
            up,
        );
    }
}

/// The Y-view col2 status line, derived from the truthful `yt_state` machine.
/// Each state gets a distinct, actionable message. `ids_empty` distinguishes
/// "no list selected" (select a list) from "list selected but empty" (the
/// fetch returned, just empty). Exposed for unit testing.
fn yt_status_line(app: &App, ids_empty: bool) -> String {
    use crate::yt::state::YtState;
    // An error detail takes precedence over the generic state label so a
    // specific failure (e.g. "retry failed: …") is visible in the Y view too.
    if let Some(e) = &app.yt_error {
        if app.yt_state == YtState::ProviderError
            || app.yt_state == YtState::AuthExpired
            || app.yt_state == YtState::RateLimited
        {
            return format!("YT: {} — {}", app.yt_state.human_label(), e);
        }
    }
    match app.yt_state {
        YtState::Unconfigured => {
            "YouTube not configured — run :yt auth browser <chrome>".to_string()
        }
        YtState::SignedOut => "signed out — run :yt auth to reconnect".to_string(),
        YtState::Authenticating => "authenticating…".to_string(),
        YtState::AuthenticatedNotSynced | YtState::Synchronizing => {
            if app.yt_lists_loading {
                "loading…".to_string()
            } else {
                "syncing…".to_string()
            }
        }
        YtState::Ready => {
            if ids_empty {
                "select a list to load its tracks".to_string()
            } else {
                String::new()
            }
        }
        YtState::ReadyStale => "offline — showing cached lists (press R to retry)".to_string(),
        YtState::RateLimited => "rate limited — wait, then press R".to_string(),
        YtState::AuthExpired => "authorization expired — run :yt auth browser <name>".to_string(),
        YtState::ProviderError => "provider error — press R to retry".to_string(),
        YtState::Failed => "failed — run :yt setup or check your installation".to_string(),
    }
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

    // col1: artists (narrowed by the inline filter when active on col 0).
    // An empty catalog shows a dim hint to index the library; a filter that
    // excludes every artist shows a "no matches" hint instead of a bare empty
    // list — so the column never reads as broken.
    let title = filtered_title("Artists", app, 0);
    let col1_block = border(&title, app.focus_col == 0, theme);
    if app.artists.is_empty() {
        f.render_widget(
            dim_centered(
                "no artists — run `jukebox sync` to index your library".to_string(),
                theme,
            )
            .block(col1_block),
            artist_area,
        );
    } else {
        let items: Vec<ListItem> = app
            .artists
            .iter()
            .filter(|a| app.filter_matches(a))
            .map(|a| ListItem::new(a.clone()))
            .collect();
        if items.is_empty() {
            let text = filter_text_on(app, 0).unwrap_or("");
            f.render_widget(
                dim_centered(format!("no matches for '{text}'"), theme).block(col1_block),
                artist_area,
            );
        } else {
            let mut state = ListState::default();
            state.select(Some(app.cursors.artist.min(items.len().saturating_sub(1))));
            f.render_stateful_widget(
                List::new(items)
                    .block(col1_block)
                    .highlight_style(Style::default().fg(theme.accent)),
                artist_area,
                &mut state,
            );
        }
    }

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
    let track_lines = track_rows(
        app,
        &track_ids,
        track_area.width.saturating_sub(2) as usize,
        theme,
    );
    f.render_widget(
        Paragraph::new(track_lines).block(border("Tracks", app.focus_col == 2, theme)),
        track_area,
    );
}

// --- Playlists view ---------------------------------------------------------

fn render_playlists(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let cw = &app.column_widths;
    let cols =
        Layout::horizontal([Constraint::Length(cw.col1), Constraint::Min(cw.col2)]).split(area);

    // col1: playlist names (narrowed by the inline filter when active on col 0).
    // An empty playlist set shows a dim hint to create one; a filter that
    // excludes every playlist shows a "no matches" hint.
    let title = filtered_title("Playlists", app, 0);
    let col1_block = border(&title, app.focus_col == 0, theme);
    if app.playlists.is_empty() {
        f.render_widget(
            dim_centered(
                "no playlists — press `a` on a track to create one".to_string(),
                theme,
            )
            .block(col1_block),
            cols[0],
        );
    } else {
        let items: Vec<ListItem> = app
            .playlists
            .iter()
            .filter(|p| app.filter_matches(&p.name))
            .map(|p| ListItem::new(p.name.clone()))
            .collect();
        if items.is_empty() {
            let text = filter_text_on(app, 0).unwrap_or("");
            f.render_widget(
                dim_centered(format!("no matches for '{text}'"), theme).block(col1_block),
                cols[0],
            );
        } else {
            let mut state = ListState::default();
            state.select(Some(
                app.cursors.playlist.min(items.len().saturating_sub(1)),
            ));
            f.render_stateful_widget(
                List::new(items)
                    .block(col1_block)
                    .highlight_style(Style::default().fg(theme.accent)),
                cols[0],
                &mut state,
            );
        }
    }

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
    let block = border("Queue", app.focus_col == 0, theme);
    if ids.is_empty() {
        f.render_widget(
            dim_centered(
                "queue empty — press `e` on a track to enqueue it".to_string(),
                theme,
            )
            .block(block),
            area,
        );
    } else {
        let lines = track_rows(app, &ids, area.width.saturating_sub(2) as usize, theme);
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

// --- Track rows -------------------------------------------------------------

/// Build the track-column rows: `# Title Album Quality` with the right side
/// (album + quality) right-anchored via [`pad_between`] so wide/CJK titles keep
/// alignment. The now-playing track is prefixed with `▶`; the row under
/// `cursors.track` (selection) is prefixed with `▸` so the selection is
/// visible even under `NO_COLOR` (where the accent highlight is Reset). The
/// selected+now-playing row keeps `▶` (now-playing wins). The row is also
/// rendered with the accent color (selection highlight) when color is on.
fn track_rows(app: &App, ids: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim);
    let accent = Style::default().fg(theme.accent);

    ids.iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let t = app.track_by_id_fast(id)?;
            let np = app.now_playing.as_ref().map(|s| s.id()) == Some(id.as_str());
            let selected = i == app.cursors.track;
            let glyph = if np {
                "▶"
            } else if selected {
                "▸"
            } else {
                " "
            };
            let num = format!("{:>2}", i + 1);
            let album = t.album.as_deref().unwrap_or("");
            let left = format!("{glyph} {num} {} — {album}", t.title);
            let quality = t.quality_label();
            let line = pad_between(&left, &quality, width);
            let style = if selected || np { accent } else { dim };
            Some(Line::from(Span::styled(line, style)))
        })
        .collect()
}

/// YouTube track rows: resolve each video_id via the session's `track_cache`
/// (populated by search/get_playlist/watch_playlist). Falls back to the raw
/// id when a track's metadata isn't cached yet. The quality tag is the stream
/// format label (`Opus 160k · YT`) when known, else `YT`. The now-playing row
/// is prefixed `▶`; the selected row (`cursors.track`) with `▸` so selection
/// is visible under `NO_COLOR`.
fn yt_track_rows(app: &App, ids: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim);
    let accent = Style::default().fg(theme.accent);

    ids.iter()
        .enumerate()
        .map(|(i, id)| {
            let np = app.now_playing.as_ref().map(|s| s.id()) == Some(id.as_str());
            let selected = i == app.cursors.track;
            let glyph = if np {
                "▶"
            } else if selected {
                "▸"
            } else {
                " "
            };
            let num = format!("{:>2}", i + 1);
            let (title, artist, album, quality) =
                match app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
                    Some(rt) => (
                        rt.title.clone(),
                        rt.artist.clone(),
                        rt.album.clone(),
                        rt.fmt
                            .as_ref()
                            .map(|f| f.yt_label())
                            .unwrap_or_else(|| "YT".to_string()),
                    ),
                    None => (id.clone(), String::new(), None, "YT".to_string()),
                };
            let album_s = album.as_deref().unwrap_or("");
            let left = if artist.is_empty() {
                format!("{glyph} {num} {title}")
            } else {
                format!("{glyph} {num} {title} — {artist} {album_s}")
            };
            let line = pad_between(&left, &quality, width);
            let style = if selected || np { accent } else { dim };
            Line::from(Span::styled(line, style))
        })
        .collect()
}
