pub mod queue;
pub mod view;

use crate::catalog::Catalog;
use crate::player::Player;
use std::collections::BTreeMap;

pub enum Pane { Artists, Search, Queue }

pub struct App {
    pub catalog: Catalog,
    pub player: Box<dyn Player>,
    pub queue: queue::Queue,
    pub artists: Vec<String>,                       // sorted unique artist names
    pub artist_index: BTreeMap<String, Vec<usize>>, // artist -> track indices
    pub artist_cursor: usize,
    pub focus: Pane,
    pub search_input: String,
    pub results: Vec<(f32, usize)>,                  // (score, track_index)
    pub result_cursor: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(catalog: Catalog, player: Box<dyn Player>) -> Self {
        let mut idx: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, t) in catalog.tracks.iter().enumerate() {
            for a in &t.symlinked_into_artists {
                idx.entry(a.clone()).or_default().push(i);
            }
        }
        let artists: Vec<String> = idx.keys().cloned().collect();
        App {
            catalog, player, queue: queue::Queue::new(),
            artists, artist_index: idx,
            artist_cursor: 0, focus: Pane::Artists,
            search_input: String::new(), results: Vec::new(), result_cursor: 0,
            should_quit: false,
        }
    }

    pub fn artists(&self) -> &Vec<String> { &self.artists }
    pub fn queue(&self) -> &queue::Queue { &self.queue }

    pub fn enqueue_artist(&mut self, artist: &str) {
        if let Some(tracks) = self.artist_index.get(artist) {
            for &i in tracks {
                self.queue.enqueue(self.catalog.tracks[i].id.clone());
            }
        }
    }

    /// Run the terminal event loop. Returns when the user quits.
    pub fn run(&mut self) -> anyhow::Result<()> {
        use crossterm::execute;
        use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
        use ratatui::backend::CrosstermBackend;
        use ratatui::Terminal;

        terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend)?;

        let result = self.run_loop(&mut term);

        // Unconditional cleanup — runs on Ok AND Err so the terminal is never
        // left in raw mode + alt screen if the loop errors.
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        result
    }

    fn run_loop(
        &mut self,
        term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<()> {
        use crossterm::event::{self, Event, KeyEventKind};

        while !self.should_quit {
            term.draw(|f| view::draw(f, self))?;
            if event::poll(std::time::Duration::from_millis(200))? {
                let ev = event::read()?;
                if let Event::Key(k) = ev {
                    if k.kind != KeyEventKind::Press { continue; }
                    self.handle_key(k.code);
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode::*;
        match code {
            Tab => self.focus = match self.focus {
                Pane::Artists => Pane::Search, Pane::Search => Pane::Queue, Pane::Queue => Pane::Artists,
            },
            Char('q') => { self.should_quit = true; self.player.stop().ok(); }
            Down => self.cursor_down(),
            Up => self.cursor_up(),
            Char(' ') if matches!(self.focus, Pane::Artists) => {
                if let Some(a) = self.artists.get(self.artist_cursor).cloned() {
                    self.enqueue_artist(&a);
                }
            }
            _ => {}
        }
    }

    fn cursor_down(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor + 1 < self.artists.len() { self.artist_cursor += 1; } }
            _ => {}
        }
    }
    fn cursor_up(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor > 0 { self.artist_cursor -= 1; } }
            _ => {}
        }
    }
}
