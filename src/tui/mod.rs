pub mod queue;
pub mod view;

use crate::catalog::Catalog;
use crate::player::Player;
use std::collections::{BTreeMap, HashSet};

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
    pub searcher: Option<crate::search::Searcher>,
    /// Track ids whose source file is missing or a broken symlink. Marked dead
    /// at play time and skipped; shown as `[dead]` in the queue pane.
    pub dead: HashSet<String>,
    /// The id of the track most recently loaded into the player (for the
    /// Now Playing panel). `None` until the first track plays.
    pub now_playing: Option<String>,
}

impl App {
    pub fn new(catalog: Catalog, player: Box<dyn Player>, searcher: Option<crate::search::Searcher>) -> Self {
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
            searcher,
            dead: HashSet::new(),
            now_playing: None,
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

    /// Run the current `search_input` against the searcher (if any) and
    /// populate `results` with `(score, track_index)` pairs for hits that
    /// resolve to a track in the catalog. Resets the result cursor.
    pub fn run_search(&mut self) {
        if let Some(s) = self.searcher.as_ref() {
            if let Ok(hits) = s.search(&self.search_input, 50) {
                self.results = hits.into_iter().filter_map(|h| {
                    self.catalog.tracks.iter().position(|t| t.id == h.track_id).map(|p| (h.score, p))
                }).collect();
                self.result_cursor = 0;
            }
        }
    }

    /// Enqueue the track under the result cursor (no-op if no results).
    pub fn enqueue_current_result(&mut self) {
        if let Some(&(_, idx)) = self.results.get(self.result_cursor) {
            self.queue.enqueue(self.catalog.tracks[idx].id.clone());
        }
    }

    /// Load the current queue item into the player and start playback.
    ///
    /// Per spec §Error Handling ("TUI marks the track dead, skips to next,
    /// logs"), a track whose source file is missing or a broken symlink is
    /// marked dead and skipped. We iterate through the queue at most
    /// `queue.len()` times so an entirely-dead queue can't recurse forever.
    pub fn play_current_queue(&mut self) {
        let n = self.queue.len();
        if n == 0 { return; }
        let start = self.queue.current_index();
        for _ in 0..n {
            let id = match self.queue.current().cloned() {
                Some(id) => id,
                None => return,
            };
            // Already known dead? Skip to next.
            if self.dead.contains(&id) {
                self.queue.next();
                continue;
            }
            let t = match self.catalog.tracks.iter().find(|t| t.id == id) {
                Some(t) => t,
                None => { self.queue.next(); continue; }
            };
            let path = t.resolve_source(&self.catalog.source_root);
            // std::fs::metadata follows symlinks: a broken symlink or missing
            // file yields Err, which we treat as a dead track.
            if std::fs::metadata(&path).is_err() {
                eprintln!(
                    "dead track {} (source missing: {}); skipping",
                    id, path.display()
                );
                self.dead.insert(id.clone());
                self.queue.next();
                // If we've looped back to the start, the whole queue is dead.
                if self.queue.current_index() == start {
                    eprintln!("all queued tracks are dead; nothing to play");
                    return;
                }
                continue;
            }
            let _ = self.player.load(&path);
            self.now_playing = Some(id);
            return;
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
            Char('/') => { self.focus = Pane::Search; self.search_input.clear(); }
            Char(c) if matches!(self.focus, Pane::Search) => { self.search_input.push(c); self.run_search(); }
            Backspace if matches!(self.focus, Pane::Search) => { self.search_input.pop(); self.run_search(); }
            Enter if matches!(self.focus, Pane::Search) => self.enqueue_current_result(),
            Enter if matches!(self.focus, Pane::Queue) => { self.play_current_queue(); }
            Char('s') => { self.queue.shuffle(42); }
            Char('S') => { self.queue.shuffle(42); self.queue.next(); self.play_current_queue(); }
            Char('r') if matches!(self.focus, Pane::Queue) => {
                if let Some(id) = self.queue.current().cloned() { self.queue.remove(&id); }
            }
            Char('c') if matches!(self.focus, Pane::Queue) => { self.queue.clear(); }
            Char('n') => { self.queue.next(); self.play_current_queue(); }
            Char('p') => { self.queue.prev(); self.play_current_queue(); }
            Left => { let _ = self.player.seek(-5.0); }
            Right => { let _ = self.player.seek(5.0); }
            _ => {}
        }
    }

    fn cursor_down(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor + 1 < self.artists.len() { self.artist_cursor += 1; } }
            Pane::Search => { if self.result_cursor + 1 < self.results.len() { self.result_cursor += 1; } }
            Pane::Queue => { self.queue.next(); }
        }
    }
    fn cursor_up(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor > 0 { self.artist_cursor -= 1; } }
            Pane::Search => { if self.result_cursor > 0 { self.result_cursor -= 1; } }
            Pane::Queue => { self.queue.prev(); }
        }
    }
}
