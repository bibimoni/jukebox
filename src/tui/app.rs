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
use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode, Transport};

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
    pub now_playing: Option<crate::source::TrackSource>,
    pub dead: HashSet<String>,
    pub switch_sample_rate: bool,
    pub should_quit: bool,
    pub overlay: Option<Overlay>,
    /// Leader-key state for the `gg` mapping (top of column). `g` arms it;
    /// a second `g` within one dispatch consumes it and jumps to row 0.
    pub pending_g: bool,
    /// The active source mode (Local / YouTube / Mixed). Cycled by `M`.
    pub source_mode: crate::mode::SourceMode,
    /// YouTube session (sidecar + auth/cache). `None` when YT is unavailable
    /// (deps missing / spawn failed) — YT features degrade to clean stops.
    pub yt_session: Option<crate::yt::session::Session>,
    /// Autoplay radio cursor for CONT=YouTube.
    pub radio: crate::yt::session::RadioCursor,
    /// CoreAudio re-clock cadence (switch-once-per-YT-session).
    pub device_rate: crate::source::device_rate::DeviceRateState,
}

/// What an id resolves to at load time, after the Local/YouTube/Mixed policy.
enum Resolved {
    Local {
        path: std::path::PathBuf,
        sample_rate_hz: u32,
        bit_depth: u32,
    },
    Remote {
        url: String,
        fmt: crate::source::StreamFormat,
        video_id: String,
    },
}

/// A display view of the now-playing track for the player bar. Carries the
/// `TrackSource` so the bar can render the right quality readout
/// (`24-bit / 96 kHz · bit-perfect` vs `Opus 160k · YT`).
pub struct NowPlayingView {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub source: crate::source::TrackSource,
    pub bit_depth: u32,
    pub sample_rate_hz: u32,
    pub fmt: Option<crate::source::StreamFormat>,
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
    pub fn new(
        catalog: Catalog,
        player: Box<dyn Player>,
        searcher: Option<Searcher>,
        yt_session: Option<crate::yt::session::Session>,
    ) -> Self {
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
            source_mode: crate::mode::SourceMode::default(),
            yt_session,
            radio: crate::yt::session::RadioCursor::new(),
            device_rate: crate::source::device_rate::DeviceRateState::default(),
        }
    }

    fn track_by_id(&self, id: &str) -> Option<&Track> {
        self.catalog.tracks.iter().find(|t| t.id == id)
    }

    /// A display view of the now-playing track, local or remote, for the
    /// player bar. `None` when nothing is playing or the metadata isn't
    /// available yet (a remote track whose `RemoteTrack` isn't cached).
    pub fn now_playing_view(&self) -> Option<NowPlayingView> {
        let ts = self.now_playing.as_ref()?;
        match ts {
            crate::source::TrackSource::Local { track_id } => {
                let t = self.track_by_id(track_id)?;
                Some(NowPlayingView {
                    title: t.title.clone(),
                    artist: t.primary_artist.clone(),
                    album: t.album.clone(),
                    source: ts.clone(),
                    bit_depth: t.bit_depth,
                    sample_rate_hz: t.sample_rate_hz,
                    fmt: None,
                })
            }
            crate::source::TrackSource::Remote { video_id } => {
                let rt = self.yt_session.as_ref()?.track_for(video_id)?;
                Some(NowPlayingView {
                    title: rt.title.clone(),
                    artist: rt.artist.clone(),
                    album: rt.album.clone(),
                    source: ts.clone(),
                    bit_depth: 0,
                    sample_rate_hz: rt.fmt.as_ref().map(|f| f.sample_rate).unwrap_or(48000),
                    fmt: rt.fmt.clone(),
                })
            }
        }
    }

    /// All catalog tracks with the given album title, across all primary_artists,
    /// sorted by (disc, track_number). An album is a cohesive object — browsing
    /// it shows every track on it, not just the ones where the focused artist is
    /// primary (collaboration albums have tracks under several primary_artists).
    pub fn tracks_for_album(&self, album_title: &str) -> Vec<String> {
        let mut idxs: Vec<usize> = self.catalog.tracks.iter().enumerate()
            .filter(|(_, t)| t.album.as_deref() == Some(album_title))
            .map(|(i, _)| i)
            .collect();
        idxs.sort_by_key(|&i| {
            let t = &self.catalog.tracks[i];
            (t.disc_number.unwrap_or(1), t.track_number.unwrap_or(0))
        });
        idxs.into_iter().map(|i| self.catalog.tracks[i].id.clone()).collect()
    }

    /// Build the track-id list for the currently-focused track column.
    /// Clamp every browse cursor to a valid index for its current list. Stale
    /// cursors (e.g. `cursors.album` left at 5 after switching to an artist with
    /// only 2 albums) otherwise make the Tracks column render empty and make
    /// `play_selected` play the wrong/no track — the "this artist has no songs"
    /// and "Enter doesn't play after picking a list" bugs.
    pub fn clamp_cursors(&mut self) {
        let n_artists = self.artists.len();
        if n_artists > 0 && self.cursors.artist >= n_artists {
            self.cursors.artist = n_artists - 1;
        }
        let n_albums = self
            .artists
            .get(self.cursors.artist)
            .and_then(|a| self.albums_by_artist.get(a))
            .map(|v| v.len())
            .unwrap_or(0);
        if n_albums > 0 && self.cursors.album >= n_albums {
            self.cursors.album = n_albums - 1;
        }
        let n_tracks = self.current_context_ids().len();
        if n_tracks > 0 && self.cursors.track >= n_tracks {
            self.cursors.track = n_tracks - 1;
        }
        let n_playlists = self.playlists.len();
        if n_playlists > 0 && self.cursors.playlist >= n_playlists {
            self.cursors.playlist = n_playlists - 1;
        }
    }

    pub fn current_context_ids(&self) -> Vec<String> {
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
                    // The full album across all primary_artists — collaboration
                    // albums have tracks under several artists; the album is a
                    // cohesive object (see `tracks_for_album`).
                    Some(a) => self.tracks_for_album(&a.title),
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

    /// Begin playback at the current transport cursor, skipping dead tracks
    /// and resolving each id through [`resolve_source`] (Local / YouTube /
    /// Mixed policy). Remote ids that fail to resolve are treated as dead.
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
            match self.resolve_source(&id) {
                Some(Resolved::Local { path, sample_rate_hz, bit_depth }) => {
                    if std::fs::metadata(&path).is_err() {
                        self.dead.insert(id.clone());
                        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                        let _ = self.transport.next(&r, &self.catalog);
                        if self.transport.cursor == start {
                            return;
                        }
                        continue;
                    }
                    if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                        &mut self.device_rate,
                        crate::source::device_rate::LoadKind::Local { sample_rate_hz, bit_depth },
                        self.switch_sample_rate,
                    ) {
                        let _ = crate::audio::set_output_format(sr, bd);
                    }
                    let _ = self.player.load(&path);
                    self.now_playing = Some(crate::source::TrackSource::Local { track_id: id });
                    return;
                }
                Some(Resolved::Remote { url, fmt, video_id }) => {
                    if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                        &mut self.device_rate,
                        crate::source::device_rate::LoadKind::Remote { sample_rate: fmt.sample_rate },
                        self.switch_sample_rate,
                    ) {
                        let _ = crate::audio::set_output_format(sr, bd);
                    }
                    // mpv loadfile accepts an https URL via the same path —
                    // PathBuf carries the URL string verbatim.
                    let p = std::path::PathBuf::from(&url);
                    let _ = self.player.load(&p);
                    self.now_playing = Some(crate::source::TrackSource::Remote { video_id });
                    return;
                }
                None => {
                    // Unresolvable (unknown id, or remote resolve failed) → dead.
                    self.dead.insert(id.clone());
                    let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                    let _ = self.transport.next(&r, &self.catalog);
                    if self.transport.cursor == start {
                        return;
                    }
                    continue;
                }
            }
        }
    }

    /// Resolve an opaque id to a playable source under the active mode:
    /// - Local: catalog track → local file; unknown id → None.
    /// - YouTube: sidecar `resolve_url` → stream URL + fmt; no session/error
    ///   → None (degrade to dead).
    /// - Mixed: catalog track present → local; else remote stream.
    fn resolve_source(&mut self, id: &str) -> Option<Resolved> {
        // Local catalog match first (Local + Mixed both prefer it when present).
        if let Some(t) = self.track_by_id(id) {
            // In YouTube mode, catalog tracks are never played locally — only
            // streamed. So only take the local path in Local/Mixed.
            if self.source_mode != crate::mode::SourceMode::Youtube {
                return Some(Resolved::Local {
                    path: t.resolve_source(&self.catalog.source_root),
                    sample_rate_hz: t.sample_rate_hz,
                    bit_depth: t.bit_depth,
                });
            }
        }
        // Remote (YouTube / Mixed-no-local-hit). Needs a live session.
        let session = self.yt_session.as_mut()?;
        match session.resolve_url(id) {
            Ok(u) => Some(Resolved::Remote {
                url: u.url,
                fmt: crate::source::StreamFormat {
                    codec: u.codec,
                    abr: u.abr,
                    sample_rate: u.sample_rate,
                    container: u.container,
                    premium: u.premium,
                },
                video_id: id.to_string(),
            }),
            Err(_) => None, // expired/unavailable/rate-limited → dead path
        }
    }

    /// Load `id` into the player (switching the output device's sample rate
    /// first when sample-rate switching is on). Used by `next`/`prev` after
    /// they've already advanced the transport cursor and returned the id to
    /// play. We load the explicit id rather than re-reading
    /// `transport.current()` because the manual-queue advance path in
    /// `Transport::next` returns a queued id WITHOUT updating `context`/`cursor`
    /// — so `transport.current()` would still point at the just-finished track,
    /// and the player would load the wrong audio (status line correct, no
    /// playback).
    fn load_track(&mut self, id: &str) {
        if self.dead.contains(id) {
            return;
        }
        match self.resolve_source(id) {
            Some(Resolved::Local { path, sample_rate_hz, bit_depth }) => {
                if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                    &mut self.device_rate,
                    crate::source::device_rate::LoadKind::Local { sample_rate_hz, bit_depth },
                    self.switch_sample_rate,
                ) {
                    let _ = crate::audio::set_output_format(sr, bd);
                }
                let _ = self.player.load(&path);
                self.now_playing = Some(crate::source::TrackSource::Local { track_id: id.to_string() });
            }
            Some(Resolved::Remote { url, fmt, video_id }) => {
                if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                    &mut self.device_rate,
                    crate::source::device_rate::LoadKind::Remote { sample_rate: fmt.sample_rate },
                    self.switch_sample_rate,
                ) {
                    let _ = crate::audio::set_output_format(sr, bd);
                }
                let p = std::path::PathBuf::from(&url);
                let _ = self.player.load(&p);
                self.now_playing = Some(crate::source::TrackSource::Remote { video_id });
            }
            None => {
                self.dead.insert(id.to_string());
            }
        }
    }

    /// Play the track under the track-column cursor in the current view.
    pub fn play_selected(&mut self) {
        self.clamp_cursors();
        let ids = self.current_context_ids();
        if ids.is_empty() {
            return;
        }
        let start = match ids.get(self.cursors.track).cloned() {
            Some(s) => s,
            None => return,
        };
        let ctx = self.context_for_current_view(ids);
        // A context switch is a play transition: push the currently-playing
        // (track, context) to history so `prev()` can pop back to it. Only
        // `next()` pushed previously, so a switch (e.g. playing a search result
        // then a track from another context) broke `prev` across the switch.
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
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
        // Mirror `play_selected`: push the current playback to history so a
        // subsequent `prev()` returns to it across the context switch.
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport
            .switch_context(ctx, Some(start), &r, &self.catalog);
        self.start_playback();
    }

    pub fn next(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        if let Some(id) = self.transport.next(&r, &self.catalog) {
            // Load the returned id directly: `Transport::next`'s manual-queue
            // path returns a queued id without updating the cursor, so
            // re-reading `transport.current()` would load the wrong track.
            self.load_track(&id);
        } else {
            // Context exhausted (repeat off, no manual queue). The continue
            // mode decides whether playback stops or auto-advances to more
            // music — this is the "auto discover" feature.
            match self.transport.continue_mode {
                ContinueMode::Off => {
                    self.player.stop().ok();
                    self.now_playing = None;
                }
                ContinueMode::NextAlbum => {
                    if self.switch_to_next_album() {
                        self.start_playback();
                    } else {
                        // Not in an album context, or no next album: stop.
                        self.player.stop().ok();
                        self.now_playing = None;
                    }
                }
                ContinueMode::Radio => {
                    self.switch_to_radio();
                    self.start_playback();
                }
                ContinueMode::YouTube => {
                    // Drive YouTube autoplay via RadioCursor (spec §3.4). Ask
                    // for the next video id seeded by the just-finished track;
                    // switch context to a fresh radio Search + load it.
                    let seed_id = self.now_playing.clone().map(|s| s.id().to_string());
                    if let Some(session) = self.yt_session.as_mut() {
                        match self.radio.advance(session as &mut dyn crate::yt::session::YtClient, seed_id) {
                            Some(vid) => {
                                if let Some(np) = self.now_playing.clone() {
                                    self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
                                }
                                let ctx = Context::Search {
                                    query: "youtube radio".into(),
                                    track_ids: vec![vid.clone()],
                                };
                                let r = ClonedResolver {
                                    playlists: &self.playlists,
                                    manual_queue: self.transport.manual_queue.clone(),
                                };
                                self.transport.switch_context(ctx, Some(&vid), &r, &self.catalog);
                                self.start_playback();
                            }
                            None => {
                                self.player.stop().ok();
                                self.now_playing = None;
                            }
                        }
                    } else {
                        // No session — stop cleanly (degrade, spec §3.5).
                        self.player.stop().ok();
                        self.now_playing = None;
                    }
                }
            }
        }
    }

    pub fn prev(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        if let Some(id) = self.transport.prev(&r, &self.catalog) {
            self.load_track(&id);
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

    /// Cycle the continue mode, mode-dependent (spec §2 ContinueMode::YouTube):
    /// - Local:  Off → NextAlbum → Radio → Off
    /// - YouTube: Off → YouTube → Off
    /// - Mixed:  Off → NextAlbum → YouTube → Off
    pub fn cycle_continue(&mut self) {
        self.transport.continue_mode = match (self.source_mode, self.transport.continue_mode) {
            (crate::mode::SourceMode::Local, ContinueMode::Off) => ContinueMode::NextAlbum,
            (crate::mode::SourceMode::Local, ContinueMode::NextAlbum) => ContinueMode::Radio,
            (crate::mode::SourceMode::Local, ContinueMode::Radio) => ContinueMode::Off,
            (crate::mode::SourceMode::Local, ContinueMode::YouTube) => ContinueMode::Off,

            (crate::mode::SourceMode::Youtube, ContinueMode::Off) => ContinueMode::YouTube,
            (crate::mode::SourceMode::Youtube, _) => ContinueMode::Off,

            (crate::mode::SourceMode::Mixed, ContinueMode::Off) => ContinueMode::NextAlbum,
            (crate::mode::SourceMode::Mixed, ContinueMode::NextAlbum) => ContinueMode::YouTube,
            (crate::mode::SourceMode::Mixed, ContinueMode::YouTube) => ContinueMode::Off,
            (crate::mode::SourceMode::Mixed, ContinueMode::Radio) => ContinueMode::Off,
        };
    }

    /// Cycle the source mode Local → YouTube → Mixed → Local. Never stops
    /// playback — only changes where new browsing happens and which CONT
    /// engine is eligible.
    pub fn cycle_mode(&mut self) {
        self.source_mode = self.source_mode.cycle();
    }

    /// Auto-continue to the next album by the same artist. Pushes the current
    /// (track, context) to history first so `prev` can return. Returns false
    /// if the current context isn't an album or there's no next album.
    fn switch_to_next_album(&mut self) -> bool {
        let (artist, album) = match &self.transport.context {
            Context::Album { artist, album, .. } => (artist.clone(), album.clone()),
            _ => return false, // NextAlbum only applies to album contexts.
        };
        let albums = match self.albums_by_artist.get(&artist) {
            Some(a) => a.clone(),
            None => return false,
        };
        let cur_idx = albums.iter().position(|a| a.title == album);
        let next_idx = match cur_idx {
            Some(i) if i + 1 < albums.len() => i + 1,
            _ => return false, // no next album by this artist
        };
        let next = &albums[next_idx];
        let track_ids = self.tracks_for_album(&next.title);
        if track_ids.is_empty() {
            return false;
        }
        // A context switch is a play transition: push current to history so
        // `prev` can pop back to the track that just finished.
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
        let ctx = Context::Album {
            album: next.title.clone(),
            artist: next.artist.clone(),
            track_ids,
        };
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport.switch_context(ctx, None, &r, &self.catalog);
        // Keep the browse cursor in sync so the UI shows the new album.
        self.cursors.album = next_idx;
        self.cursors.track = 0;
        true
    }

    /// Auto-continue with the whole library as a shuffled "radio" context.
    /// Music never stops — when this context eventually exhausts (the entire
    /// library), `next` re-enters here and rebuilds it.
    fn switch_to_radio(&mut self) {
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
        let all_ids: Vec<String> = self.catalog.tracks.iter().map(|t| t.id.clone()).collect();
        let ctx = Context::Search { query: "radio".into(), track_ids: all_ids };
        // Radio implies shuffled play; force smart shuffle so it actually
        // discovers (catalog order would just be sequential).
        self.transport.shuffle = ShuffleMode::Smart;
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
        self.transport.switch_context(ctx, None, &r, &self.catalog);
    }

    pub fn volume_up(&mut self) {
        self.volume = self.volume.saturating_add(5).min(100);
        self.muted = false;
        let _ = self.player.set_volume(self.volume);
        let _ = self.player.set_muted(self.muted);
    }

    pub fn volume_down(&mut self) {
        self.volume = self.volume.saturating_sub(5);
        let _ = self.player.set_volume(self.volume);
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        let _ = self.player.set_muted(self.muted);
    }

    /// Set volume to an absolute 0..=100 value (used by mouse clicks/drags on
    /// the volume meter). Pushes to the player immediately so the audio
    /// matches the on-screen bar — without this, the mouse path mutated
    /// `volume` directly and mpv stayed at the old level until a keypress
    /// re-synced (the "mouse resets to 100% but audio unchanged" bug).
    pub fn set_volume(&mut self, vol: u8) {
        self.volume = vol.min(100);
        self.muted = false;
        let _ = self.player.set_volume(self.volume);
        let _ = self.player.set_muted(self.muted);
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

    /// If the active overlay is `Search`, re-run the search against its current
    /// `input` and replace `results` with the fresh id list, clamping `cursor`
    /// to the new result count. No-op for non-search overlays.
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
