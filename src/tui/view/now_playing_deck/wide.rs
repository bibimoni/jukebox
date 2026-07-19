//! Wide layout for the Now Playing Deck (≥ 120 cols, ≥ 30 rows).
//!
//! Two columns: metadata (left, flex) + Up Next (right, 30 cols). The
//! metadata column stacks the artist / title / album / quality / source
//! status / progress / controls+volume / modes rows. The Up Next column
//! shows 3-5 queue entries.
//!
//! ## Layout
//!
//! ```text
//! ╭─ ▶ NOW PLAYING ──────────────────────────────┬─ UP NEXT ───────╮
//! │  Track title                                  │  Queue is empty │
//! │  Artist                                       │                │
//! │  Album                                        │  [Q] Open queue │
//! │  ▶ PLAYING  [Space] Pause                     │                │
//! │  0:00  ━━━━━━━━━━━━●──────────  --:--         │                │
//! │  [←] Previous  [Space] Pause  [→] Next  Vol… │                │
//! │  Shuffle: Random  Repeat: One  Continue: Off │                │
//! │  YOUTUBE · ● Connected                        │                │
//! ╰───────────────────────────────────────────────┴────────────────╯
//! ```
//!
//! ## Transparency
//!
//! No `bg` is set on any cell. The bordered `Block` sets only the
//! border style; body cells retain the terminal-default background so
//! the user's wallpaper remains visible.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::{
    controls, metadata, modes, progress, source_status, state,
};
use crate::tui::view::theme::{is_ascii, play_glyph, Theme, ASCII_BORDER_SET};

/// Render the wide layout into `area`. The caller must ensure
/// `area.width >= 120 && area.height >= 30` (the deck's `pick_breakpoint`
/// enforces this).
pub fn render_wide(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let theme = Theme::default();

    // Outer block with a notched title. Use `pane_block_notched` so the
    // title sits in a notch (spec: `╭─ ▶ NOW PLAYING ────╮`, not
    // colliding with the `─` line).
    let title_marker = if focused { play_glyph() } else { "" };
    let title = if focused {
        if is_ascii() {
            "NOW PLAYING - FOCUSED"
        } else {
            "NOW PLAYING · FOCUSED"
        }
    } else {
        "NOW PLAYING"
    };
    let outer = theme.pane_block_notched(title, title_marker, focused, false);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Inner split: metadata (left, flex) + Up Next (right, 30 cols).
    let up_next_w = 30u16;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(80), Constraint::Length(up_next_w)])
        .split(inner);

    // Up Next shares the deck's top/bottom border and uses one vertical
    // divider. This avoids a nested panel inside the Now Playing surface.
    let divider = if is_ascii() {
        Block::default()
            .borders(Borders::LEFT)
            .border_set(ASCII_BORDER_SET)
            .border_style(theme.pane_border(false, false))
    } else {
        Block::default()
            .borders(Borders::LEFT)
            .border_style(theme.pane_border(false, false))
    };
    let up_next_inner = divider.inner(cols[1]);
    f.render_widget(divider, cols[1]);
    render_up_next_header(f, area, cols[1].x, &theme);
    render_up_next_column(f, up_next_inner, app);

    // Metadata column (left). Stack the rows.
    render_metadata_column(f, cols[0], app);
}

fn render_up_next_header(f: &mut Frame, area: Rect, divider_x: u16, theme: &Theme) {
    if divider_x >= area.right().saturating_sub(1) || area.height == 0 {
        return;
    }
    let width = area.right().saturating_sub(divider_x).saturating_sub(1);
    let (junction, line) = if is_ascii() {
        ('+', '-')
    } else {
        ('┬', '─')
    };
    let prefix = format!("{junction}{line} Up Next ");
    let header = format!(
        "{prefix}{}",
        line.to_string()
            .repeat((width as usize).saturating_sub(prefix.chars().count()))
    );
    f.render_widget(
        Paragraph::new(Span::styled(header, theme.pane_border(false, false))),
        Rect::new(divider_x, area.y, width, 1),
    );
    let bottom = if is_ascii() { "+" } else { "┴" };
    f.render_widget(
        Paragraph::new(Span::styled(bottom, theme.pane_border(false, false))),
        Rect::new(divider_x, area.bottom().saturating_sub(1), 1, 1),
    );
}

/// Render the metadata column (the left side of the wide layout).
fn render_metadata_column(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Reserve rows for metadata, explicit state, progress, controls, modes,
    // and subdued source status.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // metadata (artist/title/album/quality)
            Constraint::Length(1), // playback state
            Constraint::Length(1), // progress
            Constraint::Length(1), // controls + volume
            Constraint::Length(1), // modes
            Constraint::Length(1), // source status
            Constraint::Min(0),    // trailing space
        ])
        .split(area);

    metadata::render_metadata(f, rows[0], app);
    state::render_state(f, rows[1], app);
    progress::render_progress_bar(f, rows[2], app);
    controls::render_controls(f, rows[3], app);
    modes::render_modes(f, rows[4], app);
    source_status::render_source_status(f, rows[5], app);
}

/// Render the Up Next column (the right side of the wide layout).
fn render_up_next_column(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let dim_style = Style::default().fg(theme.dim);
    let key_style = Style::default().fg(theme.accent);

    // Show up to 3 queue entries from the manual queue. Each entry is
    // one row. The transport's `manual_queue` is the canonical up-next
    // list (the context-continuation order is handled by `peek_next`
    // for the single "Up next:" line; for the wide column we show the
    // explicit queue).
    let queue: Vec<String> = if app.transport.manual_queue.is_empty() {
        app.transport
            .peek_next(app, &app.catalog)
            .into_iter()
            .collect()
    } else {
        app.transport.manual_queue.iter().take(3).cloned().collect()
    };
    let mut y = area.y;
    if queue.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled("Queue is empty", dim_style)])),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    } else {
        for (i, id) in queue.iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            let title = resolve_title(app, id);
            let truncated = crate::tui::view::player_bar::truncate_title(
                &title,
                area.width.saturating_sub(4) as usize,
            );
            let line = format!("{:02}  {}", i + 1, truncated);
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(line, dim_style)])),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
        }
    }

    // Footer: [Q] Open queue hint.
    if y < area.bottom() {
        y = area.bottom().saturating_sub(1);
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled("[Q] Open queue", key_style)]))
                .alignment(Alignment::Left),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

/// Resolve a track id to a display title (local catalog first, then YT).
fn resolve_title(app: &App, id: &str) -> String {
    if let Some(t) = app.track_by_id_fast(id) {
        return format!("{} — {}", t.title, t.primary_artist);
    }
    if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
        return format!("{} — {}", rt.title, rt.artist);
    }
    format!("Loading{}", crate::tui::view::theme::ellipsis())
}
