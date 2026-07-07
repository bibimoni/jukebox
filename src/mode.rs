//! The source mode: where playback material comes from.
//!
//! `Local` plays only the on-disk filtered-lossless catalog. `YouTube` plays
//! only streamed-from-YouTube tracks (account playlists, suggested, search,
//! autoplay radio). `Mixed` plays the local copy when a robust match exists,
//! else streams from YouTube.
//!
//! Cycled by the `M` key in the TUI and persisted in `state.db` via
//! `LayoutState.source_mode`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SourceMode {
    #[default]
    Local,
    Youtube,
    Mixed,
}

impl SourceMode {
    pub fn cycle(self) -> Self {
        match self {
            SourceMode::Local => SourceMode::Youtube,
            SourceMode::Youtube => SourceMode::Mixed,
            SourceMode::Mixed => SourceMode::Local,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            SourceMode::Local => "local",
            SourceMode::Youtube => "youtube",
            SourceMode::Mixed => "mixed",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "youtube" => SourceMode::Youtube,
            "mixed" => SourceMode::Mixed,
            _ => SourceMode::Local,
        }
    }
}
