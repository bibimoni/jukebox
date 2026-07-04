use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

use crate::tui::{App, Pane};

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(50), Constraint::Percentage(25)])
        .split(f.area());

    // Artists
    let items: Vec<ListItem> = app.artists.iter().enumerate()
        .map(|(i, a)| {
            let style = if i == app.artist_cursor { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
            ListItem::new(a.as_str()).style(style)
        }).collect();
    let list = List::new(items).block(border("Artists", matches!(app.focus, Pane::Artists)));
    f.render_widget(list, chunks[0]);

    // Search pane: input line + ranked results
    let mut lines: Vec<ListItem> = vec![ListItem::new(format!("/ {}", app.search_input))];
    for (i, (score, tidx)) in app.results.iter().enumerate() {
        let t = &app.catalog.tracks[*tidx];
        let label = format!("{:>3.0}%  {} — {}", (score * 100.0).clamp(0.0, 100.0), t.title, t.primary_artist);
        let style = if i == app.result_cursor {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else { Style::default() };
        lines.push(ListItem::new(label).style(style));
    }
    let list = List::new(lines).block(border("Search", matches!(app.focus, Pane::Search)));
    f.render_widget(list, chunks[1]);

    // Queue pane: items with ▶ on the current
    let cur = app.queue.current_index();
    let qitems: Vec<ListItem> = app.queue().items().iter().enumerate()
        .map(|(i, id)| {
            let prefix = if Some(i) == cur { "▶ " } else { "  " };
            let track = app.catalog.tracks.iter().find(|t| &t.id == id);
            let label = match track {
                Some(t) => format!("{prefix}{} — {}", t.title, t.primary_artist),
                None => format!("{prefix}{id}"),
            };
            ListItem::new(label)
        }).collect();
    let qlist = List::new(qitems).block(border("Queue", matches!(app.focus, Pane::Queue)));
    f.render_widget(qlist, chunks[2]);
}

fn border<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
    Block::default().borders(Borders::ALL).title(title).border_style(style)
}
