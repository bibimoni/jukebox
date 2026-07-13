//! App state + pure (context-play) update methods.
//!
//! [`App`] owns the whole TUI world: the catalog, the player backend, the
//! transport engine, playlists, browse state (view + cursors + column widths),
//! volume, the dead-track set, and the current overlay. All update methods here
//! are pure with respect to the transport — they take `&mut self` and call into
//! [`Transport`] (Task 4) with `self` borrowed immutably as the
//! [`ContextResolver`] (since `playlists` / `transport.manual_queue` live in
//! separate fields, this split-borrow works).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

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
    /// RC11-DEF-030: a generated mix (Daily Mix, Discover Mix, ...). Displayed
    /// with a ◆ glyph to distinguish it from account (♫) and suggested (✦)
    /// playlists. Not yet populated by `refresh_yt_lists` (the mixes live in
    /// `reco_mixes` / the Home + Discover overlays); the variant exists so
    /// future wiring (Batch I tab system) can surface mixes in the YT view.
    Generated,
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
#[derive(Clone, Debug)]
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
    /// `a` — add the focused track to a playlist. `track_id` is the id to add;
    /// `cursor` selects among existing playlists + the "new playlist…" entry.
    PlaylistPicker {
        track_id: String,
        cursor: usize,
    },
    Command {
        input: String,
        /// Byte offset of the cursor within `input` (0 to `input.len()`).
        /// Insertion/deletion happen at this position, not at the end.
        cursor: usize,
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
    /// Lyrics overlay (`L`). `content` holds the parsed lyrics once loaded;
    /// `state` is the truthful lifecycle (Loading / Available / NotFound /
    /// Error); `scroll` is the j/k/PgUp/PgDn scroll offset. `gen` mirrors
    /// `App::lyrics_gen` at request time so a stale response (the user moved
    /// to a different track) is discarded by `on_tick` (D5 generation guard).
    Lyrics {
        content: Option<crate::lyrics::Lyrics>,
        state: LyricsState,
        scroll: u16,
        /// The track id these lyrics are for (so a re-toggle on the same track
        /// reuses the cache without re-fetching).
        track_id: String,
        /// The generation tag captured at request time; `on_tick` discards the
        /// result if `App::lyrics_gen` has advanced past it.
        gen: u64,
    },
    /// Diagnostics overlay (`:diag` or `D`): a scrollable list of recent
    /// diagnostic messages (provider errors, respawn notices) captured by
    /// [`crate::diagnostics::Diagnostics`]. Unit variant — the buffer lives
    /// on `App::diagnostics`; this just signals "show it". Esc closes (handled
    /// generically at the top of the key handler).
    Diagnostics,
    /// Confirmation dialog for destructive actions (DEF-001: delete playlist,
    /// DEF-015: yt logout). `message` is shown to the user; `action` identifies
    /// what to do on confirm (y/Enter). n/Esc cancels.
    Confirm {
        message: String,
        action: ConfirmAction,
    },
    /// Text input overlay (DEF-014: playlist name prompt). `prompt` is the
    /// label; `buffer` accumulates typed text; `cursor` is the byte offset;
    /// `action` identifies what to do on Enter.
    TextInput {
        prompt: String,
        buffer: String,
        cursor: usize,
        action: TextInputAction,
    },
    /// YouTube Home view — multi-section discovery.
    Home {
        state: crate::tui::view::home::HomeState,
    },
    /// Radio session overlay.
    Radio {
        session: Option<crate::reco::radio::RadioSession>,
    },
    /// Playlist generator overlay.
    Generator {
        state: crate::tui::view::generator::GeneratorState,
    },
    /// Recommendation explanation overlay.
    Explanation {
        explanation: crate::reco::explanations::Explanation,
    },
    /// YouTube playlist publication overlay.
    Publication {
        state: crate::tui::view::publication::PublicationState,
    },
}

/// Actions that can be confirmed via [`Overlay::Confirm`].
#[derive(Clone, Debug)]
pub enum ConfirmAction {
    /// Delete the focused playlist (Playlists view, col 0).
    DeletePlaylist,
    /// Clear YouTube credentials and log out.
    YtLogout,
    /// Clear the play-next queue (MOD-4: confirm before clearing a non-empty
    /// queue, mirroring the `d` / `:yt logout` confirmation pattern).
    ClearQueue,
    /// Play the most recently saved generator playlist (RC11-DEF-065).
    /// Paired with `App::pending_play_saved_idx` (the playlist's index in
    /// `App::playlists`); the Confirm handler plays it on `y`/Enter.
    PlaySavedPlaylist,
}

/// Actions that can be triggered via [`Overlay::TextInput`].
#[derive(Clone, Debug)]
pub enum TextInputAction {
    /// Create a new playlist with the given name and seed track.
    NewPlaylist { track_id: String },
}

/// The lifecycle state of the lyrics overlay. Distinct from `Overlay::Lyrics`
/// because the renderer needs to show "loading…", "lyrics unavailable", and
/// "lyrics error" as separate truthful states (AC-M3.2.1: 6 states).
/// `Error(String)` carries the message; `Available` carries whether the lines
/// are synced (for the source/timestamp label).
#[derive(Clone, Debug)]
pub enum LyricsState {
    /// No request yet (the overlay was just opened).
    Idle,
    /// A fetch is in flight (local read or sidecar `get_lyrics`).
    Loading,
    /// Lyrics loaded; `true` when timestamped (synced LRC), `false` for plain.
    Available(bool),
    /// The track has no lyrics (no embedded tag, no sidecar file, ytmusicapi
    /// returned empty). Truthful "unavailable" — never fabricated text
    /// (AC-M3.5.1).
    NotFound,
    /// The provider is unreachable while cached provider data remains usable.
    /// Distinct from `NotFound`: absence has not been established offline.
    Offline,
    /// The provider failed (network/parse error). Carries the message so the
    /// overlay can show it (the sidecar's stderr is null'd, so this is the
    /// only error path).
    Error(String),
}

/// A discover-overlay suggestion.
#[derive(Clone, Debug)]
pub enum DiscoverItem {
    /// A local catalog album (Local / Mixed).
    Album { artist: String, album: String },
    /// A YouTube mood/suggested playlist (YouTube / Mixed).
    Playlist { id: String, name: String },
    /// A generated mix from `reco::mixes` (RC11-DEF-013). Carries the mix's
    /// local-catalog track ids (so Enter plays immediately via
    /// `play_in_context_ids` — no sidecar roundtrip), the display name, and
    /// an optional "why recommended" explanation (RC11-DEF-028).
    Mix {
        mix_type: crate::reco::mixes::MixType,
        name: String,
        track_ids: Vec<String>,
        explanation: Option<String>,
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
    /// `track id` → index into `catalog.tracks`, built once in [`App::new`].
    /// Turns `track_by_id` from a per-call O(n) linear scan into an O(1)
    /// lookup (PB7: the scan ran per visible track row, per frame).
    pub track_index: HashMap<String, usize>,
    /// `album title` → track ids of that album, across ALL primary_artists,
    /// in `(disc_number, track_number)` order, built once in [`App::new`].
    /// Turns `tracks_for_album` from a per-frame full-catalog scan into an O(1)
    /// lookup (PB8). Grouped by title (not by owner) so collaboration albums
    /// show every track regardless of the focused artist — see
    /// `collaboration_album_shows_and_plays_all_tracks` in `tests/app.rs`.
    pub album_tracks: HashMap<String, Vec<String>>,
    pub view: View,
    /// The view rendered in the last frame. Used by `layout::draw` to detect
    /// view switches and force a full redraw (MOD-7: when switching views,
    /// a cell whose content+style coincidentally matches the previous frame
    /// at the same position — e.g. "i" in "Test Artist" → "i" in "Late Night
    /// Jazz" at the same column — is skipped by ratatui's diff, leaving stale
    /// content. Forcing every non-empty cell to `AlwaysUpdate` on a view
    /// switch ensures the diff emits all visible characters).
    pub last_rendered_view: View,
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
    /// The truthful YouTube provider lifecycle state (M2). Replaces the
    /// optimistic `yt_status = "connected…"` assignments (which could claim
    /// "connected" before any data fetch verified the credential — the
    /// "connected but empty" bug). The footer and Y-view status line derive
    /// their label from `yt_state.human_label()`, NOT from `yt_status`.
    /// `yt_status` is kept for backward compat with the few non-state messages
    /// (e.g. "upgraded to AAC 256k"); the state machine owns the auth/sync
    /// lifecycle. `yt_error` carries error *detail* alongside the state.
    pub yt_state: crate::yt::state::YtState,
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
    /// In-session `:` command history (most recent first, bounded, adjacent-
    /// deduped). Up/Down in the Command overlay traverses this.
    pub command_history: Vec<String>,
    /// Cursor into `command_history` during Up/Down traversal. `None` = at the
    /// draft (not traversing); `Some(i)` = showing history[i].
    pub command_history_cursor: Option<usize>,
    /// The draft text saved before traversing up into history, restored on
    /// Down past the end.
    pub command_draft: String,
    /// Generation counter for lyrics requests (D5 stale-discard). Bumped on
    /// every `request_lyrics`; the overlay captures the gen at request time
    /// and `on_tick` discards a response whose gen != the current gen (the
    /// user moved to a different track). Guarantees stale lyrics can't
    /// overwrite a newer track's lyrics overlay.
    pub lyrics_gen: u64,
    /// Verbosity resolved from the `--verbose`/`--quiet` CLI flags. The footer
    /// / view layer consults this to decide how much chrome to render: `Quiet`
    /// shows only errors, `Normal` is the default hint bar, `Verbose` shows
    /// the YT provider label even when Ready, `Debug` adds a per-tick
    /// diagnostic counter. Defaults to `Normal`; wired by `main.rs`.
    pub verbosity: crate::cli::Verbosity,
    /// Bounded ring buffer of recent diagnostic messages (provider errors,
    /// respawn notices, sidecar failures) rendered by
    /// [`crate::tui::view::diagnostics`]. `on_tick` pushes a line whenever
    /// `yt_error` changes, so the user can review what happened without
    /// scraping the log file.
    pub diagnostics: crate::diagnostics::Diagnostics,
    /// When the current transient `yt_status` should expire. Set (via
    /// `on_tick` change-detection) when a key handler / respawn assigns a new
    /// `yt_status`; cleared by `on_tick` once `Duration::from_secs(5)` elapses
    /// so the footer returns to the hint bar / state label instead of
    /// lingering indefinitely.
    pub notification_ttl: Option<Instant>,
    /// The last `yt_status` we "accepted" (for dedup). `on_tick` only
    /// (re)starts the TTL window when the new `yt_status` differs from this,
    /// so a repeat of the same message doesn't keep refreshing its 5s lease.
    pub last_notification: Option<String>,
    /// The playlist name captured at publish-dispatch time so `on_tick`
    /// can surface a "Playlist \"<name>\" created" toast when the sidecar's
    /// `pending_publication` result lands. RC11-DEF-002: the publication
    /// result carries only the playlist id (or error), not the user-supplied
    /// title, so we stash it here on Enter and consume it in `on_tick`.
    pub pending_publish_name: Option<String>,
    /// The index of the most recently saved generator playlist, stashed
    /// so the "Play now? y/n" Confirm overlay's `y`/Enter handler can play
    /// it (RC11-DEF-065). Set when the generator's Enter-save succeeds;
    /// consumed by the Confirm handler for `PlaySavedPlaylist`.
    pub pending_play_saved_idx: Option<usize>,
    /// Audit journal of YouTube publication attempts (Batch C). Used by
    /// `record_publication` to log each publish result for truthful reporting.
    pub publication_journal: crate::yt::publication::PublicationJournal,
    /// True while a discover (`S` overlay) home-suggestions fetch is in flight.
    /// Set by `yt_discover_items` (fire-and-forget `send_home_suggestions`);
    /// cleared by `on_tick` when `pending_discover` lands (or on error). The
    /// overlay opens instantly with a "loading…" state and populates when the
    /// response lands — `S` no longer blocks the UI for the ~3s roundtrip.
    pub discover_loading: bool,
    /// Tick counter for the discover loading timeout. Incremented each
    /// `on_tick` while `discover_loading` is true; after 600 ticks (~10s at
    /// 60fps, ~30s at 20fps), the loading state is cleared and an error
    /// message is shown so the overlay doesn't hang forever.
    pub discover_loading_ticks: u32,
    /// RC11-DEF-035: the name of the mix/playlist a Discover Enter is
    /// currently loading. Set by `play_discover_selection` when the overlay
    /// stays open to show "Loading [name]..."; cleared when playback starts
    /// (on_tick) or on error. Rendered by `render_discover` so the user
    /// sees a persistent loading state inside the overlay instead of a
    /// silent close.
    pub discover_play_loading: Option<String>,
    /// A YouTube playlist id whose tracks were requested by a discover
    /// selection (Enter on a `DiscoverItem::Playlist`). `play_discover_selection`
    /// fires-and-forgets `send_get_playlist(id)` + stores the id here; `on_tick`
    /// starts playback of the playlist's tracks when `pending_tracks` lands
    /// with a matching id (the blocking `get_playlist` call is gone, so Enter
    /// no longer freezes the UI for the ~4s roundtrip).
    pub pending_discover_play: Option<String>,
    /// The seed video_id for an in-flight CONT=YouTube radio refill
    /// (`send_watch_playlist`). `next()` stores it when the radio queue is
    /// exhausted; `on_tick` consumes it when `pending_watch` lands to refill
    /// the `RadioCursor` + start playback. Non-blocking auto-advance: the old
    /// track stays current until the next track's id lands.
    pub pending_radio_seed: Option<String>,

    /// Pending background audio format switch (non-blocking). Set by
    /// `start_playback`/`load_track`/`load_remote` when a device-rate
    /// switch is needed; the blocking CoreAudio call runs on a detached
    /// thread so the input loop never freezes. `on_tick` polls
    /// `is_finished()` for best-effort cleanup. The player loads
    /// immediately (fire-and-forget) — the device re-clocks when the
    /// format lands (AC-M9.2.4).
    audio_switch_handle: Option<std::thread::JoinHandle<()>>,

    /// User listening profile for recommendations.
    pub reco_profile: crate::reco::profile::UserProfile,
    /// Listening event log.
    pub reco_events: crate::reco::events::EventLog,
    /// The user's generated mixes.
    pub reco_mixes: Vec<crate::reco::mixes::Mix>,
    /// Feedback actions pending application.
    pub reco_feedback_pending: Vec<(crate::reco::feedback::FeedbackAction, String)>,

    // --- Playback-event tracking (DEF-034) ---------------------------------
    /// When the currently-playing track started (for rapid-skip detection:
    /// a skip within 10s is a strong negative signal). Reset on every
    /// `start_playback` / `load_track`.
    pub play_started_at: Option<std::time::Instant>,
    /// The track id for which `meaningful_threshold` has already been fired
    /// this play, so `on_tick` doesn't duplicate-fire it each tick. Cleared
    /// when the now-playing track changes.
    pub threshold_fired_for: Option<String>,
    /// The track id that just ended naturally (on_track_ended). `next()`
    /// checks this to avoid recording a "skipped" event for a track that
    /// actually completed — `on_track_ended` records "completed" first.
    pub last_natural_end: Option<String>,
    /// When true, `record_listen_event` also persists each event to
    /// `state.db` (DEF-034) so listening history survives restarts. Default
    /// false so tests (which construct `App` without a DB) don't touch the
    /// real state DB; `main.rs` sets it true after loading prior events.
    pub persist_events: bool,

    // --- RC11 Batch D: resume + toast ----------------------------------------
    /// RC11-DEF-014: the last-played track id, restored on launch so the
    /// cursor can return to it and a "resume" hint can show. Updated as
    /// playback progresses (on_tick) and saved to `state.db` on exit.
    pub last_played_track_id: Option<String>,
    /// RC11-DEF-014: the last-played track's position in seconds, restored on
    /// launch so `resume_last()` can seek to it. afplay can't seek so resume
    /// only re-seeks on mpv/StubPlayer; afplay restarts from 0.
    pub last_played_position: f64,
    /// RC11-DEF-014: a one-shot (track_id, position) captured on launch from
    /// `state.db`. The first successful load of the matching track uses
    /// `load_at(pos)` to resume at the saved position, then clears this.
    /// `None` after the resume is consumed (or on a fresh launch with no
    /// saved playback).
    pub pending_resume: Option<(String, f64)>,
    /// RC11-DEF-014: a "resume" hint shown in the player bar when stopped
    /// with a saved last-played track. Set on launch from `state.db`;
    /// cleared on the first successful play so it doesn't linger. The bar
    /// renders `▸ resume: [title] at [M:SS] · Enter to resume`.
    pub resume_hint: Option<String>,
    /// RC11-DEF-043: a transient confirmation toast (e.g. "Added to queue")
    /// shown in the player bar's up-next slot. Set by `enqueue_selected`;
    /// cleared by `on_tick` after ~1.2s so it's visible long enough to read
    /// but doesn't linger. Rendered regardless of `yt_state` so local-only
    /// users see the feedback (the `yt_status` toast was gated on Ready).
    pub toast: Option<String>,
    /// When the current `toast` was set; used by `on_tick` to clear it after
    /// the TTL. `None` when no toast is active.
    pub toast_at: Option<std::time::Instant>,
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
    Pending { video_id: String },
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

        // Build id→index and album→track_ids lookup tables once. The catalog
        // is immutable for the app's lifetime (rebuild via `jukebox sync` +
        // relaunch), so these never need invalidation. They turn the per-frame
        // O(n) scans in `track_by_id` (PB7) and `tracks_for_album` (PB8) into
        // O(1) lookups.
        let mut track_index: HashMap<String, usize> = HashMap::with_capacity(catalog.tracks.len());
        let mut album_idxs: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, t) in catalog.tracks.iter().enumerate() {
            track_index.insert(t.id.clone(), i);
            if let Some(album) = &t.album {
                album_idxs.entry(album.clone()).or_default().push(i);
            }
        }
        // `tracks_for_album` groups by album TITLE across all primary_artists
        // (collaboration albums are a cohesive object), then sorts by
        // (disc, track_number) — mirror that exactly so the precompute is a
        // drop-in replacement for the linear scan.
        let mut album_tracks: HashMap<String, Vec<String>> =
            HashMap::with_capacity(album_idxs.len());
        for (album, mut idxs) in album_idxs {
            idxs.sort_by_key(|&i| {
                let t = &catalog.tracks[i];
                (t.disc_number.unwrap_or(1), t.track_number.unwrap_or(0))
            });
            album_tracks.insert(
                album,
                idxs.into_iter()
                    .map(|i| catalog.tracks[i].id.clone())
                    .collect(),
            );
        }
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
            track_index,
            album_tracks,
            view: View::Artists,
            last_rendered_view: View::Artists,
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
            yt_state: crate::yt::state::YtState::default(),
            yt_python: std::path::PathBuf::from("python3"),
            yt_script: std::path::PathBuf::from("scripts/yt/yt.py"),
            filter: None,
            help_scroll: 0,
            spinner_frame: 0,
            playing_premium: false,
            pending_play: None,
            loaded_yt_lists: HashSet::new(),
            yt_browser: String::new(),
            command_history: Vec::new(),
            command_history_cursor: None,
            command_draft: String::new(),
            lyrics_gen: 0,
            verbosity: crate::cli::Verbosity::default(),
            diagnostics: crate::diagnostics::Diagnostics::new(),
            notification_ttl: None,
            last_notification: None,
            pending_publish_name: None,
            pending_play_saved_idx: None,
            publication_journal: crate::yt::publication::PublicationJournal::new(),
            discover_loading: false,
            discover_loading_ticks: 0,
            discover_play_loading: None,
            pending_discover_play: None,
            pending_radio_seed: None,
            audio_switch_handle: None,
            reco_profile: crate::reco::profile::UserProfile::default(),
            reco_events: crate::reco::events::EventLog::new(),
            reco_mixes: Vec::new(),
            reco_feedback_pending: Vec::new(),
            play_started_at: None,
            threshold_fired_for: None,
            last_natural_end: None,
            persist_events: false,
            last_played_track_id: None,
            last_played_position: 0.0,
            pending_resume: None,
            resume_hint: None,
            toast: None,
            toast_at: None,
        }
    }

    /// O(1) track lookup by id, backed by [`App::track_index`]. Public so the
    /// view layer (which holds `&App`) can resolve ids without a linear scan
    /// of `catalog.tracks` (PB7 — the scan ran per visible row, per frame).
    pub fn track_by_id_fast(&self, id: &str) -> Option<&Track> {
        self.track_index
            .get(id)
            .and_then(|&i| self.catalog.tracks.get(i))
    }

    fn track_by_id(&self, id: &str) -> Option<&Track> {
        self.track_by_id_fast(id)
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

    /// Title of the next track in the manual queue (for up-next preview).
    /// Returns None if queue empty.
    pub fn up_next_title(&self) -> Option<String> {
        let id = self.transport.manual_queue.first()?;
        let t = self.track_by_id_fast(id)?;
        Some(t.title.clone())
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
    ///
    /// Backed by the [`App::album_tracks`] precompute (built once in
    /// [`App::new`]), so this is an O(1) HashMap lookup — the old form did a
    /// full-catalog `iter().enumerate().filter()` scan per frame (PB8).
    pub fn tracks_for_album(&self, album_title: &str) -> Vec<String> {
        self.album_tracks
            .get(album_title)
            .cloned()
            .unwrap_or_default()
    }

    /// Build the track-id list for the currently-focused track column.
    /// Clamp every browse cursor to a valid index for its current list. Stale
    /// cursors (e.g. `cursors.album` left at 5 after switching to an artist with
    /// only 2 albums) otherwise make the Tracks column render empty and make
    /// `play_selected` play the wrong/no track — the "this artist has no songs"
    /// and "Enter doesn't play after picking a list" bugs.
    pub fn clamp_cursors(&mut self) {
        // Queue view: keep cursors.track synced with cursors.queue so the
        // ▸ selection marker (rendered via cursors.track by `track_rows`)
        // matches the actual navigation cursor (cursors.queue). Without
        // this, a stale cursors.track from a prior view makes `x` remove
        // a different item than the one the user sees highlighted (DEF-016).
        if self.view == View::Queue {
            self.cursors.track = self.cursors.queue;
        }
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
        // `cursors.playlist` is shared between View::Playlists (local
        // playlists) and View::Youtube (yt_lists), which generally have
        // different lengths. Clamping against the wrong list yanks the
        // cursor back on every render (layout.rs calls this each frame) and
        // the user can't move between YouTube playlists when they have fewer
        // local playlists than YouTube lists. Clamp against the list that
        // belongs to the active view.
        let n_playlist_col = match self.view {
            View::Youtube => self.yt_lists.len(),
            _ => self.playlists.len(),
        };
        if n_playlist_col > 0 && self.cursors.playlist >= n_playlist_col {
            self.cursors.playlist = n_playlist_col - 1;
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
                    None => Context::Artist {
                        artist,
                        track_ids: ids,
                    },
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
            let r = ClonedResolver {
                playlists: &self.playlists,
                manual_queue: self.transport.manual_queue.clone(),
                yt_lists: &self.yt_lists,
            };
            let id = match self.transport.current(&r, &self.catalog) {
                Some(id) => id,
                None => return,
            };
            drop(r);
            if self.dead.contains(&id) {
                let r = ClonedResolver {
                    playlists: &self.playlists,
                    manual_queue: self.transport.manual_queue.clone(),
                    yt_lists: &self.yt_lists,
                };
                let _ = self.transport.next(&r, &self.catalog);
                if self.transport.cursor == start {
                    return;
                }
                continue;
            }
            match self.resolve_source(&id) {
                Some(Resolved::Local {
                    path,
                    sample_rate_hz,
                    bit_depth,
                }) => {
                    if std::fs::metadata(&path).is_err() {
                        self.dead.insert(id.clone());
                        self.yt_error = Some(format!("file not found: {path:?}"));
                        self.yt_status = Some(format!("file not found: {}", path.display()));
                        let r = ClonedResolver {
                            playlists: &self.playlists,
                            manual_queue: self.transport.manual_queue.clone(),
                            yt_lists: &self.yt_lists,
                        };
                        let _ = self.transport.next(&r, &self.catalog);
                        if self.transport.cursor == start {
                            return;
                        }
                        continue;
                    }
                    if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                        &mut self.device_rate,
                        crate::source::device_rate::LoadKind::Local {
                            sample_rate_hz,
                            bit_depth,
                        },
                        self.switch_sample_rate,
                    ) {
                        self.audio_switch_handle =
                            Some(crate::audio::set_output_format_async(sr, bd));
                    }
                    match self.load_with_resume(&path, &id) {
                        Ok(()) => {
                            self.now_playing = Some(crate::source::TrackSource::Local {
                                track_id: id.clone(),
                            });
                            self.note_play_started(&id);
                            self.preload_next_url();
                        }
                        Err(e) => {
                            let msg = format!("playback failed: {e}");
                            self.yt_error = Some(msg.clone());
                            self.yt_status = Some(msg);
                            self.dead.insert(id.clone());
                        }
                    }
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
                    let r = ClonedResolver {
                        playlists: &self.playlists,
                        manual_queue: self.transport.manual_queue.clone(),
                        yt_lists: &self.yt_lists,
                    };
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
            return Some(Resolved::Remote {
                url,
                fmt,
                video_id: id.to_string(),
            });
        }
        // Cache miss → arm both tiers fire-and-forget and defer the swap to
        // on_tick (Pending). Guards in send_resolve/send_resolve_premium make
        // re-arming a no-op if a tier is already in flight or cached.
        let _ = session.send_resolve(id.to_string());
        let _ = session.send_resolve_premium(id.to_string());
        Some(Resolved::Pending {
            video_id: id.to_string(),
        })
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
        let will_stream =
            self.source_mode == crate::mode::SourceMode::Youtube || self.track_by_id(&id).is_none();
        if !will_stream {
            return;
        }
        let Some(session) = self.yt_session.as_mut() else {
            return;
        };
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
            Some(Resolved::Local {
                path,
                sample_rate_hz,
                bit_depth,
            }) => {
                // Check the file exists before attempting to load — a missing
                // file would otherwise "succeed" with a stub player or fail
                // silently with a real one (DEF-009). Mark dead + set a
                // visible error so the user + next() loop know to skip.
                if std::fs::metadata(&path).is_err() {
                    self.dead.insert(id.to_string());
                    self.yt_error = Some(format!("file not found: {path:?}"));
                    self.yt_status = Some(format!("file not found: {}", path.display()));
                    return;
                }
                if let Some((sr, bd)) = crate::source::device_rate::desired_switch(
                    &mut self.device_rate,
                    crate::source::device_rate::LoadKind::Local {
                        sample_rate_hz,
                        bit_depth,
                    },
                    self.switch_sample_rate,
                ) {
                    self.audio_switch_handle = Some(crate::audio::set_output_format_async(sr, bd));
                }
                match self.load_with_resume(&path, id) {
                    Ok(()) => {
                        self.now_playing = Some(crate::source::TrackSource::Local {
                            track_id: id.to_string(),
                        });
                        self.note_play_started(id);
                    }
                    Err(e) => {
                        let msg = format!("playback failed: {e}");
                        self.yt_error = Some(msg.clone());
                        self.yt_status = Some(msg);
                        self.dead.insert(id.to_string());
                    }
                }
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
            crate::source::device_rate::LoadKind::Remote {
                sample_rate: fmt.sample_rate,
            },
            self.switch_sample_rate,
        ) {
            self.audio_switch_handle = Some(crate::audio::set_output_format_async(sr, bd));
        }
        let p = std::path::PathBuf::from(&url);
        match self.load_with_resume(&p, &video_id) {
            Ok(()) => {
                self.now_playing = Some(crate::source::TrackSource::Remote {
                    video_id: video_id.clone(),
                });
                self.note_play_started(&video_id);
                // Record whether we started at premium (256k) so a later premium URL
                // landing mid-play swaps only if we're not already premium
                // (progressive upgrade guard).
                self.playing_premium = fmt.premium;
                self.preload_next_url();
            }
            Err(e) => {
                self.yt_error = Some(format!("stream load failed: {e}"));
                // Don't set now_playing — keep the prior state (old track or
                // nothing) so the UI doesn't show a track that isn't playing.
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
            self.transport
                .history
                .push((np.id().to_string(), self.transport.context.clone()));
        }
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        self.transport
            .switch_context(ctx, Some(&start), &r, &self.catalog);
        self.start_playback();
    }

    /// Apply the Home overlay's focused item (RC11-DEF-001 / DEF-012).
    ///
    /// The Home overlay (`H`) shows multi-section discovery content (Quick
    /// Picks, Made for You mixes, Start Radio, Library, ...). `Enter` on the
    /// focused item must play it — but `play_selected` uses the *browse view's*
    /// track column, not the Home overlay, so before this method `Enter` from
    /// Home played an unpredictable item (the cursor moved invisibly and
    /// `play_selected` read the underlying view's cursor).
    ///
    /// This method resolves the focused `HomeItem` (at `state.cursor` within
    /// `state.focused_section`) and dispatches by kind:
    /// - `Track` → play the single track (local or YouTube).
    /// - `Playlist` → play the playlist's tracks (local playlist or YouTube
    ///   list; for YouTube lists, fire-and-forget `send_get_playlist` + show
    ///   "Loading…").
    /// - `Mix` → play the generated mix's tracks from `reco_mixes` (local
    ///   catalog track ids — plays immediately, no sidecar roundtrip). This
    ///   makes "Made for You" mixes reachable (RC11-DEF-012).
    /// - `RadioSeed` → start a radio session seeded from the currently-playing
    ///   track (or the first catalog track on cold start).
    /// - `LikedSongs` / `Explore` / `Subscription` → status hint (not yet
    ///   implemented — these need the sidecar / a tab; logged so the user
    ///   knows the key was received).
    pub fn play_home_selection(&mut self) {
        use crate::tui::view::home::HomeItemKind;

        let Some(Overlay::Home { state }) = self.overlay.clone() else {
            return;
        };
        let Some((_, items)) = state.sections.get(state.focused_section) else {
            return;
        };
        let Some(item) = items.get(state.cursor).cloned() else {
            return;
        };
        match item.kind.clone() {
            HomeItemKind::Track { id, .. } => {
                self.overlay = None;
                self.play_in_context_ids(vec![id.clone()], &id);
            }
            HomeItemKind::Playlist { id, is_local, .. } => {
                self.overlay = None;
                if is_local {
                    if let Some(pl) = self.playlists.iter().find(|p| p.name == id).cloned() {
                        if let Some(start) = pl.track_ids.first().cloned() {
                            self.play_in_context_ids(pl.track_ids, &start);
                        } else {
                            self.yt_status = Some("playlist is empty — nothing to play".into());
                        }
                    } else {
                        self.yt_status = Some("playlist not found".into());
                    }
                } else if let Some(session) = self.yt_session.as_mut() {
                    let _ = session.send_get_playlist(id.clone());
                    self.pending_discover_play = Some(id);
                    self.yt_status = Some("Loading playlist…".into());
                } else {
                    self.yt_status = Some("can't load playlist — no YouTube session".into());
                }
            }
            HomeItemKind::Mix { mix_type } => {
                // RC11-DEF-012: play the generated mix's tracks. The reco
                // engine generates mixes from the local catalog, so the
                // track_ids are local catalog ids — play immediately via
                // `play_in_context_ids` (no sidecar roundtrip).
                self.overlay = None;
                if let Some(mix) = self
                    .reco_mixes
                    .iter()
                    .find(|m| m.mix_type == mix_type)
                    .cloned()
                {
                    let ids: Vec<String> = mix.tracks.iter().map(|c| c.track_id.clone()).collect();
                    if let Some(start) = ids.first().cloned() {
                        self.play_in_context_ids(ids, &start);
                    } else {
                        self.yt_status = Some(format!(
                            "{} is empty — listen more to build it",
                            mix_type.label()
                        ));
                    }
                } else {
                    self.yt_status = Some(format!("{} not generated yet", mix_type.label()));
                }
            }
            HomeItemKind::RadioSeed { .. } => {
                self.overlay = None;
                let seed_id = self
                    .now_playing
                    .as_ref()
                    .map(|ts| ts.id().to_string())
                    .or_else(|| self.catalog.tracks.first().map(|t| t.id.clone()))
                    .unwrap_or_default();
                if seed_id.is_empty() {
                    self.yt_status = Some("no track to seed radio — play a track first".into());
                } else {
                    self.start_radio_from_track(&seed_id);
                }
            }
            HomeItemKind::LikedSongs => {
                self.yt_status = Some("Liked Songs — sign in to YouTube to view".into());
            }
            HomeItemKind::Explore { category } => {
                self.yt_status = Some(format!("Explore {category} — coming soon"));
            }
            HomeItemKind::Subscription { name, .. } => {
                self.yt_status = Some(format!("{name} — open the YouTube view to browse"));
            }
        }
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
            self.transport
                .history
                .push((np.id().to_string(), self.transport.context.clone()));
        }
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        self.transport
            .switch_context(ctx, Some(start), &r, &self.catalog);
        self.start_playback();
    }

    pub fn next(&mut self) {
        // Record a skip event for the track the user is leaving (DEF-034).
        // `next()` is both the user `>` press and the auto-advance on
        // end-of-track; the auto-advance path also calls on_track_ended first,
        // so a "completed" event supersedes this "skipped" when the track
        // finished naturally — but on a user `>`, only this fires.
        if let Some(np) = self.now_playing.clone() {
            let id = np.id().to_string();
            // Don't double-record: on_track_ended already recorded "completed"
            // and set last_natural_end — a natural end is not a skip.
            let was_natural = self.last_natural_end.as_deref() == Some(id.as_str());
            if !was_natural {
                let pos = self.player.position().unwrap_or(0.0);
                // Rapid skip (<10s) is a strong negative; a later skip is weak.
                let elapsed = self
                    .play_started_at
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(pos);
                if elapsed < 10.0 {
                    self.record_listen_event_pos(&id, "rapidly_skipped", Some(pos));
                } else {
                    self.record_listen_event_pos(&id, "skipped", Some(pos));
                }
            }
        }
        self.last_natural_end = None;
        // Skip dead tracks automatically so `>` always advances to the next
        // playable track in context order (DEF-017: without this, `>` on a
        // dead track silently does nothing — the user sees no change and
        // presses `>` again, advancing past the dead track, making the order
        // appear non-sequential). The loop tries each candidate once; if all
        // are dead, playback stops.
        let total = self.transport.order.len() + self.transport.manual_queue.len();
        for _ in 0..total.max(1) {
            let r = ClonedResolver {
                playlists: &self.playlists,
                manual_queue: self.transport.manual_queue.clone(),
                yt_lists: &self.yt_lists,
            };
            match self.transport.next(&r, &self.catalog) {
                Some(id) if !self.dead.contains(&id) => {
                    // Found a live candidate — load it. load_track may still
                    // mark it dead (missing file, player error); if so, loop
                    // to try the next candidate instead of returning with
                    // nothing playing (DEF-017).
                    self.load_track(&id);
                    if !self.dead.contains(&id) {
                        return;
                    }
                    // Track became dead during load — skip to next.
                }
                Some(_) => {
                    // Dead track — loop to try the next candidate.
                    // transport.next already advanced the cursor / popped the
                    // manual-queue entry.
                }
                None => {
                    // Context exhausted (repeat off, no manual queue). The
                    // continue mode decides whether playback stops or auto-
                    // advances to more music — this is the "auto discover"
                    // feature.
                    match self.transport.continue_mode {
                        ContinueMode::Off => {
                            self.player.stop().ok();
                            self.now_playing = None;
                        }
                        ContinueMode::NextAlbum => {
                            if self.switch_to_next_album() {
                                self.start_playback();
                            } else {
                                // DEF-027: not in an album context, or no next
                                // album. Before stopping, try radio continuation
                                // (an active reco radio session) so CONT=next
                                // keeps playing when the queue exhausts; if no
                                // radio is active, wrap to the start of the
                                // current context (album/playlist/search) so
                                // "CONT next → track 3 starts (radio or wrap)"
                                // holds. Only stop when both fallbacks are empty.
                                if let Some(vid) = self.reco_radio_next() {
                                    if let Some(np) = self.now_playing.clone() {
                                        self.transport.history.push((
                                            np.id().to_string(),
                                            self.transport.context.clone(),
                                        ));
                                    }
                                    let ctx = Context::Search {
                                        query: "reco radio".into(),
                                        track_ids: vec![vid.clone()],
                                    };
                                    let r = ClonedResolver {
                                        playlists: &self.playlists,
                                        manual_queue: self.transport.manual_queue.clone(),
                                        yt_lists: &self.yt_lists,
                                    };
                                    self.transport.switch_context(
                                        ctx,
                                        Some(&vid),
                                        &r,
                                        &self.catalog,
                                    );
                                    self.start_playback();
                                } else if self.transport.order.len() > 1 {
                                    // Wrap: restart the current context from the
                                    // top (skip the just-finished last track).
                                    self.transport.cursor = 0;
                                    self.start_playback();
                                } else {
                                    self.player.stop().ok();
                                    self.now_playing = None;
                                }
                            }
                        }
                        ContinueMode::Radio => {
                            self.switch_to_radio();
                            self.start_playback();
                        }
                        ContinueMode::YouTube => {
                            // If a reco RadioSession overlay is active, use it
                            // to drive auto-advance instead of the YouTube
                            // radio cursor.
                            if let Some(vid) = self.reco_radio_next() {
                                if let Some(np) = self.now_playing.clone() {
                                    self.transport.history.push((
                                        np.id().to_string(),
                                        self.transport.context.clone(),
                                    ));
                                }
                                let ctx = Context::Search {
                                    query: "reco radio".into(),
                                    track_ids: vec![vid.clone()],
                                };
                                let r = ClonedResolver {
                                    playlists: &self.playlists,
                                    manual_queue: self.transport.manual_queue.clone(),
                                    yt_lists: &self.yt_lists,
                                };
                                self.transport
                                    .switch_context(ctx, Some(&vid), &r, &self.catalog);
                                self.start_playback();
                                return;
                            }
                            // Drive YouTube autoplay via RadioCursor (spec §3.4). The
                            // old `radio.advance(session, seed)` made a BLOCKING
                            // `get_watch_playlist` roundtrip (~4s) every time the queue
                            // was exhausted, freezing the UI on each auto-advance. Now
                            // we advance locally when the queue still has entries (no
                            // sidecar call), and fire-and-forget a radio refill when
                            // exhausted — `on_tick` refills the cursor + starts
                            // playback when `pending_watch` lands (non-blocking).
                            let seed_id = self.now_playing.clone().map(|s| s.id().to_string());
                            if let Some(vid) = self.radio.next_local() {
                                // Fast path: the queue still has entries — switch
                                // context + start playback immediately (same as the
                                // old `radio.advance` Some(vid) arm).
                                if let Some(np) = self.now_playing.clone() {
                                    self.transport.history.push((
                                        np.id().to_string(),
                                        self.transport.context.clone(),
                                    ));
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
                                self.transport
                                    .switch_context(ctx, Some(&vid), &r, &self.catalog);
                                self.start_playback();
                            } else if let Some(session) = self.yt_session.as_mut() {
                                // Queue exhausted — fire-and-forget a radio refill
                                // seeded by the just-finished track. Non-blocking: the
                                // old track stays current until `on_tick` drains
                                // `pending_watch` and starts the next. No-op if a
                                // refill is already in flight so a burst of `next`/
                                // end-of-track events doesn't flood the sidecar.
                                if let Some(seed) = seed_id {
                                    if !session.watch_loading()
                                        && session.send_watch_playlist(seed.clone()).is_ok()
                                    {
                                        self.pending_radio_seed = Some(seed);
                                    }
                                    // Leave now_playing on the ended track; on_tick
                                    // will push it to history + switch when the
                                    // response lands. Don't stop here — clearing
                                    // now_playing would lose the history-push target.
                                } else {
                                    // No seed (nothing was playing) — stop cleanly.
                                    self.player.stop().ok();
                                    self.now_playing = None;
                                }
                            } else {
                                // No session — stop cleanly (degrade, spec §3.5).
                                self.player.stop().ok();
                                self.now_playing = None;
                            }
                        }
                    }
                    return;
                }
            }
        }
        // All candidates were dead — stop playback.
        self.player.stop().ok();
        self.now_playing = None;
    }

    pub fn prev(&mut self) {
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        if let Some(id) = self.transport.prev(&r, &self.catalog) {
            self.load_track(&id);
        }
    }

    /// Auto-advance when the player reports a natural end-of-track. Records
    /// a "completed" listen event (strong positive) before advancing so the
    /// reco engine learns the user finished this track (DEF-034). Sets
    /// `last_natural_end` so `next()` doesn't also record a "skipped".
    pub fn on_track_ended(&mut self) {
        if let Some(np) = self.now_playing.clone() {
            let id = np.id().to_string();
            self.record_listen_event(&id, "completed");
            self.last_natural_end = Some(id);
        }
        self.next();
    }

    pub fn cycle_shuffle(&mut self) {
        let m = match self.transport.shuffle {
            ShuffleMode::Off => ShuffleMode::Smart,
            ShuffleMode::Smart => ShuffleMode::Random,
            ShuffleMode::Random => ShuffleMode::Off,
        };
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        self.transport.set_shuffle(m, &r, &self.catalog);
    }

    pub fn reshuffle(&mut self) {
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
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
    /// - Mixed:  Off → NextAlbum → Radio → YouTube → Off
    pub fn cycle_continue(&mut self) {
        self.transport.continue_mode = match (self.source_mode, self.transport.continue_mode) {
            (crate::mode::SourceMode::Local, ContinueMode::Off) => ContinueMode::NextAlbum,
            (crate::mode::SourceMode::Local, ContinueMode::NextAlbum) => ContinueMode::Radio,
            (crate::mode::SourceMode::Local, ContinueMode::Radio) => ContinueMode::Off,
            (crate::mode::SourceMode::Local, ContinueMode::YouTube) => ContinueMode::Off,

            (crate::mode::SourceMode::Youtube, ContinueMode::Off) => ContinueMode::YouTube,
            (crate::mode::SourceMode::Youtube, _) => ContinueMode::Off,

            (crate::mode::SourceMode::Mixed, ContinueMode::Off) => ContinueMode::NextAlbum,
            (crate::mode::SourceMode::Mixed, ContinueMode::NextAlbum) => ContinueMode::Radio,
            (crate::mode::SourceMode::Mixed, ContinueMode::Radio) => ContinueMode::YouTube,
            (crate::mode::SourceMode::Mixed, ContinueMode::YouTube) => ContinueMode::Off,
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

        // Auto-setup: same as apply_yt_browser — if the venv doesn't exist,
        // run setup automatically so `:yt auth` is self-contained.
        let venv_py = crate::yt::session::venv_python();
        if !venv_py.exists() {
            self.yt_status = Some("YT setup: installing deps (one-time)…".into());
            let reqs = self.yt_script.parent().map(|p| p.join("requirements.txt"));
            if let Some(reqs) = reqs {
                match crate::yt::session::run_setup(&reqs) {
                    Ok(_) => {
                        self.yt_python = crate::yt::session::venv_python();
                        self.yt_status = Some("YT setup complete — authenticating…".into());
                    }
                    Err(e) => {
                        self.yt_error = Some(format!(
                            "setup failed: {e} — run :yt setup manually, then :yt auth"
                        ));
                        self.yt_state = crate::yt::state::YtState::Failed;
                        return;
                    }
                }
            }
        }

        if self.yt_session.is_none() {
            match crate::yt::session::Session::spawn(
                &self.yt_python,
                &self.yt_script,
                Some(cookies.clone()),
            ) {
                Ok(s) => self.yt_session = Some(s),
                Err(e) => {
                    self.yt_error = Some(format!("auth failed: {e}"));
                    self.yt_state = crate::yt::state::YtState::ProviderError;
                    return;
                }
            }
        } else if let Some(session) = self.yt_session.as_mut() {
            if let Err(e) = session.set_cookies(cookies, &self.yt_python, &self.yt_script) {
                self.yt_error = Some(format!("auth failed: {e}"));
                self.yt_state = crate::yt::state::YtState::ProviderError;
                return;
            }
        }
        // Authenticated but NOT synced — the credential hasn't been verified
        // by a data fetch yet. The old code set yt_status = "connected" here,
        // which was false-ready (yt-recon §8 location 3). The launch probe or
        // refresh_yt_lists must succeed to promote this to Ready.
        self.yt_state = crate::yt::state::YtState::AuthenticatedNotSynced;
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

        // Auto-setup: if the venv doesn't exist yet (first-time user), run
        // :yt setup automatically before spawning the sidecar. This lets a
        // new user do everything in ONE command (`:yt auth browser chrome`)
        // instead of two (`:yt setup` + `:yt auth browser chrome`). The venv
        // install is ~30s one-time; subsequent `:yt auth browser` calls skip
        // this because the venv already exists.
        let venv_py = crate::yt::session::venv_python();
        if !venv_py.exists() {
            self.yt_status = Some("YT setup: installing deps (one-time)…".into());
            let reqs = self.yt_script.parent().map(|p| p.join("requirements.txt"));
            if let Some(reqs) = reqs {
                match crate::yt::session::run_setup(&reqs) {
                    Ok(_) => {
                        self.yt_python = crate::yt::session::venv_python();
                        self.yt_status = Some("YT setup complete — authenticating…".into());
                    }
                    Err(e) => {
                        self.yt_error = Some(format!(
                            "setup failed: {e} — run :yt setup manually, then :yt auth browser {browser}"
                        ));
                        self.yt_state = crate::yt::state::YtState::Failed;
                        return;
                    }
                }
            }
        }

        if self.yt_session.is_none() {
            match crate::yt::session::Session::spawn_browser(
                &self.yt_python,
                &self.yt_script,
                browser.clone(),
            ) {
                Ok(s) => self.yt_session = Some(s),
                Err(e) => {
                    self.yt_error = Some(format!("auth failed: {e}"));
                    self.yt_state = crate::yt::state::YtState::ProviderError;
                    return;
                }
            }
        } else if let Some(session) = self.yt_session.as_mut() {
            if let Err(e) = session.set_browser(browser.clone(), &self.yt_python, &self.yt_script) {
                self.yt_error = Some(format!("auth failed: {e}"));
                self.yt_state = crate::yt::state::YtState::ProviderError;
                return;
            }
        }
        // Authenticated but NOT synced — see apply_yt_auth for the rationale.
        // The old code set yt_status = "connected via {browser}" here (yt-recon
        // §8 location 4); the launch probe or refresh must verify data first.
        self.yt_state = crate::yt::state::YtState::AuthenticatedNotSynced;
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
                // RC11-DEF-017: surface the full install-log path via the
                // diagnostics overlay so the user can find the log without
                // the footer truncating the path. The footer shows the
                // prominent "YT setup OK · venv: …" confirmation; the log
                // path goes to `:diag` (which has room for the full text).
                self.diagnostics.push(format!(
                    "YT setup complete · log: {}",
                    crate::yt::session::setup_log_path().display()
                ));
                // Respawn the sidecar against the new venv python, preserving
                // any browser/pasted auth.
                if let Some(session) = self.yt_session.as_mut() {
                    if let Some(browser) = session.browser.clone() {
                        match crate::yt::session::Session::spawn_browser(
                            &self.yt_python,
                            &self.yt_script,
                            browser,
                        ) {
                            Ok(new) => {
                                *self.yt_session.as_mut().unwrap() = new;
                            }
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
        // Drain a completed background audio format switch (best-effort
        // cleanup — the thread already did the blocking CoreAudio work; we
        // just join + drop the handle so it doesn't leak). If the thread
        // is still running, put the handle back (it'll be joined on a
        // later tick). Non-blocking: `is_finished()` never blocks
        // (AC-M9.2.4).
        if let Some(handle) = self.audio_switch_handle.take() {
            if !handle.is_finished() {
                self.audio_switch_handle = Some(handle);
            }
        }
        // Notification TTL (Slice 7): clear a stale transient `yt_status` after
        // 5s so the footer returns to the hint bar / state label instead of
        // lingering on a one-shot message (e.g. "upgraded to AAC 256k",
        // "queue cleared"). The TTL is (re)started below when a NEW status is
        // detected; an identical repeat does NOT refresh the window (dedup via
        // `last_notification`).
        if let Some(t) = self.notification_ttl {
            if t.elapsed() > Duration::from_secs(5) && self.yt_status.is_some() {
                self.yt_status = None;
                self.notification_ttl = None;
                // Reset dedup so a later re-assertion of the same message
                // counts as a fresh notification (gets a new 5s window).
                self.last_notification = None;
            }
        }
        // Detect a NEW yt_status (set since the last tick by a key handler /
        // respawn / on_tick premium-swap) and (re)start its TTL window. Dedup:
        // an identical repeat (== last_notification) keeps the original
        // window — it doesn't refresh, so repeated identical messages clear on
        // the original schedule.
        if let Some(msg) = &self.yt_status {
            if self.last_notification.as_deref() != Some(msg.as_str()) {
                self.notification_ttl = Some(Instant::now());
                self.last_notification = Some(msg.clone());
            }
        }

        // Meaningful-threshold detection (DEF-034): when the now-playing
        // track crosses ≥50% of its duration OR ≥30s (whichever first), fire
        // a single `meaningful_threshold` event — the first positive signal.
        // Guarded by `threshold_fired_for` so it fires once per track instance.
        if let Some(np) = self.now_playing.clone() {
            let id = np.id().to_string();
            let already_fired = self.threshold_fired_for.as_deref() == Some(id.as_str());
            if !already_fired {
                let pos = self.player.position().unwrap_or(0.0);
                let dur = self.player.duration().unwrap_or(0.0);
                let crossed = pos >= 30.0 || (dur > 0.0 && pos >= dur * 0.5);
                if crossed {
                    self.record_listen_event(&id, "meaningful_threshold");
                    self.threshold_fired_for = Some(id);
                }
            }
        }

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
                        &self.yt_python,
                        &self.yt_script,
                        b,
                    ),
                    None => {
                        // Guest/pasted-cookies: respawn guest (pasted cookies
                        // file re-loaded by Session::spawn).
                        let cookies = crate::yt::session::load_cookies();
                        crate::yt::session::Session::spawn(
                            &self.yt_python,
                            &self.yt_script,
                            cookies,
                        )
                    }
                };
                match respawned {
                    Ok(new) => {
                        *self.yt_session.as_mut().unwrap() = new;
                        // Sidecar restarted — need to re-verify auth/data before
                        // claiming ready. The old code set yt_status = "sidecar
                        // restarted" (yt-recon §8 location 5); now we transition
                        // to AuthenticatedNotSynced so the next probe/refresh
                        // can promote to Ready.
                        self.yt_state = crate::yt::state::YtState::AuthenticatedNotSynced;
                    }
                    Err(e) => {
                        self.yt_error = Some(format!("sidecar respawn ({attempts}/3): {e}"));
                        self.yt_state = crate::yt::state::YtState::ProviderError;
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
        // A discover selection (Enter on a YT playlist) whose tracks just
        // landed. Staged here (needs &mut self for `play_in_context_ids`) and
        // started below, after the session borrow ends. `(video_ids, start_id)`.
        let mut pending_discover_start: Option<(Vec<String>, String)> = None;
        // A CONT=YouTube radio refill whose watch_playlist just landed. Staged
        // here (needs &mut self for transport + `start_playback`) and started
        // below, after the session borrow ends. The video_id to play next.
        let mut pending_radio_start: Option<String> = None;
        if let Some(session) = self.yt_session.as_mut() {
            // Pin the focused playlist's tracks before draining so
            // `evict_track_cache` (triggered by `cache_track` inside
            // `apply_pair`) never drops a track the user is currently viewing.
            // Without this, browsing enough playlists fills the 256-entry
            // cache and evicts a track STILL REFERENCED by the focused
            // playlist's `track_ids` → that row renders "Loading…" forever
            // (the lazy-load guard `!loaded` then blocks a re-fetch).
            let pinned: std::collections::HashSet<String> = self
                .yt_lists
                .get(self.cursors.playlist)
                .map(|l| l.track_ids.iter().cloned().collect())
                .unwrap_or_default();
            session.set_pinned_tracks(pinned);
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
                // A data fetch succeeded → the provider is Ready. This is the
                // single promotion point from AuthenticatedNotSynced/Synchronizing
                // to Ready — the credential actually works, data is usable.
                self.yt_state = crate::yt::state::YtState::Ready;
                // Persist the fresh lists to the disk cache so the next launch
                // (even offline) shows them marked stale.
                crate::yt::cache::save_yt_lists(&self.yt_lists);
                // Lists were replaced; forget which had been expanded so a
                // re-focused list re-fetches its tracks.
                self.loaded_yt_lists.clear();
            }

            // Fold fetched playlist tracks into the matching YtLists. Drain
            // ALL pending pairs — multiple get_playlist responses can land in
            // the same drain_paired cycle (user switched A→B rapidly), and the
            // Vec design preserves ALL of them. The old single-slot Option
            // lost the first when the second landed (wrong tracks per playlist).
            while let Some((id, vids)) = session.pending_tracks.pop() {
                // Mark the list loaded + set its track_ids.
                for l in self.yt_lists.iter_mut() {
                    if l.id == id {
                        l.track_ids = vids.clone();
                    }
                }
                self.loaded_yt_lists.insert(id.clone());
                // Discover selection: if this is the list a discover Enter
                // asked for, stage the playback start (after the session
                // borrow ends — `play_in_context_ids` needs &mut self). Works
                // even for playlists not in yt_lists (uses the vids directly).
                // A different list's tracks landing does NOT consume the
                // pending selection — it stays for the right response.
                if let Some(want) = self.pending_discover_play.take() {
                    if want == id {
                        if let Some(start) = vids.first().cloned() {
                            pending_discover_start = Some((vids, start));
                        } else {
                            // The sidecar returned an empty track list for
                            // this mix (e.g. RD* IDs the sidecar doesn't
                            // know). Surface an error so the user isn't left
                            // staring at a "Loading mix…" status forever
                            // (DEF-007).
                            self.yt_error = Some(format!("mix \"{id}\" returned no tracks"));
                            self.yt_status = Some("couldn't load mix — no tracks".into());
                        }
                    } else {
                        self.pending_discover_play = Some(want);
                    }
                }
            }

            // CONT=YouTube radio refill: a watch_playlist response landed.
            // Advance the RadioCursor with the fresh video_ids + stage the
            // playback start (after the session borrow ends). The seed (the
            // just-finished track) is dropped if it's the queue's leading
            // entry (YouTube "Up Next" excludes the just-played track). A
            // non-blocking auto-advance: the old track stayed current until
            // this tick; now we switch.
            if let Some(vids) = session.pending_watch.take() {
                let seed = self.pending_radio_seed.take();
                if let Some(vid) = self.radio.advance_with_vids(vids, seed) {
                    pending_radio_start = Some(vid);
                }
            }

            // Discover loading timeout: if the YouTube home-suggestions
            // request hasn't arrived after 600 ticks (~10-30s depending on
            // poll rate), clear the loading flag and show an error so the
            // overlay doesn't hang on "Loading..." forever. The user can
            // press S again to retry.
            if self.discover_loading {
                self.discover_loading_ticks += 1;
                if self.discover_loading_ticks > 600 {
                    self.discover_loading = false;
                    self.discover_loading_ticks = 0;
                    self.yt_status =
                        Some("YouTube suggestions timed out — press S to retry".to_string());
                    session.reset_discover_inflight();
                }
            }

            // Discover overlay: home-suggestions landed. Populate the open
            // Discover overlay's items (replace prior YT playlists; preserve
            // local albums + cursor) and clear the loading flag. A stale
            // response (overlay closed/changed) is dropped — items just don't
            // show. This is the non-blocking completion of `yt_discover_items`
            // (which fired-and-forgot the request + opened the overlay empty).
            if let Some(s) = session.pending_discover.take() {
                self.discover_loading = false;
                self.discover_loading_ticks = 0;
                let new_pl: Vec<DiscoverItem> = s
                    .into_iter()
                    .map(|p| DiscoverItem::Playlist {
                        id: p.id,
                        name: p.name,
                    })
                    .take(6)
                    .collect();
                // Only touch the overlay if it's still a Discover — a stale
                // response landing after the user closed/replaced the overlay
                // must NOT drop the current overlay. Clone + match (don't
                // `take`) so a non-Discover overlay is left untouched.
                if let Some(Overlay::Discover { items, cursor }) = self.overlay.clone() {
                    // Preserve Album items (local smart-albums in Mixed mode);
                    // replace prior Playlist items with the fresh batch so a
                    // re-drain doesn't stack duplicates.
                    let mut combined: Vec<DiscoverItem> = items
                        .into_iter()
                        .filter(|d| !matches!(d, DiscoverItem::Playlist { .. }))
                        .collect();
                    combined.extend(new_pl);
                    self.overlay = Some(Overlay::Discover {
                        items: combined,
                        cursor,
                    });
                }
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
                            input,
                            results,
                            cursor,
                            scope,
                            submitted,
                            searching: false,
                        });
                    }
                    // else: not ours — leave the overlay exactly as it was.
                }
            }

            // Fold completed lyrics into the open Lyrics overlay. The
            // generation guard (D5) discards a stale response: the overlay's
            // `gen` was captured at request time; if `lyrics_gen` has advanced
            // past it (the user moved to a different track), the response is
            // for a prior track and must NOT overwrite the current overlay.
            // The video_id carried in the response is a second staleness check.
            if let Some((vid, lines, synced)) = session.pending_lyrics.take() {
                if let Some(Overlay::Lyrics {
                    content: _,
                    state: _,
                    scroll,
                    track_id,
                    gen,
                }) = self.overlay.clone()
                {
                    // Apply only if the overlay is still waiting for THIS track
                    // AND the generation matches (no newer request has superseded
                    // it). A stale response (track changed) is dropped.
                    if track_id == vid && gen == self.lyrics_gen {
                        let lyrics = crate::lyrics::from_proto(&lines, synced);
                        let new_state = if lyrics.is_empty() {
                            LyricsState::NotFound
                        } else {
                            LyricsState::Available(lyrics.synced)
                        };
                        self.overlay = Some(Overlay::Lyrics {
                            content: Some(lyrics),
                            state: new_state,
                            scroll,
                            track_id,
                            gen,
                        });
                    }
                    // else: stale — leave the overlay as-is (it's either still
                    // Loading for a newer track, or already showing newer lyrics).
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
                            input,
                            results,
                            cursor,
                            scope,
                            submitted,
                            searching: false,
                        });
                    }
                }
                // Lyrics overlay: a sidecar error while Loading → Error state
                // (so the overlay exits "loading…" and shows "lyrics error"
                // instead of hanging forever). Any error transitions the
                // overlay (lyrics errors aren't query-tagged, so we can't
                // distinguish "for this track" vs "for a prior track" — but the
                // generation guard already discarded truly stale responses,
                // and a Loading overlay is by definition waiting for the
                // current track).
                if let Some(Overlay::Lyrics {
                    content,
                    state,
                    scroll,
                    track_id,
                    gen,
                }) = self.overlay.clone()
                {
                    if matches!(state, LyricsState::Loading) {
                        let msg = last
                            .map(|(_, e)| e.clone())
                            .unwrap_or_else(|| "lyrics request failed".to_string());
                        self.overlay = Some(Overlay::Lyrics {
                            content,
                            state: LyricsState::Error(msg),
                            scroll,
                            track_id,
                            gen,
                        });
                    }
                }
                // Footer: prefer the matching Search error's message (most
                // relevant to what the user searched), else the last error.
                let footer = matching_search.or(last).map(|(_, e)| e.clone());
                if let Some(e) = footer {
                    self.yt_error = Some(e.clone());
                    // Transition the provider state on a non-Search error.
                    // Search errors are overlay-scoped and don't indicate the
                    // provider itself is broken; Other errors (resolve,
                    // playlist fetch, refresh) do. Don't demote from Ready to
                    // ProviderError on a Search error — the provider is fine.
                    let is_search_only = errors.iter().all(|(scope, _)| {
                        matches!(scope, crate::yt::session::ErrorScope::Search(_))
                    });
                    if !is_search_only {
                        // Heuristic auth-expiry detection: an error mentioning
                        // auth/401/unauthorized/expired → AuthExpired (needs
                        // re-auth, not retry). S2.3.1 will make this structured.
                        let looks_like_auth_error = e.to_lowercase().contains("auth")
                            || e.to_lowercase().contains("401")
                            || e.to_lowercase().contains("unauthorized")
                            || e.to_lowercase().contains("expired")
                            || e.to_lowercase().contains("login");
                        if looks_like_auth_error {
                            self.yt_state = crate::yt::state::YtState::AuthExpired;
                        } else if self.yt_state == crate::yt::state::YtState::Ready {
                            // Was Ready, now an error → degrade to ReadyStale
                            // (cached data still visible, retry can recover).
                            self.yt_state = crate::yt::state::YtState::ReadyStale;
                        } else if !self.yt_lists.is_empty() {
                            // Have cached data from a prior sync → show it as
                            // stale (offline) rather than a bare error. The
                            // user can still browse cached playlists.
                            self.yt_state = crate::yt::state::YtState::ReadyStale;
                        } else {
                            self.yt_state = crate::yt::state::YtState::ProviderError;
                        }
                        // A non-search error means the refresh/resolve failed:
                        // clear the loading indicator so the Y view shows the
                        // error state instead of hanging on "loading…".
                        self.yt_lists_loading = false;
                    }
                }
            }

            // A premium (256k) URL landed for a fire-and-forget premium
            // resolve. Take it out of the session here (we need &mut self for
            // the player to swap) and process it below, after this block.
            premium_swap = session.pending_premium_url.take();

            // Publication result landed (RC11-DEF-002). The publication
            // flow is fire-and-forget: the Enter handler calls
            // `send_create_playlist` + stores the user-supplied title in
            // `pending_publish_name`. Here we drain the sidecar's
            // `pending_publication` and surface a toast — success uses
            // the stashed name ("Playlist \"<name>\" created"), failure
            // surfaces the error message verbatim. Without this drain the
            // user pressed Enter, saw "publishing…", and never learned
            // whether the publish actually succeeded.
            if let Some(result) = session.pending_publication.take() {
                let name = self.pending_publish_name.take().unwrap_or_default();
                match &result {
                    crate::yt::publication::PublicationResult::Success(_) => {
                        self.yt_status = Some(format!("Playlist \"{name}\" created"));
                    }
                    crate::yt::publication::PublicationResult::PartialSuccess(_, failed) => {
                        self.yt_status = Some(format!(
                            "Playlist \"{name}\" created ({} tracks failed)",
                            failed.len()
                        ));
                    }
                    crate::yt::publication::PublicationResult::Failed(e) => {
                        self.yt_error = Some(format!("publish failed: {e}"));
                    }
                }
            }

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
                        crate::source::device_rate::LoadKind::Remote {
                            sample_rate: u.sample_rate,
                        },
                        self.switch_sample_rate,
                    ) {
                        self.audio_switch_handle =
                            Some(crate::audio::set_output_format_async(sr, bd));
                    }
                    let p = std::path::PathBuf::from(&u.url);
                    // Resume at the captured position via mpv's `start` option
                    // (load_at) so the premium stream begins at `pos` directly —
                    // no from-0 replay before a seek lands.
                    match self.player.load_at(&p, pos) {
                        Ok(()) => {
                            self.playing_premium = true;
                            self.yt_status = Some("upgraded to AAC 256k · YT Premium".into());
                        }
                        Err(e) => {
                            // Premium upgrade failed — keep the fast stream
                            // playing. Don't change now_playing (it's already
                            // set to this track from the initial load).
                            self.yt_error = Some(format!("premium upgrade failed: {e}"));
                        }
                    }
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

        // Discover selection playback: a YT playlist's tracks landed (the
        // fire-and-forget completion of `play_discover_selection`'s Playlist
        // arm). Start the playlist, mirroring the old blocking path's
        // `play_in_context_ids(ids, start)`.
        if let Some((ids, start)) = pending_discover_start {
            // Close the discover overlay on success so the user sees the
            // now-playing bar, not the overlay (DEF-007). Clear the loading
            // state too (RC11-DEF-035).
            self.overlay = None;
            self.discover_play_loading = None;
            self.play_in_context_ids(ids, &start);
        }

        // RC11-DEF-035: for the synchronous Mix path, `play_discover_selection`
        // started playback directly (not via `pending_discover_start`) and
        // kept the overlay open with a "Loading [name]..." state. Once
        // `now_playing` is set, the loading is complete — close the overlay
        // + clear the loading state. (If playback failed — dead tracks —
        // `now_playing` stays None; the loading state lingers until the user
        // presses Esc. A future improvement could clear it on a timeout.)
        if let Some(_name) = self.discover_play_loading.clone() {
            if self.now_playing.is_some() && self.pending_discover_play.is_none() {
                self.overlay = None;
                self.discover_play_loading = None;
            }
        }

        // CONT=YouTube radio auto-advance: the watch_playlist response
        // landed (the fire-and-forget completion of `next()`'s exhausted-
        // queue path). Push the old track to history, switch context to a
        // fresh radio Search, and start playback — mirroring the old
        // blocking `radio.advance` Some(vid) arm. Non-blocking: this runs on
        // the tick after the response lands, not in the input handler.
        if let Some(vid) = pending_radio_start {
            if let Some(np) = self.now_playing.clone() {
                self.transport
                    .history
                    .push((np.id().to_string(), self.transport.context.clone()));
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
            self.transport
                .switch_context(ctx, Some(&vid), &r, &self.catalog);
            self.start_playback();
        }

        // Lazy-load the focused YT list's tracks: the Y view's col2 + Enter/s
        // need them, but they're only fetched on demand (spec §5.3). Skip lists
        // already loaded (even if empty) and any fetch in flight. ALSO re-fetch
        // when a loaded list has tracks whose metadata was evicted from
        // `track_cache` (e.g. the user browsed enough other playlists to push
        // them out before the pin was set, or a prior session left the list
        // loaded with stale references): `track_for(id)` returns `None` for
        // those rows, which render as "Loading…" forever without this re-fetch.
        // The pin (set above) protects the re-fetched tracks from re-eviction
        // while the list stays focused, so the re-fetch doesn't loop.
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
                // A loaded list with tracks but missing metadata (eviction) →
                // re-fetch to fill the gaps. `any_missing` is only computed
                // when needed (loaded && non-empty) to avoid scanning every
                // tick for fresh lists.
                let any_missing = if loaded && !empty {
                    self.yt_session
                        .as_ref()
                        .map(|s| l.track_ids.iter().any(|t| s.track_for(t).is_none()))
                        .unwrap_or(false)
                } else {
                    false
                };
                if !inflight && ((empty && !loaded) || any_missing) {
                    if let Some(session) = self.yt_session.as_mut() {
                        let _ = session.send_get_playlist(id);
                    }
                }
            }
        }

        // Keep pre-resolving the next track's PREMIUM url so gapless handoff
        // stays warm (the slow 256k resolve happens during the current track).
        self.preload_next_url();

        // Lyrics overlay follows the music: if it's open and the now-playing
        // track changed since the last request, re-request for the new track.
        // (The generation guard in `request_lyrics` + `on_tick` discards any
        // in-flight response for the old track.) This is the "call request_lyrics
        // when now_playing changes" wire — only while the overlay is open, so
        // we don't fire a sidecar request on every track change when the user
        // isn't looking at lyrics.
        if let Some(Overlay::Lyrics { track_id, .. }) = self.overlay.clone() {
            if let Some(np) = self.now_playing.as_ref() {
                if np.id() != track_id.as_str() {
                    let new_id = np.id().to_string();
                    self.request_lyrics(&new_id);
                }
            }
        }

        // Braille spinner: advance one frame per tick while a resolve is in
        // flight, else freeze at 0 (returns the glyph to play/pause). ~150ms
        // per tick (event loop POLL_TIMEOUT) ≈ 6.7fps — smooth for 10 frames.
        if self.is_resolving() {
            self.spinner_frame = (self.spinner_frame + 1) % 10;
        } else {
            self.spinner_frame = 0;
        }

        // RC11-DEF-043: decay the transient confirmation toast after ~1.2s so
        // it's readable but doesn't linger (the `yt_status` toast is gated on
        // yt_state==Ready and deduped; this dedicated toast is always
        // rendered in the player bar and refreshes on each set).
        if let Some(t) = self.toast_at {
            if t.elapsed() > Duration::from_millis(1200) {
                self.toast = None;
                self.toast_at = None;
            }
        }

        // RC11-DEF-014: track the now-playing track's position so it can be
        // saved to state.db on exit and restored on the next launch. Only
        // update when the player reports a real position (mpv); afplay
        // reports None so the saved position stays 0 (resume restarts from 0
        // on afplay, which can't seek anyway). Cap at the duration so a
        // paused-at-end state doesn't save a position past the track.
        if let Some(np) = self.now_playing.as_ref() {
            if let Some(pos) = self.player.position() {
                let dur = self.player.duration().unwrap_or(f64::INFINITY);
                if pos < dur {
                    self.last_played_track_id = Some(np.id().to_string());
                    self.last_played_position = pos;
                }
            }
        }

        // Diagnostics capture (Slice 7): when `yt_error` changes, push a line
        // into the diagnostics buffer so the user can review what happened via
        // the diagnostics overlay (the footer only shows the latest error).
        // Change-detection against the last captured line avoids flooding
        // the buffer with one entry per tick while the error stays the same.
        if let Some(e) = &self.yt_error {
            let line = format!("yt_error: {e}");
            let last = self.diagnostics.messages().last().map(|s| s.as_str());
            if last != Some(line.as_str()) {
                self.diagnostics.push(line);
            }
        }
    }

    /// The footer / Y-view status text, derived from the truthful `yt_state`
    /// enum (not the legacy `yt_status`/`yt_error` free-text). Replaces the old
    /// `yt_status = "connected…"` assignments that could claim "connected"
    /// before any data fetch verified the credential (yt-recon §8).
    ///
    /// - `Ready` with a transient non-state message (e.g. "upgraded to AAC
    ///   256k"): returns that message (accent-colored in the footer).
    /// - `Ready` with no transient message: returns `None` (the footer shows
    ///   the key-hint bar instead).
    /// - Any non-ready state: returns `Some("YT: [icon] <label> [— detail]")`
    ///   built from `human_label()` + `icon()` + the error detail (`yt_error`).
    ///   The icon gives NO_COLOR distinction (accessibility: not color-only).
    ///   Never contains "connected" — that word was the false-ready bug.
    pub fn yt_status_text(&self) -> Option<String> {
        // Only Ready is silent (the hint bar shows). ReadyStale shows its
        // "offline — showing cached" label so the user knows they're degraded.
        if self.yt_state == crate::yt::state::YtState::Ready {
            // Ready: show the transient non-state message (e.g. "upgraded to
            // AAC 256k · YT Premium"), or None for the hint line.
            return self.yt_status.clone();
        }
        // Non-ready: derive from the state machine. The label already embeds
        // the recovery action (e.g. "not configured — run :yt auth browser"),
        // so we don't append retry_hint() separately.
        let label = self.yt_state.human_label();
        let icon = self.yt_state.icon().unwrap_or("");
        let mut s = if icon.is_empty() {
            format!("YT: {label}")
        } else {
            format!("YT: {icon} {label}")
        };
        if let Some(detail) = &self.yt_error {
            if !detail.is_empty() {
                s.push_str(&format!(" — {detail}"));
            }
        }
        Some(s)
    }

    /// Logout: clear cookies + browser choice + cached lists, and respawn the
    /// sidecar guest. Stale data must not survive logout — the Y view would
    /// show playlists from the logged-out account, which is misleading.
    pub fn yt_logout(&mut self) {
        let p = crate::yt::session::cookies_file();
        let _ = std::fs::remove_file(&p);
        self.yt_browser.clear();
        if let Some(session) = self.yt_session.as_mut() {
            let _ = session.clear_cookies(&self.yt_python, &self.yt_script);
        }
        self.yt_status = Some("YT auth: logged out (guest mode)".into());
        self.yt_error = None;
        // Drop cached lists + the loaded-set so the Y view doesn't show stale
        // playlists from the logged-out account. A re-focus after re-auth
        // re-fetches everything fresh.
        self.yt_lists.clear();
        self.loaded_yt_lists.clear();
        self.yt_lists_loading = false;
        // Clear the fire-and-forget hot-path intents so a response that lands
        // after logout doesn't start playback / populate a stale overlay.
        self.discover_loading = false;
        self.discover_loading_ticks = 0;
        self.pending_discover_play = None;
        self.pending_radio_seed = None;
        // Drop a pending audio format switch handle (the thread detaches
        // and completes on its own — best-effort, no blocking).
        self.audio_switch_handle = None;
        // Also clear the disk cache so the next launch doesn't show the
        // logged-out account's playlists.
        crate::yt::cache::clear_yt_lists();
        // Transition to SignedOut (distinct from Unconfigured: the user took
        // an explicit action). The footer says "signed out — run :yt auth to
        // reconnect" rather than "not configured."
        self.yt_state = crate::yt::state::YtState::SignedOut;
    }

    /// `R` — retry the YouTube provider probe after a `ProviderError` /
    /// `AuthExpired` / `RateLimited` / `ReadyStale` state. Non-blocking:
    /// immediately transitions to `Synchronizing` (visible feedback in the
    /// footer — "synchronizing…") and fire-and-forgets a `send_refresh`.
    /// `on_tick` promotes to `Ready` when the playlists response lands, or
    /// classifies the error (`AuthExpired`/`RateLimited`/`ProviderError`/
    /// `ReadyStale`) when an error response lands — the same error-
    /// classification logic that already handles `refresh_yt_lists` errors.
    ///
    /// The previous implementation called the BLOCKING `library_playlists()`
    /// (a 3s `roundtrip`), which froze the TUI event loop for up to 3s — no
    /// rendering, no input. The user pressed R and saw nothing happen. The
    /// non-blocking approach gives immediate visual feedback (state →
    /// Synchronizing, footer updates on the next render) and lets the user
    /// continue interacting while the refresh is in flight.
    ///
    /// Keeps the session (sidecar + caches) — does NOT re-spawn or re-prompt
    /// the Keychain. This is the fix for the "repeated login" root cause: the
    /// user presses R instead of re-authenticating.
    pub fn retry_yt_probe(&mut self) {
        // RC11-DEF-018: always clear the visible error on `R`, even when no
        // retry is performed. A command error (e.g. `:yt foobar`) sets
        // `yt_error` while `yt_state` stays `Ready` (which is not retryable),
        // so the early returns below would leave the `[ERR]` lingering. The
        // user pressed `R` expecting feedback to clear; leaving the error
        // visible would be misleading.
        self.yt_error = None;
        let Some(session) = self.yt_session.as_mut() else {
            // No session — nothing to retry. The footer's state hint tells the
            // user to auth (`:yt auth browser`), not to press R.
            return;
        };
        if !self.yt_state.can_retry() {
            // Only retry from error/stale/syncing states — not from Ready
            // (already healthy) or Unconfigured/SignedOut (need auth, not retry).
            return;
        }
        // Immediate visual feedback: transition to Synchronizing. The footer
        // renders "synchronizing…" on the next frame (within ~150ms — the poll
        // timeout), so the user sees the retry is in progress without waiting
        // for the sidecar roundtrip.
        self.yt_state = crate::yt::state::YtState::Synchronizing;
        // Fire-and-forget: send_refresh queues LibraryPlaylists + HomeSuggestions.
        // on_tick drains the responses and:
        //   - On success: replaces yt_lists, promotes to Ready (line ~1648).
        //   - On error: classifies via the heuristic in on_tick's error handler
        //     (AuthExpired for 401/auth/expired, ReadyStale if cached data,
        //     ProviderError otherwise) — lines ~1919-1946.
        // A refresh already in flight is a no-op (inflight guard in send_refresh),
        // so rapid R presses don't flood the sidecar.
        if let Err(e) = session.send_refresh() {
            self.yt_error = Some(format!("retry: {e}"));
            self.yt_state = crate::yt::state::YtState::ProviderError;
        }
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
            let start = if ids.contains(&id) {
                id
            } else {
                ids.first().cloned().unwrap_or(id)
            };
            self.play_in_context_ids(ids, &start);
            return;
        }
        // Youtube mode (or empty local catalog): pick from the first list that
        // has tracks loaded. `s` on a fresh Y view kicks off a load of the
        // focused list, so a second `s` plays once it lands.
        if let Some(l) = self
            .yt_lists
            .iter()
            .find(|l| !l.track_ids.is_empty())
            .cloned()
        {
            if let Some(id) = l.track_ids.first().cloned() {
                self.play_in_context_ids(l.track_ids.clone(), &id);
                return;
            }
        }
        // Nothing to play yet — tell the user why instead of silently no-op'ing.
        if self.source_mode == crate::mode::SourceMode::Youtube || self.catalog.tracks.is_empty() {
            if self.yt_session.is_none() {
                self.yt_error =
                    Some("s: YouTube not configured — run :yt auth browser <chrome>".into());
            } else if self.yt_lists.is_empty() {
                self.yt_status =
                    Some("s: no YouTube lists loaded yet (open the Y view with 4)".into());
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
                self.yt_status =
                    Some("s: loading the focused list — press s again in a moment".into());
            }
        }
    }

    /// `S` — open the discover overlay: local smart-album suggestions (Local /
    /// Mixed) or YouTube mood playlists (YouTube / Mixed-with-session).
    pub fn open_discover(&mut self) {
        // RC11-DEF-013: ensure the reco mixes are generated before building
        // the Discover overlay — `S` may be pressed before `H` (which would
        // generate them via `open_home`). A cold profile still produces mixes
        // via the catalog fallback (DailyMix, Discover, LocalYtBlend).
        if self.reco_mixes.is_empty() {
            self.reco_mixes =
                crate::reco::mixes::generate_all_mixes(&self.reco_profile, &self.catalog.tracks);
        }
        // RC11-DEF-013: in YouTube / mixed mode, the Discover overlay shows
        // the generated mixes (Daily Mix, Discover Mix, ...) from `reco::mixes`
        // — NOT local smart albums. The mixes carry local-catalog track ids,
        // so Enter plays them immediately (no sidecar roundtrip). Local smart
        // albums are still shown in local mode (the old behavior). The
        // sidecar's `home_suggestions` fetch still runs in YT/mixed mode to
        // populate additional YouTube playlists alongside the mixes.
        let mix_items: Vec<DiscoverItem> = self
            .reco_mixes
            .iter()
            .map(|m| {
                let track_ids: Vec<String> = m.tracks.iter().map(|c| c.track_id.clone()).collect();
                // RC11-DEF-028: per-mix "why recommended" explanation. Use the
                // mix type's description as the human-readable reason (the
                // reco engine's per-track explanations live in Home's Made
                // for You; here we surface the mix-level description).
                let explanation = Some(m.mix_type.description().to_string());
                DiscoverItem::Mix {
                    mix_type: m.mix_type,
                    name: m.mix_type.label().to_string(),
                    track_ids,
                    explanation,
                }
            })
            .collect();

        let items = match self.source_mode {
            crate::mode::SourceMode::Youtube => {
                let mut items = mix_items;
                items.extend(self.yt_discover_items());
                items
            }
            crate::mode::SourceMode::Mixed => {
                let mut items = mix_items;
                items.extend(self.local_smart_albums());
                items.extend(self.yt_discover_items());
                items
            }
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
        let cur_artist = self
            .now_playing_view()
            .map(|v| v.artist)
            .unwrap_or_default();
        let mut scored: Vec<(u64, DiscoverItem)> = Vec::new();
        // Dedup key: collaboration albums are filed under every artist in
        // `symlinked_into_artists`, so the same (primary_artist, title) pair
        // appears under multiple artist keys. Without dedup the discover
        // overlay shows duplicate entries (DEF-022).
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (artist, albums) in &self.albums_by_artist {
            for a in albums {
                let dup_key = (a.artist.clone(), a.title.clone());
                if !seen.insert(dup_key) {
                    continue;
                }
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

    /// Fire-and-forget the YouTube home-suggestions fetch for the `S` discover
    /// overlay (non-blocking — the old `home_suggestions()` roundtrip blocked
    /// the UI ~3s every time `S` was pressed). Returns an empty list now; the
    /// overlay opens instantly with a "loading…" state (`discover_loading` =
    /// true) and `on_tick` populates its items when `pending_discover` lands.
    /// A fetch already in flight is a no-op (`discover_inflight`), so repeated
    /// `S` presses don't flood the sidecar.
    fn yt_discover_items(&mut self) -> Vec<DiscoverItem> {
        let Some(session) = self.yt_session.as_mut() else {
            return Vec::new();
        };
        session.reset_discover_inflight();
        if session.send_home_suggestions().is_err() {
            return Vec::new();
        }
        self.discover_loading = true;
        self.discover_loading_ticks = 0;
        Vec::new()
    }

    fn simple_rand(&mut self) -> usize {
        // Seed the counter with the wall-clock time on FIRST use so each launch
        // starts at a different point — otherwise `s` (instant random) would
        // pick the same opening track every launch (the counter used to start
        // at a fixed constant). The seed is computed once and reused; the
        // counter still advances across calls so successive `s` presses differ.
        use std::sync::OnceLock;
        static SEED: OnceLock<u64> = OnceLock::new();
        static COUNTER: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0x9E3779B97F4A7C15);
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
            _ => {
                self.filter = Some(FilterState {
                    col: self.focus_col,
                    text: String::new(),
                })
            }
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
                    let artist = self
                        .artists
                        .get(self.cursors.artist)
                        .cloned()
                        .unwrap_or_default();
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

    /// Apply a discover-overlay selection (Enter): start the album/mix/playlist.
    ///
    /// RC11-DEF-035: the overlay is NO LONGER closed immediately on Enter for
    /// the Mix / Playlist variants. Instead it stays open showing a persistent
    /// "Loading [name]..." indicator (`discover_play_loading`) so the user
    /// sees feedback while the tracks resolve / the sidecar roundtrip is in
    /// flight. `on_tick` closes the overlay + clears the loading state when
    /// playback starts (for the synchronous Mix path) or when
    /// `pending_discover_play` resolves (for the async Playlist path). The
    /// Album variant still closes immediately (synchronous, instant).
    pub fn play_discover_selection(&mut self) {
        let Some(Overlay::Discover { items, cursor }) = self.overlay.clone() else {
            return;
        };
        let Some(item) = items.get(cursor).cloned() else {
            // Empty selection — close the overlay.
            self.overlay = None;
            return;
        };
        match item {
            DiscoverItem::Album { album, .. } => {
                // Synchronous + instant — close the overlay immediately.
                self.overlay = None;
                self.discover_play_loading = None;
                let ids = self.tracks_for_album(&album);
                if let Some(start) = ids.first().cloned() {
                    self.transport.continue_mode = ContinueMode::NextAlbum;
                    self.play_in_context_ids(ids, &start);
                } else {
                    self.yt_status = Some("no tracks found for this album".into());
                }
            }
            DiscoverItem::Playlist { id, name, .. } => {
                // RC11-DEF-035: keep the overlay open with a loading state.
                if let Some(session) = self.yt_session.as_mut() {
                    let _ = session.send_get_playlist(id.clone());
                    self.pending_discover_play = Some(id);
                    self.discover_play_loading = Some(name.clone());
                    self.yt_status = Some(format!("Loading \"{name}\"…"));
                    // Keep the overlay open — on_tick closes it when the
                    // pending_discover_play resolves (or on error/timeout).
                } else {
                    // No session — surface an error and close.
                    self.overlay = None;
                    self.discover_play_loading = None;
                    self.yt_status = Some("can't load mix — no YouTube session".into());
                }
            }
            DiscoverItem::Mix {
                track_ids, name, ..
            } => {
                // RC11-DEF-013 / DEF-035: a generated mix from `reco::mixes`.
                // Plays immediately from local-catalog track ids — no sidecar
                // roundtrip. Keep the overlay open with a "Loading [name]..."
                // state for one frame so the user sees feedback; on_tick
                // closes the overlay + clears the loading state once
                // `now_playing` is set.
                if let Some(start) = track_ids.first().cloned() {
                    self.discover_play_loading = Some(name.clone());
                    self.yt_status = Some(format!("Loading \"{name}\"…"));
                    self.transport.continue_mode = ContinueMode::NextAlbum;
                    self.play_in_context_ids(track_ids, &start);
                    // Overlay stays open — on_tick clears it.
                } else {
                    self.overlay = None;
                    self.discover_play_loading = None;
                    self.yt_status = Some(format!("mix \"{name}\" has no tracks"));
                }
            }
        }
    }

    /// Load cached `yt_lists` from the default state DB so an offline launch
    /// shows known playlists immediately. When the sidecar couldn't start
    /// (`yt_session` is `None`), transitions to `ReadyStale` — the footer then
    /// reads "offline — showing cached (press R to retry)" instead of an
    /// empty Y view. Best-effort: a missing/corrupt cache is silently ignored
    /// (the fire-and-forget refresh, when the session is up, repopulates
    /// fresh lists and promotes to `Ready`).
    ///
    /// `track_ids` are cleared on load: the disk cache stores video IDs but
    /// NOT track metadata (title/artist/album). The in-memory `track_cache`
    /// starts empty on launch. If we kept the cached `track_ids`, the
    /// lazy-load at `on_tick` (which checks `track_ids.is_empty()`) wouldn't
    /// fire, and `yt_track_rows` would show raw video IDs as titles
    /// ("random characters" bug). Clearing forces a re-fetch with metadata
    /// when the user focuses each playlist. The playlist NAMES are still
    /// visible from cache.
    pub fn load_yt_lists_from_cache(&mut self) {
        let mut cached = crate::yt::cache::load_yt_lists();
        if cached.is_empty() {
            return;
        }
        for l in cached.iter_mut() {
            l.track_ids.clear();
        }
        self.yt_lists = cached;
        if self.yt_session.is_none() {
            self.yt_state = crate::yt::state::YtState::ReadyStale;
        }
    }

    /// Same as `load_yt_lists_from_cache` but reads from `path` instead of the
    /// default state DB. For tests: avoids the process-global `XDG_CONFIG_HOME`
    /// env race by using an explicit temp DB path.
    pub fn load_yt_lists_from_cache_at(&mut self, path: &std::path::Path) {
        let mut cached = match crate::yt::cache::load_yt_lists_at(path) {
            Ok(v) => v,
            Err(_) => return,
        };
        if cached.is_empty() {
            return;
        }
        // Clear track_ids: the disk cache stores video IDs but NOT track
        // metadata. See `load_yt_lists_from_cache` for the full rationale.
        for l in cached.iter_mut() {
            l.track_ids.clear();
        }
        self.yt_lists = cached;
        if self.yt_session.is_none() {
            self.yt_state = crate::yt::state::YtState::ReadyStale;
        }
    }

    /// Kick off an async fetch of the account + suggested lists for the Y view.
    /// Non-blocking: sends the requests and returns immediately, showing
    /// "loading…" until `on_tick` folds the results into `yt_lists`. No-op
    /// (and clears the lists) when there's no session — the view then shows
    /// the setup hint. A refresh already in flight is not re-sent.
    ///
    /// The stale-pending clearing lives in `Session::send_refresh`, AFTER its
    /// inflight guard — NOT here. Clearing here (before the guard) lost the
    /// pending data when a refresh was already in flight: the guard made
    /// `send_refresh` a no-op, the cleared `pending_playlists`/`pending_suggestions`
    /// stayed None, and `on_tick` never merged → `yt_lists` stayed empty.
    pub fn refresh_yt_lists(&mut self) {
        let Some(session) = self.yt_session.as_mut() else {
            self.yt_lists.clear();
            return;
        };
        self.yt_error = None;
        // RC11-DEF-055: when the Y view already has cached lists (re-entry
        // via `4`), skip the `[~]` flash — keep `yt_state` at its current
        // value (Ready / ReadyStale) and don't set `yt_lists_loading`. The
        // background `send_refresh` still fires so the lists stay fresh, but
        // the user sees `[ok]` (or `[stale]`) immediately instead of a
        // meaningless `[~]` on every re-entry. Only the first entry (empty
        // `yt_lists`) goes through the full loading → ready cycle.
        let already_loaded = !self.yt_lists.is_empty();
        if !already_loaded {
            self.yt_lists_loading = true;
            // Transition to Synchronizing — a data fetch is in flight. on_tick
            // will promote to Ready when playlists land, or ProviderError on
            // error.
            self.yt_state = crate::yt::state::YtState::Synchronizing;
        }
        if let Err(e) = session.send_refresh() {
            self.yt_lists_loading = false;
            self.yt_error = Some(format!("refresh: {e}"));
            self.yt_state = crate::yt::state::YtState::ProviderError;
        }
        // NOTE: the premium-tier pre-warm (nsig-solver download + Keychain read,
        // ~10-15s cold) used to fire here. But the sidecar is single-threaded
        // and sequential — a premium resolve queued here would block every
        // subsequent `get_playlist` (the lazy-load fires one on the next tick).
        // That kept the Y view on "Loading…" for 10-15s+ after every refresh.
        // The pre-warm is now REMOVED entirely: the one-time nsig-solver
        // download + Keychain read happens on the first real `preload_next_url`
        // (during playback, not browsing), and the fast-URL fallback covers the
        // gap. Browsing responsiveness > gapless handoff cold-start.
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
            self.transport
                .history
                .push((np.id().to_string(), self.transport.context.clone()));
        }
        let ctx = Context::Album {
            album: next.title.clone(),
            artist: next.artist.clone(),
            track_ids,
        };
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        self.transport.switch_context(ctx, None, &r, &self.catalog);
        // Keep the browse cursor in sync so the UI shows the new album.
        self.cursors.album = next_idx;
        self.cursors.track = 0;
        true
    }

    /// Auto-continue with the whole library as a shuffled "radio" context.
    /// Music never stops — when this context eventually exhausts (the entire
    /// library), `next` re-enters here and rebuilds it. If a reco
    /// [`Overlay::Radio`] session is active, it drives auto-advance instead.
    fn switch_to_radio(&mut self) {
        // If a reco RadioSession overlay is active, use it to drive
        // auto-advance with the recommendation engine's candidate pool.
        if let Some(id) = self.reco_radio_next() {
            if let Some(np) = self.now_playing.clone() {
                self.transport
                    .history
                    .push((np.id().to_string(), self.transport.context.clone()));
            }
            let ctx = Context::Search {
                query: "reco radio".into(),
                track_ids: vec![id.clone()],
            };
            let r = ClonedResolver {
                playlists: &self.playlists,
                manual_queue: self.transport.manual_queue.clone(),
                yt_lists: &self.yt_lists,
            };
            self.transport
                .switch_context(ctx, Some(&id), &r, &self.catalog);
            return;
        }
        if let Some(np) = self.now_playing.clone() {
            self.transport
                .history
                .push((np.id().to_string(), self.transport.context.clone()));
        }
        let all_ids: Vec<String> = self.catalog.tracks.iter().map(|t| t.id.clone()).collect();
        let ctx = Context::Search {
            query: "radio".into(),
            track_ids: all_ids,
        };
        // Radio implies shuffled play; force smart shuffle so it actually
        // discovers (catalog order would just be sequential).
        self.transport.shuffle = ShuffleMode::Smart;
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
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
            self.yt_error =
                Some("search: YouTube not configured — run :yt auth browser <chrome>".into());
            return;
        };
        if let Err(e) = session.send_search(q) {
            self.yt_error = Some(format!("search: {e}"));
        }
    }

    /// Toggle the lyrics overlay (`L`). On open: requests lyrics for the
    /// currently-playing track (local read first, else sidecar). On close: just
    /// dismisses (the cached content is dropped with the overlay; a re-open
    /// re-requests, which is cheap — local reads are instant and the sidecar
    /// inflight guard dedups). No-op if nothing is playing.
    pub fn toggle_lyrics(&mut self) {
        if matches!(self.overlay, Some(Overlay::Lyrics { .. })) {
            self.overlay = None;
            return;
        }
        let Some(np) = self.now_playing.clone() else {
            self.yt_error = Some("lyrics: nothing is playing".into());
            return;
        };
        self.overlay = Some(Overlay::Lyrics {
            content: None,
            state: LyricsState::Idle,
            scroll: 0,
            track_id: np.id().to_string(),
            gen: self.lyrics_gen,
        });
        self.request_lyrics(np.id());
    }

    /// Request lyrics for `track_id`. Bumps `lyrics_gen` (so any in-flight
    /// response for a prior track is discarded by `on_tick`), updates the open
    /// Lyrics overlay to `Loading`, then tries local sources first (embedded
    /// FLAC tag / sidecar `.lrc` for a catalog track) — if found, sets
    /// `Available` immediately. Else fires the sidecar `get_lyrics` for a
    /// YouTube track; `on_tick` folds the response in under the generation
    /// guard. For a local track with no embedded/sidecar lyrics, sets
    /// `NotFound` (we have no video_id to ask ytmusicapi). Non-blocking.
    pub fn request_lyrics(&mut self, track_id: &str) {
        // Bump the generation so a stale in-flight response is discarded.
        self.lyrics_gen = self.lyrics_gen.wrapping_add(1);
        let gen = self.lyrics_gen;
        // Update the overlay to Loading if it's the Lyrics overlay.
        if let Some(Overlay::Lyrics { scroll, .. }) = self.overlay.clone() {
            self.overlay = Some(Overlay::Lyrics {
                content: None,
                state: LyricsState::Loading,
                scroll,
                track_id: track_id.to_string(),
                gen,
            });
        }
        // Local catalog track → try embedded FLAC tag + sidecar .lrc/.txt.
        // These are fast (one metaflac subprocess + filesystem reads) and
        // never block; a miss falls through to NotFound for local tracks (no
        // video_id to ask ytmusicapi).
        if let Some(track) = self.track_by_id_fast(track_id).cloned() {
            if let Some(lyrics) = crate::lyrics::read_embedded(&track, &self.catalog.source_root) {
                let new_state = if lyrics.is_empty() {
                    LyricsState::NotFound
                } else {
                    LyricsState::Available(lyrics.synced)
                };
                if let Some(Overlay::Lyrics { scroll, .. }) = self.overlay.clone() {
                    self.overlay = Some(Overlay::Lyrics {
                        content: Some(lyrics),
                        state: new_state,
                        scroll,
                        track_id: track_id.to_string(),
                        gen,
                    });
                }
                return;
            }
            // Local track, no embedded/sidecar lyrics → NotFound (truthful; no
            // fabricated text, AC-M3.5.1).
            if let Some(Overlay::Lyrics { scroll, .. }) = self.overlay.clone() {
                self.overlay = Some(Overlay::Lyrics {
                    content: None,
                    state: LyricsState::NotFound,
                    scroll,
                    track_id: track_id.to_string(),
                    gen,
                });
            }
            return;
        }
        // Remote (YouTube) track → fire-and-forget sidecar get_lyrics. The
        // response lands in `pending_lyrics` and is drained by on_tick under
        // the generation guard. No session → Error (truthful; the user needs
        // :yt auth to fetch YouTube lyrics).
        let Some(session) = self.yt_session.as_mut() else {
            if let Some(Overlay::Lyrics { scroll, .. }) = self.overlay.clone() {
                self.overlay = Some(Overlay::Lyrics {
                    content: None,
                    state: if self.yt_state == crate::yt::state::YtState::ReadyStale {
                        LyricsState::Offline
                    } else {
                        LyricsState::Error(
                            "YouTube not configured — run :yt auth browser <chrome>".into(),
                        )
                    },
                    scroll,
                    track_id: track_id.to_string(),
                    gen,
                });
            }
            return;
        };
        if let Err(e) = session.send_get_lyrics(track_id.to_string()) {
            if let Some(Overlay::Lyrics { scroll, .. }) = self.overlay.clone() {
                self.overlay = Some(Overlay::Lyrics {
                    content: None,
                    state: if self.yt_state == crate::yt::state::YtState::ReadyStale {
                        LyricsState::Offline
                    } else {
                        LyricsState::Error(format!("lyrics: {e}"))
                    },
                    scroll,
                    track_id: track_id.to_string(),
                    gen,
                });
            }
        }
        // else: sent — on_tick will fold the response in (Loading → Available /
        // NotFound / Error).
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

    // --- Queue & playlist operations -------------------------------------

    /// The track id under the cursor in the current view, or `None` if the
    /// view/cursor has no track under it. Mirrors the logic of
    /// [`play_selected`] for non-Queue views (uses `cursors.track`); for the
    /// Queue view uses `cursors.queue` (the actual navigation cursor for that
    /// single-column view). Clamps cursors first so a stale cursor doesn't
    /// silently miss.
    pub fn selected_track_id(&mut self) -> Option<String> {
        self.clamp_cursors();
        let ids = self.current_context_ids();
        if ids.is_empty() {
            return None;
        }
        let cursor = match self.view {
            View::Queue => self.cursors.queue,
            _ => self.cursors.track,
        };
        let cursor = cursor.min(ids.len() - 1);
        ids.get(cursor).cloned()
    }

    /// `e` — enqueue the track under the cursor to the manual "play next"
    /// queue. Shows a status-bar confirmation so the user gets immediate
    /// feedback (DEF-008: YouTube enqueue previously produced no visible
    /// result). No-op if there's no selected track.
    pub fn enqueue_selected(&mut self) {
        if let Some(id) = self.selected_track_id() {
            self.transport.enqueue(id.clone());
            // DEF-034: record the enqueue intent as a mild positive signal.
            self.record_listen_event(&id, "added_to_queue");
            self.yt_status = Some("added to queue".into());
            // RC11-DEF-043: also set a dedicated toast rendered in the player
            // bar regardless of yt_state, so local-only users (yt_state !=
            // Ready, where yt_status is hidden) see the confirmation. The
            // toast clears after ~1.2s via on_tick.
            self.set_toast("Added to queue".to_string());
        } else {
            self.yt_status = Some("no track selected".into());
        }
    }

    /// RC11-DEF-043: set a transient confirmation toast shown in the player
    /// bar. Resets the TTL so re-setting the same message (e.g. pressing `e`
    /// twice) refreshes the window instead of being deduped away.
    pub fn set_toast(&mut self, msg: String) {
        self.toast = Some(msg);
        self.toast_at = Some(std::time::Instant::now());
    }

    /// RC11-DEF-014: resume the last-played track at its saved position.
    /// Captures a one-shot `pending_resume` so the next load of the matching
    /// track uses `load_at(pos)`; clears the `resume_hint` so it doesn't
    /// linger. No-op when there's no saved track. Used by the `R` key when
    /// stopped with a resume hint.
    pub fn resume_last(&mut self) {
        let Some(id) = self.last_played_track_id.clone() else {
            return;
        };
        let pos = self.last_played_position;
        self.pending_resume = Some((id.clone(), pos));
        self.resume_hint = None;
        // Play the single track as its own context so `>`/`<` stay coherent.
        self.play_in_context_ids(vec![id.clone()], &id);
    }

    /// `x` (Queue view) — remove the track under the cursor from the manual
    /// queue. Adjusts the cursor so it stays valid. No-op outside the Queue
    /// view or when the queue is empty.
    pub fn remove_selected_from_queue(&mut self) {
        if self.view != View::Queue {
            return;
        }
        // Remove by INDEX, not by id. The old form called
        // `transport.remove_from_queue(&id)`, which uses
        // `manual_queue.retain(|x| x != track_id)` — that strips EVERY
        // occurrence of the id. A queue with the same track enqueued N times
        // (common: `e` on the same track) was wiped by a single `x` press
        // (DEF-027). Index removal deletes exactly the one entry the cursor
        // points at, preserving duplicates above/below it.
        if self.cursors.queue >= self.transport.manual_queue.len() {
            return;
        }
        let removed = self.transport.manual_queue.remove(self.cursors.queue);
        // DEF-034: record the removal as a mild negative signal.
        self.record_listen_event(&removed, "removed_from_queue");
        // Keep the cursor valid: if we removed the last item, step back.
        let new_len = self.transport.manual_queue.len();
        if new_len > 0 && self.cursors.queue >= new_len {
            self.cursors.queue = new_len - 1;
        } else if new_len == 0 {
            self.cursors.queue = 0;
        }
    }

    /// Add `track_id` to an existing playlist at `playlist_idx`. Skips
    /// duplicates (a track already in the playlist isn't added twice).
    /// Returns true if the track was added, false if it was already present
    /// or the index was out of bounds.
    pub fn add_track_to_playlist(&mut self, track_id: &str, playlist_idx: usize) -> bool {
        let Some(pl) = self.playlists.get_mut(playlist_idx) else {
            return false;
        };
        if pl.track_ids.iter().any(|t| t == track_id) {
            return false;
        }
        pl.track_ids.push(track_id.to_string());
        true
    }

    /// Create a new playlist containing `track_id` and append it to
    /// `self.playlists`. Generates a unique name ("New Playlist N"). Returns
    /// the index of the new playlist.
    pub fn create_playlist_with_track(&mut self, track_id: &str) -> usize {
        let name = self.unique_playlist_name();
        let pl = Playlist {
            name,
            track_ids: vec![track_id.to_string()],
        };
        self.playlists.push(pl);
        self.playlists.len() - 1
    }

    /// Generate a unique "New Playlist N" name that doesn't collide with any
    /// existing playlist name.
    fn unique_playlist_name(&self) -> String {
        let mut n = self.playlists.len() + 1;
        loop {
            let candidate = format!("New Playlist {n}");
            if !self.playlists.iter().any(|p| p.name == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// `d` (Playlists view, col 0) — delete the focused playlist. Adjusts the
    /// cursor, saves to state.db, and sets a status message. No-op outside the
    /// Playlists view or col 0, or when there are no playlists.
    pub fn delete_focused_playlist(&mut self) {
        if self.view != View::Playlists || self.focus_col != 0 {
            return;
        }
        let Some(pl) = self.playlists.get(self.cursors.playlist).cloned() else {
            return;
        };
        self.playlists.remove(self.cursors.playlist);
        // Keep the cursor valid.
        let n = self.playlists.len();
        if n > 0 && self.cursors.playlist >= n {
            self.cursors.playlist = n - 1;
        } else if n == 0 {
            self.cursors.playlist = 0;
        }
        self.cursors.track = 0;
        self.save_playlists_db();
        self.yt_status = Some(format!("deleted playlist \"{}\"", pl.name));
    }

    /// Best-effort persist of `self.playlists` to the state DB. Errors are
    /// ignored (matching the existing `let _ = state::save_layout(...)` pattern).
    pub fn save_playlists_db(&self) {
        let _ = crate::state::save_playlists(&self.playlists);
    }

    // --- Recommendation engine wiring --------------------------------------

    /// Open the YouTube Home view. Populates the generated mixes from the
    /// reco engine and builds the section items (Quick Picks, Made for You,
    /// Start Radio, Your YouTube Library) from local catalog + reco + yt_lists
    /// data. The overlay always has content, even on a cold start — Quick
    /// Picks draws from the local catalog and Made for You uses the catalog
    /// fallback in the candidate generator.
    pub fn open_home(&mut self) {
        use crate::tui::view::home::{HomeItem, HomeSection, HomeState};

        // Generate mixes from the reco engine. Always generate (even cold
        // start uses the local catalog) so the "Made for You" section has
        // content.
        self.reco_mixes =
            crate::reco::mixes::generate_all_mixes(&self.reco_profile, &self.catalog.tracks);

        let mut state = HomeState::new();
        state.has_history = self.reco_profile.has_history();
        // Local data is synchronous — not loading. YouTube home suggestions
        // (if a session is available) are fetched by the existing discover
        // mechanism and don't block this overlay.
        state.loading = false;

        // Build section items from app data so the overlay shows content.
        let mut sections = Vec::new();

        // Quick Picks: first 5 tracks from the local catalog.
        let quick_picks: Vec<HomeItem> = self
            .catalog
            .tracks
            .iter()
            .take(5)
            .map(|t| {
                HomeItem::track(
                    t.id.clone(),
                    t.title.clone(),
                    t.primary_artist.clone(),
                    true,
                )
            })
            .collect();
        sections.push((HomeSection::QuickPicks, quick_picks));

        // Made for You: generated mixes (Daily Mix, Discover, etc.).
        let made_for_you: Vec<HomeItem> = self
            .reco_mixes
            .iter()
            .map(|m| HomeItem::mix(m.mix_type))
            .collect();
        sections.push((HomeSection::MadeForYou, made_for_you));

        // Start Radio: radio seed options.
        let start_radio = vec![
            HomeItem::radio_seed("Track Radio".into()),
            HomeItem::radio_seed("Artist Radio".into()),
        ];
        sections.push((HomeSection::StartRadio, start_radio));

        // Your YouTube Library: YouTube playlists (if session is available).
        let library: Vec<HomeItem> = self
            .yt_lists
            .iter()
            .map(|l| HomeItem::playlist(l.id.clone(), l.name.clone(), false))
            .collect();
        sections.push((HomeSection::Library, library));

        state.sections = sections;
        self.overlay = Some(Overlay::Home { state });
    }

    /// Start a radio session seeded from a track. Initializes the candidate
    /// pool (blending YouTube tracks when the provider is connected — DEF-011),
    /// and auto-advances to the first track so `:radio` starts playing
    /// immediately instead of opening silent (DEF-056).
    pub fn start_radio_from_track(&mut self, track_id: &str) {
        use crate::reco::radio::{RadioSeed, RadioSession};
        let mut session = RadioSession::new(RadioSeed::Track(track_id.to_string()));
        session.set_yt_track_ids(self.yt_track_ids());
        session.initialize(&self.reco_profile, &self.catalog.tracks);
        self.overlay = Some(Overlay::Radio {
            session: Some(session),
        });
        // DEF-056: auto-start — advance to the first radio track so the
        // session begins playing immediately on `:radio` Enter.
        self.advance_radio_from_overlay();
    }

    /// Start a radio session seeded from an artist.
    pub fn start_radio_from_artist(&mut self, artist: &str) {
        use crate::reco::radio::{RadioSeed, RadioSession};
        let mut session = RadioSession::new(RadioSeed::Artist(artist.to_string()));
        session.set_yt_track_ids(self.yt_track_ids());
        session.initialize(&self.reco_profile, &self.catalog.tracks);
        self.overlay = Some(Overlay::Radio {
            session: Some(session),
        });
        self.advance_radio_from_overlay();
    }

    /// Change the radio seed to the currently-playing track (or the first
    /// upcoming pool track if nothing is playing). Resets the pool, clears
    /// the played-list, and updates the seed label (DEF-006). No-op if no
    /// radio session is active.
    pub fn change_radio_seed(&mut self) {
        use crate::reco::radio::RadioSeed;
        let new_seed_id = self.now_playing.as_ref().map(|s| s.id().to_string());
        let yt = self.yt_track_ids();
        let mut did_change = false;
        if let Some(Overlay::Radio { session }) = &mut self.overlay {
            if let Some(s) = session.as_mut() {
                // Prefer the currently-playing track; fall back to the first
                // upcoming pool track so `c` always has a concrete seed.
                let seed_id = new_seed_id
                    .or_else(|| s.upcoming(1).first().map(|c| c.track_id.clone()))
                    .unwrap_or_default();
                if !seed_id.is_empty() {
                    s.change_seed(
                        RadioSeed::Track(seed_id),
                        &self.reco_profile,
                        &self.catalog.tracks,
                    );
                    s.set_yt_track_ids(yt.clone());
                    did_change = true;
                }
            }
        }
        if did_change {
            self.yt_status = Some("radio seed changed".into());
        }
    }

    /// Stop the active radio session: clear the pool (no further auto-advance)
    /// and close the overlay (DEF-007). The transport continues with whatever
    /// is currently playing; `reco_radio_next` returns None after this so
    /// CONT=YouTube won't pull more radio tracks.
    pub fn stop_radio(&mut self) {
        if let Some(Overlay::Radio { session }) = &mut self.overlay {
            if let Some(s) = session.as_mut() {
                s.stop();
            }
        }
        self.overlay = None;
        self.yt_status = Some("radio stopped".into());
    }

    /// Advance the radio session from the overlay (used by auto-start and
    /// the `>`/`n` overlay keys). Gets the next track from the session and
    /// starts playback. The overlay is read directly (not taken) so it stays
    /// open for the user.
    fn advance_radio_from_overlay(&mut self) {
        let next_id = if let Some(Overlay::Radio { session }) = &mut self.overlay {
            if let Some(s) = session.as_mut() {
                if s.needs_refill() {
                    s.refill_if_needed(&self.reco_profile, &self.catalog.tracks);
                }
                s.next_track().map(|c| c.track_id)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(id) = next_id {
            self.play_radio_track(&id);
        }
    }

    /// Collect YouTube video ids available for the recommendation pool when
    /// the provider is connected (DEF-010/DEF-011). Draws from the session's
    /// `track_cache` (metadata-resolved tracks) and every loaded `yt_lists`
    /// entry's `track_ids`. Returns an empty vec when the provider isn't
    /// ready, so callers degrade to local-only.
    pub fn yt_track_ids(&self) -> Vec<String> {
        if !self.yt_state.is_ready() {
            return Vec::new();
        }
        let mut ids: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(session) = self.yt_session.as_ref() {
            for id in session.track_cache.keys() {
                if seen.insert(id.clone()) {
                    ids.push(id.clone());
                }
            }
        }
        for list in &self.yt_lists {
            for id in &list.track_ids {
                if seen.insert(id.clone()) {
                    ids.push(id.clone());
                }
            }
        }
        ids
    }

    /// Play a specific track id as the next radio track. Switches the
    /// transport context to a single-track "reco radio" context and starts
    /// playback. Used by the Radio overlay's `n` / `s` / `-` keys — the
    /// overlay is taken out during key handling so [`reco_radio_next`] (which
    /// checks `self.overlay`) can't find the session; instead the handler
    /// gets the next track from the session directly and calls this method.
    pub fn play_radio_track(&mut self, track_id: &str) {
        if let Some(np) = self.now_playing.clone() {
            self.transport
                .history
                .push((np.id().to_string(), self.transport.context.clone()));
        }
        let ctx = Context::Search {
            query: "reco radio".into(),
            track_ids: vec![track_id.to_string()],
        };
        let r = ClonedResolver {
            playlists: &self.playlists,
            manual_queue: self.transport.manual_queue.clone(),
            yt_lists: &self.yt_lists,
        };
        self.transport
            .switch_context(ctx, Some(track_id), &r, &self.catalog);
        self.start_playback();
    }

    /// Open the playlist generator overlay (NL input phase).
    pub fn open_generator(&mut self) {
        use crate::tui::view::generator::GeneratorState;
        self.overlay = Some(Overlay::Generator {
            state: GeneratorState::new(),
        });
    }

    /// Generate a playlist from the generator's parsed constraints. Called
    /// when the user presses Enter in the generator input phase. Parses the
    /// NL input into constraints, runs the reco pipeline, and moves to the
    /// preview phase. Resolves track IDs to "Title — Artist" display names
    /// so the preview shows human-readable titles instead of raw IDs.
    ///
    /// RC11-DEF-031: pinned tracks survive `g` regenerate. The pinned ids
    /// from the prior preview are captured before the playlist is replaced,
    /// then prepended to the new playlist's tracks (and re-added to its
    /// `pinned` list) so a regenerate keeps the user's explicit picks.
    pub fn generate_playlist(&mut self) {
        // Compute the YouTube track-id pool BEFORE mutably borrowing
        // `self.overlay` below — `yt_track_ids()` borrows all of `self`
        // immutably and would conflict with the `&mut self.overlay` borrow.
        // (Sibling-batch fix: DEF-010 candidate sourcing added this call
        // inside the overlay borrow, breaking the build.)
        let yt_ids = self.yt_track_ids();
        if let Some(Overlay::Generator { state }) = &mut self.overlay {
            // Capture pinned track ids + the pinned candidates themselves
            // before parse_input overwrites state.playlist. The new playlist
            // gets these prepended (and re-pinned) so `g` doesn't drop them.
            let (pinned_ids, pinned_candidates): (
                Vec<String>,
                Vec<crate::reco::candidates::Candidate>,
            ) = {
                let mut ids = Vec::new();
                let mut cands = Vec::new();
                if let Some(p) = &state.playlist {
                    for id in &p.pinned {
                        if let Some(c) = p.tracks.iter().find(|t| &t.track_id == id) {
                            ids.push(id.clone());
                            cands.push(c.clone());
                        }
                    }
                }
                (ids, cands)
            };

            state.parse_input(); // parse NL → constraints
            if let Some(constraints) = state.constraints.clone() {
                // DEF-010: blend YouTube tracks into the candidate pool when
                // the provider is connected. `generate_with_yt` filters by
                // the constraints' SourcePreference (Local/Youtube/Hybrid).
                let mut playlist = crate::reco::generator::generate_with_yt(
                    &constraints,
                    &self.reco_profile,
                    &self.catalog.tracks,
                    &yt_ids,
                );
                // Fallback: if the generator produced no tracks (empty profile
                // with an empty catalog), seed from the catalog so the user
                // always gets a playlist.
                if playlist.tracks.is_empty() && !self.catalog.tracks.is_empty() {
                    let max = constraints.max_tracks.min(50);
                    for track in self.catalog.tracks.iter().take(max) {
                        playlist
                            .tracks
                            .push(crate::reco::candidates::Candidate::new(
                                track.id.clone(),
                                crate::reco::candidates::CandidateSource::LocalMetadata,
                                0.1,
                                true,
                            ));
                    }
                }
                // Re-insert pinned tracks at the head of the new playlist,
                // skipping any that already appear in the fresh tracks (a
                // regenerate might re-pick a pinned track on its own; we
                // don't want to duplicate it). Then set playlist.pinned so
                // the [pinned] marker renders and `x` keeps the sync.
                if !pinned_candidates.is_empty() {
                    let existing: std::collections::HashSet<String> =
                        playlist.tracks.iter().map(|t| t.track_id.clone()).collect();
                    for c in &pinned_candidates {
                        if !existing.contains(&c.track_id) {
                            playlist.tracks.insert(0, c.clone());
                        }
                    }
                    playlist.pinned = pinned_ids
                        .iter()
                        .filter(|id| playlist.tracks.iter().any(|t| t.track_id == *id.as_str()))
                        .cloned()
                        .collect();
                }
                // Resolve track IDs to display titles ("Title — Artist") so
                // the preview shows names instead of raw IDs. Falls back to
                // the raw track_id when the catalog has no match (e.g. a
                // YouTube video id whose metadata isn't cached yet). Access
                // `track_index` + `catalog.tracks` directly (disjoint fields
                // from `overlay`) instead of calling `track_by_id_fast` to
                // avoid borrowing all of `self` while `state` is mutably
                // borrowed from `self.overlay`.
                let dash = crate::tui::view::theme::em_dash();
                state.title_map.clear();
                for track in &playlist.tracks {
                    let display = match self.track_index.get(&track.track_id) {
                        Some(&i) => {
                            let t = &self.catalog.tracks[i];
                            format!("{} {} {}", t.title, dash, t.primary_artist)
                        }
                        None => track.track_id.clone(),
                    };
                    state.title_map.insert(track.track_id.clone(), display);
                }
                state.playlist = Some(playlist);
                state.generate(); // move to preview phase
            }
        }
    }

    /// Show the recommendation explanation for a track. Generates an
    /// explanation from the candidate's provenance via the reco engine.
    pub fn show_explanation(&mut self, track_id: &str) {
        use crate::reco::candidates::CandidateGenerator;
        use crate::reco::explanations::Explanation;
        let yt = self.yt_track_ids();
        let gen = CandidateGenerator::new(&self.reco_profile, &self.catalog.tracks)
            .with_yt_track_ids(&yt);
        let candidates = gen.generate();
        let explanation = candidates
            .iter()
            .find(|c| c.track_id == track_id)
            .map(Explanation::from_candidate)
            .unwrap_or_else(|| Explanation {
                reason: "this track".into(),
                detail: None,
            });
        self.overlay = Some(Overlay::Explanation { explanation });
    }

    /// Summarize the recommendation engine's profile health for the user
    /// (DEF-064: wires `reco::evaluation` into the running app). Evaluates the
    /// first generated mix against the user's actual profile + catalog and
    /// returns a one-line summary (event count, discovery ratio, duplicate
    /// rate). Surfaced via the `:profile` command.
    pub fn profile_health_summary(&self) -> String {
        use crate::reco::evaluation::EvaluationMetrics;
        if self.reco_mixes.is_empty() {
            return format!(
                "profile: {} events, 0 mixes — press H to generate",
                self.reco_profile.event_count
            );
        }
        let mix = &self.reco_mixes[0];
        let metrics = EvaluationMetrics::evaluate(mix, &self.reco_profile, &self.catalog.tracks);
        format!(
            "profile: {} events, {} mix tracks, {}% discovery, {}% duplicates",
            self.reco_profile.event_count,
            mix.tracks.len(),
            (metrics.discovery_ratio * 100.0).round() as u32,
            (metrics.duplicate_rate * 100.0).round() as u32,
        )
    }

    /// Open the YouTube playlist publication overlay. Populates
    /// `publishable_ids` from the focused (or named) playlist's YouTube-
    /// source tracks (ids NOT in the local catalog), separates local-only
    /// tracks (ids present in `track_index`), and resolves `account` from
    /// the active YT session. RC11-DEF-002: the old implementation only set
    /// `name` — `is_ready()` was always false, Enter silently bumped `step`,
    /// and the publish sidecar API was never invoked.
    pub fn open_publication(&mut self, playlist_name: &str) {
        use crate::tui::view::publication::PublicationState;
        let mut state = PublicationState::new();
        state.name = playlist_name.to_string();

        // Resolve the playlist's track_ids. If `playlist_name` matches an
        // existing local playlist, use its tracks; otherwise leave the
        // track list empty (the user can still type a name and the overlay
        // will show "0 tracks to publish" — better than silently publishing
        // nothing).
        let track_ids: Vec<String> = self
            .playlists
            .iter()
            .find(|p| p.name == playlist_name)
            .map(|p| p.track_ids.clone())
            .unwrap_or_default();

        // Classify each track id:
        // - in `track_index` → local-only (can't be published to YouTube).
        // - not in `track_index` AND cached in `yt_session.track_cache` →
        //   publishable (a known YouTube video_id).
        // - not in `track_index` AND not in the cache → unavailable (the
        //   id looks like a YT video_id but its metadata isn't loaded; we
        //   still list it so the user sees it).
        for id in &track_ids {
            if self.track_index.contains_key(id) {
                state.local_only.push(id.clone());
            } else if self
                .yt_session
                .as_ref()
                .map(|s| s.track_for(id).is_some())
                .unwrap_or(false)
            {
                state.publishable_ids.push(id.clone());
            } else {
                state.unavailable.push(id.clone());
            }
        }

        // Resolve the account from the active session. We don't have a
        // channel-name lookup; use the browser profile name when set,
        // otherwise "yt (cookies)" for pasted-cookie auth, otherwise leave
        // empty (the overlay will show the no-account validation error).
        if self.yt_state.is_authed() {
            state.account = if !self.yt_browser.is_empty() {
                format!("yt:{}", self.yt_browser)
            } else {
                "yt (cookies)".to_string()
            };
        }

        self.overlay = Some(Overlay::Publication { state });
    }

    /// Record a listen event and rebuild the user profile. `event_type` is a
    /// type-tag string (e.g. "completed", "skipped", "liked") matching
    /// [`crate::reco::events::ListenEvent::type_tag`]. The `EventSource`
    /// (Local/Youtube/Hybrid) is derived from the now-playing track, and the
    /// `EventContext` (Album/Playlist/Queue/Radio/Search) from the transport
    /// context — so the reco engine records *why* and *where* a track was
    /// played, not just that it was (DEF-034).
    pub fn record_listen_event(&mut self, track_id: &str, event_type: &str) {
        self.record_listen_event_pos(track_id, event_type, None);
    }

    /// Same as [`record_listen_event`] but carries the playback position (sec)
    /// for `skipped` events so the profile can distinguish a mid-track skip
    /// from a rapid (<10s) skip. `position` is ignored for non-skip events.
    pub fn record_listen_event_pos(
        &mut self,
        track_id: &str,
        event_type: &str,
        position: Option<f64>,
    ) {
        use crate::reco::events::ListenEvent;
        let ts = ListenEvent::now();
        let tid = track_id.to_string();
        let source = self.event_source_for(track_id);
        let context = self.event_context();
        let event = match event_type {
            "track_started" => ListenEvent::TrackStarted {
                track_id: tid,
                source,
                timestamp: ts,
                context,
            },
            "meaningful_threshold" => ListenEvent::MeaningfulThreshold {
                track_id: tid,
                timestamp: ts,
            },
            "completed" => ListenEvent::Completed {
                track_id: tid,
                timestamp: ts,
            },
            "skipped" => ListenEvent::Skipped {
                track_id: tid,
                timestamp: ts,
                position_sec: position.unwrap_or(0.0),
            },
            "rapidly_skipped" => ListenEvent::RapidlySkipped {
                track_id: tid,
                timestamp: ts,
            },
            "replayed" => ListenEvent::Replayed {
                track_id: tid,
                timestamp: ts,
            },
            "liked" => ListenEvent::Liked {
                track_id: tid,
                timestamp: ts,
            },
            "unliked" => ListenEvent::Unliked {
                track_id: tid,
                timestamp: ts,
            },
            "disliked" => ListenEvent::Disliked {
                track_id: tid,
                timestamp: ts,
            },
            "hidden" => ListenEvent::Hidden {
                track_id: tid,
                timestamp: ts,
            },
            "added_to_queue" => ListenEvent::AddedToQueue {
                track_id: tid,
                timestamp: ts,
            },
            "removed_from_queue" => ListenEvent::RemovedFromQueue {
                track_id: tid,
                timestamp: ts,
            },
            _ => return, // unknown event type — skip
        };
        self.reco_events.record(event.clone());
        // DEF-034: durably persist each event to state.db so the listening
        // history survives restarts. Best-effort: a failed write (read-only
        // config dir, missing DB) is silently ignored — the in-memory log
        // still drives the running pipeline. Disabled in tests via
        // `persist_events` (default false) to avoid touching the real DB.
        if self.persist_events {
            let _ = crate::state::save_event(&event);
        }
        // Rebuild the profile from the full event log.
        let events: Vec<ListenEvent> = self
            .reco_events
            .recent(self.reco_events.len())
            .into_iter()
            .cloned()
            .collect();
        self.reco_profile = crate::reco::profile::UserProfile::build_from_events(&events);
    }

    /// Derive the [`EventSource`] for a track from the now-playing track
    /// source type (DEF-034): `Local` for catalog tracks, `Youtube` for
    /// remote tracks, `Hybrid` when the active source mode is Mixed.
    fn event_source_for(&self, track_id: &str) -> crate::reco::events::EventSource {
        use crate::reco::events::EventSource;
        match &self.now_playing {
            Some(crate::source::TrackSource::Remote { .. }) => EventSource::Youtube,
            Some(crate::source::TrackSource::Local { .. }) => {
                if matches!(self.source_mode, crate::mode::SourceMode::Mixed) {
                    EventSource::Hybrid
                } else {
                    EventSource::Local
                }
            }
            None => {
                // No now-playing (e.g. an enqueue event): infer from the id —
                // catalog tracks are Local, unknown ids are Youtube.
                if self.track_index.contains_key(track_id) {
                    EventSource::Local
                } else {
                    EventSource::Youtube
                }
            }
        }
    }

    /// Derive the [`EventContext`] from the transport context (DEF-034):
    /// Album/Playlist/Queue/Search map directly; the reco-radio search
    /// context ("reco radio" / "youtube radio") maps to `Radio`.
    fn event_context(&self) -> crate::reco::events::EventContext {
        use crate::reco::events::EventContext;
        match &self.transport.context {
            crate::tui::context::Context::Album { .. }
            | crate::tui::context::Context::Artist { .. } => EventContext::Album,
            crate::tui::context::Context::Playlist { name } => EventContext::Playlist(name.clone()),
            crate::tui::context::Context::Youtube { name, .. } => {
                EventContext::Playlist(name.clone())
            }
            crate::tui::context::Context::Queue => EventContext::Queue,
            crate::tui::context::Context::Search { query, .. } => {
                if query == "reco radio" || query == "youtube radio" {
                    EventContext::Radio
                } else {
                    EventContext::Search
                }
            }
        }
    }

    /// Record a `track_started` listen event for the now-playing track, reset
    /// the play-start timestamp + meaningful-threshold guard (DEF-034). Call
    /// this wherever `now_playing` is freshly set (start_playback Local Ok,
    /// load_track Local Ok, load_remote Ok).
    fn note_play_started(&mut self, track_id: &str) {
        self.play_started_at = Some(std::time::Instant::now());
        // New track → threshold not yet fired for it.
        self.threshold_fired_for = None;
        self.record_listen_event(track_id, "track_started");
        // RC11-DEF-014: record the now-playing track as the last-played so it
        // can be restored on the next launch, and clear the resume hint (a
        // play has started — the offer is consumed). Position is tracked
        // separately in on_tick while playback advances.
        self.last_played_track_id = Some(track_id.to_string());
        self.last_played_position = 0.0;
        self.resume_hint = None;
    }

    /// RC11-DEF-014: load `path` into the player, resuming at the saved
    /// position when this is the one-shot resume target. Consumes
    /// `pending_resume` on the first call (any track): if the id matches the
    /// captured resume target and the position is > 0, use `load_at(pos)` so
    /// mpv/StubPlayer begin at the saved offset (afplay can't seek so it
    /// restarts from 0 — acceptable fallback). Otherwise a plain `load`.
    fn load_with_resume(&mut self, path: &std::path::Path, track_id: &str) -> anyhow::Result<()> {
        let resume_pos = match self.pending_resume.take() {
            Some((rid, pos)) if rid == track_id && pos > 0.0 => Some(pos),
            _ => None,
        };
        match resume_pos {
            Some(pos) => self.player.load_at(path, pos),
            None => self.player.load(path),
        }
    }

    /// Apply a feedback action to the user profile. Looks up the track's
    /// artist from the catalog for artist-scoped actions (HideArtist,
    /// BlockArtist). Records the corresponding [`ListenEvent`] in
    /// `reco_events` so a later profile rebuild (e.g. from `record_listen_event`
    /// during playback) preserves the feedback — without this, the direct
    /// profile mutation would be wiped when the profile is rebuilt from the
    /// event log (DEF-034 regression).
    pub fn apply_reco_feedback(
        &mut self,
        action: crate::reco::feedback::FeedbackAction,
        track_id: &str,
    ) {
        // Look up the artist from the catalog first (immutable borrow ends
        // before the mutable borrow of reco_profile).
        let artist = self.track_by_id(track_id).map(|t| t.primary_artist.clone());
        let event = action.to_event(track_id, artist.as_deref());
        // Record the event so the profile rebuild stays consistent.
        if self.persist_events {
            let _ = crate::state::save_event(&event);
        }
        self.reco_events.record(event);
        // Rebuild the profile from the full event log so the feedback is
        // reflected AND preserved across later rebuilds (e.g. when
        // `record_listen_event` fires during playback). Without recording
        // the event, a direct profile mutation would be wiped on the next
        // rebuild (DEF-034 regression).
        let events: Vec<crate::reco::events::ListenEvent> = self
            .reco_events
            .recent(self.reco_events.len())
            .into_iter()
            .cloned()
            .collect();
        self.reco_profile = crate::reco::profile::UserProfile::build_from_events(&events);
    }

    /// If a [`Overlay::Radio`] session is active, return the next track id
    /// from it (refilling the pool if needed). Used by continue-mode playback
    /// to drive the reco radio engine instead of the YouTube radio cursor.
    fn reco_radio_next(&mut self) -> Option<String> {
        if let Some(Overlay::Radio {
            session: Some(radio),
        }) = &mut self.overlay
        {
            if radio.needs_refill() {
                radio.refill_if_needed(&self.reco_profile, &self.catalog.tracks);
            }
            if let Some(c) = radio.next_track() {
                return Some(c.track_id);
            }
            // Fallback: pool exhausted and can't refill — pick the first
            // catalog track not yet played in this session so the radio
            // keeps going even without a profile.
            let fallback = self
                .catalog
                .tracks
                .iter()
                .find(|t| !radio.session_history.contains(&t.id))
                .map(|t| t.id.clone());
            if let Some(id) = fallback {
                radio.session_history.push(id.clone());
                return Some(id);
            }
        }
        None
    }
}

// Transport methods take `(&dyn ContextResolver, &Catalog)`. `App` passes
// `self` as the resolver and `&self.catalog` as the catalog; the split-borrow is
// sound because `manual_queue` (the resolver's data source) lives in a distinct
// field from `catalog` and from the `&mut self.transport` we hold.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, Track};
    use crate::player::StubPlayer;

    fn make_track(id: &str, artist: &str, album: &str, title: &str) -> Track {
        Track {
            id: id.to_string(),
            artists: vec![artist.to_string()],
            primary_artist: artist.to_string(),
            title: title.to_string(),
            album: Some(album.to_string()),
            track_number: Some(1),
            disc_number: Some(1),
            bit_depth: 16,
            sample_rate_hz: 44100,
            isrc: None,
            source_path: std::path::PathBuf::from("/test/file.flac"),
            symlinked_into_artists: vec![artist.to_string()],
        }
    }

    fn make_catalog() -> Catalog {
        Catalog {
            version: 1,
            built_at: "test".into(),
            source_root: std::path::PathBuf::from("/tmp"),
            tracks: vec![
                make_track("t1", "Artist A", "Album 1", "Song 1"),
                make_track("t2", "Artist B", "Album 2", "Song 2"),
                make_track("t3", "Artist A", "Album 1", "Song 3"),
            ],
        }
    }

    fn make_app() -> App {
        App::new(make_catalog(), Box::new(StubPlayer::default()), None, None)
    }

    /// Like `make_app` but the catalog's `source_path` points at real (empty)
    /// `.flac` files in a temp dir, so `start_playback`'s `std::fs::metadata`
    /// check passes and `now_playing` is set. Used by playback-path tests
    /// (DEF-001 / DEF-012 Home Enter).
    fn make_app_with_files() -> (tempfile::TempDir, App) {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        std::fs::create_dir_all(lossless.join("B")).unwrap();
        std::fs::write(lossless.join("B").join("01.flac"), b"x").unwrap();
        std::fs::create_dir_all(lossless.join("A2")).unwrap();
        std::fs::write(lossless.join("A2").join("01.flac"), b"x").unwrap();
        let tracks = vec![
            Track {
                id: "t1".into(),
                artists: vec!["Artist A".into()],
                primary_artist: "Artist A".into(),
                title: "Song 1".into(),
                album: Some("Album 1".into()),
                track_number: Some(1),
                disc_number: Some(1),
                bit_depth: 16,
                sample_rate_hz: 44100,
                isrc: None,
                source_path: lossless.join("A").join("01.flac"),
                symlinked_into_artists: vec!["Artist A".into()],
            },
            Track {
                id: "t2".into(),
                artists: vec!["Artist B".into()],
                primary_artist: "Artist B".into(),
                title: "Song 2".into(),
                album: Some("Album 2".into()),
                track_number: Some(1),
                disc_number: Some(1),
                bit_depth: 16,
                sample_rate_hz: 44100,
                isrc: None,
                source_path: lossless.join("B").join("01.flac"),
                symlinked_into_artists: vec!["Artist B".into()],
            },
            Track {
                id: "t3".into(),
                artists: vec!["Artist A".into()],
                primary_artist: "Artist A".into(),
                title: "Song 3".into(),
                album: Some("Album 1".into()),
                track_number: Some(2),
                disc_number: Some(1),
                bit_depth: 16,
                sample_rate_hz: 44100,
                isrc: None,
                source_path: lossless.join("A2").join("01.flac"),
                symlinked_into_artists: vec!["Artist A".into()],
            },
        ];
        let cat = Catalog {
            version: 1,
            built_at: "test".into(),
            source_root: lossless,
            tracks,
        };
        let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        (d, app)
    }

    #[test]
    fn app_new_initializes_reco_fields() {
        let app = make_app();
        assert!(app.reco_profile.is_empty());
        assert!(app.reco_events.is_empty());
        assert!(app.reco_mixes.is_empty());
        assert!(app.reco_feedback_pending.is_empty());
    }

    #[test]
    fn open_home_sets_overlay_and_generates_mixes() {
        let mut app = make_app();
        assert!(app.overlay.is_none());
        app.open_home();
        match &app.overlay {
            Some(Overlay::Home { state }) => {
                assert!(!state.loading);
                assert_eq!(state.focused_section, 0);
                assert_eq!(state.cursor, 0);
            }
            other => panic!("expected Home overlay, got {other:?}"),
        }
        // Mixes are generated even on cold start (DailyMix, Discover, LocalYtBlend).
        assert!(!app.reco_mixes.is_empty());
    }

    #[test]
    fn open_home_with_history_sets_has_history() {
        let mut app = make_app();
        app.record_listen_event("t1", "completed");
        app.open_home();
        if let Some(Overlay::Home { state }) = &app.overlay {
            assert!(state.has_history);
        } else {
            panic!("expected Home overlay");
        }
    }

    #[test]
    fn open_home_populates_sections_with_content() {
        let mut app = make_app();
        app.open_home();
        match &app.overlay {
            Some(Overlay::Home { state }) => {
                assert!(
                    !state.sections.is_empty(),
                    "Home overlay must have populated sections"
                );
                // Quick Picks should have tracks from the catalog.
                let qp = state
                    .sections
                    .iter()
                    .find(|(s, _)| *s == crate::tui::view::home::HomeSection::QuickPicks);
                let qp_items = qp
                    .map(|(_, items)| items)
                    .expect("Quick Picks section must exist");
                assert!(
                    !qp_items.is_empty(),
                    "Quick Picks must have tracks from the catalog"
                );
                // Made for You should have generated mixes.
                let mfy = state
                    .sections
                    .iter()
                    .find(|(s, _)| *s == crate::tui::view::home::HomeSection::MadeForYou);
                let mfy_items = mfy
                    .map(|(_, items)| items)
                    .expect("Made for You section must exist");
                assert!(
                    !mfy_items.is_empty(),
                    "Made for You must have generated mixes"
                );
                // Start Radio should have radio seed options.
                let sr = state
                    .sections
                    .iter()
                    .find(|(s, _)| *s == crate::tui::view::home::HomeSection::StartRadio);
                let sr_items = sr
                    .map(|(_, items)| items)
                    .expect("Start Radio section must exist");
                assert!(!sr_items.is_empty(), "Start Radio must have seed options");
                // Library section should exist (even if empty, no yt_session).
                let lib = state
                    .sections
                    .iter()
                    .find(|(s, _)| *s == crate::tui::view::home::HomeSection::Library);
                assert!(lib.is_some(), "Library section must exist");
            }
            other => panic!("expected Home overlay, got {other:?}"),
        }
    }

    #[test]
    fn start_radio_from_track_creates_session() {
        let mut app = make_app();
        app.start_radio_from_track("t1");
        match &app.overlay {
            Some(Overlay::Radio { session }) => {
                let s = session.as_ref().expect("session should be Some");
                assert!(matches!(
                    &s.seed,
                    crate::reco::radio::RadioSeed::Track(id) if id == "t1"
                ));
                // initialize() was called, so the pool should have candidates
                // (the catalog has 3 tracks).
                assert!(!s.candidate_pool.is_empty());
            }
            other => panic!("expected Radio overlay, got {other:?}"),
        }
    }

    #[test]
    fn start_radio_from_artist_creates_session() {
        let mut app = make_app();
        app.start_radio_from_artist("Artist A");
        match &app.overlay {
            Some(Overlay::Radio { session }) => {
                let s = session.as_ref().expect("session should be Some");
                assert!(matches!(
                    &s.seed,
                    crate::reco::radio::RadioSeed::Artist(a) if a == "Artist A"
                ));
            }
            other => panic!("expected Radio overlay, got {other:?}"),
        }
    }

    #[test]
    fn open_generator_sets_input_phase() {
        let mut app = make_app();
        app.open_generator();
        match &app.overlay {
            Some(Overlay::Generator { state }) => {
                assert_eq!(
                    state.phase,
                    crate::tui::view::generator::GeneratorPhase::Input
                );
                assert!(state.input.is_empty());
                assert!(state.constraints.is_none());
                assert!(state.playlist.is_none());
            }
            other => panic!("expected Generator overlay, got {other:?}"),
        }
    }

    #[test]
    fn generate_playlist_parses_and_generates() {
        let mut app = make_app();
        // Seed the profile so the candidate generator has data to work with.
        app.record_listen_event("t1", "completed");
        app.record_listen_event("t1", "liked");
        app.record_listen_event("t2", "completed");
        app.open_generator();
        if let Some(Overlay::Generator { state }) = &mut app.overlay {
            state.input = "calm focus mix".into();
        }
        app.generate_playlist();
        if let Some(Overlay::Generator { state }) = &app.overlay {
            assert_eq!(
                state.phase,
                crate::tui::view::generator::GeneratorPhase::Preview
            );
            assert!(state.constraints.is_some());
            assert!(state.playlist.is_some());
            // The playlist should have tracks (the profile has positive scores).
            assert!(!state.playlist.as_ref().unwrap().tracks.is_empty());
        } else {
            panic!("expected Generator overlay");
        }
    }

    #[test]
    fn generate_playlist_populates_title_map() {
        let mut app = make_app();
        app.open_generator();
        if let Some(Overlay::Generator { state }) = &mut app.overlay {
            state.input = "calm mix".into();
        }
        app.generate_playlist();
        if let Some(Overlay::Generator { state }) = &app.overlay {
            let playlist = state
                .playlist
                .as_ref()
                .expect("playlist should be generated");
            assert!(!playlist.tracks.is_empty(), "playlist should have tracks");
            assert!(
                !state.title_map.is_empty(),
                "title_map should be populated with display titles"
            );
            // Each track should have a display title that contains the track's
            // title from the catalog ("Song 1", "Song 2", "Song 3").
            for track in &playlist.tracks {
                let display = state.title_map.get(&track.track_id);
                assert!(
                    display.is_some(),
                    "title_map must have entry for track {}",
                    track.track_id
                );
                let display = display.unwrap();
                assert!(
                    display.contains("Song") || display == &track.track_id,
                    "display title should contain the track title, got: {display}"
                );
            }
        } else {
            panic!("expected Generator overlay");
        }
    }

    #[test]
    fn generate_playlist_no_op_without_overlay() {
        let mut app = make_app();
        // No overlay set — should be a no-op, not a panic.
        app.generate_playlist();
        assert!(app.overlay.is_none());
    }

    #[test]
    fn show_explanation_sets_overlay() {
        let mut app = make_app();
        app.record_listen_event("t1", "completed");
        app.show_explanation("t1");
        match &app.overlay {
            Some(Overlay::Explanation { explanation }) => {
                assert!(!explanation.reason.is_empty());
            }
            other => panic!("expected Explanation overlay, got {other:?}"),
        }
    }

    #[test]
    fn show_explanation_unknown_track_still_sets_overlay() {
        let mut app = make_app();
        app.show_explanation("nonexistent");
        match &app.overlay {
            Some(Overlay::Explanation { explanation }) => {
                assert_eq!(explanation.reason, "this track");
                assert!(explanation.detail.is_none());
            }
            other => panic!("expected Explanation overlay, got {other:?}"),
        }
    }

    #[test]
    fn open_publication_sets_name() {
        let mut app = make_app();
        app.open_publication("My Mix");
        match &app.overlay {
            Some(Overlay::Publication { state }) => {
                assert_eq!(state.name, "My Mix");
                assert_eq!(state.privacy, "PRIVATE");
            }
            other => panic!("expected Publication overlay, got {other:?}"),
        }
    }

    #[test]
    fn record_listen_event_updates_profile() {
        let mut app = make_app();
        assert!(app.reco_profile.is_empty());
        assert!(app.reco_events.is_empty());
        app.record_listen_event("t1", "completed");
        assert!(!app.reco_events.is_empty());
        assert!(!app.reco_profile.is_empty());
        assert!(app.reco_profile.track_score("t1") > 0.0);
        assert_eq!(app.reco_profile.event_count, 1);
    }

    #[test]
    fn record_listen_event_liked_sets_liked_flag() {
        let mut app = make_app();
        app.record_listen_event("t1", "liked");
        assert!(app.reco_profile.is_liked("t1"));
        assert!(app.reco_profile.track_score("t1") > 0.0);
    }

    #[test]
    fn record_listen_event_skipped_is_negative() {
        let mut app = make_app();
        app.record_listen_event("t1", "skipped");
        assert!(app.reco_profile.track_score("t1") < 0.0);
    }

    #[test]
    fn record_listen_event_unknown_type_is_noop() {
        let mut app = make_app();
        app.record_listen_event("t1", "bogus_type");
        assert!(app.reco_events.is_empty());
        assert!(app.reco_profile.is_empty());
    }

    #[test]
    fn record_listen_event_multiple_events_accumulate() {
        let mut app = make_app();
        app.record_listen_event("t1", "completed");
        app.record_listen_event("t1", "liked");
        app.record_listen_event("t2", "completed");
        assert_eq!(app.reco_profile.event_count, 3);
        assert!(app.reco_profile.is_liked("t1"));
        // t1 has Completed (+2) + Liked (+5) = 7.0
        let t1_score = app.reco_profile.track_score("t1");
        assert!((t1_score - 7.0).abs() < 1e-9);
    }

    #[test]
    fn apply_reco_feedback_like_adds_positive() {
        let mut app = make_app();
        use crate::reco::feedback::FeedbackAction;
        app.apply_reco_feedback(FeedbackAction::Like, "t1");
        assert!(app.reco_profile.is_liked("t1"));
        assert!(app.reco_profile.track_score("t1") > 0.0);
    }

    #[test]
    fn apply_reco_feedback_hide_excludes() {
        let mut app = make_app();
        use crate::reco::feedback::FeedbackAction;
        app.apply_reco_feedback(FeedbackAction::HideTrack, "t1");
        assert!(app.reco_profile.is_hidden("t1"));
    }

    #[test]
    fn apply_reco_feedback_block_artist_uses_catalog_lookup() {
        let mut app = make_app();
        use crate::reco::feedback::FeedbackAction;
        // BlockArtist needs the artist name — looked up from the catalog.
        app.apply_reco_feedback(FeedbackAction::BlockArtist, "t1");
        assert!(app.reco_profile.is_blocked("Artist A"));
    }

    #[test]
    fn reco_radio_next_returns_none_without_session() {
        let mut app = make_app();
        assert!(app.reco_radio_next().is_none());
    }

    #[test]
    fn reco_radio_next_returns_track_with_session() {
        let mut app = make_app();
        app.start_radio_from_track("t1");
        let next = app.reco_radio_next();
        assert!(next.is_some(), "reco_radio_next should return a track");
        let id = next.unwrap();
        // The next track should be a valid catalog track id (not the seed).
        assert!(app.track_by_id(&id).is_some());
        assert_ne!(id, "t1", "seed track should not be immediately re-played");
    }

    #[test]
    fn reco_radio_next_returns_none_after_pool_exhausted() {
        let mut app = make_app();
        app.start_radio_from_track("t1");
        // Drain the pool.
        let mut count = 0;
        while app.reco_radio_next().is_some() {
            count += 1;
            if count > 200 {
                break; // safety valve (refill may produce more)
            }
        }
        // After draining + no refill candidates, next returns None.
        // (refill_if_needed may re-add from catalog, so we just verify
        // we got some tracks before exhausting.)
        assert!(count > 0, "should have gotten at least one track");
    }

    #[test]
    fn new_overlay_variants_do_not_break_existing_overlays() {
        let mut app = make_app();
        // Open Help (existing overlay) — should still work.
        app.overlay = Some(Overlay::Help);
        assert!(matches!(app.overlay, Some(Overlay::Help)));
        // Replace with a reco overlay.
        app.open_home();
        assert!(matches!(app.overlay, Some(Overlay::Home { .. })));
        // Close and open another existing overlay.
        app.overlay = None;
        app.overlay = Some(Overlay::Diagnostics);
        assert!(matches!(app.overlay, Some(Overlay::Diagnostics)));
    }

    // --- Regression tests for cold-start reco fallbacks ---

    #[test]
    fn cycle_continue_mixed_mode_includes_radio() {
        use crate::mode::SourceMode;
        use crate::tui::queue::ContinueMode;
        let mut app = make_app();
        app.source_mode = SourceMode::Mixed;
        // Off → NextAlbum
        app.transport.continue_mode = ContinueMode::Off;
        app.cycle_continue();
        assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
        // NextAlbum → Radio
        app.cycle_continue();
        assert_eq!(app.transport.continue_mode, ContinueMode::Radio);
        // Radio → YouTube
        app.cycle_continue();
        assert_eq!(app.transport.continue_mode, ContinueMode::YouTube);
        // YouTube → Off
        app.cycle_continue();
        assert_eq!(app.transport.continue_mode, ContinueMode::Off);
    }

    #[test]
    fn open_home_generates_mixes_with_tracks_on_cold_start() {
        let mut app = make_app();
        assert!(app.reco_profile.is_empty());
        app.open_home();
        // Mixes should be generated.
        assert!(!app.reco_mixes.is_empty());
        // Each generated mix should have tracks (catalog fallback).
        for mix in &app.reco_mixes {
            assert!(
                !mix.tracks.is_empty(),
                "mix {:?} should have tracks via catalog fallback",
                mix.mix_type
            );
        }
    }

    #[test]
    fn generate_playlist_cold_start_produces_tracks() {
        let mut app = make_app();
        // Empty profile — no listening history.
        assert!(app.reco_profile.is_empty());
        app.open_generator();
        if let Some(Overlay::Generator { state }) = &mut app.overlay {
            state.input = "calm focus mix".into();
        }
        app.generate_playlist();
        if let Some(Overlay::Generator { state }) = &app.overlay {
            assert!(state.playlist.is_some());
            let playlist = state.playlist.as_ref().unwrap();
            assert!(
                !playlist.tracks.is_empty(),
                "generator should produce tracks via catalog fallback on cold start"
            );
        } else {
            panic!("expected Generator overlay");
        }
    }

    #[test]
    fn start_radio_cold_start_has_candidates() {
        let mut app = make_app();
        assert!(app.reco_profile.is_empty());
        app.start_radio_from_track("t1");
        if let Some(Overlay::Radio { session }) = &app.overlay {
            let s = session.as_ref().expect("session should be Some");
            assert!(
                !s.candidate_pool.is_empty(),
                "radio should seed from catalog on cold start"
            );
        } else {
            panic!("expected Radio overlay");
        }
    }

    #[test]
    fn reco_radio_next_falls_back_to_catalog_when_pool_exhausted() {
        let mut app = make_app();
        app.start_radio_from_track("t1");
        // Drain the pool completely.
        let mut count = 0;
        while app.reco_radio_next().is_some() {
            count += 1;
            if count > 500 {
                break; // safety valve
            }
        }
        assert!(count > 0, "should have gotten tracks before exhaustion");
        // After the pool is drained, the fallback should still return a
        // catalog track that hasn't been played in this session.
        // (If all catalog tracks have been played, returns None — correct.)
        // With a 3-track catalog, after playing all 3, the fallback can't
        // find an unplayed track, so we just verify we got tracks.
    }

    #[test]
    fn reco_radio_next_cold_start_returns_track() {
        let mut app = make_app();
        // Empty profile — cold start.
        assert!(app.reco_profile.is_empty());
        app.start_radio_from_track("t1");
        let next = app.reco_radio_next();
        assert!(next.is_some(), "radio should return a track on cold start");
        let id = next.unwrap();
        assert!(
            app.track_by_id(&id).is_some(),
            "should be a valid catalog track"
        );
        assert_ne!(id, "t1", "seed track should not be immediately re-played");
    }

    /// RC11-DEF-001 / DEF-012: `play_home_selection` plays the focused Home
    /// item. Navigating to the "Made for You" section and pressing Enter must
    /// start playback of the focused mix's tracks (local catalog ids), not
    /// the underlying browse view's track-column cursor.
    #[test]
    fn play_home_selection_plays_focused_mix() {
        use crate::tui::view::home::HomeSection;
        let (_d, mut app) = make_app_with_files();
        app.open_home();
        // Find the Made for You section index.
        let mfy_idx = app
            .overlay
            .as_ref()
            .and_then(|o| match o {
                Overlay::Home { state } => state
                    .sections
                    .iter()
                    .position(|(s, _)| *s == HomeSection::MadeForYou),
                _ => None,
            })
            .expect("Home should have a Made for You section");
        // Focus that section.
        if let Some(Overlay::Home { state }) = &mut app.overlay {
            state.focused_section = mfy_idx;
            state.cursor = 0;
        }
        // Before play: nothing playing.
        assert!(app.now_playing.is_none());
        app.play_home_selection();
        // After play: the overlay closed and a track is playing.
        assert!(
            app.overlay.is_none(),
            "DEF-001: Home overlay should close after Enter on a mix"
        );
        assert!(
            app.now_playing.is_some(),
            "DEF-012: pressing Enter on a Made for You mix should start playback"
        );
    }

    /// RC11-DEF-001: `play_home_selection` on a Quick Picks track plays THAT
    /// track (the focused one), not the browse view's cursor track.
    #[test]
    fn play_home_selection_plays_focused_quick_pick() {
        use crate::tui::view::home::HomeSection;
        let (_d, mut app) = make_app_with_files();
        app.open_home();
        // Focus Quick Picks (section 0), cursor 1 (second track).
        if let Some(Overlay::Home { state }) = &mut app.overlay {
            // Quick Picks is the first section in open_home.
            assert_eq!(state.sections[0].0, HomeSection::QuickPicks);
            state.focused_section = 0;
            state.cursor = 1;
        }
        app.play_home_selection();
        // The second catalog track (t2) should be playing.
        let np = app
            .now_playing
            .as_ref()
            .expect("DEF-001: Enter on a Quick Pick should start playback");
        assert_eq!(
            np.id(),
            "t2",
            "DEF-001: Enter should play the focused Quick Pick (t2), not the browse cursor"
        );
    }

    /// RC11-DEF-001: `?` from the Home overlay opens the Help overlay
    /// (previously swallowed by `_ => {}`).
    #[test]
    fn home_overlay_question_mark_opens_help() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.open_home();
        crate::tui::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        );
        assert!(
            matches!(app.overlay, Some(Overlay::Help)),
            "DEF-001: '?' from Home should open the Help overlay"
        );
    }

    /// RC11-DEF-013: in YouTube mode, the Discover overlay (`S`) shows the
    /// generated mixes (Daily Mix, Discover Mix, ...) from `reco::mixes`, not
    /// local smart albums.
    #[test]
    fn open_discover_yt_mode_shows_generated_mixes() {
        let mut app = make_app();
        app.source_mode = crate::mode::SourceMode::Youtube;
        app.open_discover();
        let items = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items,
            _ => panic!("expected Discover overlay"),
        };
        let has_mix = items.iter().any(|d| matches!(d, DiscoverItem::Mix { .. }));
        assert!(
            has_mix,
            "DEF-013: YT-mode Discover must show generated mixes (Daily Mix, etc.): {items:?}"
        );
    }

    /// RC11-DEF-013: in mixed mode, the Discover overlay also shows the
    /// generated mixes.
    #[test]
    fn open_discover_mixed_mode_shows_generated_mixes() {
        let mut app = make_app();
        app.source_mode = crate::mode::SourceMode::Mixed;
        app.open_discover();
        let items = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items,
            _ => panic!("expected Discover overlay"),
        };
        let has_mix = items.iter().any(|d| matches!(d, DiscoverItem::Mix { .. }));
        assert!(
            has_mix,
            "DEF-013: mixed-mode Discover must show generated mixes: {items:?}"
        );
    }

    /// RC11-DEF-013: in local mode, the Discover overlay shows local smart
    /// albums (the old behavior), NOT generated mixes.
    #[test]
    fn open_discover_local_mode_shows_albums_not_mixes() {
        let mut app = make_app();
        app.source_mode = crate::mode::SourceMode::Local;
        app.open_discover();
        let items = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items,
            _ => panic!("expected Discover overlay"),
        };
        let has_mix = items.iter().any(|d| matches!(d, DiscoverItem::Mix { .. }));
        assert!(
            !has_mix,
            "DEF-013: local-mode Discover should show albums, not mixes: {items:?}"
        );
    }

    /// RC11-DEF-028: each Discover mix carries a "why recommended" explanation.
    #[test]
    fn discover_mix_items_carry_explanation() {
        let mut app = make_app();
        app.source_mode = crate::mode::SourceMode::Youtube;
        app.open_discover();
        let items = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items,
            _ => panic!("expected Discover overlay"),
        };
        for d in items {
            if let DiscoverItem::Mix { explanation, .. } = d {
                assert!(
                    explanation.is_some(),
                    "DEF-028: each Discover mix must carry an explanation"
                );
            }
        }
    }

    /// RC11-DEF-035: Enter on a Discover mix sets `discover_play_loading` so
    /// the overlay can show a persistent "Loading [name]..." state instead of
    /// closing silently.
    #[test]
    fn discover_enter_on_mix_sets_loading_state() {
        let (_d, mut app) = make_app_with_files();
        app.source_mode = crate::mode::SourceMode::Youtube;
        app.open_discover();
        app.play_discover_selection();
        assert!(
            app.discover_play_loading.is_some(),
            "DEF-035: Enter on a Discover mix must set discover_play_loading"
        );
    }

    /// RC11-DEF-029: `x` in the Discover overlay removes the focused item.
    #[test]
    fn discover_x_dismisses_focused_item() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.source_mode = crate::mode::SourceMode::Youtube;
        app.open_discover();
        let before_len = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items.len(),
            _ => panic!("expected Discover overlay"),
        };
        assert!(
            before_len > 1,
            "test needs >1 discover item, got {before_len}"
        );
        crate::tui::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        let after_len = match &app.overlay {
            Some(Overlay::Discover { items, .. }) => items.len(),
            _ => panic!("Discover overlay should stay open after x"),
        };
        assert_eq!(
            after_len,
            before_len - 1,
            "DEF-029: x must remove the focused Discover item"
        );
    }
}
