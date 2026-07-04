use anyhow::{Context, Result};
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
        let khz = if self.sample_rate_hz % 1000 == 0 {
            format!("{}kHz", self.sample_rate_hz / 1000)
        } else {
            format!("{:.1}kHz", self.sample_rate_hz as f64 / 1000.0)
        };
        format!("{}bit-{}", self.bit_depth, khz)
    }
}
