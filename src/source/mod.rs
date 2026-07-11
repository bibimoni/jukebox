//! The local-vs-YouTube source abstraction.
//!
//! [`Transport`] shuffles opaque id [`String`]s. At load time [`App`] asks a
//! [`SourceResolver`] what a given id *means*: a local catalog track (play the
//! file) or a YouTube video (stream the resolved URL). This module owns the
//! types that cross that boundary.
//!
//! - [`TrackSource`] — what a currently-playing (or queued) track *is*.
//! - [`RemoteTrack`] — a YouTube track's metadata (no stream URL; resolved lazy).
//! - [`StreamFormat`] — the resolved audio stream's format (known before load,
//!   so CoreAudio can re-clock the device; spec §3.2/§3.3).

pub mod device_rate;
pub mod match_local;

use serde::{Deserialize, Serialize};

/// What a currently-playing (or queued) track is. The opaque id [`Transport`]
/// already shuffles is available via [`TrackSource::id`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackSource {
    /// A track in the on-disk filtered-lossless catalog.
    Local { track_id: String },
    /// A YouTube video, streamed via a yt-dlp-resolved URL.
    Remote { video_id: String },
}

impl TrackSource {
    /// The opaque id string `Transport` shuffles. Equal for the same logical
    /// track regardless of source kind.
    pub fn id(&self) -> &str {
        match self {
            TrackSource::Local { track_id } => track_id,
            TrackSource::Remote { video_id } => video_id,
        }
    }
    pub fn is_remote(&self) -> bool {
        matches!(self, TrackSource::Remote { .. })
    }
}

/// A YouTube track's metadata, as returned by the sidecar's `search` /
/// `get_playlist` / `get_watch_playlist` commands. The stream URL is **not**
/// stored here — it's resolved lazily by the sidecar's `resolve_url`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub dur: Option<f64>,
    /// Known once `resolve_url` has run; `None` until then.
    #[serde(default)]
    pub fmt: Option<StreamFormat>,
    /// ISRC for officially-sourced music; used by `match_local` (spec §4.1).
    #[serde(default)]
    pub isrc: Option<String>,
}

/// The resolved audio stream's format. Reported by the sidecar so the app
/// knows the format *before* loading (spec §3.2/§3.3 — CoreAudio re-clock).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamFormat {
    pub codec: String,
    /// Audio bitrate in kbps.
    pub abr: u32,
    /// Sample rate in Hz (e.g. 48000, 44100).
    pub sample_rate: u32,
    pub container: String,
    /// True when the Premium ad-free manifest was selected.
    pub premium: bool,
}

impl StreamFormat {
    /// Short label for the player bar: "Opus 160k · YT" or "AAC 256k · YT Premium".
    pub fn yt_label(&self) -> String {
        let bitrate = format!("{}k", self.abr);
        let tier = if self.premium { " · YT Premium" } else { " · YT" };
        format!("{} {}{}", self.codec, bitrate, tier)
    }
}
