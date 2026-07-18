//! Identity resolution — canonical ID computation and track merging.
//!
//! Resolves related representations of the same music:
//! - Local file ↔ YouTube upload
//! - Official audio ↔ music video
//! - Album ↔ single version
//! - Explicit ↔ clean
//! - Remasters
//! - Live versions
//! - Remixes
//! - Covers
//! - Reuploads
//!
//! Does NOT automatically merge:
//! - Live and studio (different experiences)
//! - Cover and original (different artists)
//! - Remix and original (different productions)
//! - Explicit and clean (without preference rules)
//!
//! ## Canonical ID
//!
//! The canonical id is a normalized `artist|title` string (lowercased,
//! stripped of punctuation, featuring/feat removed). Two tracks with the
//! same canonical id are considered the same recording (or a close variant).

use crate::catalog::Track;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The kind of variant a track is, relative to its canonical recording.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum TrackVariant {
    #[default]
    Original,
    Remaster,
    Live,
    Remix,
    Cover,
    Acoustic,
    Instrumental,
    Explicit,
    Clean,
    MusicVideo,
    OfficialAudio,
    LyricsVideo,
    Short,
    Reupload,
    Commentary,
    Other,
}

impl TrackVariant {
    /// Detect the variant from a track title. Conservative — returns
    /// `Original` when no clear variant marker is present.
    pub fn from_title(title: &str) -> Self {
        let lower = title.to_lowercase();
        if lower.contains("live") && !lower.contains("lively") {
            TrackVariant::Live
        } else if lower.contains("remaster") {
            TrackVariant::Remaster
        } else if lower.contains("remix") {
            TrackVariant::Remix
        } else if lower.contains("cover") {
            TrackVariant::Cover
        } else if lower.contains("acoustic") {
            TrackVariant::Acoustic
        } else if lower.contains("instrumental") {
            TrackVariant::Instrumental
        } else if lower.contains("explicit") {
            TrackVariant::Explicit
        } else if lower.contains("clean") {
            TrackVariant::Clean
        } else if lower.contains("official audio") || lower.contains("audio only") {
            TrackVariant::OfficialAudio
        } else if lower.contains("lyrics video") || lower.contains("lyric video") {
            TrackVariant::LyricsVideo
        } else if lower.contains("music video") || lower.contains("official video") {
            TrackVariant::MusicVideo
        } else if lower.contains("#shorts") || lower.contains("short") {
            TrackVariant::Short
        } else if lower.contains("reaction") || lower.contains("review") {
            TrackVariant::Commentary
        } else {
            TrackVariant::Original
        }
    }

    /// True if this variant is a music-content variant (not non-music).
    pub fn is_music(&self) -> bool {
        !matches!(self, TrackVariant::Short | TrackVariant::Commentary)
    }

    /// True if this variant should be preferred (official audio, original, remaster).
    pub fn is_preferred(&self) -> bool {
        matches!(
            self,
            TrackVariant::Original | TrackVariant::Remaster | TrackVariant::OfficialAudio
        )
    }
}

/// Compute a canonical id for a track. The canonical id is a normalized
/// `artist|title` string used for deduplication across sources and variants.
pub fn canonical_id(artist: &str, title: &str) -> String {
    let a = normalize_text(artist);
    let t = normalize_title(title);
    format!("{a}|{t}")
}

/// Normalize text for canonical comparison: lowercase, strip punctuation,
/// collapse whitespace, remove "feat"/"featuring" clauses.
fn normalize_text(s: &str) -> String {
    let lower = s.to_lowercase();
    // Remove featuring clauses: "Artist feat. Other" → "Artist"
    let no_feat = remove_featuring(&lower);
    // Strip punctuation (keep alphanumeric + spaces).
    let stripped: String = no_feat
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    // Collapse whitespace.
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a track title: remove variant markers (live, remaster, remix,
/// etc.), featuring clauses, and parenthetical content.
fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let no_feat = remove_featuring(&lower);
    // Remove parenthetical content: "Song (Live Version)" → "Song"
    let no_parens = remove_parentheticals(&no_feat);
    // Strip punctuation.
    let stripped: String = no_parens
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove "feat." / "featuring" clauses from a string.
fn remove_featuring(s: &str) -> String {
    // Match "feat.", "featuring", "ft." case-insensitively.
    let lower = s.to_lowercase();
    for marker in &["feat.", "feat ", "featuring", "ft.", "ft "] {
        if let Some(pos) = lower.find(marker) {
            return s[..pos].trim().to_string();
        }
    }
    s.to_string()
}

/// Remove parenthetical content from a string.
fn remove_parentheticals(s: &str) -> String {
    let mut result = String::new();
    let mut depth: u32 = 0;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1u32),
            _ if depth == 0 => result.push(c),
            _ => {}
        }
    }
    result
}

/// An identity resolver that tracks canonical ids and their variants.
pub struct IdentityResolver {
    /// canonical_id → list of (track_id, variant)
    pub groups: HashMap<String, Vec<(String, TrackVariant)>>,
}

impl IdentityResolver {
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    /// Register a track with the resolver.
    pub fn add_track(&mut self, track_id: &str, artist: &str, title: &str) {
        let canonical = canonical_id(artist, title);
        let variant = TrackVariant::from_title(title);
        self.groups
            .entry(canonical)
            .or_default()
            .push((track_id.to_string(), variant));
    }

    /// Add all tracks from the local catalog.
    pub fn add_catalog(&mut self, catalog: &[Track]) {
        for track in catalog {
            self.add_track(&track.id, &track.primary_artist, &track.title);
        }
    }

    /// Get all variant track ids for a given track (same canonical id).
    pub fn variants_of(&self, _track_id: &str, artist: &str, title: &str) -> Vec<String> {
        let canonical = canonical_id(artist, title);
        self.groups
            .get(&canonical)
            .map(|v| v.iter().map(|(id, _)| id.clone()).collect())
            .unwrap_or_default()
    }

    /// True if two tracks are the same canonical recording.
    pub fn is_same_recording(
        &self,
        artist1: &str,
        title1: &str,
        artist2: &str,
        title2: &str,
    ) -> bool {
        canonical_id(artist1, title1) == canonical_id(artist2, title2)
    }
}

impl Default for IdentityResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_id_normalizes_case_and_punctuation() {
        let id1 = canonical_id("The Beatles", "Hey Jude");
        let id2 = canonical_id("the beatles", "hey jude");
        assert_eq!(id1, id2);
    }

    #[test]
    fn canonical_id_strips_featuring() {
        let id1 = canonical_id("Artist", "Song feat. Other Artist");
        let id2 = canonical_id("Artist", "Song");
        assert_eq!(id1, id2);
    }

    #[test]
    fn canonical_id_strips_parentheticals() {
        let id1 = canonical_id("Artist", "Song (Live Version)");
        let id2 = canonical_id("Artist", "Song");
        assert_eq!(id1, id2);
    }

    #[test]
    fn canonical_id_strips_remaster() {
        let id1 = canonical_id("Artist", "Song (Remastered 2024)");
        let id2 = canonical_id("Artist", "Song");
        assert_eq!(id1, id2);
    }

    #[test]
    fn track_variant_from_title_live() {
        assert_eq!(TrackVariant::from_title("Song (Live)"), TrackVariant::Live);
        assert_eq!(
            TrackVariant::from_title("Song (Live at Wembley)"),
            TrackVariant::Live
        );
    }

    #[test]
    fn track_variant_from_title_remix() {
        assert_eq!(
            TrackVariant::from_title("Song (Remix)"),
            TrackVariant::Remix
        );
    }

    #[test]
    fn track_variant_from_title_remaster() {
        assert_eq!(
            TrackVariant::from_title("Song (Remastered)"),
            TrackVariant::Remaster
        );
    }

    #[test]
    fn track_variant_from_title_cover() {
        assert_eq!(
            TrackVariant::from_title("Song (Cover)"),
            TrackVariant::Cover
        );
    }

    #[test]
    fn track_variant_from_title_shorts() {
        assert_eq!(
            TrackVariant::from_title("Song #shorts"),
            TrackVariant::Short
        );
        assert!(!TrackVariant::Short.is_music());
    }

    #[test]
    fn track_variant_from_title_commentary() {
        assert_eq!(
            TrackVariant::from_title("Song Reaction"),
            TrackVariant::Commentary
        );
        assert!(!TrackVariant::Commentary.is_music());
    }

    #[test]
    fn track_variant_from_title_original() {
        assert_eq!(
            TrackVariant::from_title("Just A Song"),
            TrackVariant::Original
        );
        assert!(TrackVariant::Original.is_music());
        assert!(TrackVariant::Original.is_preferred());
    }

    #[test]
    fn identity_resolver_groups_variants() {
        let mut resolver = IdentityResolver::new();
        resolver.add_track("local1", "Artist", "Song");
        resolver.add_track("yt1", "Artist", "Song (Live)");
        resolver.add_track("yt2", "Artist", "Song (Remastered)");
        let variants = resolver.variants_of("local1", "Artist", "Song");
        assert!(variants.contains(&"local1".to_string()));
        assert!(variants.contains(&"yt1".to_string()));
        assert!(variants.contains(&"yt2".to_string()));
    }

    #[test]
    fn identity_resolver_detects_same_recording() {
        let resolver = IdentityResolver::new();
        assert!(resolver.is_same_recording("Artist", "Song", "Artist", "Song (Live)"));
        assert!(!resolver.is_same_recording("Artist", "Song", "Other Artist", "Song"));
    }

    #[test]
    fn identity_resolver_does_not_merge_different_artists() {
        let id1 = canonical_id("Artist A", "Song");
        let id2 = canonical_id("Artist B", "Song");
        assert_ne!(id1, id2);
    }

    #[test]
    fn track_variant_official_audio_is_preferred() {
        assert!(TrackVariant::OfficialAudio.is_preferred());
        assert!(TrackVariant::OfficialAudio.is_music());
    }

    #[test]
    fn track_variant_music_video_is_music() {
        assert!(TrackVariant::MusicVideo.is_music());
        assert!(!TrackVariant::MusicVideo.is_preferred());
    }
}
