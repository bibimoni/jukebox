use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub built_at: String,
    pub source_root: PathBuf,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub artists: Vec<String>,
    pub primary_artist: String,
    pub title: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub bit_depth: u32,
    #[serde(default)]
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub isrc: Option<String>,
    pub source_path: PathBuf,
    pub symlinked_into_artists: Vec<String>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Catalog> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading catalog {}", path.display()))?;
        let c: Catalog = serde_json::from_str(&text)
            .with_context(|| format!("parsing catalog {}", path.display()))?;
        Ok(c)
    }

    /// Load a catalog for playback. Returns `Ok(Some(cat))` when the catalog
    /// exists and has at least one track, `Ok(None)` when it is missing or
    /// empty so the caller can print a recovery hint instead of crashing into
    /// a mid-playback "file not found", and `Err` only for a read/parse error
    /// that isn't "missing" (so a corrupt catalog isn't silently treated as
    /// absent).
    pub fn load_for_playback(path: &Path) -> Result<Option<Catalog>> {
        if !path.exists() {
            return Ok(None);
        }
        let cat = Self::load(path)?;
        if cat.tracks.is_empty() {
            return Ok(None);
        }
        Ok(Some(cat))
    }

    /// Reject an empty catalog with an actionable message so `jukebox sync`
    /// fails loudly instead of reporting silent success ("synced: 0 tracks").
    pub fn require_tracks(&self) -> Result<()> {
        if self.tracks.is_empty() {
            return Err(anyhow!(
                "sync produced 0 tracks — check that the source dir contains \
                 playable .flac files and that ffprobe/metaflac are installed, \
                 then run `jukebox sync` again"
            ));
        }
        Ok(())
    }
}

impl Track {
    /// Resolve the absolute source path. `source_path` in the catalog is relative
    /// to the parent of `source_root` (e.g. `lossless/...` under `~/Music`).
    pub fn resolve_source(&self, source_root: &Path) -> PathBuf {
        match source_root.parent() {
            Some(parent) => parent.join(&self.source_path),
            None => self.source_path.clone(),
        }
    }

    /// Format the audio quality as `{bitDepth}bit-{sampleRateKHz}kHz`,
    /// e.g. `24bit-48kHz` or `16bit-44.1kHz`.
    pub fn quality_label(&self) -> String {
        let khz = if self.sample_rate_hz.is_multiple_of(1000) {
            format!("{}kHz", self.sample_rate_hz / 1000)
        } else {
            format!("{:.1}kHz", self.sample_rate_hz as f64 / 1000.0)
        };
        format!("{}bit-{}", self.bit_depth, khz)
    }
}
