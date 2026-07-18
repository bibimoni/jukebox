//! YouTube tab system + in-pane panel rendering (I.4 + I.1).
//!
//! Replaces the Miller-column YouTube view with a tabbed, card-based layout.
//! Five tabs at the top (Home / Library / Search / Discover / Radio), each
//! rendering its own panel below the sub-tab bar. The view-switch tab bar
//! (`1`-`4` top-level) lives in `layout::render_tab_bar`; the YT sub-tab bar
//! lives here (`render_yt_tab_bar`) and renders inside the YT content area.
//!
//! Tab switches are pure state mutations (`app.yt_view.tab`); no async. Each
//! tab's content state lives on `YtViewState` so cursors persist across tab
//! changes. Home/Discover/Radio reuse the existing `Overlay::{Home,Discover,
//! Radio}` key dispatch — the overlay is still set when its matching tab is
//! active, but `overlay::render` skips the popup paint so the in-pane renderer
//! here takes over (no visible popup). Search reuses `Overlay::Search` the
//! same way. Library is new: yt_lists grouped by `YtListKind` with track
//! counts.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::app::{App, Overlay, YtListKind, YtTab};
use crate::tui::view::columns::{truncate_ellipsis, yt_header_tag, yt_track_rows};
use crate::tui::view::home;
use crate::tui::view::icons::IconRenderer;
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, h_line, is_ascii, marker_glyph, no_color, sep_dot, v_sep, Theme,
    ASCII_BORDER_SET,
};

/// Render the YouTube tab system: a 1-row sub-tab bar at the top + the
/// active tab's content below. Called by `columns::render` when
/// `view == View::Youtube`.
pub fn render_yt_view(f: &mut Frame, area: Rect, app: &mut App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Split: sub-tab bar (1 row) + content (rest). The tab bar mirrors the
    // style of `layout::render_tab_bar` (accent + BOLD + REVERSED on active,
    // dim on inactive, │ separators).
    let split = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    render_yt_tab_bar(f, split[0], app);
    match app.yt_view.tab {
        YtTab::Home => render_yt_home(f, split[1], app),
        YtTab::Library => render_yt_library(f, split[1], app),
        YtTab::Search => render_yt_search(f, split[1], app),
        YtTab::Discover => render_yt_discover(f, split[1], app),
        YtTab::Radio => render_yt_radio(f, split[1], app),
        // Task 4: stub arms so the 7-tab match is exhaustive. Task 5 replaces
        // these with real `render_yt_explore` / `render_yt_charts` that render
        // from `app.yt_view.explore_cached` / `charts_cached`. The placeholder
        // shows a "loading…" state so the user can cycle to the tab without a
        // crash while the fetch is in flight (or after it lands — Task 5 will
        // surface the cached data).
        YtTab::Explore => render_yt_tab_placeholder(f, split[1], "Explore"),
        YtTab::Charts => render_yt_tab_placeholder(f, split[1], "Charts"),
    }
}

/// Task 4: minimal placeholder for the new Explore/Charts tabs so the
/// 7-tab `render_yt_view` match is exhaustive and the user can cycle to
/// the tabs without a crash. Task 5 replaces this with real rendering that
/// reads from `app.yt_view.explore_cached` / `charts_cached`. The
/// placeholder mirrors the `border` + empty-paragraph pattern used by
/// `render_yt_radio`'s "No active radio session." state.
fn render_yt_tab_placeholder(f: &mut Frame, area: Rect, title: &str) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });
    let block = border(title, true, &theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let msg = if title == "Explore" {
        "loading explore playlists…"
    } else {
        "loading charts…"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg.to_string(), dim))),
        inner,
    );
}

/// A titled border whose color reflects focus: accent when `focused`, dim
/// otherwise. Mirrors `columns::border` for the in-pane Library split. In
/// ASCII font mode the border uses `+`, `-`, `|`.
fn border<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let color = if focused { theme.accent } else { theme.dim };
    if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(color))
            .title(Span::styled(title, Style::default().fg(color)))
    } else {
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
}

/// Render the 5-tab sub-tab bar: `1:Home | 2:Library | 3:Search | 4:Discover
/// | 5:Radio`. Active = accent + BOLD + REVERSED (mirrors
/// `layout::render_tab_bar`). The `1`-`5` prefixes match the YT tab-switch
/// keys. Inactive tabs are dim. `│` separators (or `|` in ASCII). The
/// breadcrumb on the right shows the YT provider state tag (`YouTube [ok]`,
/// `YouTube [err]`, ...).
pub fn render_yt_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = Theme::default();
    let nc = no_color();
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });
    let active = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::REVERSED);
    let text = Style::default().fg(if nc { Color::Reset } else { theme.text });

    let tabs = YtTab::all();
    let sep: &'static str = if v_sep() == "|" { " | " } else { " │ " };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut tabs_w: usize = 0;
    for (i, (label, tab)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(sep, dim));
            tabs_w += disp_width(sep);
        }
        let style = if app.yt_view.tab == *tab { active } else { dim };
        spans.push(Span::styled((*label).to_string(), style));
        tabs_w += disp_width(label);
    }

    // Breadcrumb (right-aligned): `YouTube [ok]` / `YouTube [err]` / ...
    // Derived from the truthful `yt_state` machine (Issue 1).
    let tag = yt_header_tag(app.yt_state);
    let bc = format!("YouTube {tag}");
    let bc_w = disp_width(&bc);
    let gap = 2usize;
    if tabs_w + gap + bc_w <= area.width as usize {
        let spaces = area.width as usize - tabs_w - bc_w;
        spans.push(Span::raw(" ".repeat(spaces)));
        spans.push(Span::styled(bc, text));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans).alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// YtListKind → (glyph, label) for the Library tab section headers. Glyphs
/// are ASCII-safe (`>`, `*`, `+`) in ASCII font mode, Unicode (`♫`, `✦`,
/// `◆`) otherwise. Labels stay text so the section is identifiable under
/// `NO_COLOR` + ASCII.
fn kind_glyph_label(kind: YtListKind) -> (&'static str, &'static str) {
    match kind {
        YtListKind::Account => {
            if is_ascii() {
                (">", "Account")
            } else {
                ("♫", "Account")
            }
        }
        YtListKind::Suggested => {
            if is_ascii() {
                ("*", "Suggested")
            } else {
                ("✦", "Suggested")
            }
        }
        YtListKind::Generated => {
            if is_ascii() {
                ("+", "Generated")
            } else {
                ("◆", "Generated")
            }
        }
    }
}

/// Library tab (I.1): yt_lists with section headers per `YtListKind`
/// (Account / Suggested / Generated) + track counts. Lists render in their
/// original `yt_lists` order (so the flat index == `cursors.playlist` and
/// j/k via the existing `move_down`/`move_up` dispatch works without
/// remapping); a section header is emitted whenever the kind changes between
/// consecutive lists, so the user sees the `♫ Account — N lists` grouping
/// without reordering their lists. Wider column (32 at ≥100 cols, 24 at
/// <100). When a list is focused (Enter / `l`), splits: left = grouped
/// list, right = track rows.
pub fn render_yt_library(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = Theme::default();
    let nc = no_color();
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });
    let header_style = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD);

    // Pre-compute per-kind counts so section headers can show
    // `♫ Account — 12 lists` even when the lists of that kind are
    // interleaved with other kinds in `yt_lists`.
    let mut counts: std::collections::HashMap<YtListKind, usize> = std::collections::HashMap::new();
    for l in &app.yt_lists {
        *counts.entry(l.kind).or_default() += 1;
    }
    let mut emitted: std::collections::HashSet<YtListKind> = std::collections::HashSet::new();

    // I.7: variable width from `app.playlist_col.width`. Clamp to the area
    // so a too-wide width doesn't push the tracks pane off-screen.
    let max_w = area.width.saturating_sub(1);
    let list_w: u16 = app.playlist_col.width.min(max_w);
    let show_counts = app.playlist_col.show_counts;
    let group_by_type = app.playlist_col.group_by_type;

    // Split into list + tracks when a list is focused (focus_col >= 1).
    let split = if app.focus_col >= 1 && !app.yt_lists.is_empty() {
        Layout::horizontal([Constraint::Length(list_w), Constraint::Min(1)]).split(area)
    } else {
        Layout::horizontal([Constraint::Min(1)]).split(area)
    };
    let list_area = split[0];
    let tracks_area = split.get(1).copied();

    // Build the list as a Paragraph with per-line styles. Each list row
    // carries the section glyph + name on the left and the track count on
    // the right (right-aligned via pad_between, when show_counts). The
    // selected row uses `theme.selected_style()` + `▸` marker (3 non-color
    // cues under NO_COLOR: REVERSED + BOLD + glyph).
    let col_w = list_area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut prev_kind: Option<YtListKind> = None;
    for (idx, l) in app.yt_lists.iter().enumerate() {
        // Emit a section header when the kind changes (or at the start) and
        // group_by_type is on.
        if group_by_type && prev_kind != Some(l.kind) {
            let (glyph, label) = kind_glyph_label(l.kind);
            let n = counts.get(&l.kind).copied().unwrap_or(0);
            let header = format!("{glyph} {label} {} {} lists", em_dash(), n);
            lines.push(Line::from(Span::styled(header, header_style)));
            // Section divider: a dim horizontal rule under the header.
            let rule_w = col_w.min(60);
            let rule = h_line().repeat(rule_w);
            lines.push(Line::from(Span::styled(rule, dim)));
            emitted.insert(l.kind);
        }
        prev_kind = Some(l.kind);

        let is_selected = idx == app.cursors.playlist;
        // Track count: `42 tracks` when loaded, `…` while unfetched.
        // `loaded_yt_lists` tracks fetch state so a genuinely-empty list
        // (fetched but 0 tracks) shows `0 tracks`, not `…`. Suppressed
        // entirely when `show_counts` is off.
        let count_label = if show_counts {
            if app.loaded_yt_lists.contains(&l.id) {
                format!("{} tracks", l.track_ids.len())
            } else if is_ascii() {
                "...".to_string()
            } else {
                "…".to_string()
            }
        } else {
            String::new()
        };
        let (glyph, _label) = kind_glyph_label(l.kind);
        // Reserve 2 cols for the marker prefix (`▸ ` / `  `).
        let count_w = disp_width(&count_label);
        let name_w = if show_counts {
            col_w.saturating_sub(2).saturating_sub(count_w + 1)
        } else {
            col_w.saturating_sub(2)
        };
        let name = truncate_ellipsis(&l.name, name_w);
        let row_text = if is_selected {
            format!("{} {glyph} {name}", marker_glyph())
        } else {
            format!("  {glyph} {name}")
        };
        let row_w = col_w;
        let style = if is_selected {
            theme.selected_style()
        } else {
            Style::default().fg(theme.text)
        };
        let count_style = if is_selected {
            theme.selected_style()
        } else {
            dim
        };
        if show_counts {
            let row_w_disp = disp_width(&row_text);
            let pad = row_w.saturating_sub(row_w_disp + count_w);
            let spans: Vec<Span<'static>> = vec![
                Span::styled(row_text, style),
                Span::styled(" ".repeat(pad), Style::default()),
                Span::styled(count_label, count_style),
            ];
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(Span::styled(row_text, style)));
        }
    }
    // Empty state.
    if app.yt_lists.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "no lists {} run :yt auth browser <name> to connect",
                em_dash()
            ),
            dim,
        )));
    }

    // Scroll-to-cursor: keep the selected row visible when the list is
    // longer than the pane (Paragraph doesn't auto-scroll like List).
    let visible_h = list_area.height.saturating_sub(2) as usize;
    let cursor = app.cursors.playlist;
    let scroll = if cursor >= visible_h {
        (cursor - visible_h + 1) as u16
    } else {
        0
    };

    let block = border("Library", app.focus_col == 0, &theme);
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(block),
        list_area,
    );

    // Right pane: tracks of the focused list (when focus_col >= 1).
    if let Some(tracks_area) = tracks_area {
        let ids = app
            .yt_lists
            .get(app.cursors.playlist)
            .map(|l| l.track_ids.clone())
            .unwrap_or_default();
        let name = app
            .yt_lists
            .get(app.cursors.playlist)
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let title = format!("Tracks {} {}", em_dash(), name);
        if ids.is_empty() {
            let body =
                crate::tui::view::columns::yt_status_line_pub(app, app.yt_lists.is_empty(), true);
            let lines = if app.yt_state.is_error() {
                crate::tui::view::columns::yt_error_lines_pub(app, &theme)
            } else {
                vec![Line::from(Span::styled(body, dim))]
            };
            f.render_widget(
                Paragraph::new(lines).block(border(&title, app.focus_col == 1, &theme)),
                tracks_area,
            );
        } else {
            let lines = yt_track_rows(
                app,
                &ids,
                tracks_area.width.saturating_sub(2) as usize,
                &theme,
            );
            let visible_h = tracks_area.height.saturating_sub(2) as usize;
            let cursor = app.cursors.track;
            let scroll = if cursor >= visible_h {
                (cursor - visible_h + 1) as u16
            } else {
                0
            };
            f.render_widget(
                Paragraph::new(lines).scroll((scroll, 0)).block(border(
                    &title,
                    app.focus_col == 1,
                    &theme,
                )),
                tracks_area,
            );
        }
    }
}

/// Home tab (I.1): reuses `home::render_compact` in-pane (not popup). Pulls
/// the `HomeState` from `yt_view.home` (populated by `open_home`). If the
/// overlay is also set (Overlay::Home), `overlay::render` skips the popup
/// paint so this in-pane render is the visible one.
///
/// RC19-D2: when the user enters the YT Home tab via tab-switching (not the
/// `H` overlay key), `yt_view.home` starts empty (default `HomeState`) and
/// the tab showed the welcome screen even with a populated catalog. Now
/// `populate_home_state` is called on first entry when sections are empty
/// (and not loading), so the tab shows Quick Picks / Made for You / etc.
/// instead of "Welcome! Your Home will grow as you listen." The populate is
/// idempotent — after the first call `sections` is non-empty, so subsequent
/// frames skip it. No overlay is set (the tab is the visible surface).
pub fn render_yt_home(f: &mut Frame, area: Rect, app: &mut App) {
    use crate::yt::state::YtState;
    let icons = IconRenderer::auto();
    // RB-2: a signed-out account shows a sign-in prompt, not the cold-start
    // growth messaging ("listen more to build your profile").
    if matches!(app.yt_state, YtState::Unconfigured | YtState::SignedOut) {
        let p = home::render_signed_out(&icons);
        f.render_widget(p, area);
        return;
    }
    // RC19-D2: populate sections on first entry so the Home tab shows real
    // content instead of the welcome screen. `populate_home_state` doesn't
    // set the overlay (so no popup paints over the tab). After the first
    // call `sections` is non-empty, so subsequent renders skip this branch.
    if !app.yt_view.home.loading && app.yt_view.home.sections.is_empty() {
        let state = app.populate_home_state();
        app.yt_view.home = state;
    }
    let state = &app.yt_view.home;
    if state.loading {
        let lines = home::render_header(area, state, &icons);
        f.render_widget(Paragraph::new(lines), area);
    } else if state.sections.is_empty() {
        let p = home::render_empty(&icons);
        f.render_widget(p, area);
    } else {
        // render_compact renders a Clear + body + hint bar. In-pane the Clear
        // only erases the pane area (not the full screen), which is what we
        // want. Clone sections to satisfy the borrow (render_compact takes
        // &[...], state by &).
        let sections = state.sections.clone();
        home::render_compact(f, area, &sections, state, &icons);
    }
}

/// Search tab (I.1): in-pane search — input line at top, results list
/// below. Reuses the `Overlay::Search` data when present (the overlay is set
/// by `/` and key dispatch routes typing there). `overlay::render` skips the
/// popup paint when this tab is active, so the in-pane render is the visible
/// one. If no overlay is active, show a "press / to search" placeholder so
/// the tab isn't a dead end.
pub fn render_yt_search(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });

    let (input, results, cursor, scope, submitted, searching) = match &app.overlay {
        Some(Overlay::Search {
            input,
            results,
            cursor,
            scope,
            submitted,
            searching,
        }) => (
            input.clone(),
            results.clone(),
            *cursor,
            *scope,
            submitted.clone(),
            *searching,
        ),
        _ => {
            // No active search overlay — placeholder hint.
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("Search YouTube {} press / to start a search", em_dash()),
                    dim,
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Tab scope (local/youtube)   ·   Enter to submit   ·   Esc close",
                    dim,
                )),
            ];
            f.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Center)
                    .block(border("Search", true, &theme)),
                area,
            );
            return;
        }
    };

    // Split: input line (1) + status/hint (1) + results (rest).
    let inner = border("Search", true, &theme).inner(area);
    f.render_widget(border("Search", true, &theme), area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    // Input line: `/ query` with a block cursor.
    let input_line = Line::from(vec![
        Span::styled("/ ", Style::default().fg(theme.accent)),
        Span::styled(input.clone(), Style::default().fg(theme.text)),
        Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    f.render_widget(input_line, rows[0]);

    // Status/hint line.
    let status: Line = {
        use crate::yt::state::YtState;
        // RB-2: distinguish "search succeeded, zero matches" from "search
        // failed/offline." When the provider is down, show the truthful provider
        // state, not "No results" as if the search ran and was empty.
        let provider_down = app.yt_state.is_error()
            || matches!(
                app.yt_state,
                YtState::Unconfigured | YtState::SignedOut | YtState::Failed
            );
        let provider_msg = match app.yt_state {
            YtState::Unconfigured | YtState::SignedOut => {
                "YouTube not connected — :yt auth browser <name>".to_string()
            }
            YtState::AuthExpired => "authorization expired — :yt auth browser <name>".to_string(),
            YtState::RateLimited => "rate limited — wait, then press R".to_string(),
            YtState::Failed => "YouTube failed — run :yt setup".to_string(),
            _ => "YouTube offline — press R to retry".to_string(),
        };
        if searching && provider_down {
            Line::from(Span::styled(
                format!("{provider_msg} {} Tab {} local", sep_dot(), sep_dot()),
                dim,
            ))
        } else if searching {
            Line::from(Span::styled(
                format!(
                    "searching{} Tab {} local {} Esc cancel",
                    ellipsis(),
                    sep_dot(),
                    sep_dot()
                ),
                dim,
            ))
        } else if input.trim().is_empty() {
            Line::from(Span::styled(
                format!(
                    "type a query, then Enter to search {} Tab {} local",
                    sep_dot(),
                    sep_dot()
                ),
                dim,
            ))
        } else if results.is_empty() && submitted.as_deref() == Some(input.as_str()) {
            if provider_down {
                Line::from(Span::styled(
                    format!("{provider_msg} {} Tab {} local", sep_dot(), sep_dot()),
                    dim,
                ))
            } else {
                Line::from(Span::styled(
                    format!(
                        "No results for '{input}' {} Tab {} local",
                        sep_dot(),
                        sep_dot()
                    ),
                    dim,
                ))
            }
        } else if !results.is_empty() {
            Line::from(Span::styled(
                format!(
                    "{} result{} {} Enter plays {} Tab {} local",
                    results.len(),
                    if results.len() == 1 { "" } else { "s" },
                    sep_dot(),
                    sep_dot(),
                    sep_dot()
                ),
                dim,
            ))
        } else {
            Line::from(Span::styled(
                format!(
                    "Enter to search YouTube {} Tab {} local",
                    sep_dot(),
                    sep_dot()
                ),
                dim,
            ))
        }
    };
    let _ = scope;
    f.render_widget(status, rows[1]);

    // Results list.
    let items: Vec<ListItem> = results
        .iter()
        .map(|id| {
            let label = yt_search_label(app, id);
            let truncated = truncate_ellipsis(&label, rows[2].width.saturating_sub(2) as usize);
            ListItem::new(truncated)
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(cursor.min(results.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.selected_style())
            .block(Block::default().borders(Borders::NONE)),
        rows[2],
        &mut state,
    );
}

/// Resolve a search-result id to a display label. Catalog tracks use the
/// local title; YouTube ids resolve via the session's `track_cache`. Falls
/// back to the raw id when no metadata is cached yet.
fn yt_search_label(app: &App, id: &str) -> String {
    if let Some(t) = app.track_by_id_fast(id) {
        let dash = em_dash();
        return format!("{} {dash} {}", t.title, t.primary_artist);
    }
    if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
        if rt.artist.is_empty() {
            return rt.title.clone();
        }
        let dash = em_dash();
        return format!("{} {dash} {}", rt.title, rt.artist);
    }
    id.to_string()
}

/// Discover tab (I.1): reuses `Overlay::Discover` items in-pane. The overlay
/// is set by `S` (in Youtube/Mixed mode); `overlay::render` skips the popup
/// paint when this tab is active. If no overlay is active, show a "press S
/// to discover" placeholder.
pub fn render_yt_discover(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });
    let dash = em_dash();

    let (items, cursor, loading, loading_ticks, play_loading) = match &app.overlay {
        Some(Overlay::Discover { items, cursor }) => (
            items.clone(),
            *cursor,
            app.discover_loading,
            app.discover_loading_ticks,
            app.discover_play_loading.clone(),
        ),
        _ => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("Discover mixes {} press S to load suggestions", dash),
                    dim,
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "j/k navigate   ·   Enter play   ·   x dismiss   ·   Esc close",
                    dim,
                )),
            ];
            f.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Center)
                    .block(border("Discover", true, &theme)),
                area,
            );
            return;
        }
    };

    let block = border("Discover", true, &theme);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (glyph, text, explanation) = match d {
                crate::tui::app::DiscoverItem::Album { artist, album } => {
                    let g = if is_ascii() { "#" } else { "♫" };
                    (g, format!("{artist} {dash} {album}"), None)
                }
                crate::tui::app::DiscoverItem::Playlist { name, .. } => {
                    let g = if is_ascii() { "*" } else { "✦" };
                    (g, name.clone(), None)
                }
                crate::tui::app::DiscoverItem::Mix {
                    name, explanation, ..
                } => {
                    let g = if is_ascii() { "+" } else { "◆" };
                    (g, name.clone(), explanation.clone())
                }
            };
            let style = if i == cursor {
                theme.selected_style()
            } else {
                Style::default().fg(theme.text)
            };
            let mut spans = vec![Span::styled(format!("{glyph} {text}"), style)];
            if let Some(expl) = explanation {
                let corner = if is_ascii() { "\\" } else { "└" };
                spans.push(Span::raw(format!("\n  {corner} ")));
                spans.push(Span::styled(expl, dim));
            }
            Line::from(spans)
        })
        .collect();

    // Loading indicator.
    if loading {
        let frames: &[&str] = if is_ascii() {
            &["|", "/", "-", "\\"]
        } else {
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        };
        let frame = frames[(loading_ticks as usize) % frames.len()];
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("{frame} Loading YouTube suggestions..."),
            Style::default().fg(theme.hi_fg),
        )));
    }

    // Play-loading or hint line.
    let dot = sep_dot();
    if let Some(name) = play_loading {
        let frames: &[&str] = if is_ascii() {
            &["|", "/", "-", "\\"]
        } else {
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        };
        let frame = frames[(loading_ticks as usize) % frames.len()];
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("{frame} Loading \"{name}\"{dash}"),
            Style::default().fg(theme.hi_fg),
        )));
        lines.push(Line::from(Span::styled(
            format!("Esc cancel {dot} wait for playback to start"),
            dim,
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("j/k navigate {dot} Enter play {dot} x dismiss {dot} Esc close"),
            dim,
        )));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Radio tab (I.1): reuses `Overlay::Radio` content in-pane. The overlay is
/// set by `:radio`; `overlay::render` skips the popup paint when this tab is
/// active. If no overlay is active, show a "no active session" placeholder.
pub fn render_yt_radio(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = Theme::default();
    let dim = Style::default().fg(if no_color() { Color::Reset } else { theme.dim });

    let block = border("Radio", true, &theme);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let session = match &app.overlay {
        Some(Overlay::Radio { session }) => session.clone(),
        _ => None,
    };

    let para = if let Some(s) = session.as_ref() {
        // Resolve seed + upcoming + played to display titles (mirrors
        // `render_radio_overlay`).
        let seed_title = match &s.seed {
            crate::reco::radio::RadioSeed::Track(id) => yt_search_label(app, id),
            other => other.description(),
        };
        let upcoming: Vec<String> = s
            .upcoming(8)
            .into_iter()
            .map(|c| yt_search_label(app, &c.track_id))
            .collect();
        let played: Vec<String> = s
            .history()
            .iter()
            .map(|id| yt_search_label(app, id))
            .collect();
        let icons = IconRenderer::auto();
        crate::tui::view::radio::render(area, s, &icons, &seed_title, &upcoming, &played)
    } else {
        Paragraph::new(Line::from(Span::styled(
            "No active radio session.".to_string(),
            dim,
        )))
    };
    f.render_widget(para, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::{App, View, YtList, YtListKind, YtTab, YtViewState};
    use ratatui::{backend::TestBackend, Terminal};

    /// Minimal one-track catalog so `App::new` succeeds.
    fn one_track_app() -> App {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
               "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        // Leak the tempdir so the catalog file stays alive for the app's
        // lifetime (App borrows the source_root path).
        let cat = Catalog::load(&p).unwrap();
        std::mem::forget(d);
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    /// Render the YT view into a TestBackend buffer and return the joined
    /// cell text (rows separated by `\n`).
    fn yt_view_text(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_yt_view(f, area, app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            out.push('\n');
        }
        out
    }

    /// I.4: YT view shows 7-tab bar with all seven labels at 100×30. Task 4
    /// added `Explore` and `Charts` variants and removed the `1:`-`5:`
    /// number prefixes (BB-027).
    #[test]
    fn yt_view_shows_seven_tab_bar() {
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Home;
        let text = yt_view_text(&mut app, 100, 30);
        for label in [
            "Home",
            "Library",
            "Search",
            "Discover",
            "Radio",
            "Explore",
            "Charts",
        ] {
            assert!(
                text.contains(label),
                "I.4: tab bar must show {label}: {text:?}"
            );
        }
    }

    /// I.4: the active tab is distinguishable (REVERSED + BOLD modifier on
    /// its label cells). Task 4 removed the `1:`-`5:` number prefixes, so the
    /// probe strings look for "Library" (active) and "Home" (inactive).
    #[test]
    fn yt_view_active_tab_has_selection_style() {
        use ratatui::style::Modifier;
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Library;
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_yt_view(f, f.area(), &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Find the row containing the tab labels, then check the "Library"
        // cells carry REVERSED + BOLD (active). Other tabs should not. We
        // probe by scanning for the "L" of "Library" and the "H" of "Home",
        // then reading a window to confirm the full label is present (the
        // bare letters alone could collide with other words; the window
        // check disambiguates).
        let mut found_active_reversed = false;
        let mut found_inactive_not_reversed = false;
        for y in 0..3u16 {
            for x in 0..100u16 {
                let cell = &buf[(x, y)];
                let sym = cell.symbol();
                if sym.contains('L') {
                    let mut probe = String::new();
                    for dx in 0..10u16 {
                        if let Some(c) = buf.cell((x + dx, y)) {
                            probe.push(c.symbol().chars().next().unwrap_or(' '));
                        }
                    }
                    if probe.contains("Library")
                        && cell.modifier.contains(Modifier::REVERSED)
                        && cell.modifier.contains(Modifier::BOLD)
                    {
                        found_active_reversed = true;
                    }
                }
                if sym.contains('H') {
                    let mut probe = String::new();
                    for dx in 0..6u16 {
                        if let Some(c) = buf.cell((x + dx, y)) {
                            probe.push(c.symbol().chars().next().unwrap_or(' '));
                        }
                    }
                    if probe.contains("Home") && !cell.modifier.contains(Modifier::REVERSED) {
                        found_inactive_not_reversed = true;
                    }
                }
            }
        }
        assert!(
            found_active_reversed,
            "I.4: active tab 'Library' must have REVERSED+BOLD"
        );
        assert!(
            found_inactive_not_reversed,
            "I.4: inactive tab 'Home' must NOT have REVERSED"
        );
    }

    /// I.4: switching tabs changes the rendered content. Home tab shows the
    /// welcome/section content; Library tab shows "Library" header; Search
    /// tab shows the search placeholder; Discover tab shows the discover
    /// placeholder; Radio tab shows "no active radio session".
    #[test]
    fn yt_view_tab_switch_changes_content() {
        let mut app = one_track_app();
        app.view = View::Youtube;

        // Home tab (no open_home → empty sections → welcome).
        app.yt_view.tab = YtTab::Home;
        let home_text = yt_view_text(&mut app, 100, 30);

        // Library tab.
        app.yt_view.tab = YtTab::Library;
        let lib_text = yt_view_text(&mut app, 100, 30);
        assert!(
            lib_text.contains("Library"),
            "I.1: Library tab must show 'Library' header: {lib_text:?}"
        );

        // Search tab (no overlay → placeholder).
        app.yt_view.tab = YtTab::Search;
        let search_text = yt_view_text(&mut app, 100, 30);
        assert!(
            search_text.contains("press / to start"),
            "I.1: Search tab placeholder must mention '/': {search_text:?}"
        );

        // Discover tab (no overlay → placeholder).
        app.yt_view.tab = YtTab::Discover;
        let disc_text = yt_view_text(&mut app, 100, 30);
        assert!(
            disc_text.contains("press S to load"),
            "I.1: Discover tab placeholder must mention 'S': {disc_text:?}"
        );

        // Radio tab (no overlay → placeholder).
        app.yt_view.tab = YtTab::Radio;
        let radio_text = yt_view_text(&mut app, 100, 30);
        assert!(
            radio_text.contains("No active radio session"),
            "I.1: Radio tab placeholder must mention 'no active session': {radio_text:?}"
        );

        // Home text should differ from Library text (content changes).
        assert_ne!(
            home_text, lib_text,
            "I.4: switching tabs must change content"
        );
    }

    /// I.1: Library tab groups yt_lists by YtListKind with section headers.
    #[test]
    fn yt_library_groups_by_kind() {
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Library;
        app.yt_lists = vec![
            YtList {
                id: "PL1".into(),
                name: "Liked Music".into(),
                kind: YtListKind::Account,
                track_ids: vec!["v1".into(), "v2".into(), "v3".into()],
            },
            YtList {
                id: "PL2".into(),
                name: "JPop".into(),
                kind: YtListKind::Account,
                track_ids: vec![],
            },
            YtList {
                id: "RD1".into(),
                name: "Japan Ballads".into(),
                kind: YtListKind::Suggested,
                track_ids: vec!["v4".into()],
            },
            YtList {
                id: "DM1".into(),
                name: "Daily Mix 1".into(),
                kind: YtListKind::Generated,
                track_ids: vec!["v5".into(), "v6".into()],
            },
        ];
        // Mark all as loaded so track counts show numbers (not …).
        for l in &app.yt_lists {
            app.loaded_yt_lists.insert(l.id.clone());
        }
        let text = yt_view_text(&mut app, 100, 30);
        assert!(
            text.contains("Account"),
            "I.1: Library must show Account section: {text:?}"
        );
        assert!(
            text.contains("Suggested"),
            "I.1: Library must show Suggested section: {text:?}"
        );
        assert!(
            text.contains("Generated"),
            "I.1: Library must show Generated section: {text:?}"
        );
        assert!(
            text.contains("3 tracks"),
            "I.1: Library must show track count '3 tracks': {text:?}"
        );
        assert!(
            text.contains("0 tracks"),
            "I.1: Library must show '0 tracks' for empty loaded list: {text:?}"
        );
    }

    /// I.1: Library tab shows `…` (or `...` in ASCII) for unfetched lists.
    #[test]
    fn yt_library_shows_ellipsis_for_unfetched() {
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Library;
        app.yt_lists = vec![YtList {
            id: "PL1".into(),
            name: "Liked Music".into(),
            kind: YtListKind::Account,
            track_ids: vec![],
        }];
        // NOT loaded → should show ellipsis.
        let text = yt_view_text(&mut app, 100, 30);
        assert!(
            text.contains('…') || text.contains("..."),
            "I.1: Library must show ellipsis for unfetched list: {text:?}"
        );
    }

    /// I.4: YtTab::next/prev cycle through all seven tabs (Task 4 added
    /// `Explore` and `Charts`).
    #[test]
    fn yt_tab_next_prev_cycle() {
        assert_eq!(YtTab::Home.next(), YtTab::Library);
        assert_eq!(YtTab::Library.next(), YtTab::Search);
        assert_eq!(YtTab::Search.next(), YtTab::Discover);
        assert_eq!(YtTab::Discover.next(), YtTab::Radio);
        assert_eq!(YtTab::Radio.next(), YtTab::Explore);
        assert_eq!(YtTab::Explore.next(), YtTab::Charts);
        assert_eq!(YtTab::Charts.next(), YtTab::Home);
        assert_eq!(YtTab::Home.prev(), YtTab::Charts);
        assert_eq!(YtTab::Charts.prev(), YtTab::Explore);
        assert_eq!(YtTab::Explore.prev(), YtTab::Radio);
        assert_eq!(YtTab::Radio.prev(), YtTab::Discover);
    }

    /// I.4: YtTab::all() returns 7 tabs with no number prefixes (Task 4 /
    /// BB-027).
    #[test]
    fn yt_tab_all_has_seven_no_number_prefix() {
        let tabs = YtTab::all();
        assert_eq!(tabs.len(), 7);
        assert_eq!(tabs[0].0, "Home");
        assert_eq!(tabs[1].0, "Library");
        assert_eq!(tabs[2].0, "Search");
        assert_eq!(tabs[3].0, "Discover");
        assert_eq!(tabs[4].0, "Radio");
        assert_eq!(tabs[5].0, "Explore");
        assert_eq!(tabs[6].0, "Charts");
    }

    /// I.4: YtViewState::default() has Home tab + zeroed cursors.
    #[test]
    fn yt_view_state_default() {
        let s = YtViewState::default();
        assert_eq!(s.tab, YtTab::Home);
        assert_eq!(s.library_cursor, 0);
        assert_eq!(s.library_section, 0);
    }

    /// I.4: render_yt_view with a zero-height area is a no-op (doesn't panic).
    #[test]
    fn yt_view_zero_area_no_panic() {
        let mut app = one_track_app();
        app.view = View::Youtube;
        let backend = TestBackend::new(100, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 100, 0);
                render_yt_view(f, area, &mut app);
            })
            .unwrap();
    }

    /// I.4: `open_home` switches to YT view + Home tab and sets yt_view.home.
    #[test]
    fn open_home_switches_to_yt_view_home_tab() {
        let mut app = one_track_app();
        app.open_home();
        assert_eq!(app.view, View::Youtube, "open_home must switch to YT view");
        assert_eq!(app.yt_view.tab, YtTab::Home, "open_home must set Home tab");
        assert!(
            !app.yt_view.home.sections.is_empty(),
            "open_home must populate yt_view.home sections"
        );
    }

    /// RC19-D2: the YT Home tab must show real content (Quick Picks, Made
    /// for You, etc.) when the user enters it via tab-switching, not just
    /// the "Welcome! Your Home will grow as you listen." screen. The fix
    /// calls `populate_home_state` on first entry when sections are empty.
    /// The `H` overlay path already populated sections via `open_home`; the
    /// tab-switch path (1-5 / Tab) didn't, so the welcome screen showed
    /// even with a populated catalog + connected YouTube sidecar.
    #[test]
    fn rc19_d2_yt_home_tab_shows_sections_not_welcome() {
        use crate::yt::state::YtState;
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Home;
        app.yt_state = YtState::Ready;
        // Default HomeState: not loading, empty sections → welcome screen
        // would render before the fix.
        assert!(app.yt_view.home.sections.is_empty());
        let text = yt_view_text(&mut app, 100, 30);
        // After the fix, populate_home_state is called on first render, so
        // sections are populated and the rendered text shows section content.
        assert!(
            !app.yt_view.home.sections.is_empty(),
            "RC19-D2: render_yt_home must populate sections on first entry: {text:?}"
        );
        // The welcome screen must NOT show (catalog has a track, so Quick
        // Picks has content).
        assert!(
            !text.contains("Welcome! Your Home will grow"),
            "RC19-D2: YT Home tab must not show welcome screen when catalog has tracks: {text:?}"
        );
        // Section content must be visible. Quick Picks is the first section
        // and shows the catalog track title ("Freedom" in `one_track_app`).
        assert!(
            text.contains("Quick Picks") || text.contains("Quick"),
            "RC19-D2: YT Home tab must show Quick Picks section: {text:?}"
        );
        // Made for You is the second section.
        assert!(
            text.contains("Made for You") || text.contains("Made"),
            "RC19-D2: YT Home tab must show Made for You section: {text:?}"
        );
    }

    /// RC19-D2: the populate-on-entry must be idempotent — calling
    /// `render_yt_home` twice doesn't re-populate (sections already set).
    /// Also verifies the overlay is NOT set by the tab path (only `open_home`
    /// sets the overlay for the `H` key path).
    #[test]
    fn rc19_d2_yt_home_tab_populate_is_idempotent_no_overlay() {
        use crate::yt::state::YtState;
        let mut app = one_track_app();
        app.view = View::Youtube;
        app.yt_view.tab = YtTab::Home;
        app.yt_state = YtState::Ready;
        assert!(app.overlay.is_none(), "no overlay before first render");
        // First render populates sections.
        let _ = yt_view_text(&mut app, 100, 30);
        let sections_after_first = app.yt_view.home.sections.len();
        assert!(
            sections_after_first > 0,
            "RC19-D2: sections must be populated after first render"
        );
        assert!(
            app.overlay.is_none(),
            "RC19-D2: tab path must NOT set the overlay (no popup over tab): {:?}",
            app.overlay
        );
        // Second render: sections already populated, populate_home_state
        // must not be called again (idempotent). We can verify by checking
        // the sections count stays the same.
        let _ = yt_view_text(&mut app, 100, 30);
        assert_eq!(
            app.yt_view.home.sections.len(),
            sections_after_first,
            "RC19-D2: second render must not re-populate sections (idempotent)"
        );
        assert!(
            app.overlay.is_none(),
            "RC19-D2: second render must still NOT set the overlay: {:?}",
            app.overlay
        );
    }
}
