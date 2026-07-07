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
    if !venv_python().exists() {
        let status = std::process::Command::new("python3")
            .args(["-m", "venv", dir.to_str().unwrap_or("")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .status()?;
        if !status.success() {
            anyhow::bail!("python3 -m venv failed (exit {status})");
        }
    }
    let pip = dir.join("bin").join("pip");
    let status = std::process::Command::new(&pip)
        .args(["install", "-q", "-r"])
        .arg(requirements)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()?;
    if !status.success() {
        anyhow::bail!("pip install failed (exit {status})");
    }
    Ok(format!("installed YT deps into {}", dir.display()))
}

pub struct Session {
    sidecar: Sidecar,
    cookies: Option<String>,
    /// Browser profile to read cookies from (`"chrome"` etc.), when set instead
    /// of a pasted cookies file. Mutually exclusive with `cookies`.
    pub browser: Option<String>,
    /// video_id → RemoteTrack metadata seen via search/get_playlist/watch.
    pub track_cache: HashMap<String, RemoteTrack>,
    /// video_id → (url, expires_at). Capped at current+next.
    url_cache: Vec<(String, String, Option<f64>)>,
}

const URL_CACHE_CAP: usize = 2;

impl Session {
    /// Spawn with pasted `cookies` (Netscape) OR `browser` (profile name) —
    /// pass exactly one; both `None` runs guest mode. `browser` makes the
    /// sidecar read cookies from the browser profile (no file written).
    pub fn spawn(python: &Path, script: &Path, cookies: Option<String>) -> Result<Self> {
        let sidecar = Sidecar::spawn(python, script, cookies.clone(), None)?;
        Ok(Session {
            sidecar,
            cookies,
            browser: None,
            track_cache: HashMap::new(),
            url_cache: Vec::new(),
        })
    }

    /// Spawn reading cookies from a browser profile (e.g. `"chrome"`). No
    /// cookie file is written; values stay in the browser. Used by
    /// `:yt auth browser <name>`.
    pub fn spawn_browser(python: &Path, script: &Path, browser: String) -> Result<Self> {
        let sidecar = Sidecar::spawn(python, script, None, Some(browser.clone()))?;
        Ok(Session {
            sidecar,
            cookies: None,
            browser: Some(browser),
            track_cache: HashMap::new(),
            url_cache: Vec::new(),
        })
    }

    pub fn is_alive(&mut self) -> bool {
        self.sidecar.is_alive()
    }

    pub fn has_cookies(&self) -> bool {
        self.cookies.is_some() || self.browser.is_some()
    }

    /// Clear cookies and respawn guest.
    pub fn clear_cookies(&mut self, python: &Path, script: &Path) -> Result<()> {
        self.cookies = None;
        self.browser = None;
        self.sidecar = Sidecar::spawn(python, script, None, None)?;
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
        self.sidecar = Sidecar::spawn(python, script, self.cookies.clone(), None)?;
        Ok(())
    }

    /// Respawn the sidecar reading cookies from a browser profile (e.g.
    /// `"chrome"`, `"firefox"`, `"safari"`, `"edge"`). No cookie file is
    /// written; the cookie values stay in the browser. Clears any pasted
    /// cookies (mutually exclusive). Used by `:yt auth browser <name>`.
    pub fn set_browser(&mut self, browser: String, python: &Path, script: &Path) -> Result<()> {
        self.browser = Some(browser.clone());
        self.cookies = None;
        self.sidecar = Sidecar::spawn(python, script, None, Some(browser))?;
        Ok(())
    }

    /// Send a request and poll for the matching response until `deadline`.
    fn roundtrip(&mut self, req: Request, deadline: Duration) -> Result<Response> {
        self.sidecar.send(&req)?;
        let start = Instant::now();
        loop {
            match self.sidecar.try_recv()? {
                Some(resp) => return Ok(resp),
                None => {
                    if start.elapsed() >= deadline {
                        return Err(anyhow!("sidecar roundtrip timeout"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// Drain any pending responses without blocking (used by App::on_tick to
    /// apply async list-fetch results). Returns parsed responses.
    pub fn drain(&mut self) -> Vec<Response> {
        let mut out = Vec::new();
        while let Ok(Some(r)) = self.sidecar.try_recv() {
            out.push(r);
        }
        out
    }

    pub fn auth_status(&mut self) -> Result<AuthStatus> {
        match self.roundtrip(Request::AuthStatus, Duration::from_secs(2))? {
            Response::Auth(a) => Ok(a),
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected auth_status response")),
        }
    }

    pub fn search(&mut self, q: &str, limit: u32) -> Result<Vec<RemoteTrackSummary>> {
        match self.roundtrip(Request::Search { q: q.into(), limit }, Duration::from_secs(3))? {
            Response::Search(v) => {
                for t in &v {
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
                Ok(v)
            }
            Response::Error(e) => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected search response")),
        }
    }

    /// Resolve a stream URL + format for `video_id`. Premium-aware. Caches the
    /// resolved url (current + next) for gapless handoff; the format is always
    /// returned fresh so CoreAudio re-clocks against the real stream.
    pub fn resolve_url(&mut self, video_id: &str) -> Result<ResolvedUrl> {
        match self.roundtrip(Request::ResolveUrl { video_id: video_id.into() }, Duration::from_secs(8))? {
            Response::Resolve(u) => {
                // Cache current+next.
                self.url_cache.retain(|(v, _, _)| v != video_id);
                self.url_cache.push((video_id.into(), u.url.clone(), u.expires_at));
                if self.url_cache.len() > URL_CACHE_CAP {
                    self.url_cache.remove(0);
                }
                // Record format on the cached track.
                if let Some(t) = self.track_cache.get_mut(video_id) {
                    t.fmt = Some(StreamFormat {
                        codec: u.codec.clone(),
                        abr: u.abr,
                        sample_rate: u.sample_rate,
                        container: u.container.clone(),
                        premium: u.premium,
                    });
                }
                Ok(u)
            }
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
            Response::Tracks(v) => {
                for t in &v {
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
                Ok(v)
            }
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
            Response::WatchPlaylist(v) => {
                for t in &v {
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
                Ok(v.into_iter().map(|t| t.video_id).collect())
            }
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
