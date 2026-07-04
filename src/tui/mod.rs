pub mod queue;
pub mod view;

use crate::catalog::Catalog;
use crate::player::Player;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy)]
pub enum Pane { Artists, Search, Queue }

impl Pane {
    /// Stable storage key for this pane, written to the state DB. Keep these
    /// in sync with `state::ARTISTS` etc.
    pub fn db_key(&self) -> &'static str {
        match self {
            Pane::Artists => crate::state::ARTISTS,
            Pane::Search => crate::state::SEARCH,
            Pane::Queue => crate::state::QUEUE,
        }
    }
    /// Parse a stored DB key back into a Pane. Unknown values fall back to
    /// Artists so a corrupted/garbage row can't crash startup.
    pub fn from_db_key(s: &str) -> Self {
        match s {
            "search" => Pane::Search,
            "queue" => Pane::Queue,
            _ => Pane::Artists,
        }
    }
}

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
    /// When true, switch the macOS default output device's sample rate + bit
    /// depth to match each track before playback (CoreAudio, in-process).
    /// No-op on non-macOS. Set from config in main.rs.
    pub switch_sample_rate: bool,
    /// Track ids enqueued this session — used to mark Search results that are
    /// already in the queue with a `+` so space/enter gives visible feedback.
    pub enqueued: HashSet<String>,
    /// Last mouse click (timestamp + position) for double-click detection —
    /// a double-click in Search/Artists/Queue enqueues/plays the clicked row.
    pub last_click: Option<(std::time::Instant, u16, u16)>,
    /// Visual cursor row in the Queue pane. Decoupled from the queue's playback
    /// cursor (which jumps around under shuffle) so ↑/↓ move the highlight to the
    /// adjacent visible row instead of the next-in-playback-order. Enter or
    /// double-click on the highlighted row jumps playback there via jump_to.
    pub queue_cursor: usize,
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
            switch_sample_rate: true,
            enqueued: HashSet::new(),
            last_click: None,
            queue_cursor: 0,
        }
    }

    pub fn artists(&self) -> &Vec<String> { &self.artists }
    pub fn queue(&self) -> &queue::Queue { &self.queue }

    /// Populate the Search results with a specific artist's tracks (sorted by
    /// title), without enqueueing any of them. Lets the user browse an
    /// artist's songs and pick one with `space`/`enter` instead of dumping
    /// the whole artist into the queue. Switches focus to the Search pane.
    /// Pressing `/` or typing replaces this browse view with a normal search.
    pub fn browse_artist(&mut self) {
        if let Some(a) = self.artists.get(self.artist_cursor).cloned() {
            if let Some(tracks) = self.artist_index.get(&a).cloned() {
                self.results = tracks
                    .into_iter()
                    .map(|i| (1.0, i))
                    .collect();
                // Sort by title for a stable, scannable browse order.
                self.results.sort_by(|(_, a), (_, b)| {
                    self.catalog.tracks[*a]
                        .title
                        .to_lowercase()
                        .cmp(&self.catalog.tracks[*b].title.to_lowercase())
                });
                self.result_cursor = 0;
                self.focus = Pane::Search;
                self.search_input.clear();
            }
        }
    }

    pub fn enqueue_artist(&mut self, artist: &str) {
        if let Some(tracks) = self.artist_index.get(artist) {
            let was_empty = self.now_playing.is_none() && self.queue.is_empty();
            for &i in tracks {
                let id = self.catalog.tracks[i].id.clone();
                self.queue.enqueue(id.clone());
                self.enqueued.insert(id);
            }
            if was_empty {
                self.play_current_queue();
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

    /// Enqueue the track under the result cursor. If nothing is playing yet,
    /// start playback immediately so the user gets audio feedback (otherwise
    /// space feels like a no-op — the track goes into the queue but the user
    /// sees no change). Marks the track `+` in the Search pane.
    pub fn enqueue_current_result(&mut self) {
        if let Some(&(_, idx)) = self.results.get(self.result_cursor) {
            let id = self.catalog.tracks[idx].id.clone();
            let was_empty = self.now_playing.is_none() && self.queue.is_empty();
            self.queue.enqueue(id.clone());
            self.enqueued.insert(id);
            if was_empty {
                self.play_current_queue();
            }
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
            // Switch the macOS output device to the track's sample rate + bit
            // depth before loading. Best-effort: a failure to switch (e.g. the
            // device doesn't support the format) should not block playback.
            if self.switch_sample_rate {
                if let Err(e) = crate::audio::set_output_format(t.sample_rate_hz, t.bit_depth) {
                    eprintln!("sample-rate switch failed (continuing): {e}");
                }
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
        // left in raw mode + alt screen + mouse capture if the loop errors.
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        result
    }

    fn run_loop(
        &mut self,
        term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<()> {
        use crossterm::event::{self, Event, KeyEventKind};
        // Enable mouse capture so click-to-focus + wheel scrolling work. Disabled
        // unconditionally on exit (in `run`) so the terminal is never left in a
        // captured state on panic/error.
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;

        while !self.should_quit {
            term.draw(|f| view::draw(f, self))?;
            // Auto-advance: when the current track ends naturally (mpv end-file
            // eof, or afplay child exit), drop the finished track if consume is
            // on (so the queue drains as you listen), then play the next.
            if self.player.track_ended() && !self.queue.is_empty() {
                // Consume: a finished track is removed from the queue on
                // natural end (mpv end-file eof / afplay exit), so the queue
                // drains as you listen. This is unconditional — there's no
                // toggle, finished tracks always leave the queue.
                self.queue.consume_current();
                self.play_current_queue();
                self.sync_queue_cursor();
            }
            if event::poll(std::time::Duration::from_millis(200))? {
                let ev = event::read()?;
                match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.handle_key(k.code),
                    Event::Mouse(m) => self.handle_mouse(m),
                    _ => {}
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
            // Shift+Tab (crossterm delivers it as BackTab) moves focus backward.
            BackTab => self.focus = match self.focus {
                Pane::Artists => Pane::Queue, Pane::Search => Pane::Artists, Pane::Queue => Pane::Search,
            },
            Char('q') => { self.should_quit = true; self.player.stop().ok(); }
            Down => self.cursor_down(),
            Up => self.cursor_up(),
            // space = play/pause in EVERY pane. The footer advertises it
            // globally, so it must actually work globally — previously it only
            // fired in the Queue pane, so pressing space in Artists/Search
            // looked like pause was broken.
            Char(' ') => { let _ = self.player.play_pause(); }
            // `a` enqueues all of the focused artist's tracks (was on space;
            // moved so space can be the global pause).
            Char('a') if matches!(self.focus, Pane::Artists) => {
                if let Some(a) = self.artists.get(self.artist_cursor).cloned() {
                    self.enqueue_artist(&a);
                }
            }
            Enter if matches!(self.focus, Pane::Artists) => self.browse_artist(),
            Char('/') => { self.focus = Pane::Search; self.search_input.clear(); }
            Char(c) if matches!(self.focus, Pane::Search) => { self.search_input.push(c); self.run_search(); }
            Backspace if matches!(self.focus, Pane::Search) => { self.search_input.pop(); self.run_search(); }
            Enter if matches!(self.focus, Pane::Search) => self.enqueue_current_result(),
            Enter if matches!(self.focus, Pane::Queue) => {
                // Play the highlighted row (queue_cursor), not the playback
                // cursor — so enter on a visually-selected track under shuffle
                // plays that exact track.
                if let Some(id) = self.queue().items().get(self.queue_cursor).cloned() {
                    self.queue.jump_to(&id);
                    self.play_current_queue();
                } else {
                    self.play_current_queue();
                }
            }
            Char('s') => { self.queue.shuffle(42); self.sync_queue_cursor(); }
            Char('S') => { self.queue.shuffle(42); self.queue.next(); self.play_current_queue(); self.sync_queue_cursor(); }
            Char('x') if matches!(self.focus, Pane::Queue) => {
                if let Some(id) = self.queue.current().cloned() { self.queue.remove(&id); }
                self.sync_queue_cursor();
            }
            Char('r') if matches!(self.focus, Pane::Queue) => {
                if let Some(id) = self.queue.current().cloned() { self.queue.remove(&id); }
                self.sync_queue_cursor();
            }
            Char('c') if matches!(self.focus, Pane::Queue) => { self.queue.clear(); self.queue_cursor = 0; }
            Char('n') => { self.queue.next(); self.play_current_queue(); self.sync_queue_cursor(); }
            Char('p') => { self.queue.prev(); self.play_current_queue(); self.sync_queue_cursor(); }
            Left => { let _ = self.player.seek(-5.0); }
            Right => { let _ = self.player.seek(5.0); }
            // R clears the saved UI state (focused pane) so the next launch
            // restores defaults instead of the last-focused pane.
            Char('R') => {
                if let Err(e) = crate::state::clear() {
                    eprintln!("failed to clear state: {e}");
                }
            }
            _ => {}
        }
    }

    /// Map a mouse event to a pane focus + cursor move. Click in a pane focuses
    /// it and selects the clicked row; double-click (two clicks on the same row
    /// within 400ms) enqueues/plays it. The wheel scrolls the focused pane.
    fn handle_mouse(&mut self, m: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseEventKind, MouseButton};
        // Pane layout must match view::draw: outer[0] split into 3 horizontal
        // columns — Artists (25%) | Search+NowPlaying (50%) | Queue (25%).
        // We re-derive which column the click landed in from the terminal width.
        // The view splits outer[0] horizontally, so a click's column decides
        // the pane; a click's row (minus the 1-line top border) decides the item.
        match m.kind {
            MouseEventKind::ScrollDown => { for _ in 0..3 { self.cursor_down(); } }
            MouseEventKind::ScrollUp => { for _ in 0..3 { self.cursor_up(); } }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                self.click_pane(m.column, m.row);
            }
            _ => {}
        }
    }

    /// Focus the pane under (col, row) and set its cursor to the clicked row.
    /// Double-click on the same row within 400ms enqueues/plays it.
    fn click_pane(&mut self, col: u16, row: u16) {
        // Determine pane from column. Layout: [25% | 50% | 25%] of terminal width.
        let w = term_width();
        let a_end = w / 4;
        let q_start = w * 3 / 4;
        let clicked_pane = if (col as usize) < a_end {
            Some(Pane::Artists)
        } else if (col as usize) >= q_start {
            Some(Pane::Queue)
        } else {
            Some(Pane::Search) // center column; sub-split into Search/NowPlaying by row handled below
        };
        // Row within a pane = row - pane_top - 1 (top border). The three columns
        // share the same vertical span (outer[0]), so pane_top is 0.
        let item_row = row.saturating_sub(1) as usize;

        let pane = match clicked_pane { Some(p) => p, None => return };
        // Center column upper 70% is Search; lower 30% is Now Playing. A click
        // below the search area just focuses Search without selecting past end.
        if matches!(pane, Pane::Search) || matches!(pane, Pane::Artists) || matches!(pane, Pane::Queue) {
            self.focus = pane;
        }
        match pane {
            Pane::Artists => {
                if item_row < self.artists.len() { self.artist_cursor = item_row; }
                if self.is_double_click(col, row) { self.browse_artist(); }
            }
            Pane::Search => {
                if item_row > 0 && item_row - 1 < self.results.len() {
                    self.result_cursor = item_row - 1; // row 0 is the "/ query" input line
                    if self.is_double_click(col, row) { self.enqueue_current_result(); }
                }
            }
            Pane::Queue => {
                // Single click moves the VISUAL highlight to the clicked row;
                // double-click jumps playback there and plays.
                let items_len = self.queue().items().len();
                if item_row < items_len {
                    self.queue_cursor = item_row;
                    if self.is_double_click(col, row) {
                        let id = self.queue().items()[item_row].clone();
                        self.queue.jump_to(&id);
                        self.play_current_queue();
                    }
                }
            }
        }
    }

    fn is_double_click(&mut self, col: u16, row: u16) -> bool {
        let now = std::time::Instant::now();
        if let Some((t, c, r)) = self.last_click {
            if now.duration_since(t).as_millis() < 400 && c == col && r == row {
                self.last_click = None;
                return true;
            }
        }
        self.last_click = Some((now, col, row));
        false
    }

    /// Point the Queue pane's visual cursor at the currently-playing track and
    /// clamp it to the queue length. Called after operations that change what's
    /// playing (shuffle, next/prev, remove, consume-on-end) so the highlight
    /// follows playback — but ↑/↓ move it off again freely afterward.
    fn sync_queue_cursor(&mut self) {
        let items = self.queue().items();
        if items.is_empty() { self.queue_cursor = 0; return; }
        match self.queue.current() {
            Some(id) => {
                self.queue_cursor = items.iter().position(|x| x == id).unwrap_or(0);
            }
            None => { self.queue_cursor = self.queue_cursor.min(items.len() - 1); }
        }
    }

    pub fn cursor_down(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor + 1 < self.artists.len() { self.artist_cursor += 1; } }
            Pane::Search => { if self.result_cursor + 1 < self.results.len() { self.result_cursor += 1; } }
            // Move the VISUAL highlight one row down — not queue.next(), which
            // jumps to the next-in-playback-order (and skips around under shuffle).
            Pane::Queue => {
                if self.queue_cursor + 1 < self.queue().items().len() { self.queue_cursor += 1; }
            }
        }
    }
    pub fn cursor_up(&mut self) {
        match self.focus {
            Pane::Artists => { if self.artist_cursor > 0 { self.artist_cursor -= 1; } }
            Pane::Search => { if self.result_cursor > 0 { self.result_cursor -= 1; } }
            // Move the VISUAL highlight one row up — not queue.prev(), which
            // jumps to the previous-in-playback-order under shuffle.
            Pane::Queue => {
                if self.queue_cursor > 0 { self.queue_cursor -= 1; }
            }
        }
    }
}

/// Terminal width in columns, used by mouse hit-testing to map a click column
/// to one of the three layout panes. Falls back to 80 if the size can't be
/// read (shouldn't happen inside the alt screen, but don't crash on it).
fn term_width() -> usize {
    crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80)
}
