//! App state + pure (context-play) update methods.
//!
//! [`App`] owns the whole TUI world: the catalog, the player backend, the
//! transport engine, playlists, browse state (view + cursors + column widths),
//! volume, the dead-track set, and the current overlay. All update methods here
//! are pure with respect to the transport — they take `&mut self` and call into
//! [`Transport`] (Task 4) with `self` borrowed immutably as the
//! [`ContextResolver`] (since `playlists` / `transport.manual_queue` live in
//! separate fields, this split-borrow works).

use std::collections::{BTreeMap, HashSet};

use crate::catalog::{Catalog, Track};
use crate::player::Player;
use crate::search::Searcher;
use crate::tui::context::{build_albums_by_artist, Album, Context, ContextResolver};
use crate::tui::queue::{RepeatMode, ShuffleMode, Transport};

/// Which top-level browse view is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Artists,
    Playlists,
    Queue,
}

/// A user-defined playlist: name + ordered track ids.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Playlist {
    pub name: String,
    #[serde(default)]
    pub track_ids: Vec<String>,
}

/// Per-column browse cursors.
#[derive(Clone, Default)]
pub struct ColumnCursors {
    pub artist: usize,
    pub album: usize,
    pub track: usize,
    pub playlist: usize,
    pub queue: usize,
    pub search: usize,
}

/// Column widths for the three-pane browse layout.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ColumnWidths {
    pub rail: u16,
    pub col1: u16,
    pub col2: u16,
    pub col3: u16,
}
impl Default for ColumnWidths {
    fn default() -> Self {
        ColumnWidths {
            rail: 4,
            col1: 24,
            col2: 28,
            col3: 48,
        }
    }
}

/// Modal overlays drawn on top of the browse layout. Defined minimally here so
/// `App.overlay: Option<Overlay>` compiles; Task 11 fills in the full surface.
#[derive(Clone)]
pub enum Overlay {
    Search {
        input: String,
        results: Vec<String>,
        cursor: usize,
    },
    Help,
    PlaylistPicker,
    Command {
        input: String,
    },
}

/// The central TUI state struct.
pub struct App {
    pub catalog: Catalog,
    pub player: Box<dyn Player>,
    pub searcher: Option<Searcher>,
    pub transport: Transport,
    pub playlists: Vec<Playlist>,
    pub artists: Vec<String>,
    pub artist_index: BTreeMap<String, Vec<usize>>,
    pub albums_by_artist: BTreeMap<String, Vec<Album>>,
    pub view: View,
    pub focus_col: usize,
    pub cursors: ColumnCursors,
    pub column_widths: ColumnWidths,
    pub volume: u8,
    pub muted: bool,
    pub now_playing: Option<String>,
    pub dead: HashSet<String>,
    pub switch_sample_rate: bool,
    pub should_quit: bool,
    pub overlay: Option<Overlay>,
    /// Leader-key state for the `gg` mapping (top of column). `g` arms it;
    /// a second `g` within one dispatch consumes it and jumps to row 0.
    pub pending_g: bool,
}

impl ContextResolver for App {
    fn playlist_ids(&self, name: &str) -> Vec<String> {
        self.playlists
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.track_ids.clone())
            .unwrap_or_default()
    }
    fn queue_ids(&self) -> Vec<String> {
        self.transport.manual_queue.clone()
    }
}

/// A [`ContextResolver`] view backed by borrowed `playlists` (a disjoint field
/// from `transport`) and a *cloned* snapshot of `transport.manual_queue`.
///
/// This exists to satisfy the borrow checker: `Transport` methods take
/// `&dyn ContextResolver`, which would borrow all of `self` and conflict with
/// the `&mut self.transport` the same call needs. By cloning just the
/// `manual_queue` snapshot (cheap; the TUI's manual queue is small) and
/// borrowing `playlists` separately, the only outstanding borrow of `self` when
/// we call `&mut self.transport` is `&self.playlists` — a disjoint field — so
/// the split-borrow is sound.
struct ClonedResolver<'a> {
    playlists: &'a [Playlist],
    manual_queue: Vec<String>,
}

impl ContextResolver for ClonedResolver<'_> {
    fn playlist_ids(&self, name: &str) -> Vec<String> {
        self.playlists
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.track_ids.clone())
            .unwrap_or_default()
    }
    fn queue_ids(&self) -> Vec<String> {
        self.manual_queue.clone()
    }
}

impl App {
    pub fn new(catalog: Catalog, player: Box<dyn Player>, searcher: Option<Searcher>) -> Self {
        let mut artist_index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, t) in catalog.tracks.iter().enumerate() {
            for a in &t.symlinked_into_artists {
                artist_index.entry(a.clone()).or_default().push(i);
            }
        }
        let artists: Vec<String> = artist_index.keys().cloned().collect();
        let albums_by_artist = build_albums_by_artist(&catalog);
        let transport = Transport::new(Context::Artist {
            artist: String::new(),
            track_ids: vec![],
        });
        App {
            catalog,
            player,
            searcher,
            transport,
            playlists: Vec::new(),
            artists,
            artist_index,
            albums_by_artist,
            view: View::Artists,
            focus_col: 0,
            cursors: ColumnCursors::default(),
            column_widths: ColumnWidths::default(),
            volume: 70,
            muted: false,
            now_playing: None,
            dead: HashSet::new(),
            switch_sample_rate: true,
            should_quit: false,
            overlay: None,
            pending_g: false,
        }
    }

    fn track_by_id(&self, id: &str) -> Option<&Track> {
        self.catalog.tracks.iter().find(|t| t.id == id)
    }

    /// Build the track-id list for the currently-focused track column.
    fn current_context_ids(&self) -> Vec<String> {
        match self.view {
            View::Artists => {
                let artist = self
                    .artists
                    .get(self.cursors.artist)
                    .cloned()
                    .unwrap_or_default();
                let album = self
                    .albums_by_artist
                    .get(&artist)
                    .and_then(|a| a.get(self.cursors.album))
                    .cloned();
                match album {
                    Some(a) => a
                        .track_indices
                        .iter()
                        .map(|&i| self.catalog.tracks[i].id.clone())
                        .collect(),
                    None => vec![],
                }
            }
            View::Playlists => self
                .playlists
                .get(self.cursors.playlist)
                .map(|p| p.track_ids.clone())
                .unwrap_or_default(),
            View::Queue => self.transport.manual_queue.clone(),
        }
    }

    /// Resolve the [`Context`] for the current view + cursor position.
    fn context_for_current_view(&self, ids: Vec<String>) -> Context {
        match self.view {
            View::Artists => {
                let artist = self
                    .artists
                    .get(self.cursors.artist)
                    .cloned()
                    .unwrap_or_default();
                let album = self
                    .albums_by_artist
                    .get(&artist)
                    .and_then(|a| a.get(self.cursors.album))
                    .map(|a| (a.title.clone(), a.artist.clone()));
                match album {
                    Some((title, artist)) => Context::Album {
                        album: title,
                        artist,
                        track_ids: ids,
                    },
                    None => Context::Artist { artist, track_ids: ids },
                }
            }
            View::Playlists => Context::Playlist {
                name: self
                    .playlists
                    .get(self.cursors.playlist)
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
            },
            View::Queue => Context::Queue,
        }
    }

    /// Begin playback at the current transport cursor, skipping dead tracks.
    fn start_playback(&mut self) {
        let n = self.transport.order.len();
        if n == 0 {
            return;
        }
        let start = self.transport.cursor;
        for _ in 0..n.max(1) {
            let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
            let id = match self.transport.current(&r, &self.catalog) {
                Some(id) => id,
                None => return,
            };
            drop(r);
            if self.dead.contains(&id) {
                let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                let _ = self.transport.next(&r, &self.catalog);
                if self.transport.cursor == start {
                    return;
                }
                continue;
            }
            let t = match self.track_by_id(&id) {
                Some(t) => t,
                None => {
                    let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                    let _ = self.transport.next(&r, &self.catalog);
                    if self.transport.cursor == start {
                        return;
                    }
                    continue;
                }
            };
            let path = t.resolve_source(&self.catalog.source_root);
            if std::fs::metadata(&path).is_err() {
                self.dead.insert(id.clone());
                let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                let _ = self.transport.next(&r, &self.catalog);
                if self.transport.cursor == start {
                    return;
                }
                continue;
            }
            if self.switch_sample_rate {
                let _ = crate::audio::set_output_format(t.sample_rate_hz, t.bit_depth);
            }
            let _ = self.player.load(&path);
            self.now_playing = Some(id);
            return;
        }
    }

    /// Load the track at the current transport cursor into the player (no
    /// advance). Used after `next`/`prev` already moved the cursor.
    fn start_playback_at_current(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        if let Some(id) = self.transport.current(&r, &self.catalog) {
            if self.dead.contains(&id) {
                return;
            }
            if let Some(t) = self.track_by_id(&id) {
                let path = t.resolve_source(&self.catalog.source_root);
                if self.switch_sample_rate {
                    let _ = crate::audio::set_output_format(t.sample_rate_hz, t.bit_depth);
                }
                let _ = self.player.load(&path);
                self.now_playing = Some(id);
            }
        }
    }

    /// Play the track under the track-column cursor in the current view.
    pub fn play_selected(&mut self) {
        let ids = self.current_context_ids();
        if ids.is_empty() {
            return;
        }
        let start = match ids.get(self.cursors.track).cloned() {
            Some(s) => s,
            None => return,
        };
        let ctx = self.context_for_current_view(ids);
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport
            .switch_context(ctx, Some(&start), &r, &self.catalog);
        self.start_playback();
    }

    /// Test helper: play within an explicit id list (for the dead-track test).
    pub fn play_in_context_ids(&mut self, ids: Vec<String>, start: &str) {
        let ctx = Context::Search {
            query: String::new(),
            track_ids: ids,
        };
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport
            .switch_context(ctx, Some(start), &r, &self.catalog);
        self.start_playback();
    }

    pub fn next(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        if let Some(id) = self.transport.next(&r, &self.catalog) {
            self.now_playing = Some(id);
            self.start_playback_at_current();
        } else {
            self.player.stop().ok();
            self.now_playing = None;
        }
    }

    pub fn prev(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        if let Some(id) = self.transport.prev(&r, &self.catalog) {
            self.now_playing = Some(id);
            self.start_playback_at_current();
        }
    }

    /// Auto-advance when the player reports a natural end-of-track.
    pub fn on_track_ended(&mut self) {
        self.next();
    }

    pub fn cycle_shuffle(&mut self) {
        let m = match self.transport.shuffle {
            ShuffleMode::Off => ShuffleMode::Smart,
            ShuffleMode::Smart => ShuffleMode::Random,
            ShuffleMode::Random => ShuffleMode::Off,
        };
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport.set_shuffle(m, &r, &self.catalog);
    }

    pub fn reshuffle(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport.reshuffle(&r, &self.catalog);
    }

    pub fn cycle_repeat(&mut self) {
        self.transport.set_repeat(match self.transport.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        });
    }

    pub fn volume_up(&mut self) {
        self.volume = self.volume.saturating_add(5).min(100);
        self.muted = false;
    }

    pub fn volume_down(&mut self) {
        self.volume = self.volume.saturating_sub(5);
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
        self.player.stop().ok();
    }

    /// Run a catalog search for `q` against `self.searcher` and return the
    /// track ids (in BM25 order, up to 50) that resolve to extant tracks in
    /// `self.catalog.tracks`. Returns an empty `Vec` if no index is present.
    ///
    /// This is the single source of truth for the search overlay's `results`
    /// field: every mutation of the overlay's `input` should be followed by a
    /// call to [`App::update_search_results`] (which calls this internally).
    pub fn run_search(&self, q: &str) -> Vec<String> {
        let Some(searcher) = self.searcher.as_ref() else {
            return Vec::new();
        };
        let hits = match searcher.search(q, 50) {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        // Keep only ids that exist in the catalog (the index may lag behind a
        // re-scan). A linear scan per hit is fine for ~50 results.
        hits.into_iter()
            .map(|h| h.track_id)
            .filter(|id| self.catalog.tracks.iter().any(|t| t.id == *id))
            .collect()
    }

    /// If the active overlay is `Search`, re-run the search against its current
    /// `input` and replace `results` with the fresh id list, clamping `cursor`
    /// to the new result count. No-op for non-search overlays.
    /// The current browse view as a stable string key, for state persistence.
    /// Keep these strings stable — `state::load_layout` parses them back into a
    /// [`View`] on the next launch, so renaming one would orphan previously-
    /// saved state.
    pub fn focus_key(&self) -> &'static str {
        match self.view {
            View::Artists => "artists",
            View::Playlists => "playlists",
            View::Queue => "queue",
        }
    }

    pub fn update_search_results(&mut self) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        let overlay = match overlay {
            Overlay::Search { input, results: _, mut cursor } => {
                let ids = self.run_search(&input);
                let results = ids;
                if cursor >= results.len() {
                    cursor = results.len().saturating_sub(1);
                }
                Overlay::Search { input, results, cursor }

            }
            other => other,
        };
        self.overlay = Some(overlay);
    }
}

// Transport methods take `(&dyn ContextResolver, &Catalog)`. `App` passes
// `self` as the resolver and `&self.catalog` as the catalog; the split-borrow is
// sound because `manual_queue` (the resolver's data source) lives in a distinct
// field from `catalog` and from the `&mut self.transport` we hold.
