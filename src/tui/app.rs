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
    Youtube,
}

/// A YouTube playlist/mood list shown in the Y view. `track_ids` are the
/// video_ids of the list's tracks, fetched lazily (via the sidecar) when the
/// user focuses the list.
#[derive(Clone, Default)]
pub struct YtList {
    pub id: String,
    pub name: String,
    pub kind: YtListKind,
    pub track_ids: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum YtListKind {
    #[default]
    Account,
    Suggested,
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

/// Which index the `/` search overlay queries. `Local` is the on-disk BM25 index
/// (instant, live-as-you-type). `Youtube` is ytmusicapi search over the sidecar
/// (slow, so it's explicit-submit on Enter, not per keystroke). `Tab` cycles.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SearchScope {
    #[default]
    Local,
    Youtube,
}

impl SearchScope {
    pub fn as_str(self) -> &'static str {
        match self {
            SearchScope::Local => "local",
            SearchScope::Youtube => "youtube",
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
        /// Which index the query runs against. Defaults from the active view on
        /// open (Youtube in the Y view, Local elsewhere); `Tab` toggles so the
        /// user can search the local catalog from the Y view or YouTube from a
        /// local view.
        scope: SearchScope,
        /// The query that produced `results` (None = never submitted). For
        /// Local scope this tracks `input` live; for Youtube it's set on
        /// Enter-submit, so a second Enter on fresh results picks the track
        /// instead of re-searching.
        submitted: Option<String>,
        /// True while a Youtube search request is in flight (set on Enter,
        /// cleared when the response lands in `on_tick`). Drives the
        /// "searching…" indicator.
        searching: bool,
    },
    Help,
    PlaylistPicker,
    Command {
        input: String,
    },
    /// YouTube cookie-paste overlay (spec §5.7). `input` accumulates the pasted
    /// Netscape cookies.txt content; `Enter` saves, `Esc` cancels.
    YtAuth {
        input: String,
    },
    /// Suggested albums / playlists to start from (`S`). Local mode lists local
    /// albums; YouTube/Mixed can list YT mood playlists.
    Discover {
        items: Vec<DiscoverItem>,
        cursor: usize,
    },
}

/// A discover-overlay suggestion.
#[derive(Clone)]
pub enum DiscoverItem {
    /// A local catalog album (Local / Mixed).
    Album { artist: String, album: String },
    /// A YouTube mood/suggested playlist (YouTube / Mixed).
    Playlist { id: String, name: String },
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
    /// YouTube lists for the Y view (account playlists + suggested/mood).
    /// Empty until `refresh_yt_lists` populates them.
    pub yt_lists: Vec<YtList>,
    pub yt_lists_loading: bool,
    /// Playlist ids whose tracks have been fetched (even if empty), so a
    /// genuinely-empty list isn't re-fetched every tick while focused.
    pub loaded_yt_lists: HashSet<String>,
    pub yt_error: Option<String>,
    /// Transient status message (e.g. "YT auth: connected via chrome"), shown
    /// in the footer until overwritten/cleared. Success counterpart to
    /// `yt_error`.
    pub yt_status: Option<String>,
    /// `python3` path + the sidecar script path, used to (re)spawn the sidecar
    /// when cookies change via `:yt auth`. Set by `main.rs`; defaults to
    /// `python3` + the manifest-dir script (works in dev).
    pub yt_python: std::path::PathBuf,
    pub yt_script: std::path::PathBuf,
    /// The browser profile the sidecar reads YouTube cookies from ("chrome",
    /// etc.), or empty for guest / pasted-cookies. Source of truth for auth:
    /// set by `:yt auth browser <name>`, cleared by `:yt logout`/`:yt auth`,
    /// restored at startup from `LayoutState.yt_browser`, and saved on exit so
    /// you don't re-auth every launch.
    pub yt_browser: String,
    /// Active inline filter (`f`) on the focused column, or `None`.
    pub filter: Option<FilterState>,
    /// Vertical scroll offset for the `?` Help overlay. The keymap is taller
    /// than a typical popup, so the overlay scrolls with j/k/↑/↓/PgUp/PgDn/g/G.
    pub help_scroll: u16,
    /// Braille spinner frame index (0..10) for the "a resolve is in flight"
    /// indicator in the player bar. Advanced in `on_tick` only while resolving,
    /// reset to 0 otherwise so the glyph returns to play/pause.
    pub spinner_frame: u8,
    /// True when the currently-playing remote stream is the premium (256k) URL,
    /// false while it's the fast (129k) one. Gates the progressive upgrade: a
    /// premium URL landing mid-play swaps the stream up to 256k only once.
    pub playing_premium: bool,
    /// The video_id of a cold-miss YouTube pick whose URL hasn't landed yet.
    /// Set by `start_playback`/`load_track` on a `Resolved::Pending`; `on_tick`
    /// swaps the player in (and clears this) the moment `url_for` returns it,
    /// or clears it if the fast resolve finishes without a URL. Single slot:
    /// a new pick replaces it. The old track keeps playing until the swap.
    pub pending_play: Option<String>,
}

/// Inline filter state for the `f` filter-on-focused-column (spec §5.4).
/// `col` is the focus_col the filter was opened on; `text` is the query.
#[derive(Clone, Default)]
pub struct FilterState {
    pub col: usize,
    pub text: String,
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
    /// A YouTube id whose stream URL isn't cached yet (cold miss). The resolve
    /// is fire-and-forget; `on_tick` swaps the player in once the URL lands.
    /// The old track keeps playing (or nothing plays, on a cold start) and the
    /// spinner signals the pending swap — the UI never blocks on the ~1.3s
    /// resolve. `App::pending_play` carries the id between this call and the
    /// `on_tick` swap.
    Pending {
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
    yt_lists: &'a [YtList],
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
    fn yt_playlist_ids(&self, key: &str) -> Vec<String> {
        self.yt_lists
            .iter()
            .find(|l| l.id == key)
            .map(|l| l.track_ids.clone())
            .unwrap_or_default()
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
            yt_lists: Vec::new(),
            yt_lists_loading: false,
            yt_error: None,
            yt_status: None,
            yt_python: std::path::PathBuf::from("python3"),
            yt_script: std::path::PathBuf::from("scripts/yt/yt.py"),
            filter: None,
            help_scroll: 0,
            spinner_frame: 0,
            playing_premium: false,
            pending_play: None,
            loaded_yt_lists: HashSet::new(),
            yt_browser: String::new(),
        }
    }

    fn track_by_id(&self, id: &str) -> Option<&Track> {
        self.catalog.tracks.iter().find(|t| t.id == id)
    }

    /// A display view of the now-playing track, local or remote, for the
    /// player bar. `None` when nothing is playing or the metadata isn't
    /// available yet (a remote track whose `RemoteTrack` isn't cached).
    /// True while a YouTube resolve is in flight (fast or premium). Drives the
    /// braille spinner in the player bar — a global "the code is working"
    /// signal, so it spins during the CONT-radio premium preload of the next
    /// track and the progressive-upgrade resolve, not just the current track.
    pub fn is_resolving(&self) -> bool {
        self.yt_session
            .as_ref()
            .map(|s| s.resolve_busy() || s.premium_resolve_busy())
            .unwrap_or(false)
    }

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
            View::Youtube => self
                .yt_lists
                .get(self.cursors.playlist)
                .map(|l| l.track_ids.clone())
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
            View::Youtube => {
                let l = self.yt_lists.get(self.cursors.playlist).cloned();
                Context::Youtube {
                    key: l.as_ref().map(|l| l.id.clone()).unwrap_or_default(),
                    name: l.as_ref().map(|l| l.name.clone()).unwrap_or_default(),
                }
            }
            View::Queue => Context::Queue,
        }
    }

    /// Begin playback at the current transport cursor, skipping dead tracks
    /// and resolving each id through [`resolve_source`] (Local / YouTube /
    /// Mixed policy). Remote ids that fail to resolve are treated as dead.
    fn start_playback(&mut self) {
        // A fresh play intent owns the pending slot: clear any stale cold-miss
        // swap from a prior pick. (A Pending arm below may set it again.)
        self.pending_play = None;
        let n = self.transport.order.len();
        if n == 0 {
            return;
        }
        let start = self.transport.cursor;
        for _ in 0..n.max(1) {
            let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
            let id = match self.transport.current(&r, &self.catalog) {
                Some(id) => id,
                None => return,
            };
            drop(r);
            if self.dead.contains(&id) {
                let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
                        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
                    self.preload_next_url();
                    return;
                }
                Some(Resolved::Remote { url, fmt, video_id }) => {
                    // Cached URL (fast or premium) → swap in immediately.
                    self.load_remote(url, fmt, video_id);
                    return;
                }
                Some(Resolved::Pending { video_id }) => {
                    // Cold miss: the URL isn't cached yet. Don't block — keep
                    // the old track playing (or nothing, on a cold start), set
                    // the pending slot, and let on_tick swap the player in the
                    // moment the URL lands. resolve_source already armed both
                    // resolve tiers fire-and-forget.
                    self.pending_play = Some(video_id);
                    return;
                }
                None => {
                    // Genuinely unresolvable (unknown local id) → dead. Remote
                    // cold misses return Pending (handled above), NOT None, so
                    // they aren't dead-marked.
                    self.dead.insert(id.clone());
                    let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
        // Remote (YouTube / Mixed-no-local-hit). Two paths:
        //   - cached (url_for prefers premium → fast): instant, no block.
        //   - cache miss: fire-and-forget BOTH tiers (fast ~1.3s + premium
        //     ~10-15s) and return Pending. on_tick swaps the player in the
        //     moment the fast URL lands, then the progressive-upgrade path
        //     swaps up to 256k once the premium URL lands. This is fully
        //     non-blocking — the old track keeps playing and the spinner
        //     signals the swap, so a cold miss never freezes the UI.
        let session = self.yt_session.as_mut()?;
        if let Some(url) = session.url_for(id) {
            // The url_cache entry's own fmt is the tier source of truth (premium
            // wins when present). track_cache's fmt can lag — a track cached by
            // search BEFORE its premium resolve lands has fmt=None there even
            // though url_for returned the premium URL — so prefer the cache
            // entry's fmt and only fall back to track_cache's.
            let fmt = session
                .cache_fmt_for(id)
                .or_else(|| session.track_for(id).and_then(|t| t.fmt.clone()))
                .unwrap_or_else(|| crate::source::StreamFormat {
                    codec: "AAC".into(),
                    abr: 0,
                    sample_rate: 48000,
                    container: "m4a".into(),
                    premium: false,
                });
            return Some(Resolved::Remote { url, fmt, video_id: id.to_string() });
        }
        // Cache miss → arm both tiers fire-and-forget and defer the swap to
        // on_tick (Pending). Guards in send_resolve/send_resolve_premium make
        // re-arming a no-op if a tier is already in flight or cached.
        let _ = session.send_resolve(id.to_string());
        let _ = session.send_resolve_premium(id.to_string());
        Some(Resolved::Pending { video_id: id.to_string() })
    }

    /// Pre-resolve the next track's stream URL (fire-and-forget) so gapless
    /// handoff has it ready. Called after a track starts + on tick.
    fn preload_next_url(&mut self) {
        // Resolve the next id + whether it'll stream BEFORE the session borrow,
        // so we don't hold an immutable borrow of self across the mutable one.
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        let next_id = self.transport.peek_next(&r, &self.catalog);
        let Some(id) = next_id else { return };
        // Only pre-resolve ids we'll actually stream (not local catalog hits in
        // Local/Mixed — those play from disk).
        let will_stream = self.source_mode == crate::mode::SourceMode::Youtube
            || self.track_by_id(&id).is_none();
        if !will_stream {
            return;
        }
        let Some(session) = self.yt_session.as_mut() else { return };
        // Pre-resolve the PREMIUM (256k) URL ahead of time — it's slow (~10-15s
        // cold, with the EJS nsig solver) but happens during the current track,
        // so the next track starts instantly at Premium quality (gapless). The
        // fast (129k) URL is only fetched at play time on a cache miss, as the
        // instant-start fallback. Guard on the premium inflight, not the fast
        // one, so a concurrent fast sync doesn't suppress the premium preload.
        if session.premium_resolve_busy() {
            return;
        }
        let _ = session.send_resolve_premium(id);
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
        // A fresh explicit load (next/prev) owns the pending slot.
        self.pending_play = None;
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
                // Cached URL → swap in immediately.
                self.load_remote(url, fmt, video_id);
            }
            Some(Resolved::Pending { video_id }) => {
                // Cold miss: keep the previous track playing (load_track is a
                // single next/prev load, no loop-advance) and let on_tick swap
                // once the URL lands. resolve_source already armed both tiers.
                self.pending_play = Some(video_id);
            }
            None => {
                // Genuinely unresolvable (unknown local id). Remote cold misses
                // return Pending (handled above), not None.
                self.dead.insert(id.to_string());
            }
        }
    }

    /// Swap the player to a cached YouTube stream URL (device-rate switch →
    /// load → set now_playing + playing_premium → preload the next track's
    /// premium URL). Shared by the cached `Resolved::Remote` path in
    /// `start_playback`/`load_track` and the `on_tick` cold-miss swap. mpv
    /// loadfile accepts an https URL via the same path — PathBuf carries the
    /// URL string verbatim.
    fn load_remote(&mut self, url: String, fmt: crate::source::StreamFormat, video_id: String) {
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
        // Record whether we started at premium (256k) so a later premium URL
        // landing mid-play swaps only if we're not already premium
        // (progressive upgrade guard).
        self.playing_premium = fmt.premium;
        self.preload_next_url();
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
        // A fresh context starts with a clean dead-set: a transient resolve
        // failure earlier (network blip, sidecar hiccup) must not permanently
        // blacklist a track for the whole session. Genuinely-missing local
        // files re-add themselves on this pass if still missing.
        self.dead.clear();
        // A context switch is a play transition: push the currently-playing
        // (track, context) to history so `prev()` can pop back to it. Only
        // `next()` pushed previously, so a switch (e.g. playing a search result
        // then a track from another context) broke `prev` across the switch.
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
        // Mirror `play_selected`: clear the dead-set on a context switch, then
        // push the current playback to history so a subsequent `prev()` returns.
        self.dead.clear();
        if let Some(np) = self.now_playing.clone() {
            self.transport.history.push((np.id().to_string(), self.transport.context.clone()));
        }
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
        self.transport
            .switch_context(ctx, Some(start), &r, &self.catalog);
        self.start_playback();
    }

    pub fn next(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
                                    yt_lists: &self.yt_lists,
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
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
        self.transport.set_shuffle(m, &r, &self.catalog);
    }

    pub fn reshuffle(&mut self) {
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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

    /// Apply pasted YouTube cookies (from the `:yt auth` overlay). Spawns a
    /// session if none, persists the cookies, and respawns the sidecar with
    /// them. Best-effort: on failure sets `yt_error` so the Y view surfaces it.
    pub fn apply_yt_auth(&mut self, cookies: String) {
        self.yt_error = None;
        // Pasted cookies are a distinct auth path from the browser profile;
        // clear the saved browser so the next launch doesn't try to read a
        // browser profile the user abandoned.
        self.yt_browser.clear();
        if self.yt_session.is_none() {
            match crate::yt::session::Session::spawn(&self.yt_python, &self.yt_script, Some(cookies.clone())) {
                Ok(s) => self.yt_session = Some(s),
                Err(e) => {
                    self.yt_error = Some(format!("auth failed: {e}"));
                    return;
                }
            }
        } else if let Some(session) = self.yt_session.as_mut() {
            if let Err(e) = session.set_cookies(cookies, &self.yt_python, &self.yt_script) {
                self.yt_error = Some(format!("auth failed: {e}"));
                return;
            }
        }
        self.yt_status = Some("YT auth: connected via pasted cookies".into());
    }

    /// `:yt auth browser <name>` — respawn the sidecar reading cookies from a
    /// browser profile (chrome/firefox/safari/edge/brave). No cookie file is
    /// written; the values stay in the browser. The preferred auth path: no
    /// credentials ever enter the conversation or a paste buffer.
    pub fn apply_yt_browser(&mut self, browser: String) {
        self.yt_error = None;
        // Remember the choice so the next launch auto-connects from the same
        // browser profile (no re-auth). Saved to state.db on clean exit.
        self.yt_browser = browser.clone();
        if self.yt_session.is_none() {
            match crate::yt::session::Session::spawn_browser(&self.yt_python, &self.yt_script, browser.clone()) {
                Ok(s) => self.yt_session = Some(s),
                Err(e) => {
                    self.yt_error = Some(format!("auth failed: {e}"));
                    return;
                }
            }
        } else if let Some(session) = self.yt_session.as_mut() {
            if let Err(e) = session.set_browser(browser.clone(), &self.yt_python, &self.yt_script) {
                self.yt_error = Some(format!("auth failed: {e}"));
                return;
            }
        }
        self.yt_status = Some(format!("YT auth: connected via {browser}"));
    }

    /// `:yt setup` — create the jukebox venv and install the YT deps into it,
    /// so the sidecar runs against a python that has them. Blocks (~30s,
    /// one-time). On success, respawn the sidecar against the new venv python.
    pub fn yt_setup(&mut self) {
        self.yt_error = None;
        self.yt_status = Some("YT setup: installing deps…".into());
        let reqs = self.yt_script.parent().map(|p| p.join("requirements.txt"));
        let Some(reqs) = reqs else {
            self.yt_error = Some("setup: could not find requirements.txt".into());
            self.yt_status = None;
            return;
        };
        match crate::yt::session::run_setup(&reqs) {
            Ok(msg) => {
                self.yt_python = crate::yt::session::venv_python();
                self.yt_status = Some(msg);
                // Respawn the sidecar against the new venv python, preserving
                // any browser/pasted auth.
                if let Some(session) = self.yt_session.as_mut() {
                    if let Some(browser) = session.browser.clone() {
                        match crate::yt::session::Session::spawn_browser(&self.yt_python, &self.yt_script, browser) {
                            Ok(new) => { *self.yt_session.as_mut().unwrap() = new; }
                            Err(e) => self.yt_error = Some(format!("respawn after setup: {e}")),
                        }
                    }
                }
            }
            Err(e) => {
                self.yt_error = Some(format!("setup failed: {e}"));
                self.yt_status = None;
            }
        }
    }

    /// Per-tick housekeeping (called from the event loop's poll):
    /// - auto-respawn a crashed sidecar once (so a mid-session sidecar death
    ///   doesn't permanently blackhole every subsequent remote id);
    /// - apply drained async responses (refresh lists → yt_lists, pre-resolve
    ///   → url_cache; Search/Tracks caching happens in Session::apply_pair).
    pub fn on_tick(&mut self) {
        // Auto-respawn a dead sidecar (best-effort, once per tick). Preserves
        // the browser/pasted auth; local playback is unaffected either way.
        if let Some(session) = self.yt_session.as_mut() {
            if session.is_alive() {
                session.mark_alive();
            } else if session.should_respawn() {
                // Backoff-gated auto-respawn (≤3 attempts, ≥5s apart) so a
                // sidecar that dies on spawn (bad cookies, missing deps) isn't
                // respawned every tick into a tight loop.
                session.note_respawn();
                let attempts = session.respawn_attempts;
                // `yt_browser` is the source of truth (set by `:yt auth browser`,
                // restored at startup). Fall back to the session's in-memory
                // browser if the field was unset (e.g. session created before
                // the field existed), then to pasted cookies, then guest.
                let browser = if !self.yt_browser.is_empty() {
                    Some(self.yt_browser.clone())
                } else {
                    session.browser.clone()
                };
                let respawned = match browser {
                    Some(b) => crate::yt::session::Session::spawn_browser(
                        &self.yt_python, &self.yt_script, b,
                    ),
                    None => {
                        // Guest/pasted-cookies: respawn guest (pasted cookies
                        // file re-loaded by Session::spawn).
                        let cookies = crate::yt::session::load_cookies();
                        crate::yt::session::Session::spawn(
                            &self.yt_python, &self.yt_script, cookies,
                        )
                    }
                };
                match respawned {
                    Ok(new) => {
                        *self.yt_session.as_mut().unwrap() = new;
                        self.yt_status = Some("YT: sidecar restarted".into());
                    }
                    Err(e) => {
                        self.yt_error = Some(format!("sidecar respawn ({attempts}/3): {e}"));
                        if attempts >= 3 {
                            self.yt_status =
                                Some("YT: sidecar keeps dying — run :yt setup / :yt auth".into());
                        }
                    }
                }
            }
        }

        // Drain + apply async responses. Session::apply_pair already cached
        // tracks/URLs; here we also fold the fetched lists into yt_lists. Take
        // the premium-swap signal out of the session here too — it's processed
        // AFTER the session borrow ends (it needs &mut self for the player).
        let mut premium_swap: Option<(String, crate::yt::proto::ResolvedUrl)> = None;
        // A cold-miss pick whose URL just landed. Staged here (needs &mut self
        // for the player) and swapped in below, after the session borrow ends.
        // (video_id, url, fmt).
        let mut pending_swap: Option<(String, String, crate::source::StreamFormat)> = None;
        // True if the pending fast resolve finished WITHOUT a URL (error / no
        // audio) → give up the cold-miss swap (the error already surfaced via
        // pending_errors). Don't wait the ~10s for the premium tier on a "play
        // now" miss.
        let mut pending_give_up = false;
        if let Some(session) = self.yt_session.as_mut() {
            session.drain_paired();
            // Pull any lists the session buffered for us.
            let got_playlists = session.pending_playlists.take();
            let got_suggestions = session.pending_suggestions.take();
            // Merge once we have at least the playlists (suggestions optional).
            if let Some(p) = got_playlists {
                let mut lists: Vec<YtList> = p
                    .into_iter()
                    .map(|pl| YtList {
                        id: pl.id,
                        name: pl.name,
                        kind: YtListKind::Account,
                        track_ids: Vec::new(),
                    })
                    .collect();
                if let Some(s) = got_suggestions {
                    lists.extend(s.into_iter().map(|pl| YtList {
                        id: pl.id,
                        name: pl.name,
                        kind: YtListKind::Suggested,
                        track_ids: Vec::new(),
                    }));
                }
                self.yt_lists = lists;
                self.yt_lists_loading = false;
                // Lists were replaced; forget which had been expanded so a
                // re-focused list re-fetches its tracks.
                self.loaded_yt_lists.clear();
            }

            // Fold a fetched playlist's tracks into the matching YtList. The
            // session paired the response with the list id we requested.
            if let Some((id, vids)) = session.pending_tracks.take() {
                // Re-arming the inflight guard lets a re-focus re-fetch only
                // if the list genuinely changed (not every tick on an empty
                // result). Mark the list loaded either way.
                for l in self.yt_lists.iter_mut() {
                    if l.id == id {
                        l.track_ids = vids.clone();
                    }
                }
                self.loaded_yt_lists.insert(id);
            }

            // Fold a completed YouTube search into the open search overlay.
            // Only applies if the overlay is still a Youtube-scope Search whose
            // submitted query matches — a stale response (user typed more /
            // closed the overlay) is dropped (the tracks are still cached).
            if let Some((q, vids)) = session.pending_search.take() {
                // Only touch the overlay if it's actually a Search — a stale
                // search response landing while a non-Search overlay (Help /
                // PlaylistPicker / Command / YtAuth / Discover) is open must
                // NOT drop that overlay. Clone + match (don't `take`) so a
                // non-Search overlay is left untouched; a Search that isn't
                // ours (different query / not searching / local scope) is also
                // left as-is (the tracks are still cached in track_cache).
                if let Some(Overlay::Search {
                    input,
                    results: _,
                    cursor,
                    scope,
                    submitted,
                    searching,
                }) = self.overlay.clone()
                {
                    if scope == crate::tui::app::SearchScope::Youtube
                        && submitted.as_deref() == Some(q.as_str())
                        && searching
                    {
                        let results = vids;
                        let mut cursor = cursor;
                        if !results.is_empty() && cursor >= results.len() {
                            cursor = results.len().saturating_sub(1);
                        }
                        self.overlay = Some(Overlay::Search {
                            input, results, cursor, scope, submitted, searching: false,
                        });
                    }
                    // else: not ours — leave the overlay exactly as it was.
                }
            }

            // A sidecar error frees any "searching…/loading…" overlay state and
            // surfaces the message in the footer (the sidecar's stderr is
            // null'd, so this is the only error path). Without this a failed
            // search wedged: the inflight guard never cleared, so every later
            // Enter no-oped and the overlay stayed on "searching…" forever.
            // Drain ALL staged errors (a Vec, so none is dropped even when two
            // Search errors for different queries land in one cycle). For the
            // search overlay: clear its `searching` flag ONLY when a Search
            // error's query matches the overlay's `submitted` query (so an
            // error for an abandoned prior query doesn't drop the current
            // query's results, and the current query's own error DOES clear
            // it so the overlay exits "searching…"). Surface the most relevant
            // message in the footer: prefer the Search error matching the
            // overlay's query, else the last error.
            let errors = std::mem::take(&mut session.pending_errors);
            if !errors.is_empty() {
                let overlay_q = match &self.overlay {
                    Some(Overlay::Search { submitted, .. }) => submitted.clone(),
                    _ => None,
                };
                // Find the Search error matching the overlay's query (if any),
                // and the last error overall (for the footer fallback).
                let mut matching_search: Option<&(crate::yt::session::ErrorScope, String)> = None;
                let mut last: Option<&(crate::yt::session::ErrorScope, String)> = None;
                for er in &errors {
                    if let (Some(q), crate::yt::session::ErrorScope::Search(err_q)) =
                        (overlay_q.as_deref(), &er.0)
                    {
                        if q == err_q.as_str() {
                            matching_search = Some(er);
                        }
                    }
                    last = Some(er);
                }
                // Clear the overlay's searching flag if the matching Search
                // error was found (clone + match, never take, so a non-Search
                // overlay or a Search for a different query is left untouched).
                if let Some((crate::yt::session::ErrorScope::Search(_), _)) = matching_search {
                    if let Some(Overlay::Search {
                        input,
                        results,
                        cursor,
                        scope,
                        submitted,
                        searching: _,
                    }) = self.overlay.clone()
                    {
                        self.overlay = Some(Overlay::Search {
                            input, results, cursor, scope, submitted, searching: false,
                        });
                    }
                }
                // Footer: prefer the matching Search error's message (most
                // relevant to what the user searched), else the last error.
                let footer = matching_search.or(last).map(|(_, e)| e.clone());
                if let Some(e) = footer {
                    self.yt_error = Some(e);
                }
            }

            // A premium (256k) URL landed for a fire-and-forget premium
            // resolve. Take it out of the session here (we need &mut self for
            // the player to swap) and process it below, after this block.
            premium_swap = session.pending_premium_url.take();

            // Cold-miss swap: if a pick is pending and its URL just landed,
            // stage it for the player swap below. If the fast resolve finished
            // with no URL (error), give up so the user isn't stuck on the
            // spinner waiting for the slow premium tier. url_for prefers
            // premium→fast, so a premium URL landing first also satisfies this.
            if let Some(id) = self.pending_play.clone() {
                if let Some(url) = session.url_for(&id) {
                    let fmt = session
                        .cache_fmt_for(&id)
                        .or_else(|| session.track_for(&id).and_then(|t| t.fmt.clone()))
                        .unwrap_or_else(|| crate::source::StreamFormat {
                            codec: "AAC".into(),
                            abr: 0,
                            sample_rate: 48000,
                            container: "m4a".into(),
                            premium: false,
                        });
                    pending_swap = Some((id, url, fmt));
                } else if !session.resolve_busy() {
                    // Fast resolve done, no URL of either tier yet → it failed.
                    pending_give_up = true;
                }
            }
        }

        // Progressive upgrade: a premium URL landed while we're playing the
        // FAST (129k) stream of the SAME track. Swap the player up to 256k +
        // resume at the current position. Guards:
        //   - same track (premium vid == now_playing vid; else the user moved
        //     on and the URL just caches for later),
        //   - not near end (swapping in the last few seconds is pointless and
        //     can race the natural end-of-track advance),
        //   - not already premium (avoid a redundant reload that restarts
        //     audio).
        // Sample rate: 129k and 256k AAC are the same rate (44100/48000), so
        // desired_switch returns None (no CoreAudio re-clock) — the
        // once-per-session re-clock invariant is preserved. We still route
        // through desired_switch rather than calling set_output_format
        // directly, so a hypothetical rate change is handled safely.
        if let Some((vid, u)) = premium_swap {
            let same_track = matches!(
                &self.now_playing,
                Some(crate::source::TrackSource::Remote { video_id }) if video_id == &vid
            );
            if same_track && !self.playing_premium {
                let pos = self.player.position().unwrap_or(0.0);
                let dur = self.player.duration().unwrap_or(f64::MAX);
                let near_end = dur.is_finite() && dur - pos < 5.0;
                if !near_end {
                    if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                        &mut self.device_rate,
                        crate::source::device_rate::LoadKind::Remote { sample_rate: u.sample_rate },
                        self.switch_sample_rate,
                    ) {
                        let _ = crate::audio::set_output_format(sr, bd);
                    }
                    let p = std::path::PathBuf::from(&u.url);
                    // Resume at the captured position via mpv's `start` option
                    // (load_at) so the premium stream begins at `pos` directly —
                    // no from-0 replay before a seek lands.
                    let _ = self.player.load_at(&p, pos);
                    self.playing_premium = true;
                    self.yt_status = Some("upgraded to AAC 256k · YT Premium".into());
                }
            }
        }

        // Cold-miss swap: the URL for a pending pick just landed — swap the
        // player from the old track to it. (If the old track was still playing,
        // this is where it stops; that's the intended "play this now" switch.)
        if let Some((id, url, fmt)) = pending_swap {
            self.pending_play = None;
            self.load_remote(url, fmt, id);
        } else if pending_give_up {
            // Fast resolve finished without a URL — drop the pending intent.
            // The error already surfaced to yt_error via pending_errors above;
            // the old track either kept playing or advanced on its own.
            self.pending_play = None;
        }

        // Lazy-load the focused YT list's tracks: the Y view's col2 + Enter/s
        // need them, but they're only fetched on demand (spec §5.3). Skip
        // lists already loaded (even if empty) and any fetch in flight.
        if self.view == View::Youtube {
            if let Some(l) = self.yt_lists.get(self.cursors.playlist).cloned() {
                let id = l.id.clone();
                let empty = l.track_ids.is_empty();
                let loaded = self.loaded_yt_lists.contains(&id);
                let inflight = self
                    .yt_session
                    .as_ref()
                    .map(|s| s.playlist_loading(&id))
                    .unwrap_or(false);
                if empty && !loaded && !inflight {
                    if let Some(session) = self.yt_session.as_mut() {
                        let _ = session.send_get_playlist(id);
                    }
                }
            }
        }

        // Keep pre-resolving the next track's PREMIUM url so gapless handoff
        // stays warm (the slow 256k resolve happens during the current track).
        self.preload_next_url();

        // Braille spinner: advance one frame per tick while a resolve is in
        // flight, else freeze at 0 (returns the glyph to play/pause). ~150ms
        // per tick (event loop POLL_TIMEOUT) ≈ 6.7fps — smooth for 10 frames.
        if self.is_resolving() {
            self.spinner_frame = (self.spinner_frame + 1) % 10;
        } else {
            self.spinner_frame = 0;
        }
    }

    /// Logout: clear cookies + browser choice + respawn the sidecar guest.
    pub fn yt_logout(&mut self) {
        let p = crate::yt::session::cookies_file();
        let _ = std::fs::remove_file(&p);
        self.yt_browser.clear();
        if let Some(session) = self.yt_session.as_mut() {
            let _ = session.clear_cookies(&self.yt_python, &self.yt_script);
        }
        self.yt_status = Some("YT auth: logged out (guest mode)".into());
        self.yt_error = None;
    }

    /// `s` — instant random track from the active source, played *in context*
    /// (its album/playlist becomes the context so `>`/`<` and CONT behave
    /// coherently). Local/Mixed: a random catalog track. YouTube: a random
    /// track from the first loaded YT list (if any).
    pub fn instant_random(&mut self) {
        if self.source_mode != crate::mode::SourceMode::Youtube && !self.catalog.tracks.is_empty() {
            let i = self.simple_rand() % self.catalog.tracks.len();
            let id = self.catalog.tracks[i].id.clone();
            let album = self.catalog.tracks[i].album.clone();
            let ids = match album {
                Some(a) => self.tracks_for_album(&a),
                None => vec![id.clone()],
            };
            let start = if ids.contains(&id) { id } else { ids.first().cloned().unwrap_or(id) };
            self.play_in_context_ids(ids, &start);
            return;
        }
        // Youtube mode (or empty local catalog): pick from the first list that
        // has tracks loaded. `s` on a fresh Y view kicks off a load of the
        // focused list, so a second `s` plays once it lands.
        if let Some(l) = self.yt_lists.iter().find(|l| !l.track_ids.is_empty()).cloned() {
            if let Some(id) = l.track_ids.first().cloned() {
                self.play_in_context_ids(l.track_ids.clone(), &id);
                return;
            }
        }
        // Nothing to play yet — tell the user why instead of silently no-op'ing.
        if self.source_mode == crate::mode::SourceMode::Youtube || self.catalog.tracks.is_empty() {
            if self.yt_session.is_none() {
                self.yt_error = Some("s: YouTube not configured — run :yt auth browser <chrome>".into());
            } else if self.yt_lists.is_empty() {
                self.yt_status = Some("s: no YouTube lists loaded yet (open the Y view with 4)".into());
            } else {
                // Lists present but none expanded: nudge the focused one to load.
                if self.view != View::Youtube {
                    self.view = View::Youtube;
                }
                if let Some(l) = self.yt_lists.get(self.cursors.playlist).cloned() {
                    let id = l.id.clone();
                    let loaded = self.loaded_yt_lists.contains(&id);
                    let inflight = self
                        .yt_session
                        .as_ref()
                        .map(|s| s.playlist_loading(&id))
                        .unwrap_or(false);
                    if !loaded && !inflight {
                        if let Some(session) = self.yt_session.as_mut() {
                            let _ = session.send_get_playlist(id);
                        }
                    }
                }
                self.yt_status = Some("s: loading the focused list — press s again in a moment".into());
            }
        }
    }

    /// `S` — open the discover overlay: local smart-album suggestions (Local /
    /// Mixed) or YouTube mood playlists (YouTube / Mixed-with-session).
    pub fn open_discover(&mut self) {
        let items = match self.source_mode {
            crate::mode::SourceMode::Youtube => self.yt_discover_items(),
            _ => {
                let mut albums = self.local_smart_albums();
                if albums.is_empty() {
                    albums.extend(self.yt_discover_items());
                }
                albums
            }
        };
        self.overlay = Some(Overlay::Discover { items, cursor: 0 });
    }

    /// Local smart-album heuristic (spec §5.5): score each album by artist
    /// diversity (deprioritize the currently-playing artist) + a deterministic
    /// per-album pseudo-random so suggestions vary but stay stable. Pick 5.
    fn local_smart_albums(&self) -> Vec<DiscoverItem> {
        let cur_artist = self.now_playing_view().map(|v| v.artist).unwrap_or_default();
        let mut scored: Vec<(u64, DiscoverItem)> = Vec::new();
        for (artist, albums) in &self.albums_by_artist {
            for a in albums {
                let key = format!("{artist}|{}", a.title);
                let r = Self::hash_rand(&key);
                let penalty = if *artist == cur_artist { 1_000_000 } else { 0 };
                scored.push((
                    r + penalty,
                    DiscoverItem::Album {
                        artist: a.artist.clone(),
                        album: a.title.clone(),
                    },
                ));
            }
        }
        scored.sort_by_key(|(s, _)| *s);
        scored.into_iter().take(5).map(|(_, d)| d).collect()
    }

    fn yt_discover_items(&mut self) -> Vec<DiscoverItem> {
        let Some(session) = self.yt_session.as_mut() else {
            return Vec::new();
        };
        match session.home_suggestions() {
            Ok(s) => s
                .into_iter()
                .map(|p| DiscoverItem::Playlist { id: p.id, name: p.name })
                .take(5)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn simple_rand(&mut self) -> usize {
        // Seed the counter with the wall-clock time on FIRST use so each launch
        // starts at a different point — otherwise `s` (instant random) would
        // pick the same opening track every launch (the counter used to start
        // at a fixed constant). The seed is computed once and reused; the
        // counter still advances across calls so successive `s` presses differ.
        use std::sync::OnceLock;
        static SEED: OnceLock<u64> = OnceLock::new();
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x9E3779B97F4A7C15);
        let seed = *SEED.get_or_init(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
        });
        let mut x = COUNTER.fetch_add(0x632BE59BD9B4C0A1, std::sync::atomic::Ordering::Relaxed);
        x = x.wrapping_add(seed);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545F4914F6CDD1D)) as usize
    }

    fn hash_rand(key: &str) -> u64 {
        let mut h: u64 = 0xCBF29CE484222325;
        for b in key.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001B3);
        }
        h
    }

    /// `f` toggle: open the inline filter on the focused column, or close it.
    /// No-op in the Queue view (no list to filter) and in the track columns of
    /// Artists (col 2) where there's no jump target — only the list columns are
    /// filterable, so an accidental `f` elsewhere doesn't open a dead filter.
    pub fn toggle_filter(&mut self) {
        // Queue has nothing to filter.
        if self.view == View::Queue {
            return;
        }
        match &self.filter {
            Some(f) if f.col == self.focus_col => self.filter = None,
            _ => self.filter = Some(FilterState { col: self.focus_col, text: String::new() }),
        }
    }

    /// `Enter` while a filter is active: jump the real cursor to the first
    /// matching item on the filtered column, then clear the filter. So "type
    /// ade, Enter" lands the cursor on Adele.
    pub fn filter_jump(&mut self) {
        let Some(f) = self.filter.clone() else {
            return;
        };
        let q = f.text.trim().to_lowercase();
        if q.is_empty() {
            self.filter = None;
            return;
        }
        let matches = |s: &str| s.to_lowercase().contains(&q);
        match self.view {
            View::Artists => match f.col {
                0 => {
                    if let Some(i) = self.artists.iter().position(|a| matches(a)) {
                        self.cursors.artist = i;
                        self.cursors.album = 0;
                        self.cursors.track = 0;
                    }
                }
                1 => {
                    let artist = self.artists.get(self.cursors.artist).cloned().unwrap_or_default();
                    if let Some(albums) = self.albums_by_artist.get(&artist).cloned() {
                        if let Some(i) = albums.iter().position(|a| matches(&a.title)) {
                            self.cursors.album = i;
                            self.cursors.track = 0;
                        }
                    }
                }
                _ => {}
            },
            View::Playlists => {
                if f.col == 0 {
                    if let Some(i) = self.playlists.iter().position(|p| matches(&p.name)) {
                        self.cursors.playlist = i;
                        self.cursors.track = 0;
                    }
                }
            }
            View::Youtube => {
                if f.col == 0 {
                    if let Some(i) = self.yt_lists.iter().position(|l| matches(&l.name)) {
                        self.cursors.playlist = i;
                        self.cursors.track = 0;
                    }
                }
            }
            View::Queue => {}
        }
        self.filter = None;
    }

    /// Does `id`'s resolved display name match the active filter (case-insensitive
    /// substring)? Used by the track-column renderers.
    pub fn filter_matches(&self, name: &str) -> bool {
        let Some(f) = &self.filter else {
            return true;
        };
        if f.text.is_empty() {
            return true;
        }
        name.to_lowercase().contains(&f.text.to_lowercase())
    }

    /// Apply a discover-overlay selection (Enter): start the album/playlist.
    pub fn play_discover_selection(&mut self) {
        let Some(Overlay::Discover { items, cursor }) = self.overlay.clone() else {
            return;
        };
        let Some(item) = items.get(cursor).cloned() else {
            return;
        };
        match item {
            DiscoverItem::Album { album, .. } => {
                let ids = self.tracks_for_album(&album);
                if let Some(start) = ids.first().cloned() {
                    self.transport.continue_mode = ContinueMode::NextAlbum;
                    self.play_in_context_ids(ids, &start);
                }
            }
            DiscoverItem::Playlist { id, .. } => {
                let tracks = self
                    .yt_session
                    .as_mut()
                    .and_then(|s| s.get_playlist(&id).ok())
                    .unwrap_or_default();
                let ids: Vec<String> = tracks.into_iter().map(|t| t.video_id).collect();
                if let Some(start) = ids.first().cloned() {
                    self.play_in_context_ids(ids, &start);
                }
            }
        }
    }

    /// Kick off an async fetch of the account + suggested lists for the Y view.
    /// Non-blocking: sends the requests and returns immediately, showing
    /// "loading…" until `on_tick` folds the results into `yt_lists`. No-op
    /// (and clears the lists) when there's no session — the view then shows
    /// the setup hint. A refresh already in flight is not re-sent.
    pub fn refresh_yt_lists(&mut self) {
        let Some(session) = self.yt_session.as_mut() else {
            self.yt_lists.clear();
            return;
        };
        self.yt_error = None;
        self.yt_lists_loading = true;
        // Drop a stale partial fetch so on_tick merges the fresh pair together.
        session.pending_playlists = None;
        session.pending_suggestions = None;
        if let Err(e) = session.send_refresh() {
            self.yt_lists_loading = false;
            self.yt_error = Some(format!("refresh: {e}"));
        }
        // Pre-warm the PREMIUM tier on the first refresh: the deno EJS nsig
        // solver downloads once (~10s cold) and caches, and the macOS Keychain
        // read + cookie-file write happen once here too. Warming with the
        // premium client (not fast) means the solver download is absorbed on
        // Y-view open — otherwise the first PREMIUM preload (next-track) would
        // eat it during the first track. Fire-and-forget, so it never blocks;
        // the result (for a harmless well-known video) is just discarded.
        let _ = session.send_resolve_premium("jNQXAC9IVRw".into()); // "Me at the zoo"
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
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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
        let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone(), yt_lists: &self.yt_lists };
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

    /// Run a LOCAL catalog search for `q` against `self.searcher` (BM25, up to
    /// 50 hits) and return the track ids that resolve to extant catalog tracks.
    /// Empty if no index is present. This is the instant, live-as-you-type path
    /// for the search overlay's `Local` scope.
    pub fn run_search_local(&self, q: &str) -> Vec<String> {
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

    /// Fire-and-forget a YouTube search for `q` (the `Youtube` scope's
    /// explicit-submit path). Non-blocking: sends one `Request::Search` and
    /// returns immediately; `on_tick` folds the response into the open search
    /// overlay. No-op (and surfaces a hint via `yt_error`) when there's no
    /// session — typing locally without a configured sidecar.
    pub fn submit_yt_search(&mut self, q: String) {
        let Some(session) = self.yt_session.as_mut() else {
            self.yt_error = Some("search: YouTube not configured — run :yt auth browser <chrome>".into());
            return;
        };
        if let Err(e) = session.send_search(q) {
            self.yt_error = Some(format!("search: {e}"));
        }
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
            View::Youtube => "youtube",
        }
    }
}

// Transport methods take `(&dyn ContextResolver, &Catalog)`. `App` passes
// `self` as the resolver and `&self.catalog` as the catalog; the split-borrow is
// sound because `manual_queue` (the resolver's data source) lives in a distinct
// field from `catalog` and from the `&mut self.transport` we hold.
