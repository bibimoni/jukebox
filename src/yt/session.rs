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

/// Path to the persisted cookies file: `<config_dir>/jukebox/yt-cookies.txt`.
pub fn cookies_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.config"));
    let dir = base.join("jukebox");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("yt-cookies.txt")
}

/// Load persisted cookies (Netscape cookies.txt). `None` if absent/empty.
pub fn load_cookies() -> Option<String> {
    let p = cookies_file();
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
            anyhow::bail!("python3 -m venv failed (exit {status}); see {}", log_path.display());
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
        anyhow::bail!("pip install failed (exit {status}); see {}", log_path.display());
    }
    Ok(format!("installed YT deps into {} (log: {})", dir.display(), log_path.display()))
}

/// Where `:yt setup` writes venv/pip output so it doesn't hit the terminal.
fn setup_log_path() -> std::path::PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    cache.join("jukebox").join("yt-setup.log")
}

/// The kind of an in-flight request, so drained responses can be paired with
/// what asked for them (the sidecar replies in send order, so this is a FIFO
/// queue). `Resolve` carries the video_id the URL is for.
#[derive(Clone, Debug)]
enum Pending {
    Playlists,
    Suggestions,
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
    Watch,
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
    /// `(list_id, video_ids)` from a fire-and-forget `send_get_playlist`,
    /// picked up by `App::on_tick` to populate the focused `YtList.track_ids`.
    pub pending_tracks: Option<(String, Vec<String>)>,
    /// The list id whose tracks are currently being fetched (guards against
    /// re-sending every tick while the fetch is in flight).
    playlist_inflight: Option<String>,
    /// `(query, video_ids)` from a fire-and-forget `send_search`, picked up by
    /// `App::on_tick` to populate the search overlay's results. Carries the
    /// query so App only applies it to the overlay that asked for it.
    pub pending_search: Option<(String, Vec<String>)>,
    /// The query currently being searched (guards against re-sending the same
    /// query while in flight).
    search_inflight: Option<String>,
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
}

/// Max auto-respawn attempts before giving up (surfacing `yt_error` instead).
const RESPAWN_MAX: u32 = 3;
/// Minimum gap between respawn attempts.
const RESPAWN_GAP: Duration = Duration::from_secs(5);

const URL_CACHE_CAP: usize = 2;

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
            url_cache: Vec::new(),
            pending: std::collections::VecDeque::new(),
            resolve_inflight: None,
            premium_resolve_inflight: None,
            pending_premium_url: None,
            pending_playlists: None,
            pending_suggestions: None,
            pending_tracks: None,
            playlist_inflight: None,
            pending_search: None,
            search_inflight: None,
            pending_errors: Vec::new(),
            respawn_attempts: 0,
            last_respawn: None,
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
        let cf = cookies_file().to_string_lossy().to_string();
        let sidecar = Sidecar::spawn(python, script, None, Some(browser.clone()), Some(cf))?;
        Ok(Session {
            sidecar,
            cookies: None,
            browser: Some(browser),
            track_cache: HashMap::new(),
            url_cache: Vec::new(),
            pending: std::collections::VecDeque::new(),
            resolve_inflight: None,
            premium_resolve_inflight: None,
            pending_premium_url: None,
            pending_playlists: None,
            pending_suggestions: None,
            pending_tracks: None,
            playlist_inflight: None,
            pending_search: None,
            search_inflight: None,
            pending_errors: Vec::new(),
            respawn_attempts: 0,
            last_respawn: None,
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
            self.url_cache.push(CachedResolve { video_id: video_id.to_string(), ..Default::default() });
            if self.url_cache.len() > URL_CACHE_CAP {
                self.url_cache.remove(0);
            }
        }
        self.url_cache.iter_mut().find(|c| c.video_id == video_id).expect("just inserted")
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
        self.sidecar = Sidecar::spawn(python, script, None, None, None)?;
        Ok(())
    }

    /// Persist `cookies` (Netscape cookies.txt) to the cookies file (perms
    /// 0600) and respawn the sidecar with them. One paste feeds both
    /// `ytmusicapi` (via the cookie header) and `yt-dlp` (via the file).
    /// Clears any browser profile (mutually exclusive).
    pub fn set_cookies(&mut self, cookies: String, python: &Path, script: &Path) -> Result<()> {
        let p = cookies_file();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, &cookies)?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        self.cookies = Some(cookies);
        self.browser = None;
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
        let cf = cookies_file().to_string_lossy().to_string();
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
                self.pending_tracks = Some((id.clone(), vids));
                // Free the inflight guard on SUCCESS (not just error) so a later
                // refresh_yt_lists (which clears loaded_yt_lists) can re-fetch
                // this list. Without this the guard stays Some(id) forever after
                // the first successful load, wedging col2 on a re-focus.
                self.playlist_inflight = None;
            }
            (Response::WatchPlaylist(v), Pending::Watch) => {
                for t in v {
                    self.cache_track(t);
                }
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
            (Response::Playlists(v), Pending::Playlists) => {
                self.pending_playlists = Some(v.clone());
            }
            (Response::Suggestions(v), Pending::Suggestions) => {
                self.pending_suggestions = Some(v.clone());
            }
            (Response::Auth(_), Pending::Auth) | (Response::Pong, Pending::Pong) => {}
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
            (Response::Error(e), Pending::Tracks(_)) => {
                self.playlist_inflight = None;
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
    }

    fn kind_for(req: &Request) -> Pending {
        match req {
            Request::Search { q, .. } => Pending::Search(q.clone()),
            Request::LibraryPlaylists => Pending::Playlists,
            Request::GetPlaylist { id } => Pending::Tracks(id.clone()),
            Request::HomeSuggestions => Pending::Suggestions,
            Request::GetWatchPlaylist { .. } => Pending::Watch,
            Request::ResolveUrl { video_id, quality } => {
                if quality == "premium" {
                    Pending::ResolvePremium(video_id.clone())
                } else {
                    Pending::Resolve(video_id.clone())
                }
            }
            Request::AuthStatus => Pending::Auth,
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
                    if let Some(pk) = pk {
                        if let Some(r) = self.apply_pair(pk, resp, &target) {
                            return Ok(r);
                        }
                        continue; // applied as a stray; keep waiting for ours
                    }
                    return Ok(resp);
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
                            Pending::Search(q) => {
                                if self.search_inflight.as_deref() == Some(q.as_str()) {
                                    self.search_inflight = None;
                                }
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
        while let Ok(Some(resp)) = self.sidecar.try_recv() {
            let kind = self.pending.pop_front();
            if let Some(k) = kind {
                let target = k.clone();
                self.apply_pair(k, resp.clone(), &target);
            }
            out.push(resp);
        }
        out
    }

    /// Fire-and-forget: ask for the account playlists + suggested/mood lists.
    /// Results land in `pending_playlists`/`pending_suggestions` and are
    /// picked up by `App::on_tick`. Non-blocking — doesn't wait for a reply.
    pub fn send_refresh(&mut self) -> Result<()> {
        self.pending.push_back(Pending::Playlists);
        self.sidecar.send(&Request::LibraryPlaylists)?;
        self.pending.push_back(Pending::Suggestions);
        self.sidecar.send(&Request::HomeSuggestions)?;
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
        self.sidecar.send(&Request::ResolveUrl { video_id, quality: String::new() })?;
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
        self.pending.push_back(Pending::ResolvePremium(video_id.clone()));
        self.sidecar
            .send(&Request::ResolveUrl { video_id, quality: "premium".to_string() })?;
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
    /// (picked up by `App::on_tick`). Only one list is fetched at a time
    /// (`playlist_inflight`); re-sending the same id while in flight is a
    /// no-op so `on_tick`'s focus-trigger doesn't flood the sidecar.
    pub fn send_get_playlist(&mut self, id: String) -> Result<()> {
        if self.playlist_inflight.as_deref() == Some(id.as_str()) {
            return Ok(());
        }
        self.playlist_inflight = Some(id.clone());
        self.pending.push_back(Pending::Tracks(id.clone()));
        self.sidecar.send(&Request::GetPlaylist { id })?;
        Ok(())
    }

    /// True if a get_playlist for `id` is currently in flight.
    pub fn playlist_loading(&self, id: &str) -> bool {
        self.playlist_inflight.as_deref() == Some(id)
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

    pub fn auth_status(&mut self) -> Result<AuthStatus> {
        match self.roundtrip(Request::AuthStatus, Duration::from_secs(2))? {
            Response::Auth(a) => Ok(a),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected auth_status response")),
        }
    }

    pub fn search(&mut self, q: &str, limit: u32) -> Result<Vec<RemoteTrackSummary>> {
        // roundtrip's pairing caches each track into track_cache.
        match self.roundtrip(Request::Search { q: q.into(), limit }, Duration::from_secs(3))? {
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
            Request::ResolveUrl { video_id: video_id.into(), quality: quality.into() },
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
        match self.roundtrip(Request::GetPlaylist { id: id.into() }, Duration::from_secs(4))? {
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
        match self.roundtrip(Request::GetWatchPlaylist { video_id: video_id.into() }, Duration::from_secs(4))? {
            Response::WatchPlaylist(v) => Ok(v.into_iter().map(|t| t.video_id).collect()),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected watch_playlist response")),
        }
    }
}

/// Drives CONT=YouTube autoplay (spec §3.4). Holds a radio queue of video ids
/// and a cursor into it; when exhausted, asks the [`YtClient`] for the next
/// batch seeded by the just-finished track.
pub struct RadioCursor {
    queue: Vec<String>,
    pos: usize,
}

impl Default for RadioCursor {
    fn default() -> Self {
        RadioCursor { queue: Vec::new(), pos: 0 }
    }
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
