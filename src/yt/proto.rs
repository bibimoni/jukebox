//! The newline-delimited JSON wire protocol between jukebox and the Python
//! sidecar. Requests are one JSON object per line: `{"cmd":"search",...}`.
//! Responses: `{"ok":true,"data":{...}}` or `{"ok":false,"error":"..."}`.
//!
//! The Rust side never parses YouTube's internal format — the sidecar
//! translates it into these small typed payloads.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

// --- Requests --------------------------------------------------------------

/// A command sent to the sidecar. `to_line` serializes a single-line JSON
/// object with a `"cmd"` discriminator.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Search {
        q: String,
        limit: u32,
    },
    LibraryPlaylists,
    GetPlaylist {
        id: String,
    },
    HomeSuggestions,
    GetWatchPlaylist {
        video_id: String,
    },
    /// Resolve a playable stream URL. `quality` selects the yt-dlp client tier:
    /// `"fast"` (default) → `tv_embedded`, ~1.3s, caps at AAC 129k (itag 140);
    /// `"premium"` → `tv`/`web` + the deno EJS nsig solver, ~10-15s, reaches
    /// AAC 256k (itag 141) for Premium users. `#[serde(default)]` so an old
    /// sidecar/client that omits it still parses (defaults to "" → "fast").
    ResolveUrl {
        video_id: String,
        #[serde(default)]
        quality: String,
    },
    /// Fetch lyrics for a YouTube video (ytmusicapi `get_lyrics`). The sidecar
    /// resolves the lyrics `browseId` via `get_watch_playlist(videoId)` then
    /// calls `get_lyrics(browseId, timestamps=True)`. Fire-and-forget; the
    /// response lands in `Response::Lyrics` and is drained by `on_tick`.
    GetLyrics {
        video_id: String,
    },
    Ping,
    AuthStatus,
}

impl Request {
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("request serializes")
    }
}

// --- Payloads --------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RemoteTrackSummary {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub dur: Option<f64>,
    #[serde(default)]
    pub isrc: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ResolvedUrl {
    pub url: String,
    #[serde(default)]
    pub expires_at: Option<f64>,
    pub codec: String,
    pub abr: u32,
    pub sample_rate: u32,
    pub container: String,
    #[serde(default)]
    pub premium: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AuthStatus {
    /// Cookie is present (SAPISID/__Secure-3PAPISID string exists in the jar).
    /// This is the backwards-compat "ok" field; it does NOT mean the credential
    /// works — an expired/revoked cookie still has ok=true. Use `valid` for
    /// "the credential actually works."
    pub ok: bool,
    #[serde(default)]
    pub premium: bool,
    #[serde(default)]
    pub account: bool,
    /// True only if the sidecar's lightweight data probe (get_home(limit=1))
    /// succeeded — the credential is actually valid right now. False when the
    /// probe hasn't run (old sidecar), failed (expired/revoked), or ytmusicapi
    /// isn't installed. This is the field callers should gate UI state on.
    #[serde(default)]
    pub valid: bool,
    /// True when a cookie is present (ok=true) but the probe failed with an
    /// auth-flavored error (401/unauthorized/forbidden) — the credential has
    /// expired or been revoked. When valid=false and expired=false, the
    /// failure was non-auth (network, ytmusicapi not installed, etc.).
    #[serde(default)]
    pub expired: bool,
    /// Human-readable reason when valid=false (the probe's exception message,
    /// or "ytmusicapi not initialized"). None when valid=true or no cookie.
    #[serde(default)]
    pub reason: Option<String>,
}

/// One line of lyrics over the sidecar wire. `time` is the timestamp in
/// **seconds** (the sidecar converts ytmusicapi's milliseconds → seconds
/// before sending, so the Rust side compares directly against
/// `player.position()`). `None` for plain / unsynchronized lyrics.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LyricLineProto {
    #[serde(default)]
    pub time: Option<f64>,
    pub text: String,
}

// --- Responses -------------------------------------------------------------

/// A sidecar response. `from_line` parses the `{"ok":..., "data":...}` wrapper.
#[derive(Clone, Debug)]
pub enum Response {
    Search(Vec<RemoteTrackSummary>),
    /// Library playlists — **full pagination** is delegated to the sidecar
    /// (`yt.py` calls `get_library_playlists(limit=None)`), so the Rust side
    /// receives ALL items in one response. No `has_more`/`continuation` fields
    /// are needed — the sidecar iterates internally until ytmusicapi stops
    /// returning more pages. The Rust side never issues a follow-up page
    /// request; if a list is truncated it's because ytmusicapi's internal
    /// pagination ended.
    Playlists(Vec<PlaylistSummary>),
    /// Playlist tracks — same full-pagination design as `Playlists`: the
    /// sidecar calls `get_playlist(id, limit=None)` and returns all tracks
    /// in one response.
    Tracks(Vec<RemoteTrackSummary>),
    Suggestions(Vec<PlaylistSummary>),
    WatchPlaylist(Vec<RemoteTrackSummary>),
    Resolve(ResolvedUrl),
    Auth(AuthStatus),
    /// Lyrics from the sidecar's `get_lyrics` command. Carries the lines (with
    /// timestamps in seconds) and whether they're synchronized. Empty lines
    /// with `synced=false` means "no lyrics found" (the sidecar returns a
    /// not-found payload rather than an error, so the UI shows a truthful
    /// "lyrics unavailable" state).
    Lyrics(Vec<LyricLineProto>, bool),
    Pong,
    Error(String),
}

impl std::fmt::Display for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Response {
    pub fn from_line(line: &str) -> Result<Response> {
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| anyhow!("bad sidecar json: {e}"))?;
        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        if !ok {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown sidecar error")
                .to_string();
            return Ok(Response::Error(err));
        }
        let data = v.get("data").cloned().unwrap_or(serde_json::Value::Null);
        if let Some(o) = data.as_object() {
            if let Some(val) = o.get("pong") {
                if val.as_bool() == Some(true) {
                    return Ok(Response::Pong);
                }
            }
            if let Some(val) = o.get("search") {
                return Ok(Response::Search(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("playlists") {
                return Ok(Response::Playlists(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("tracks") {
                return Ok(Response::Tracks(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("suggestions") {
                return Ok(Response::Suggestions(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("watch_playlist") {
                return Ok(Response::WatchPlaylist(serde_json::from_value(
                    val.clone(),
                )?));
            }
            if let Some(val) = o.get("resolve") {
                return Ok(Response::Resolve(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("auth") {
                return Ok(Response::Auth(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("lyrics") {
                // The sidecar sends `{"lyrics": {"lines": [...], "synced": bool}}`.
                let empty = serde_json::Map::new();
                let obj = val.as_object().unwrap_or(&empty);
                let lines: Vec<LyricLineProto> = obj
                    .get("lines")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?
                    .unwrap_or_default();
                let synced: bool = obj.get("synced").and_then(|s| s.as_bool()).unwrap_or(false);
                return Ok(Response::Lyrics(lines, synced));
            }
        }
        // Truncate the raw line to avoid leaking cookie material if the sidecar
        // is buggy and prints auth headers to stdout. 200 chars is enough to
        // diagnose a protocol mismatch without exposing sensitive data.
        let preview: String = line.chars().take(200).collect();
        Err(anyhow!("unrecognized sidecar response: {preview}"))
    }
}
