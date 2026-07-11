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
    Search { q: String, limit: u32 },
    LibraryPlaylists,
    GetPlaylist { id: String },
    HomeSuggestions,
    GetWatchPlaylist { video_id: String },
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
    pub ok: bool,
    #[serde(default)]
    pub premium: bool,
    #[serde(default)]
    pub account: bool,
}

// --- Responses -------------------------------------------------------------

/// A sidecar response. `from_line` parses the `{"ok":..., "data":...}` wrapper.
#[derive(Clone, Debug)]
pub enum Response {
    Search(Vec<RemoteTrackSummary>),
    Playlists(Vec<PlaylistSummary>),
    Tracks(Vec<RemoteTrackSummary>),
    Suggestions(Vec<PlaylistSummary>),
    WatchPlaylist(Vec<RemoteTrackSummary>),
    Resolve(ResolvedUrl),
    Auth(AuthStatus),
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
        let v: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| anyhow!("bad sidecar json: {e}"))?;
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
                return Ok(Response::WatchPlaylist(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("resolve") {
                return Ok(Response::Resolve(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("auth") {
                return Ok(Response::Auth(serde_json::from_value(val.clone())?));
            }
        }
        Err(anyhow!("unrecognized sidecar response: {line}"))
    }
}
