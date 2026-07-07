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

pub struct Session {
    sidecar: Sidecar,
    cookies: Option<String>,
    /// video_id → RemoteTrack metadata seen via search/get_playlist/watch.
    pub track_cache: HashMap<String, RemoteTrack>,
    /// video_id → (url, expires_at). Capped at current+next.
    url_cache: Vec<(String, String, Option<f64>)>,
}

const URL_CACHE_CAP: usize = 2;

impl Session {
    pub fn spawn(python: &Path, script: &Path, cookies: Option<String>) -> Result<Self> {
        let sidecar = Sidecar::spawn(python, script, cookies.clone())?;
        Ok(Session {
            sidecar,
            cookies,
            track_cache: HashMap::new(),
            url_cache: Vec::new(),
        })
    }

    pub fn is_alive(&mut self) -> bool {
        self.sidecar.is_alive()
    }

    pub fn has_cookies(&self) -> bool {
        self.cookies.is_some()
    }

    /// Clear cookies and respawn guest.
    pub fn clear_cookies(&mut self, python: &Path, script: &Path) -> Result<()> {
        self.cookies = None;
        self.sidecar = Sidecar::spawn(python, script, None)?;
        Ok(())
    }

    /// Persist `cookies` (Netscape cookies.txt) to the cookies file (perms
    /// 0600) and respawn the sidecar with them. One paste feeds both
    /// `ytmusicapi` (via the cookie header) and `yt-dlp` (via the file).
    pub fn set_cookies(&mut self, cookies: String, python: &Path, script: &Path) -> Result<()> {
        let p = cookies_file();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, &cookies)?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        self.cookies = Some(cookies);
        self.sidecar = Sidecar::spawn(python, script, self.cookies.clone())?;
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
    /// when the current queue is exhausted. `seed` is the just-finished video.
    pub fn advance(&mut self, yt: &mut dyn YtClient, seed: Option<String>) -> Option<String> {
        if self.pos < self.queue.len() {
            let id = self.queue[self.pos].clone();
            self.pos += 1;
            return Some(id);
        }
        // Exhausted — refill.
        if let Some(seed) = seed {
            if let Ok(next) = yt.get_watch_playlist(&seed) {
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
