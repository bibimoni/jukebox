use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
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

    // Search
    let search = Paragraph::new(format!("Search: {}", app.search_input))
        .block(border("Search", matches!(app.focus, Pane::Search)));
    f.render_widget(search, chunks[1]);

    // Queue
    let q: Vec<ListItem> = app.queue().items().iter()
        .map(|id| ListItem::new(id.as_str())).collect();
    let qlist = List::new(q).block(border("Queue", matches!(app.focus, Pane::Queue)));
    f.render_widget(qlist, chunks[2]);
}

fn border<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
    Block::default().borders(Borders::ALL).title(title).border_style(style)
}
