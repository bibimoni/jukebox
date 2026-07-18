//! Auth, caching, and the autoplay radio cursor over a [`Sidecar`].
//!
//! [`Session`] owns one sidecar for the app's lifetime, the cookie material,
//! and small caches: a video-id → [`RemoteTrack`] map (so the view layer can
//! render titles for ids it's seen) and a 2-entry URL cache (current + next,
//! so gapless handoff has the next URL ready).
//!
//! [`RadioCursor`] drives the CONT=YouTube autoplay engine (spec §3.4): when a
//! context exhausts, it asks the sidecar's `get_watch_playlist(radio=True)` for
//! the next tracks YouTube would auto-play.
//!
//! **Blocking note:** `roundtrip` send+polls the sidecar with a bounded
//! deadline. `start_playback`/`resolve_url` call it at a play boundary (once,
//! bounded ~3s) — acceptable because the poll loop only calls these on
//! Enter/next, not every tick.

use crate::source::{RemoteTrack, StreamFormat};
use crate::yt::proto::{AuthStatus, RemoteTrackSummary, Request, ResolvedUrl, Response};
use crate::yt::sidecar::Sidecar;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// Which request kind a sidecar error came from, so `App::on_tick` can decide
/// whether to touch the search overlay's `searching` flag (only a Search error
/// should) vs. just surface the message in the footer (any error). `Search`
/// carries the QUERY so on_tick can distinguish an error for the overlay's
/// CURRENT query from one for a prior, abandoned query — without it, an error
/// for "adele" would clear `searching` while "adeles" is still in flight,
/// silently dropping "adeles"'s results when they land.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrorScope {
    Search(String),
    Other,
}

/// One video_id's cached resolve(s). Holds BOTH a fast (tv_embedded, AAC 129k)
/// and a premium (tv/web, AAC 256k) URL so a premium pre-resolve doesn't evict
/// the fast one we may already be playing. `fmt` is the best-known format
/// (premium wins when present, so the UI shows the upgrade).
#[derive(Clone, Default)]
struct CachedResolve {
    video_id: String,
    fast: Option<(String, Option<f64>)>,
    premium: Option<(String, Option<f64>)>,
    fmt: Option<StreamFormat>,
}

/// Resolves autoplay radio queues. Implemented by [`Session`] against the real
/// sidecar; tests use a fake. Keeping this a trait lets `RadioCursor` be unit-
/// tested without spawning Python.
pub trait YtClient {
    fn get_watch_playlist(&mut self, video_id: &str) -> Result<Vec<String>>;
}

/// Resolve the config base dir, returning `None` when the only fallback is
/// `/tmp/.config` (world-readable on multi-user systems). Cookie secrets must
/// never live there, so `cookies_file_opt()` returns `None` in that case and
/// callers degrade to guest mode instead of writing secrets to a world-readable
/// path. Config/state files (no secrets) may still use the `/tmp/.config`
/// fallback — see `config::config_path()` / `state::db_path()`.
fn config_base_opt() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)
}

/// Path to the persisted cookies file: `<config_dir>/jukebox/yt-cookies.txt`.
/// Returns `None` when no proper config dir is available (the `/tmp/.config`
/// fallback is refused — cookie secrets must not live in a world-readable
/// location). Callers should degrade to guest mode when `None`.
pub fn cookies_file_opt() -> Option<std::path::PathBuf> {
    let base = config_base_opt()?;
    let dir = base.join("jukebox");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("yt-cookies.txt"))
}

/// Backwards-compatible `PathBuf` wrapper around `cookies_file_opt()`.
/// Returns an empty `PathBuf` when no safe config dir exists (the
/// `/tmp/.config` fallback is refused for secrets). An empty path is benign:
/// `load_cookies()` returns `None`, `remove_file` on it fails silently, and
/// `set_cookies` returns an error. Kept for callers (e.g. `app::yt_logout`)
/// that expect a `PathBuf`.
pub fn cookies_file() -> std::path::PathBuf {
    cookies_file_opt().unwrap_or_default()
}

/// Load persisted cookies (Netscape cookies.txt). `None` if absent/empty or
/// if no safe config dir exists (the `/tmp/.config` fallback is refused).
pub fn load_cookies() -> Option<String> {
    let p = cookies_file_opt()?;
    let s = std::fs::read_to_string(&p).ok()?;
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// The jukebox-owned venv directory (`<config>/jukebox/yt-venv`), created by
/// `:yt setup`. Holds `ytmusicapi`/`yt-dlp`/`browser_cookie3` so the sidecar
/// doesn't depend on the system python having them.
///
/// The `/tmp/.config` fallback is acceptable here: the venv contains only
/// pip packages (no secrets). Cookie secrets use `cookies_file_opt()` which
/// refuses the fallback.
pub fn venv_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.config"));
    base.join("jukebox").join("yt-venv")
}

/// The venv's python (`<venv>/bin/python3`), used as the sidecar interpreter
/// when the venv exists.
pub fn venv_python() -> std::path::PathBuf {
    venv_dir().join("bin").join("python3")
}

/// Create the jukebox venv and install the sidecar requirements into it.
/// Used by `:yt setup`. Runs `python3 -m venv <dir>` then
/// `<venv>/bin/pip install -r <requirements>`. Returns a status string on
/// success, or an error. **Blocks** the caller (~30s one-time) — acceptable
/// because `:yt setup` is an explicit user action.
pub fn run_setup(requirements: &std::path::Path) -> Result<String> {
    let dir = venv_dir();
    // The TUI runs in raw + alt-screen mode, so a child process writing to the
    // inherited terminal would smear pip's progress lines across the UI.
    // Capture venv + pip output to the jukebox cache log so the UI stays clean
    // and the user can still read it if install fails.
    let log_path = setup_log_path();
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(std::path::Path::new(".")));
    // Truncate once up front, then open append-only handles. `Stdio::from(File)`
    // consumes the file and there's no `Stdio::try_clone`, so we re-open per
    // child instead of duplicating one handle.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path);
    let stderr_to = || -> std::process::Stdio {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(std::process::Stdio::from)
            .unwrap_or(std::process::Stdio::null())
    };
    if !venv_python().exists() {
        let status = std::process::Command::new("python3")
            .args(["-m", "venv", dir.to_str().unwrap_or("")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(stderr_to())
            .status()?;
        if !status.success() {
            anyhow::bail!(
                "python3 -m venv failed (exit {status}); see {}",
                log_path.display()
            );
        }
    }
    let pip = dir.join("bin").join("pip");
    let status = std::process::Command::new(&pip)
        .args(["install", "-q", "-r"])
        .arg(requirements)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_to())
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "pip install failed (exit {status}); see {}",
            log_path.display()
        );
    }
    // RC11-DEF-017: compact, prominent confirmation that fits a 100-col
    // footer. The full install log path is surfaced via `:diag` by
    // `App::yt_setup` (calling `setup_log_path()`), so it doesn't need to
    // share the single-line footer with the venv path.
    Ok(format!("YT setup OK · venv: {}", dir.display()))
}

/// Where `:yt setup` writes venv/pip output so it doesn't hit the terminal.
/// Public so `App::yt_setup` can surface the path via the diagnostics overlay
/// (RC11-DEF-017): the footer only shows the venv path, the log path goes in
/// `:diag` so a long home directory doesn't truncate the status message.
pub fn setup_log_path() -> std::path::PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    cache.join("jukebox").join("yt-setup.log")
}

/// The kind of an in-flight request, so drained responses can be paired with
/// what asked for them (the sidecar replies in send order, so this is a FIFO
/// queue). `Resolve` carries the video_id the URL is for.
#[derive(Clone, Debug)]
enum Pending {
    /// Carries the generation tag (Slice 2) so a stale refresh response (from
    /// a prior `send_refresh` call) is dropped by `apply_pair` when
    /// `gen != self.refresh_gen` — the old code had no guard, so a stale
    /// refresh's playlists could overwrite a fresh refresh's.
    Playlists(u64),
    Suggestions(u64),
    /// A fast-tier resolve (tv_embedded, AAC 129k) outstanding.
    Resolve(String),
    /// A premium-tier resolve (tv/web + EJS solver, AAC 256k) outstanding. Kept
    /// as a distinct variant so `matches` discriminates fast vs premium for the
    /// same video_id — otherwise FIFO pairing could apply a fast response to a
    /// premium pending slot (or vice versa).
    ResolvePremium(String),
    /// A search outstanding. Carries the QUERY (not just the kind) so FIFO
    /// pairing tags each Search RESPONSE with the query that asked for it —
    /// NOT a separate single `search_inflight` slot, which a second search's
    /// query would overwrite, tagging the first response with the latest
    /// query and silently dropping the second's results.
    Search(String),
    /// Carries the playlist id so the Tracks response can be routed back to
    /// the matching `YtList`.
    Tracks(String),
    /// A lyrics fetch outstanding. Carries the video_id so on_tick can
    /// discard stale lyrics for a different track.
    Lyrics(String),
    Watch,
    /// A discover (`S` overlay) home-suggestions fetch outstanding. Distinct
    /// from `Suggestions(gen)` (the refresh flow) so a discover response
    /// routes to `pending_discover` WITHOUT disturbing the generation-guarded
    /// refresh tracking (`refresh_gen` / `refresh_remaining`). The wire
    /// request is the same `Request::HomeSuggestions`; only the pending tag
    /// differs, and FIFO pairing (responses in send order) keeps each
    /// `HomeSuggestions` response paired with the kind that asked for it.
    Discover,
    /// A create-playlist request outstanding. Carries the title so the
    /// response can be correlated with the request.
    #[allow(dead_code)]
    CreatePlaylist(String),
    /// An add-playlist-items request outstanding. Carries the playlist id.
    #[allow(dead_code)]
    AddPlaylistItems(String),
    /// A get-liked-songs request outstanding.
    LikedSongs,
    /// A get-artist request outstanding. Carries the channel id.
    #[allow(dead_code)]
    Artist(String),
    /// A get-song-related request outstanding. Carries the browse id.
    #[allow(dead_code)]
    Related(String),
    /// A get-album request outstanding. Carries the browse id.
    #[allow(dead_code)]
    Album(String),
    Auth,
    Pong,
}

impl Pending {
    /// True if `self` and `other` are the same request kind. Resolve /
    /// ResolvePremium compare by video_id; a fast and a premium resolve for the
    /// SAME vid do NOT match each other (they are independent requests).
    fn matches(&self, other: &Pending) -> bool {
        match (self, other) {
            (Pending::Resolve(a), Pending::Resolve(b)) => a == b,
            (Pending::ResolvePremium(a), Pending::ResolvePremium(b)) => a == b,
            (Pending::Search(a), Pending::Search(b)) => a == b,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

pub struct Session {
    sidecar: Sidecar,
    cookies: Option<String>,
    /// Browser profile to read cookies from (`"chrome"` etc.), when set instead
    /// of a pasted cookies file. Mutually exclusive with `cookies`.
    pub browser: Option<String>,
    /// video_id → RemoteTrack metadata seen via search/get_playlist/watch.
    pub track_cache: HashMap<String, RemoteTrack>,
    /// Insertion-order FIFO of `track_cache` keys, used for cap eviction. A
    /// VecDeque (not the HashMap's non-deterministic iteration order) so
    /// eviction drops the oldest-seen entry. A key is pushed only on first
    /// insert (re-caching an already-seen track overwrites in place without
    /// re-pushing), keeping the deque and map in 1:1 sync.
    track_cache_order: std::collections::VecDeque<String>,
    /// Per-video_id cached fast + premium URLs (capped at current+next). A
    /// premium resolve fills the `premium` slot WITHOUT evicting `fast`, so the
    /// fast URL stays playable until the premium swap.
    url_cache: Vec<CachedResolve>,
    /// FIFO of in-flight request kinds, paired one-to-one with drained responses
    /// (the sidecar replies in send order). Lets fire-and-forget sends
    /// (refresh, pre-resolve) interleave safely with sync roundtrips.
    pending: std::collections::VecDeque<Pending>,
    /// A fast-tier resolve currently outstanding (only one at a time).
    resolve_inflight: Option<String>,
    /// A premium-tier resolve currently outstanding (only one at a time,
    /// independent of the fast one so a fast sync + a premium preload can run
    /// concurrently).
    premium_resolve_inflight: Option<String>,
    /// `(video_id, ResolvedUrl)` from a fire-and-forget premium resolve, picked
    /// up by `App::on_tick` to swap the currently-playing fast stream up to
    /// 256k. Only set when a premium URL lands; consumed once.
    pub pending_premium_url: Option<(String, ResolvedUrl)>,
    /// Lists fetched async by `send_refresh`, picked up by `App::on_tick`.
    pub pending_playlists: Option<Vec<crate::yt::proto::PlaylistSummary>>,
    pub pending_suggestions: Option<Vec<crate::yt::proto::PlaylistSummary>>,
    /// `(list_id, video_ids)` pairs from fire-and-forget `send_get_playlist`
    /// calls, picked up by `App::on_tick` to populate the focused
    /// `YtList.track_ids`. A Vec (not a single `Option`) so multiple
    /// get_playlist responses landing in the same `drain_paired` cycle (user
    /// switched A→B rapidly) don't OVERWRITE each other — the old single-slot
    /// design lost PL_A's tracks when PL_B's response landed second in the
    /// same drain, leaving PL_A with wrong/empty tracks forever.
    pub pending_tracks: Vec<(String, Vec<String>)>,
    /// The list ids whose tracks are currently being fetched (guards against
    /// re-sending every tick while the fetch is in flight). A HashSet (not a
    /// single `Option<String>`) so switching from playlist A to B while A's
    /// fetch is in flight doesn't clear A's guard — the single-slot design
    /// cleared `playlist_inflight = None` when A's response landed, making
    /// on_tick think B (still in flight) was idle → it fired a DUPLICATE
    /// `send_get_playlist(B)`. The cascading duplicates flooded the sequential
    /// sidecar, and the user's currently-focused playlist sat at the back of
    /// the queue behind redundant fetches → "Loading…" forever.
    playlist_inflight: std::collections::HashSet<String>,
    /// Playlist ids whose `get_playlist` returned an error. `on_tick` skips
    /// re-fetching these so a bad/transient id doesn't flood the single-threaded
    /// sidecar every tick (a prior bug logged 72k `get_playlist start` lines
    /// with zero completions against a poisoned cache of fake ids). Cleared on
    /// `send_refresh` so R retries failed lists.
    pub failed_playlists: std::collections::HashSet<String>,
    /// SYNC-49 fix: queue of playlist ids visited while a fetch was in flight.
    /// When the current fetch completes, on_tick checks this queue and fires
    /// the next one. This ensures rapid A→B→C→D→E switching loads ALL
    /// playlists, not just the first and last.
    pending_playlist_queue: std::collections::VecDeque<String>,
    /// `(query, video_ids)` from a fire-and-forget `send_search`, picked up by
    /// `App::on_tick` to populate the search overlay's results. Carries the
    /// query so App only applies it to the overlay that asked for it.
    pub pending_search: Option<(String, Vec<String>)>,
    /// The query currently being searched (guards against re-sending the same
    /// query while in flight).
    search_inflight: Option<String>,
    /// True while a `send_refresh` is in flight (both Playlists + Suggestions
    /// responses pending). Guards against stacking duplicate refreshes in
    /// the FIFO `pending` queue (Slice 2 — the old code had no guard, so
    /// multiple refreshes could stack and apply out of order).
    refresh_inflight: bool,
    /// Generation counter incremented on each `send_refresh`. The gen rides in
    /// `Pending::Playlists(gen)` / `Pending::Suggestions(gen)` so `apply_pair`
    /// drops a stale response (from a prior refresh) when `gen !=
    /// self.refresh_gen` (Slice 2 — stale-refresh-overwrites-fresh fix).
    refresh_gen: u64,
    /// How many of the current refresh's responses are still pending (2 at
    /// send: Playlists + Suggestions; decremented in `apply_pair` as each
    /// lands; when 0, `refresh_inflight` is cleared).
    refresh_remaining: u8,
    /// `(video_id, lines, synced)` from a fire-and-forget `send_get_lyrics`,
    /// picked up by `App::on_tick` to populate the lyrics overlay. Carries the
    /// video_id so on_tick can discard stale lyrics for a different track
    /// (generation guard lives in `App::lyrics_gen`).
    pub pending_lyrics: Option<(String, Vec<crate::yt::proto::LyricLineProto>, bool)>,
    /// The video_id whose lyrics are currently being fetched (guards against
    /// re-sending every tick while the fetch is in flight).
    lyrics_inflight: Option<String>,
    /// `(video_ids)` from a fire-and-forget `send_home_suggestions`, picked up
    /// by `App::on_tick` to populate the `S` discover overlay. Distinct from
    /// `pending_suggestions` (the refresh flow) so opening discover doesn't
    /// disturb the Y-view account+suggested lists.
    pub pending_discover: Option<Vec<crate::yt::proto::PlaylistSummary>>,
    /// True while a discover fetch is in flight (guards against re-sending
    /// every tick while the fetch is in flight).
    discover_inflight: bool,
    /// `video_ids` from a fire-and-forget `send_watch_playlist` (CONT=YouTube
    /// auto-advance), picked up by `App::on_tick` to refill the `RadioCursor`
    /// and start playback of the next track. Non-blocking so a natural
    /// end-of-track doesn't freeze the UI for the ~4s ytmusicapi roundtrip.
    pub pending_watch: Option<Vec<String>>,
    /// True while a watch_playlist fetch is in flight (guards against
    /// re-sending every tick while the fetch is in flight).
    watch_inflight: bool,
    /// The result of the last fire-and-forget publication operation
    /// (create_playlist or add_playlist_items), picked up by `App::on_tick`
    /// to surface the outcome to the user. Carries the playlist id on success,
    /// the list of failed video ids on partial success, or the error message
    /// on failure. Truthful reporting — never a blanket "done" when some
    /// tracks silently failed.
    pub pending_publication: Option<crate::yt::publication::PublicationResult>,
    /// The most recent sidecar error from an inflight-tracked request
    /// (search/get_playlist/resolve/watch), surfaced to `App::yt_error` by
    /// `on_tick`. Lets the UI exit a "searching…/loading…" state on failure
    /// instead of hanging forever (the sidecar's stderr is null'd, so without
    /// this an error response was silently dropped + the inflight guard never
    /// cleared, wedging every later request of that kind). A Vec (not a single
    /// slot) so two Search errors for different queries in one `drain_paired`
    /// cycle are BOTH staged — `on_tick` matches the overlay's `submitted`
    /// query against each, and surfaces the rest as footer messages. A single
    /// slot would drop the second (relevant) error, wedging the overlay.
    pub pending_errors: Vec<(ErrorScope, String)>,
    /// Respawn-backoff state (cap 3 attempts, ≥5s apart) so a sidecar that dies
    /// on spawn (bad cookies, missing deps) doesn't get respawned every tick.
    pub respawn_attempts: u32,
    last_respawn: Option<Instant>,
    /// Video_ids whose metadata must NOT be evicted from `track_cache` — the
    /// focused loaded playlist's `track_ids`. Without this, browsing enough
    /// playlists fills the 256-entry cache and `evict_track_cache` removes the
    /// oldest non-playing entry, which can be a track STILL REFERENCED by a
    /// loaded playlist's `track_ids`. The view then renders that row as
    /// "Loading…" forever: `track_for(id)` returns `None` (evicted), but the
    /// lazy-load guard (`!loaded`) prevents a re-fetch because the playlist is
    /// already marked loaded. Pinning the focused playlist's tracks (kept in
    /// sync by `App::on_tick` via `set_pinned_tracks`) makes `evict_track_cache`
    /// skip them, exactly like it skips `url_cache` entries (playing/next).
    pinned_tracks: std::collections::HashSet<String>,
}

/// Max auto-respawn attempts before giving up (surfacing `yt_error` instead).
const RESPAWN_MAX: u32 = 3;
/// Minimum gap between respawn attempts.
const RESPAWN_GAP: Duration = Duration::from_secs(5);

const URL_CACHE_CAP: usize = 2;

/// Max entries in [`Session::track_cache`] (video_id → RemoteTrack). The cache
/// grew without bound as the user searched/browsed YouTube (PB9); once over
/// the cap the oldest entry is evicted. Entries whose stream URL is still
/// held in the 2-entry `url_cache` (the playing / next-preload track) are
/// never evicted, so the player bar keeps its now-playing metadata mid-play.
/// `pub` so `tests/perf.rs` can assert the cap value.
pub const TRACK_CACHE_CAP: usize = 256;

impl Session {
    /// Spawn with pasted `cookies` (Netscape) OR `browser` (profile name) —
    /// pass exactly one; both `None` runs guest mode. `browser` makes the
    /// sidecar read cookies from the browser profile (no file written).
    pub fn spawn(python: &Path, script: &Path, cookies: Option<String>) -> Result<Self> {
        let sidecar = Sidecar::spawn(python, script, cookies.clone(), None, None)?;
        Ok(Session {
            sidecar,
            cookies,
            browser: None,
            track_cache: HashMap::new(),
            track_cache_order: std::collections::VecDeque::new(),
            url_cache: Vec::new(),
            pending: std::collections::VecDeque::new(),
            resolve_inflight: None,
            premium_resolve_inflight: None,
            pending_premium_url: None,
            pending_playlists: None,
            pending_suggestions: None,
            pending_tracks: Vec::new(),
            playlist_inflight: std::collections::HashSet::new(),
            failed_playlists: std::collections::HashSet::new(),
            pending_playlist_queue: std::collections::VecDeque::new(),
            pending_search: None,
            search_inflight: None,
            refresh_inflight: false,
            refresh_gen: 0,
            refresh_remaining: 0,
            pending_lyrics: None,
            lyrics_inflight: None,
            pending_discover: None,
            discover_inflight: false,
            pending_watch: None,
            watch_inflight: false,
            pending_publication: None,
            pending_errors: Vec::new(),
            respawn_attempts: 0,
            last_respawn: None,
            pinned_tracks: std::collections::HashSet::new(),
        })
    }

    /// Spawn reading cookies from a browser profile (e.g. `"chrome"`). No
    /// cookie file is written; values stay in the browser. Used by
    /// `:yt auth browser <name>`.
    pub fn spawn_browser(python: &Path, script: &Path, browser: String) -> Result<Self> {
        // Pass our persistent cookies path: the sidecar writes the decrypted
        // browser jar there (0600) so the next launch can load it WITHOUT
        // re-reading the Keychain. The single Keychain prompt happens here, on
        // the explicit `:yt auth browser` command — not at launch.
        // When no safe config dir exists (refusing /tmp/.config), pass an empty
        // string — the sidecar simply won't persist the decrypted jar.
        let cf = cookies_file_opt()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let sidecar = Sidecar::spawn(python, script, None, Some(browser.clone()), Some(cf))?;
        Ok(Session {
            sidecar,
            cookies: None,
            browser: Some(browser),
            track_cache: HashMap::new(),
            track_cache_order: std::collections::VecDeque::new(),
            url_cache: Vec::new(),
            pending: std::collections::VecDeque::new(),
            resolve_inflight: None,
            premium_resolve_inflight: None,
            pending_premium_url: None,
            pending_playlists: None,
            pending_suggestions: None,
            pending_tracks: Vec::new(),
            playlist_inflight: std::collections::HashSet::new(),
            failed_playlists: std::collections::HashSet::new(),
            pending_playlist_queue: std::collections::VecDeque::new(),
            pending_search: None,
            search_inflight: None,
            refresh_inflight: false,
            refresh_gen: 0,
            refresh_remaining: 0,
            pending_lyrics: None,
            lyrics_inflight: None,
            pending_discover: None,
            discover_inflight: false,
            pending_watch: None,
            watch_inflight: false,
            pending_publication: None,
            pending_errors: Vec::new(),
            respawn_attempts: 0,
            last_respawn: None,
            pinned_tracks: std::collections::HashSet::new(),
        })
    }

    pub fn is_alive(&mut self) -> bool {
        self.sidecar.is_alive()
    }

    /// True if a crashed sidecar should be respawned now (under the backoff
    /// cap: ≤3 attempts, ≥5s apart). Call `note_respawn` immediately after a
    /// respawn attempt; `mark_alive` resets the counter once it's healthy.
    pub fn should_respawn(&self) -> bool {
        if self.respawn_attempts >= RESPAWN_MAX {
            return false;
        }
        match self.last_respawn {
            Some(t) => t.elapsed() >= RESPAWN_GAP,
            None => true,
        }
    }
    pub fn note_respawn(&mut self) {
        self.respawn_attempts += 1;
        self.last_respawn = Some(Instant::now());
    }
    pub fn mark_alive(&mut self) {
        self.respawn_attempts = 0;
        self.last_respawn = None;
    }

    /// Look up a cached stream URL for `video_id`, PREFERRING the premium (256k)
    /// entry when present, else the fast (129k) one. This is the "prefer premium
    /// at play time" contract: a pre-resolved premium URL plays instantly at
    /// Premium quality; only when premium isn't ready do we fall back to fast.
    pub fn url_for(&self, video_id: &str) -> Option<String> {
        let e = self.url_cache.iter().find(|c| c.video_id == video_id)?;
        if let Some((url, _)) = &e.premium {
            return Some(url.clone());
        }
        e.fast.as_ref().map(|(url, _)| url.clone())
    }

    /// The cached premium URL for `video_id`, if one has landed.
    pub fn url_for_premium(&self, video_id: &str) -> Option<String> {
        self.url_cache
            .iter()
            .find(|c| c.video_id == video_id)
            .and_then(|c| c.premium.as_ref())
            .map(|(url, _)| url.clone())
    }

    /// The StreamFormat of the cached entry for `video_id`, if any. The
    /// `CachedResolve.fmt` is the tier source of truth (premium wins when
    /// present), so callers should prefer this over `track_cache`'s `fmt`
    /// (which can lag — a track cached by search BEFORE its premium resolve
    /// lands has `fmt=None` in track_cache even though url_for returns the
    /// premium URL). Returns `Some` only when a resolve has filled the entry.
    pub fn cache_fmt_for(&self, video_id: &str) -> Option<StreamFormat> {
        self.url_cache
            .iter()
            .find(|c| c.video_id == video_id)
            .and_then(|c| c.fmt.clone())
    }

    /// Find-or-create the cache entry for `video_id`, evicting the oldest when
    /// over the cap (current+next). Both tiers of an existing entry survive.
    fn cache_entry(&mut self, video_id: &str) -> &mut CachedResolve {
        if !self.url_cache.iter().any(|c| c.video_id == video_id) {
            self.url_cache.push(CachedResolve {
                video_id: video_id.to_string(),
                ..Default::default()
            });
            if self.url_cache.len() > URL_CACHE_CAP {
                self.url_cache.remove(0);
            }
        }
        self.url_cache
            .iter_mut()
            .find(|c| c.video_id == video_id)
            .expect("just inserted")
    }

    /// Stage a sidecar error for `App::on_tick`. `pending_errors` is a Vec so
    /// NO error is dropped: two Search errors for different queries in one
    /// `drain_paired` cycle are both staged — on_tick matches the overlay's
    /// `submitted` query against each Search error (clearing `searching` for
    /// the matching one) and surfaces the rest as footer messages. A single
    /// slot would drop the second (relevant) error, wedging the overlay on
    /// "searching…" forever. Cap at a small bound to avoid unbounded growth.
    fn set_error(&mut self, scope: ErrorScope, e: String) {
        if self.pending_errors.len() >= 8 {
            self.pending_errors.remove(0);
        }
        self.pending_errors.push((scope, e));
    }

    /// Is a fast resolve in flight?
    pub fn resolve_busy(&self) -> bool {
        self.resolve_inflight.is_some()
    }

    /// Is a premium resolve in flight?
    pub fn premium_resolve_busy(&self) -> bool {
        self.premium_resolve_inflight.is_some()
    }

    /// The video_id a fast resolve is in flight for, if any (for the spinner +
    /// swap guards to correlate against the currently-playing track).
    pub fn resolve_inflight_id(&self) -> Option<&str> {
        self.resolve_inflight.as_deref()
    }

    /// The video_id a premium resolve is in flight for, if any.
    pub fn premium_resolve_inflight_id(&self) -> Option<&str> {
        self.premium_resolve_inflight.as_deref()
    }

    pub fn has_cookies(&self) -> bool {
        self.cookies.is_some() || self.browser.is_some()
    }

    /// Clear cookies and respawn guest.
    pub fn clear_cookies(&mut self, python: &Path, script: &Path) -> Result<()> {
        self.cookies = None;
        self.browser = None;
        // SYNC-48 fix: clear inflight state before respawn so stale requests
        // from the killed sidecar don't block future fetches.
        self.clear_all_caches();
        self.sidecar = Sidecar::spawn(python, script, None, None, None)?;
        Ok(())
    }

    /// Clear all in-memory caches + cancel in-flight requests (S2.6.2).
    /// Called by `App::yt_logout` so stale data from the logged-out account
    /// doesn't survive logout. Also bumps `refresh_gen` so any in-flight refresh
    /// response that lands AFTER logout is discarded by the generation guard
    /// (it carries the old gen, which is now != the new `refresh_gen`).
    pub fn clear_all_caches(&mut self) {
        self.track_cache.clear();
        self.track_cache_order.clear();
        self.url_cache.clear();
        self.pinned_tracks.clear();
        self.pending_playlists = None;
        self.pending_suggestions = None;
        self.pending_tracks.clear();
        self.pending_search = None;
        self.pending_premium_url = None;
        self.pending_lyrics = None;
        self.pending_discover = None;
        self.pending_watch = None;
        self.pending_publication = None;
        self.pending_errors.clear();
        self.playlist_inflight.clear();
        self.failed_playlists.clear();
        self.pending_playlist_queue.clear();
        self.search_inflight = None;
        self.resolve_inflight = None;
        self.premium_resolve_inflight = None;
        self.lyrics_inflight = None;
        self.discover_inflight = false;
        self.watch_inflight = false;
        self.refresh_inflight = false;
        self.refresh_remaining = 0;
        // Bump the gen so any in-flight response (carrying the old gen) is
        // discarded by apply_pair's generation guard.
        self.refresh_gen = self.refresh_gen.wrapping_add(1);
    }

    /// Persist `cookies` (Netscape cookies.txt) to the cookies file (perms
    /// 0600) and respawn the sidecar with them. One paste feeds both
    /// `ytmusicapi` (via the cookie header) and `yt-dlp` (via the file).
    /// Clears any browser profile (mutually exclusive).
    pub fn set_cookies(&mut self, cookies: String, python: &Path, script: &Path) -> Result<()> {
        let p = cookies_file_opt()
            .ok_or_else(|| anyhow!("no safe config dir for cookies (refusing /tmp/.config)"))?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, &cookies)?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        self.cookies = Some(cookies);
        self.browser = None;
        // SYNC-48 fix: clear inflight state before respawn.
        self.clear_all_caches();
        self.sidecar = Sidecar::spawn(python, script, self.cookies.clone(), None, None)?;
        Ok(())
    }

    /// Respawn the sidecar reading cookies from a browser profile (e.g.
    /// `"chrome"`, `"firefox"`, `"safari"`, `"edge"`). No cookie file is
    /// written; the cookie values stay in the browser. Clears any pasted
    /// cookies (mutually exclusive). Used by `:yt auth browser <name>`.
    pub fn set_browser(&mut self, browser: String, python: &Path, script: &Path) -> Result<()> {
        self.browser = Some(browser.clone());
        self.cookies = None;
        // SYNC-48 fix: clear inflight state before respawn.
        self.clear_all_caches();
        // Same pattern as spawn_browser: pass empty string when no safe config
        // dir (refusing /tmp/.config for cookie secrets).
        let cf = cookies_file_opt()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        self.sidecar = Sidecar::spawn(python, script, None, Some(browser), Some(cf))?;
        Ok(())
    }

    /// Send a request and poll for the matching response until `deadline`.
    /// Apply one (response, its pending kind) pair to the caches Session owns
    /// (`track_cache`, `url_cache`, `pending_playlists/suggestions`). Returns
    /// `Some(response)` if it's the caller's target kind (so a sync roundtrip
    /// can return it), else `None` (applied as a stray).
    fn apply_pair(&mut self, kind: Pending, resp: Response, target: &Pending) -> Option<Response> {
        match (&resp, &kind) {
            (Response::Search(v), Pending::Search(q)) => {
                let vids: Vec<String> = v.iter().map(|t| t.video_id.clone()).collect();
                for t in v {
                    self.cache_track(t);
                }
                // Surface the ordered video_ids tagged with the QUERY that
                // asked for them (carried in the Pending variant, so a second
                // search's query can't overwrite the first's tag).
                self.pending_search = Some((q.clone(), vids));
                // Clear the search-inflight guard ONLY for this exact query,
                // so a concurrent second search stays in flight.
                if self.search_inflight.as_deref() == Some(q.as_str()) {
                    self.search_inflight = None;
                }
            }
            (Response::Tracks(v), Pending::Tracks(id)) => {
                for t in v {
                    self.cache_track(t);
                }
                // Surface the resolved video_ids back to App so it can populate
                // the matching `YtList.track_ids` (the Y view's col2 + Enter/s).
                let vids: Vec<String> = v.iter().map(|t| t.video_id.clone()).collect();
                self.pending_tracks.push((id.clone(), vids));
                // Free the inflight guard for THIS list only (not all lists).
                // The old code set `playlist_inflight = None` (clearing the
                // single slot), which also cleared the guard for any OTHER
                // list whose fetch was still in flight — causing on_tick to
                // fire a duplicate request for that other list. The HashSet
                // design removes only this list's id, preserving other lists'
                // guards.
                self.playlist_inflight.remove(id.as_str());
                self.failed_playlists.remove(id.as_str());
            }
            (Response::WatchPlaylist(v), Pending::Watch) => {
                for t in v {
                    self.cache_track(t);
                }
                // Surface the radio queue's video_ids so `App::on_tick` can
                // refill the `RadioCursor` + start playback (non-blocking
                // auto-advance). Clear the inflight guard on success so a
                // later auto-advance can fire a fresh fetch.
                let vids: Vec<String> = v.iter().map(|t| t.video_id.clone()).collect();
                self.pending_watch = Some(vids);
                self.watch_inflight = false;
            }
            // Discover (`S` overlay) home-suggestions: route to
            // `pending_discover` (NOT `pending_suggestions`, which is the
            // gen-guarded refresh flow). Distinct Pending variant → FIFO
            // pairing keeps a discover response away from a refresh's
            // Suggestions(gen) slot.
            (Response::Suggestions(v), Pending::Discover) => {
                self.pending_discover = Some(v.clone());
                self.discover_inflight = false;
            }
            (Response::Resolve(u), Pending::Resolve(vid)) => {
                // FAST tier: fill the fast slot WITHOUT evicting a premium slot
                // (the premium preload may already have landed, or will land
                // later and upgrade via on_tick). Only set fmt from the fast
                // resolve if no premium fmt is known yet (premium wins).
                let entry = self.cache_entry(vid);
                entry.fast = Some((u.url.clone(), u.expires_at));
                if entry.fmt.is_none() || entry.premium.is_none() {
                    entry.fmt = Some(StreamFormat {
                        codec: u.codec.clone(),
                        abr: u.abr,
                        sample_rate: u.sample_rate,
                        container: u.container.clone(),
                        premium: u.premium,
                    });
                }
                if let Some(t) = self.track_cache.get_mut(vid) {
                    if t.fmt.is_none() {
                        t.fmt = Some(StreamFormat {
                            codec: u.codec.clone(),
                            abr: u.abr,
                            sample_rate: u.sample_rate,
                            container: u.container.clone(),
                            premium: u.premium,
                        });
                    }
                }
                self.resolve_inflight = None;
            }
            (Response::Resolve(u), Pending::ResolvePremium(vid)) => {
                // PREMIUM tier: fill the premium slot (fast slot survives), set
                // fmt to the premium StreamFormat (premium wins for the UI), and
                // signal App to swap the currently-playing stream up to 256k.
                let entry = self.cache_entry(vid);
                let u_clone = u.clone();
                entry.premium = Some((u.url.clone(), u.expires_at));
                entry.fmt = Some(StreamFormat {
                    codec: u.codec.clone(),
                    abr: u.abr,
                    sample_rate: u.sample_rate,
                    container: u.container.clone(),
                    premium: u.premium,
                });
                if let Some(t) = self.track_cache.get_mut(vid) {
                    t.fmt = Some(StreamFormat {
                        codec: u.codec.clone(),
                        abr: u.abr,
                        sample_rate: u.sample_rate,
                        container: u.container.clone(),
                        premium: u.premium,
                    });
                }
                self.premium_resolve_inflight = None;
                // Hand the premium URL to App so on_tick can swap the live stream
                // up to 256k (guarded: same track, not near end, not already
                // premium). App takes ownership; a stale signal (user moved on)
                // is just dropped.
                self.pending_premium_url = Some((vid.clone(), u_clone));
            }
            (Response::Playlists(v), Pending::Playlists(gen)) => {
                // Generation guard (Slice 2): a stale refresh response (from a
                // prior send_refresh) is dropped — its gen != current refresh_gen.
                // This prevents a slow refresh overwriting a newer one.
                if *gen == self.refresh_gen {
                    self.pending_playlists = Some(v.clone());
                }
                // Count down the refresh-inflight tracker either way.
                if self.refresh_remaining > 0 {
                    self.refresh_remaining -= 1;
                    if self.refresh_remaining == 0 {
                        self.refresh_inflight = false;
                    }
                }
            }
            (Response::Suggestions(v), Pending::Suggestions(gen)) => {
                if *gen == self.refresh_gen {
                    self.pending_suggestions = Some(v.clone());
                }
                if self.refresh_remaining > 0 {
                    self.refresh_remaining -= 1;
                    if self.refresh_remaining == 0 {
                        self.refresh_inflight = false;
                    }
                }
            }
            (Response::Auth(_), Pending::Auth) | (Response::Pong, Pending::Pong) => {}
            (Response::Lyrics(lines, synced), Pending::Lyrics(vid)) => {
                // Stage the lyrics for App::on_tick. The video_id rides in the
                // Pending variant so on_tick can discard stale lyrics for a
                // different track (the App-side generation guard in
                // `lyrics_gen` is the second layer of staleness protection).
                self.pending_lyrics = Some((vid.clone(), lines.clone(), *synced));
                // Free the inflight guard so a later re-request (track change
                // back to the same id) isn't wedged.
                if self.lyrics_inflight.as_deref() == Some(vid.as_str()) {
                    self.lyrics_inflight = None;
                }
            }
            // Publication responses: stage the result for App::on_tick to
            // surface to the user. CreatedPlaylist carries the new playlist
            // id (Success). AddedItems carries a ytmusicapi status + count;
            // STATUS_SUCCEEDED = Success, anything else = Failed (truthful
            // reporting — never a blanket "done" when some tracks failed).
            (Response::CreatedPlaylist { id, .. }, Pending::CreatePlaylist(_)) => {
                self.pending_publication = Some(
                    crate::yt::publication::PublicationResult::Success(id.clone()),
                );
            }
            (Response::AddedItems { status, count: _ }, Pending::AddPlaylistItems(_)) => {
                if status == "STATUS_SUCCEEDED" {
                    self.pending_publication = Some(
                        crate::yt::publication::PublicationResult::Success(String::new()),
                    );
                } else {
                    self.pending_publication =
                        Some(crate::yt::publication::PublicationResult::Failed(format!(
                            "add_playlist_items returned status: {status}"
                        )));
                }
            }
            // An error response frees the inflight guard for its request kind so
            // a later retry isn't wedged, and surfaces the message so the UI
            // can exit its "searching…/loading…" state. The sidecar's stderr is
            // null'd, so this is the only path an error reaches the user.
            (Response::Error(e), Pending::Search(q)) => {
                // Clear the inflight guard ONLY for this exact query (a second
                // search for a different query may still be in flight).
                if self.search_inflight.as_deref() == Some(q.as_str()) {
                    self.search_inflight = None;
                }
                self.set_error(ErrorScope::Search(q.clone()), e.clone());
            }
            (Response::Error(e), Pending::Playlists(gen)) => {
                if *gen == self.refresh_gen && self.refresh_remaining > 0 {
                    self.refresh_remaining -= 1;
                    if self.refresh_remaining == 0 {
                        self.refresh_inflight = false;
                    }
                }
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Suggestions(gen)) => {
                if *gen == self.refresh_gen && self.refresh_remaining > 0 {
                    self.refresh_remaining -= 1;
                    if self.refresh_remaining == 0 {
                        self.refresh_inflight = false;
                    }
                }
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Tracks(id)) => {
                self.playlist_inflight.remove(id.as_str());
                self.failed_playlists.insert(id.clone());
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Resolve(_)) => {
                self.resolve_inflight = None;
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::ResolvePremium(_)) => {
                self.premium_resolve_inflight = None;
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Lyrics(vid)) => {
                // Free the inflight guard so a re-request after an error isn't
                // wedge. Surface the error so the lyrics overlay can exit its
                // "loading…" state and show "lyrics error" (the sidecar's
                // stderr is null'd, so this is the only error path).
                if self.lyrics_inflight.as_deref() == Some(vid.as_str()) {
                    self.lyrics_inflight = None;
                }
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Discover) => {
                // Free the discover inflight guard so a later `S` press can
                // re-fetch, and surface the error so `App::on_tick` can clear
                // the discover overlay's "loading…" state.
                self.discover_inflight = false;
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), Pending::Watch) => {
                // Free the watch inflight guard so a later auto-advance can
                // fire a fresh radio refill. Surface the error so `App::on_tick`
                // can stop cleanly instead of hanging on a wedged inflight.
                self.watch_inflight = false;
                self.set_error(ErrorScope::Other, e.clone());
            }
            (Response::Error(e), _) => {
                self.set_error(ErrorScope::Other, e.clone());
            }
            _ => {}
        }
        if kind.matches(target) {
            Some(resp)
        } else {
            None
        }
    }

    fn cache_track(&mut self, t: &RemoteTrackSummary) {
        // Only track insertion order for NEW keys; a re-cache (same video_id
        // seen again via a later search/get_playlist) overwrites in place and
        // must NOT get a second deque entry, or the deque/map would desync.
        let is_new = !self.track_cache.contains_key(&t.video_id);
        self.track_cache.insert(
            t.video_id.clone(),
            RemoteTrack {
                video_id: t.video_id.clone(),
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                dur: t.dur,
                fmt: None,
                isrc: t.isrc.clone(),
            },
        );
        if is_new {
            self.track_cache_order.push_back(t.video_id.clone());
            self.evict_track_cache();
        }
    }

    /// Public test helper: cache a track summary (wraps the private
    /// `cache_track`). Used by `tests/perf.rs` to verify the LRU cap + dedup.
    pub fn cache_track_pub(&mut self, t: &RemoteTrackSummary) {
        self.cache_track(t);
    }

    /// Public test helper: the number of entries in the FIFO eviction order
    /// deque. Used by `tests/perf.rs` to verify dedup (should equal
    /// `track_cache.len()`).
    pub fn track_cache_order_len(&self) -> usize {
        self.track_cache_order.len()
    }

    /// Evict the oldest `track_cache` entries while over [`TRACK_CACHE_CAP`].
    /// An entry whose stream URL is still in `url_cache` (the currently-playing
    /// or next-preload track) is re-queued to the back instead of dropped, so
    /// `now_playing_view` never loses the playing track's metadata mid-play.
    /// An entry in `pinned_tracks` (the focused playlist's `track_ids`, kept in
    /// sync by `App::on_tick`) is likewise re-queued — without this, browsing
    /// enough playlists evicts a track STILL REFERENCED by a loaded playlist's
    /// `track_ids`, and the view renders that row as "Loading…" forever (the
    /// lazy-load guard `!loaded` blocks a re-fetch). The loop terminates:
    /// protected entries (≤ 2 url_cache + the focused playlist's tracks) are a
    /// strict subset, so once `len > cap` there are `len - cap` non-protected
    /// entries to remove and each non-protected iteration shrinks `len` by 1.
    fn evict_track_cache(&mut self) {
        let mut protected_in_a_row = 0;
        while self.track_cache.len() > TRACK_CACHE_CAP {
            let Some(victim) = self.track_cache_order.pop_front() else {
                break;
            };
            if self.url_cache.iter().any(|c| c.video_id == victim)
                || self.pinned_tracks.contains(&victim)
            {
                // Playing / next-preload / focused-playlist track: keep it,
                // reconsider later.
                self.track_cache_order.push_back(victim);
                protected_in_a_row += 1;
                // Full pass without an eviction → every entry in the deque is
                // protected (e.g. the focused playlist has > cap tracks, all
                // pinned). Break to avoid an infinite loop. The cache stays
                // over the cap in this rare case, bounded by the focused
                // playlist's (finite) track count — better than hanging the
                // TUI on a 300-track list.
                if protected_in_a_row >= self.track_cache_order.len() {
                    break;
                }
                continue;
            }
            protected_in_a_row = 0;
            self.track_cache.remove(&victim);
        }
    }

    /// Replace the pinned-track set with `ids` (the focused loaded playlist's
    /// `track_ids`). Called by `App::on_tick` each tick before draining so the
    /// eviction never drops a track the user is currently looking at. Passing
    /// an empty set (no focused list / empty tracks) clears the pin so the cap
    /// stays effective when the user isn't viewing a playlist.
    pub fn set_pinned_tracks(&mut self, ids: std::collections::HashSet<String>) {
        self.pinned_tracks = ids;
    }

    fn kind_for(req: &Request) -> Pending {
        match req {
            Request::Search { q, .. } => Pending::Search(q.clone()),
            Request::LibraryPlaylists => Pending::Playlists(0),
            Request::GetPlaylist { id } => Pending::Tracks(id.clone()),
            Request::HomeSuggestions => Pending::Suggestions(0),
            Request::GetWatchPlaylist { .. } => Pending::Watch,
            Request::ResolveUrl { video_id, quality } => {
                if quality == "premium" {
                    Pending::ResolvePremium(video_id.clone())
                } else {
                    Pending::Resolve(video_id.clone())
                }
            }
            Request::AuthStatus => Pending::Auth,
            Request::GetLyrics { video_id } => Pending::Lyrics(video_id.clone()),
            Request::CreatePlaylist { title, .. } => Pending::CreatePlaylist(title.clone()),
            Request::AddPlaylistItems { playlist_id, .. } => {
                Pending::AddPlaylistItems(playlist_id.clone())
            }
            Request::GetLikedSongs { .. } => Pending::LikedSongs,
            Request::GetArtist { channel_id } => Pending::Artist(channel_id.clone()),
            Request::GetSongRelated { browse_id } => Pending::Related(browse_id.clone()),
            Request::GetAlbum { browse_id } => Pending::Album(browse_id.clone()),
            // Home/Explore/Charts wire-protocol variants are added in
            // `src/yt/proto.rs` but the session-side wiring (Pending kinds +
            // apply_pair arms) is a later task — these arms only exist to keep
            // the match exhaustive. No code path currently sends these
            // requests, so the `todo!` is never hit in practice.
            Request::Home | Request::Explore | Request::Charts => {
                todo!("session wiring for home/explore/charts is a later task")
            }
            Request::Ping => Pending::Pong,
        }
    }

    fn roundtrip(&mut self, req: Request, deadline: Duration) -> Result<Response> {
        let kind = Self::kind_for(&req);
        self.pending.push_back(kind.clone());
        self.sidecar.send(&req)?;
        let target = kind.clone();
        let start = Instant::now();
        loop {
            match self.sidecar.try_recv()? {
                Some(resp) => {
                    // Pair with the oldest in-flight kind (FIFO).
                    let pk = self.pending.pop_front();
                    let Some(pk) = pk else {
                        return Ok(resp);
                    };
                    if let Some(r) = self.apply_pair(pk, resp, &target) {
                        return Ok(r);
                    }
                    continue; // applied as a stray; keep waiting for ours
                }
                None => {
                    if start.elapsed() >= deadline {
                        // Free the inflight guard for this kind so a later retry
                        // isn't wedged (the pending entry stays; it'll be
                        // paired with whatever response lands, or dropped). For
                        // Resolve, the cold-start EJS-solver download can take
                        // ~10s on first use, exceeding an 8s deadline — without
                        // this the resolve_inflight guard would never clear.
                        match &kind {
                            Pending::Resolve(_) => self.resolve_inflight = None,
                            Pending::ResolvePremium(_) => self.premium_resolve_inflight = None,
                            Pending::Search(q)
                                if self.search_inflight.as_deref() == Some(q.as_str()) =>
                            {
                                self.search_inflight = None;
                            }
                            Pending::Lyrics(v)
                                if self.lyrics_inflight.as_deref() == Some(v.as_str()) =>
                            {
                                self.lyrics_inflight = None;
                            }
                            _ => {}
                        }
                        return Err(anyhow!("sidecar roundtrip timeout"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// Drain + apply all ready responses (used by App::on_tick). Side-effects
    /// (track_cache, url_cache, `pending_playlists`/`pending_suggestions`) are
    /// applied via `apply_pair`; the raw responses are returned so App can do
    /// any extra mapping. Non-blocking.
    pub fn drain_paired(&mut self) -> Vec<Response> {
        let mut out = Vec::new();
        loop {
            match self.sidecar.try_recv() {
                Ok(Some(resp)) => {
                    let kind = self.pending.pop_front();
                    if let Some(k) = kind {
                        let target = k.clone();
                        self.apply_pair(k, resp.clone(), &target);
                    }
                    out.push(resp);
                }
                Ok(None) => break,
                Err(_) => {
                    break;
                }
            }
        }
        out
    }

    /// Fire-and-forget: ask for the account playlists + suggested/mood lists.
    /// Results land in `pending_playlists`/`pending_suggestions` and are
    /// picked up by `App::on_tick`. Non-blocking — doesn't wait for a reply.
    ///
    /// Increments `sync_epoch` before sending and tags both fetches with it
    /// (stored in `pending_playlists_epoch`/`pending_suggestions_epoch`). When
    /// `on_tick` drains the response, it checks the epoch — if it's < the
    /// current `sync_epoch`, the response is STALE (a newer refresh was sent
    /// after this one) and is discarded. This is the D5 generation-id guard:
    /// a slow refresh that lands after the user pressed R again (or logged out
    /// + re-authed) can't overwrite the newer state.
    ///
    /// Inflight guard: if a refresh is already in flight, this is a no-op
    /// (prevents a burst of `R` presses from flooding the sidecar).
    ///
    /// The stale-pending clearing (`pending_playlists`/`pending_suggestions`)
    /// lives HERE, AFTER the inflight guard — not in `App::refresh_yt_lists`.
    /// If the guard blocks (refresh already in flight), the pending data from
    /// that in-flight fetch must be preserved so `on_tick` can still merge it
    /// when it lands. Clearing before the guard (the old code path) lost the
    /// data permanently: the second `refresh_yt_lists` cleared the pending
    /// slots, `send_refresh` was a no-op, and `yt_lists` stayed empty.
    pub fn send_refresh(&mut self) -> Result<()> {
        if self.refresh_inflight {
            return Ok(());
        }
        // Drop a stale partial fetch so on_tick merges the fresh pair together.
        // This MUST be after the inflight guard — if a refresh is already in
        // flight, the pending data from that fetch must be preserved.
        self.pending_playlists = None;
        self.pending_suggestions = None;
        self.refresh_inflight = true;
        self.refresh_gen = self.refresh_gen.wrapping_add(1);
        // Only fetch library_playlists (1 request, not 2). The previous design
        // also sent home_suggestions, but ytmusicapi's get_home() can HANG in
        // guest mode (infinite block, no timeout) — the single-threaded sidecar
        // freezes, and every subsequent get_playlist request (the lazy-load)
        // queues behind it forever. This was the "stuck on syncing" + "long
        // loading time" root cause. Suggestions are a nice-to-have (the
        // "Suggested / Up Next" pane); they're not worth blocking the sidecar.
        self.refresh_remaining = 1;
        let gen = self.refresh_gen;
        self.pending.push_back(Pending::Playlists(gen));
        self.sidecar.send(&Request::LibraryPlaylists)?;
        Ok(())
    }

    /// Fire-and-forget: pre-resolve a FAST (tv_embedded, ~1.3s, AAC 129k) stream
    /// URL for `video_id` so a cache miss at play time is instant. Only one fast
    /// resolve at a time (`resolve_inflight`); no-op if a fast URL is already
    /// cached or one is in flight. Non-blocking.
    pub fn send_resolve(&mut self, video_id: String) -> Result<()> {
        if self.resolve_inflight.is_some() {
            return Ok(());
        }
        if self.cache_entry_has_fast(&video_id) {
            return Ok(()); // fast already resolved
        }
        self.resolve_inflight = Some(video_id.clone());
        self.pending.push_back(Pending::Resolve(video_id.clone()));
        self.sidecar.send(&Request::ResolveUrl {
            video_id,
            quality: String::new(),
        })?;
        Ok(())
    }

    /// Fire-and-forget: pre-resolve a PREMIUM (tv/web + EJS solver, ~10-15s,
    /// AAC 256k) stream URL for `video_id`. Used by `preload_next_url` so the
    /// next track's 256k URL is ready before it starts (gapless Premium), and
    /// by the progressive-upgrade path to upgrade a playing fast stream to 256k.
    /// Independent inflight guard from the fast resolve, so a fast sync + a
    /// premium preload can run concurrently. No-op if premium is already cached
    /// or in flight.
    pub fn send_resolve_premium(&mut self, video_id: String) -> Result<()> {
        if self.premium_resolve_inflight.is_some() {
            return Ok(());
        }
        if self.url_for_premium(&video_id).is_some() {
            return Ok(()); // premium already resolved
        }
        self.premium_resolve_inflight = Some(video_id.clone());
        self.pending
            .push_back(Pending::ResolvePremium(video_id.clone()));
        self.sidecar.send(&Request::ResolveUrl {
            video_id,
            quality: "premium".to_string(),
        })?;
        Ok(())
    }

    /// True if a fast URL is cached for `video_id` (helper for `send_resolve`'s
    /// skip-if-cached guard, which must check the fast slot specifically —
    /// `url_for` would also return premium, masking a missing fast URL).
    fn cache_entry_has_fast(&self, video_id: &str) -> bool {
        self.url_cache
            .iter()
            .find(|c| c.video_id == video_id)
            .map(|c| c.fast.is_some())
            .unwrap_or(false)
    }

    /// Fire-and-forget: fetch the tracks of one playlist so the Y view can
    /// populate the focused list's col2. Results land in `pending_tracks`
    /// (picked up by `App::on_tick`). **Only ONE playlist fetch is allowed
    /// in flight at a time** — if a fetch is in progress, the id is QUEUED
    /// (SYNC-49 fix) rather than dropped, so rapid A→B→C→D→E switching
    /// eventually loads ALL playlists. When the current fetch completes,
    /// `drain_playlist_queue()` fires the next one.
    pub fn send_get_playlist(&mut self, id: String) -> Result<()> {
        // Don't re-fetch if already loaded, in flight, or previously failed
        // (a failed id would flood the sidecar every tick otherwise).
        if self.playlist_inflight.contains(&id) || self.failed_playlists.contains(&id) {
            return Ok(());
        }
        if !self.playlist_inflight.is_empty() {
            // A fetch is in flight → queue this id (don't drop it).
            // Dedup: don't queue the same id twice.
            if !self.pending_playlist_queue.contains(&id) {
                self.pending_playlist_queue.push_back(id);
            }
            return Ok(());
        }
        self.playlist_inflight.insert(id.clone());
        self.pending.push_back(Pending::Tracks(id.clone()));
        self.sidecar.send(&Request::GetPlaylist { id })?;
        Ok(())
    }

    /// SYNC-49 fix: fire the next queued playlist fetch (if any). Called by
    /// `App::on_tick` after a Tracks response lands and clears the inflight
    /// set. Returns the id that was fired, if any.
    pub fn drain_playlist_queue(&mut self) -> Option<String> {
        if self.playlist_inflight.is_empty() {
            if let Some(id) = self.pending_playlist_queue.pop_front() {
                // Skip if already loaded while waiting in the queue.
                if !self.playlist_inflight.contains(&id) {
                    self.playlist_inflight.insert(id.clone());
                    self.pending.push_back(Pending::Tracks(id.clone()));
                    let _ = self.sidecar.send(&Request::GetPlaylist { id: id.clone() });
                    return Some(id);
                }
            }
        }
        None
    }

    /// True if a get_playlist for `id` is currently in flight.
    pub fn playlist_loading(&self, id: &str) -> bool {
        self.playlist_inflight.contains(id)
    }

    /// The query currently being searched, if a search is in flight.
    pub fn search_inflight(&self) -> Option<&str> {
        self.search_inflight.as_deref()
    }

    /// Fire-and-forget: search YouTube for `q` and surface the ordered video_ids
    /// to `pending_search` (picked up by `App::on_tick` to fill the search
    /// overlay). Non-blocking — the search overlay's explicit-submit path uses
    /// this so typing never blocks on a ~3s ytmusicapi roundtrip. Only one
    /// search at a time (`search_inflight`); re-sending the same query while in
    /// flight is a no-op.
    pub fn send_search(&mut self, q: String) -> Result<()> {
        // Skip if this exact query is already in flight (dedup the common
        // double-submit). A DIFFERENT query is allowed through — the query
        // rides in Pending::Search(q), so each response is tagged with its own
        // query and a second search can't overwrite the first's tag.
        if self.search_inflight.as_deref() == Some(q.as_str()) {
            return Ok(());
        }
        self.search_inflight = Some(q.clone());
        self.pending.push_back(Pending::Search(q.clone()));
        self.sidecar.send(&Request::Search { q, limit: 25 })?;
        Ok(())
    }

    /// Fire-and-forget: fetch lyrics for `video_id` and surface them to
    /// `pending_lyrics` (picked up by `App::on_tick` to fill the lyrics
    /// overlay). Non-blocking — the lyrics overlay's Loading state uses this
    /// so opening it never blocks on a ~3s ytmusicapi roundtrip. Only one
    /// lyrics fetch at a time (`lyrics_inflight`); re-sending the same id while
    /// in flight is a no-op. A DIFFERENT id is allowed through — the video_id
    /// rides in `Pending::Lyrics(vid)`, so each response is tagged with its own
    /// track and on_tick discards stale lyrics via the generation guard.
    pub fn send_get_lyrics(&mut self, video_id: String) -> Result<()> {
        if self.lyrics_inflight.as_deref() == Some(video_id.as_str()) {
            return Ok(());
        }
        self.lyrics_inflight = Some(video_id.clone());
        self.pending.push_back(Pending::Lyrics(video_id.clone()));
        self.sidecar.send(&Request::GetLyrics { video_id })?;
        Ok(())
    }

    /// True if a lyrics fetch for `video_id` is currently in flight.
    pub fn lyrics_loading(&self, video_id: &str) -> bool {
        self.lyrics_inflight.as_deref() == Some(video_id)
    }

    /// Fire-and-forget: fetch home suggestions for the `S` discover overlay.
    /// Results land in `pending_discover` (picked up by `App::on_tick` to
    /// populate the overlay). Non-blocking — opening discover never blocks on
    /// the ~3s ytmusicapi roundtrip (the overlay opens instantly with a
    /// "loading…" state, items appear when the response lands). Only one
    /// discover fetch at a time (`discover_inflight`); re-sending while in
    /// flight is a no-op so repeated `S` presses don't flood the sidecar.
    /// Distinct from `send_refresh`'s `Pending::Suggestions(gen)` so a discover
    /// response routes to `pending_discover` without disturbing the
    /// generation-guarded refresh tracking.
    pub fn send_home_suggestions(&mut self) -> Result<()> {
        if self.discover_inflight {
            return Ok(());
        }
        self.discover_inflight = true;
        self.pending.push_back(Pending::Discover);
        self.sidecar.send(&Request::HomeSuggestions)?;
        Ok(())
    }

    /// True if a discover (home-suggestions) fetch is currently in flight.
    pub fn discover_loading(&self) -> bool {
        self.discover_inflight
    }

    /// Reset the discover inflight flag so a new `send_home_suggestions` can
    /// be issued. Called when the user reopens the discover overlay (`S`) —
    /// if the previous response never arrived (sidecar timeout, network
    /// drop), the flag would stay `true` forever and block all future
    /// discover fetches.
    pub fn reset_discover_inflight(&mut self) {
        self.discover_inflight = false;
    }

    /// Fire-and-forget: fetch a watch_playlist (radio queue) seeded by
    /// `video_id` for CONT=YouTube auto-advance. Results land in
    /// `pending_watch` (picked up by `App::on_tick` to refill the
    /// `RadioCursor` + start playback). Non-blocking — a natural end-of-track
    /// no longer freezes the UI for the ~4s ytmusicapi roundtrip; the old
    /// track simply stays current until the next track's id lands, then
    /// `on_tick` switches. Only one watch fetch at a time (`watch_inflight`);
    /// re-sending while in flight is a no-op.
    pub fn send_watch_playlist(&mut self, video_id: String) -> Result<()> {
        if self.watch_inflight {
            return Ok(());
        }
        self.watch_inflight = true;
        self.pending.push_back(Pending::Watch);
        self.sidecar.send(&Request::GetWatchPlaylist { video_id })?;
        Ok(())
    }

    /// True if a watch_playlist (radio refill) fetch is currently in flight.
    pub fn watch_loading(&self) -> bool {
        self.watch_inflight
    }

    /// Fire-and-forget: create a new YouTube playlist with the given title,
    /// description, privacy, and optional seed video_ids. The response
    /// (CreatedPlaylist with the new playlist id) is drained by `on_tick`.
    /// Non-blocking — publication never freezes the UI. The caller MUST show
    /// a confirmation flow before calling this (safe publication: the user
    /// must explicitly confirm the title, privacy, and track list).
    pub fn send_create_playlist(
        &mut self,
        title: String,
        description: String,
        privacy: String,
        video_ids: Vec<String>,
    ) -> Result<()> {
        self.pending
            .push_back(Pending::CreatePlaylist(title.clone()));
        self.sidecar.send(&Request::CreatePlaylist {
            title,
            description,
            privacy,
            video_ids,
        })?;
        Ok(())
    }

    /// Fire-and-forget: add tracks to an existing playlist. `duplicates=true`
    /// makes the call idempotent (ytmusicapi skips already-present items), so
    /// retry-on-failure won't double-add. The caller MUST have already shown
    /// the track list and gotten explicit confirmation (safe publication).
    pub fn send_add_playlist_items(
        &mut self,
        playlist_id: String,
        video_ids: Vec<String>,
    ) -> Result<()> {
        self.pending
            .push_back(Pending::AddPlaylistItems(playlist_id.clone()));
        self.sidecar.send(&Request::AddPlaylistItems {
            playlist_id,
            video_ids,
            duplicates: true,
        })?;
        Ok(())
    }

    /// Fire-and-forget: fetch the user's liked-songs playlist. Results land in
    /// the response queue and are drained by `on_tick`. Non-blocking.
    pub fn send_get_liked_songs(&mut self) -> Result<()> {
        self.pending.push_back(Pending::LikedSongs);
        self.sidecar.send(&Request::GetLikedSongs { limit: 100 })?;
        Ok(())
    }

    /// Fire-and-forget: fetch artist info (name, radio/shuffle ids, top songs,
    /// related artists). Results land in the response queue. Non-blocking.
    pub fn send_get_artist(&mut self, channel_id: String) -> Result<()> {
        self.pending.push_back(Pending::Artist(channel_id.clone()));
        self.sidecar.send(&Request::GetArtist { channel_id })?;
        Ok(())
    }

    /// Fire-and-forget: fetch related content for a song ("you might also
    /// like", recommended playlists, similar artists). Results land in the
    /// response queue. Non-blocking.
    pub fn send_get_song_related(&mut self, browse_id: String) -> Result<()> {
        self.pending.push_back(Pending::Related(browse_id.clone()));
        self.sidecar.send(&Request::GetSongRelated { browse_id })?;
        Ok(())
    }

    /// Fire-and-forget: fetch album info (title, artists, year, tracks).
    /// Results land in the response queue. Non-blocking.
    pub fn send_get_album(&mut self, browse_id: String) -> Result<()> {
        self.pending.push_back(Pending::Album(browse_id.clone()));
        self.sidecar.send(&Request::GetAlbum { browse_id })?;
        Ok(())
    }

    /// Drain any pending responses without pairing (legacy). Returns parsed
    /// responses in arrival order.
    pub fn drain(&mut self) -> Vec<Response> {
        let mut out = Vec::new();
        while let Ok(Some(r)) = self.sidecar.try_recv() {
            out.push(r);
        }
        out
    }

    /// Liveness probe: a cheap ping roundtrip. Used at launch to confirm the
    /// sidecar process responded (and thus ytmusicapi init didn't fail hard +
    /// exit). A network-flavored init failure prints the error and sets
    /// `have=False`, but the sidecar stays alive to serve ping/auth_status —
    /// so this returning Ok doesn't guarantee YouTube is reachable. A timeout
    /// here means the process is wedged/dead, which is what launch wants.
    pub fn ping(&mut self) -> Result<()> {
        match self.roundtrip(Request::Ping, Duration::from_secs(3))? {
            Response::Pong => Ok(()),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected ping response")),
        }
    }

    /// Query the sidecar for auth status: cookie presence (`ok`), validity
    /// (`valid` — a lightweight get_home probe succeeds), expiry (`expired` —
    /// cookie present but probe failed with an auth error), and `reason`.
    /// The probe makes a real network call (~1-2s), so the deadline is 6s
    /// (not the 2s that sufficed for the old cookie-presence-only check).
    pub fn auth_status(&mut self) -> Result<AuthStatus> {
        match self.roundtrip(Request::AuthStatus, Duration::from_secs(6))? {
            Response::Auth(a) => Ok(a),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected auth_status response")),
        }
    }

    pub fn search(&mut self, q: &str, limit: u32) -> Result<Vec<RemoteTrackSummary>> {
        // roundtrip's pairing caches each track into track_cache.
        match self.roundtrip(
            Request::Search { q: q.into(), limit },
            Duration::from_secs(3),
        )? {
            Response::Search(v) => Ok(v),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected search response")),
        }
    }

    /// Resolve a stream URL + format for `video_id` (synchronous, with an
    /// up-to-8s bound). `roundtrip`'s pairing already caches the URL + format.
    /// Prefer `send_resolve` (fire-and-forget) for pre-fetching so the hot path
    /// can hit `url_for` instead of blocking.
    /// Resolve a stream URL + format for `video_id` (synchronous). `quality`
    /// selects the tier: `"fast"` (tv_embedded, ~1-2s, AAC 129k) for the instant
    /// play-time resolve, or `"premium"` (tv/web + EJS solver, ~10-15s, AAC
    /// 256k). `roundtrip`'s pairing already caches the URL + format. Prefer the
    /// fire-and-forget `send_resolve` / `send_resolve_premium` for pre-fetching
    /// so the hot path hits `url_for` instead of blocking.
    pub fn resolve_url(&mut self, video_id: &str, quality: &str) -> Result<ResolvedUrl> {
        // The first resolve per sidecar lifetime also pays the one-time Keychain
        // read (macOS) + cookie file write (~8-10s); the warm-up in
        // `refresh_yt_lists` absorbs that. The deadline is generous to survive a
        // slow Keychain unlock prompt and the premium EJS-solver download.
        match self.roundtrip(
            Request::ResolveUrl {
                video_id: video_id.into(),
                quality: quality.into(),
            },
            Duration::from_secs(15),
        )? {
            Response::Resolve(u) => Ok(u),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected resolve_url response")),
        }
    }

    pub fn library_playlists(&mut self) -> Result<Vec<crate::yt::proto::PlaylistSummary>> {
        match self.roundtrip(Request::LibraryPlaylists, Duration::from_secs(3))? {
            Response::Playlists(v) => Ok(v),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected library_playlists response")),
        }
    }

    pub fn get_playlist(&mut self, id: &str) -> Result<Vec<RemoteTrackSummary>> {
        match self.roundtrip(
            Request::GetPlaylist { id: id.into() },
            Duration::from_secs(4),
        )? {
            Response::Tracks(v) => Ok(v),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected get_playlist response")),
        }
    }

    pub fn home_suggestions(&mut self) -> Result<Vec<crate::yt::proto::PlaylistSummary>> {
        match self.roundtrip(Request::HomeSuggestions, Duration::from_secs(3))? {
            Response::Suggestions(v) => Ok(v),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected home_suggestions response")),
        }
    }

    pub fn track_for(&self, video_id: &str) -> Option<&RemoteTrack> {
        self.track_cache.get(video_id)
    }
}

impl YtClient for Session {
    fn get_watch_playlist(&mut self, video_id: &str) -> Result<Vec<String>> {
        match self.roundtrip(
            Request::GetWatchPlaylist {
                video_id: video_id.into(),
            },
            Duration::from_secs(4),
        )? {
            Response::WatchPlaylist(v) => Ok(v.into_iter().map(|t| t.video_id).collect()),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected watch_playlist response")),
        }
    }
}

/// Drives CONT=YouTube autoplay (spec §3.4). Holds a radio queue of video ids
/// and a cursor into it; when exhausted, asks the [`YtClient`] for the next
/// batch seeded by the just-finished track.
#[derive(Default)]
pub struct RadioCursor {
    queue: Vec<String>,
    pos: usize,
}

impl RadioCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the next radio video id, refilling from `yt` (seeded by `seed`)
    /// when the current queue is exhausted. `seed` is the just-finished video;
    /// if the returned radio queue starts with the seed itself, it's dropped so
    /// the cursor advances to a *different* track (matching YouTube's "Up
    /// Next" behaviour).
    pub fn advance(&mut self, yt: &mut dyn YtClient, seed: Option<String>) -> Option<String> {
        if self.pos < self.queue.len() {
            let id = self.queue[self.pos].clone();
            self.pos += 1;
            return Some(id);
        }
        // Exhausted — refill.
        if let Some(seed) = seed {
            if let Ok(mut next) = yt.get_watch_playlist(&seed) {
                // Drop a leading entry equal to the seed so we advance past it
                // (YouTube's "Up Next" excludes the just-played track).
                if next.first().map(|s| s == &seed).unwrap_or(false) {
                    next.remove(0);
                }
                if next.is_empty() {
                    return None;
                }
                self.queue = next;
                self.pos = 1;
                return self.queue.first().cloned();
            }
        }
        None
    }

    /// Return the next queued video id WITHOUT refilling (no sidecar call).
    /// Returns `None` when the queue is exhausted — the caller then decides to
    /// fire-and-forget a refill via [`Session::send_watch_playlist`] and, on
    /// the response landing, [`RadioCursor::advance_with_vids`]. This splits
    /// the old blocking [`RadioCursor::advance`] into a non-blocking local
    /// advance + an async refill so a natural end-of-track never freezes the
    /// UI for the ~4s ytmusicapi roundtrip.
    pub fn next_local(&mut self) -> Option<String> {
        if self.pos < self.queue.len() {
            let id = self.queue[self.pos].clone();
            self.pos += 1;
            Some(id)
        } else {
            None
        }
    }

    /// Refill the queue from pre-fetched `vids` (the fire-and-forget path: the
    /// sidecar call already happened via `send_watch_playlist`, this just
    /// advances the cursor over the result). `seed` is dropped if it's the
    /// leading entry (matches YouTube's "Up Next" excludes the just-played
    /// track). Returns the first video id to play, or `None` when `vids` is
    /// empty after dropping the seed. Mirrors the refill half of
    /// [`RadioCursor::advance`] but without the sidecar roundtrip.
    pub fn advance_with_vids(&mut self, vids: Vec<String>, seed: Option<String>) -> Option<String> {
        let mut next = vids;
        if let Some(seed) = seed {
            if next.first().map(|s| s == &seed).unwrap_or(false) {
                next.remove(0);
            }
        }
        if next.is_empty() {
            return None;
        }
        self.queue = next;
        self.pos = 1;
        self.queue.first().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeYt;
    impl YtClient for FakeYt {
        fn get_watch_playlist(&mut self, _vid: &str) -> Result<Vec<String>> {
            Ok(vec!["yt1".into(), "yt2".into(), "yt3".into()])
        }
    }

    #[test]
    fn radio_cursor_advances_then_refills() {
        let mut rc = RadioCursor::new();
        let mut yt = FakeYt;
        assert_eq!(rc.advance(&mut yt, Some("seed".into())), Some("yt1".into()));
        assert_eq!(rc.advance(&mut yt, Some("yt1".into())), Some("yt2".into()));
        assert_eq!(rc.advance(&mut yt, Some("yt2".into())), Some("yt3".into()));
        // exhausted → refill from the same fake
        assert_eq!(rc.advance(&mut yt, Some("yt3".into())), Some("yt1".into()));
    }

    #[test]
    fn radio_cursor_no_seed_returns_none() {
        let mut rc = RadioCursor::new();
        let mut yt = FakeYt;
        assert_eq!(rc.advance(&mut yt, None), None);
    }
}
