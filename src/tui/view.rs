use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::{App, Pane};

pub fn draw(f: &mut Frame, app: &mut App) {
    // Main 3-pane area + a 1-line footer for keybinding hints + queue state.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(50), Constraint::Percentage(25)])
        .split(outer[0]);

    // Artists — rendered with a ListState so the view scrolls to follow the
    // cursor (without this the highlight disappears once it passes the bottom
    // visible row, which reads as "going out of bounds").
    let items: Vec<ListItem> = app.artists.iter()
        .map(|a| ListItem::new(a.as_str())).collect();
    let mut state = list_state(app.artist_cursor, app.artists.len());
    let list = List::new(items)
        .block(border("Artists", matches!(app.focus, Pane::Artists)))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list, chunks[0], &mut state);

    // Center column: Search (top) + Now Playing (bottom).
    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[1]);

    // Search pane: input line + ranked results (scrolls with the cursor).
    let mut lines: Vec<ListItem> = vec![ListItem::new(format!("/ {}", app.search_input))];
    for (score, tidx) in app.results.iter() {
        let t = &app.catalog.tracks[*tidx];
        let pct = (score * 100.0).clamp(0.0, 100.0);
        // Prefix shows queue state: `▶` playing, `+` enqueued, ` ` neither.
        let prefix = if app.now_playing.as_deref() == Some(&t.id) {
            "▶"
        } else if app.enqueued.contains(&t.id) {
            "+"
        } else {
            " "
        };
        let label = format!("{:>3.0}% {} {} — {}", pct, prefix, t.title, t.primary_artist);
        lines.push(ListItem::new(label));
    }
    let mut rstate = list_state(app.result_cursor, app.results.len());
    let slist = List::new(lines)
        .block(border("Search", matches!(app.focus, Pane::Search)))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(slist, center[0], &mut rstate);

    // Now Playing panel.
    let np = now_playing_text(app);
    f.render_widget(
        Paragraph::new(np).wrap(Wrap { trim: true }).block(border("Now Playing", false)),
        center[1],
    );

    // Queue pane: items with ▶ on the current; [dead] marks missing sources.
    let cur = app.queue.current_index();
    let qitems: Vec<ListItem> = app.queue().items().iter().enumerate()
        .map(|(i, id)| {
            let prefix = if Some(i) == cur { "▶ " } else { "  " };
            let dead_marker = if app.dead.contains(id) { " [dead]" } else { "" };
            let track = app.catalog.tracks.iter().find(|t| &t.id == id);
            let label = match track {
                Some(t) => format!("{prefix}{} — {}{dead_marker}", t.title, t.primary_artist),
                None => format!("{prefix}{id}{dead_marker}"),
            };
            ListItem::new(label)
        }).collect();
    // The queue's "cursor" is the current play position; scroll to follow it.
    let qcursor = cur.unwrap_or(0);
    let mut qstate = list_state(qcursor, app.queue().items().len());
    let qlist = List::new(qitems)
        .block(border("Queue", matches!(app.focus, Pane::Queue)))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(qlist, chunks[2], &mut qstate);

    // Footer: keybinding hints + queue count. Context-aware so the user knows
    // what space/enter will do in the focused pane.
    let hint = match app.focus {
        Pane::Search => "search: /filter  space/enter=enqueue+play  ↑↓=move  Tab=next pane  q=quit",
        Pane::Artists => "artists: enter=browse songs  space=enqueue all  ↑↓=move  /=search  Tab=next pane  q=quit",
        Pane::Queue => "queue: enter=play  x/r=remove  s=shuffle  c=clear  n/p=next/prev  q=quit",
    };
    let qcount = app.queue().items().len();
    let footer = format!(" {}  |  {} queued", hint, qcount);
    f.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
        outer[1],
    );
}

/// Build a `ListState` with `selected` set to `cursor`, clamped to the list
/// length. An empty list selects `None` (ratatui panics on `select(0)` for an
/// empty list).
fn list_state(cursor: usize, len: usize) -> ListState {
    let mut s = ListState::default();
    if len == 0 {
        s.select(None);
    } else {
        s.select(Some(cursor.min(len - 1)));
    }
    s
}

/// Build the now-playing text: title — artist, album, quality, position/duration.
fn now_playing_text(app: &App) -> String {
    let id = match &app.now_playing {
        Some(id) => id,
        None => return "Nothing playing.".to_string(),
    };
    let t = match app.catalog.tracks.iter().find(|t| &t.id == id) {
        Some(t) => t,
        None => return format!("Playing: {id}"),
    };
    let album = t.album.clone().unwrap_or_else(|| "—".to_string());
    let quality = t.quality_label();
    // When sample-rate switching is on, the output device is switched to match
    // the track's format, so the quality line doubles as the device output rate.
    let quality_label = if app.switch_sample_rate { "Output" } else { "Quality" };
    let pos = app.player.position();
    let dur = app.player.duration();
    let time = match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => format!("  {:.0}s / {:.0}s", p, d),
        (Some(p), _) => format!("  {:.0}s", p),
        _ => String::new(),
    };
    format!(
        "{} — {}\nAlbum: {}\n{}: {}{}",
        t.title, t.primary_artist, album, quality_label, quality, time
    )
}

fn border<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
    Block::default().borders(Borders::ALL).title(title).border_style(style)
}
