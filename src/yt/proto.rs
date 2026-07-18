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
    /// Create a new YouTube playlist (ytmusicapi `create_playlist`). `privacy`
    /// defaults to `"PRIVATE"` (the safest option). `video_ids` is optional —
    /// pass a non-empty vec to seed the playlist at creation, or empty for a
    /// blank playlist. Fire-and-forget; the response carries the new playlist id.
    CreatePlaylist {
        title: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        privacy: String,
        #[serde(default)]
        video_ids: Vec<String>,
    },
    /// Add tracks to an existing playlist (ytmusicapi `add_playlist_items`).
    /// `duplicates` defaults to `true` so retry-on-failure is idempotent.
    /// Fire-and-forget; the response carries a status + count.
    AddPlaylistItems {
        playlist_id: String,
        video_ids: Vec<String>,
        #[serde(default)]
        duplicates: bool,
    },
    /// Fetch the user's liked-songs playlist (ytmusicapi `get_liked_songs`).
    /// `limit` caps the fetch (default 100) so a huge library doesn't block the
    /// single-threaded sidecar. Fire-and-forget; the response is a track list.
    GetLikedSongs {
        #[serde(default)]
        limit: u32,
    },
    /// Fetch artist info (ytmusicapi `get_artist`): name, channel id,
    /// shuffleId/radioId for radio seeding, top songs, and related artists.
    /// Fire-and-forget; the response is an [`ArtistSummary`].
    GetArtist {
        channel_id: String,
    },
    /// Fetch related content for a song (ytmusicapi `get_song_related`):
    /// flattens ytmusicapi's sectioned response into tracks + playlists.
    /// Fire-and-forget; the response carries both lists.
    GetSongRelated {
        browse_id: String,
    },
    /// Fetch album info (ytmusicapi `get_album`): title, artists, year, and
    /// tracks. Fire-and-forget; the response is an [`AlbumSummary`].
    GetAlbum {
        browse_id: String,
    },
    /// Fetch the YouTube Music home feed (ytmusicapi `get_home`). Fire-and-
    /// forget; the response is a list of [`HomeSectionProto`] shelves.
    Home,
    /// Fetch the YouTube Music explore shelves (ytmusicapi `get_explore`).
    /// Fire-and-forget; the response is a list of [`PlaylistProto`] mood/genre
    /// playlists.
    Explore,
    /// Fetch the YouTube Music charts (ytmusicapi `get_charts`). Fire-and-
    /// forget; the response is a flat list of [`ChartEntryProto`] entries
    /// across the available chart categories.
    Charts,
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
    #[serde(default)]
    pub id: String,
    #[serde(default)]
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
    /// Track title from yt-dlp's extraction — cached so the player bar /
    /// Queue view show the real title instead of the raw 11-char video_id.
    /// Empty when yt-dlp didn't provide one (rare; the view falls back to
    /// "Loading…" and a get_watch_playlist will fill it later).
    #[serde(default)]
    pub title: String,
    /// Track artist from yt-dlp's extraction (the `uploader` or `artist`
    /// field). May be empty — the view falls back gracefully.
    #[serde(default)]
    pub artist: String,
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

/// A related artist entry: a name + a browse id (the channel/artist page id
/// ytmusicapi returns). Used in [`ArtistSummary::related`] and
/// [`AlbumSummary::artists`] — both are "name + browse id" pairs, so one
/// struct covers both.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RelatedArtist {
    pub name: String,
    #[serde(default)]
    pub browse_id: String,
}

/// Artist info from ytmusicapi `get_artist`, flattened into the fields the
/// Rust side needs. `shuffle_id` / `radio_id` seed the autoplay radio;
/// `songs_browse_id` can be used to fetch the full top-songs list;
/// `songs` is the first page; `related` is related artists.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ArtistSummary {
    pub name: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub shuffle_id: String,
    #[serde(default)]
    pub radio_id: String,
    #[serde(default)]
    pub subscribers: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub songs_browse_id: String,
    #[serde(default)]
    pub songs: Vec<RemoteTrackSummary>,
    #[serde(default)]
    pub related: Vec<RelatedArtist>,
}

/// Album info from ytmusicapi `get_album`: title, artists, year, and tracks.
/// `artists` reuses [`RelatedArtist`] (name + browse id).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AlbumSummary {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<RelatedArtist>,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub tracks: Vec<RemoteTrackSummary>,
}

/// One shelf of the YouTube Music home feed (ytmusicapi `get_home`). Each
/// shelf has a title and a list of [`HomeItemProto`] entries. The sidecar
/// flattens ytmusicapi's sectioned response into these small typed payloads.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HomeSectionProto {
    pub title: String,
    #[serde(default)]
    pub items: Vec<HomeItemProto>,
}

/// One entry in a home shelf. Exactly one of `playlist_id` / `video_id` is
/// set (the sidecar picks the most useful destination for a tap); `artist`
/// and `browse_id` are populated for artist-radio shelves. Every optional
/// field carries `#[serde(default)]` so the sidecar may omit fields it
/// couldn't populate without failing the parse.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HomeItemProto {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub video_id: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    /// For artist-radio shelves: the channel/artist browse id used to seed
    /// radio via a follow-up `GetArtist` request.
    #[serde(default)]
    pub browse_id: Option<String>,
}

/// One explore shelf playlist (mood/genre). Returned by `get_explore`.
/// `count` is the playlist's track count when ytmusicapi provides it; `None`
/// when the sidecar couldn't determine it.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlaylistProto {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub count: Option<usize>,
}

/// One entry in a YouTube Music chart. `chart` is the category label
/// ("Top songs", "Top videos", "Trending", "Top artists"). Exactly one of
/// `video_id` / `playlist_id` / `artist` is set depending on the category;
/// the rest are `None`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ChartEntryProto {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub video_id: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    pub chart: String,
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
    /// A newly-created playlist's id + the title/privacy we asked for (echoed
    /// back so the caller can confirm what was created).
    CreatedPlaylist {
        id: String,
        title: String,
        privacy: String,
    },
    /// The result of adding tracks to a playlist: a ytmusicapi status string
    /// (e.g. `"STATUS_SUCCEEDED"`) + the count of video_ids we asked to add.
    AddedItems {
        status: String,
        count: u32,
    },
    /// The user's liked-songs playlist as a track list.
    LikedSongs(Vec<RemoteTrackSummary>),
    /// Artist info (flattened from ytmusicapi `get_artist`).
    ArtistInfo(ArtistSummary),
    /// Related content for a song, split into tracks and playlists.
    RelatedContent {
        tracks: Vec<RemoteTrackSummary>,
        playlists: Vec<PlaylistSummary>,
    },
    /// Album info (flattened from ytmusicapi `get_album`).
    AlbumInfo(AlbumSummary),
    /// Home feed shelves (flattened from ytmusicapi `get_home`).
    HomeSections(Vec<HomeSectionProto>),
    /// Explore shelves (flattened from ytmusicapi `get_explore`).
    ExplorePlaylists(Vec<PlaylistProto>),
    /// Chart entries across categories (flattened from ytmusicapi
    /// `get_charts`).
    Charts(Vec<ChartEntryProto>),
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
            if let Some(val) = o.get("created_playlist") {
                let empty = serde_json::Map::new();
                let obj = val.as_object().unwrap_or(&empty);
                return Ok(Response::CreatedPlaylist {
                    id: obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: obj
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    privacy: obj
                        .get("privacy")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
            if let Some(val) = o.get("added_items") {
                let empty = serde_json::Map::new();
                let obj = val.as_object().unwrap_or(&empty);
                return Ok(Response::AddedItems {
                    status: obj
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    count: obj.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                });
            }
            if let Some(val) = o.get("liked_songs") {
                return Ok(Response::LikedSongs(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("artist_info") {
                return Ok(Response::ArtistInfo(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("related_content") {
                let empty = serde_json::Map::new();
                let obj = val.as_object().unwrap_or(&empty);
                let tracks: Vec<RemoteTrackSummary> = obj
                    .get("tracks")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?
                    .unwrap_or_default();
                let playlists: Vec<PlaylistSummary> = obj
                    .get("playlists")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?
                    .unwrap_or_default();
                return Ok(Response::RelatedContent { tracks, playlists });
            }
            if let Some(val) = o.get("album_info") {
                return Ok(Response::AlbumInfo(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("home_sections") {
                return Ok(Response::HomeSections(serde_json::from_value(val.clone())?));
            }
            if let Some(val) = o.get("explore_playlists") {
                return Ok(Response::ExplorePlaylists(serde_json::from_value(
                    val.clone(),
                )?));
            }
            if let Some(val) = o.get("charts") {
                return Ok(Response::Charts(serde_json::from_value(val.clone())?));
            }
        }
        // Truncate the raw line to avoid leaking cookie material if the sidecar
        // is buggy and prints auth headers to stdout. 200 chars is enough to
        // diagnose a protocol mismatch without exposing sensitive data.
        let preview: String = line.chars().take(200).collect();
        Err(anyhow!("unrecognized sidecar response: {preview}"))
    }
}
