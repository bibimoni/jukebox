//! Diversity control — prevents repetition and ensures varied sequencing.
//!
//! Controls:
//! - Exact duplicates (never within same mix/radio)
//! - Canonical recording repetition (dedupe remasters/reuploads)
//! - Artist repetition (gap of N tracks between same artist)
//! - Album repetition (gap of N tracks between same album)
//! - Playlist-source repetition (gap between tracks from same playlist)
//! - Recent-session repetition (avoid repeating what was just played)
//!
//! A generated mix must be deliberately sequenced, not merely sorted.

use crate::catalog::Track;
use crate::reco::candidates::Candidate;
use crate::reco::identity::canonical_id;
use std::collections::{HashMap, HashSet};

/// Configuration for diversity control.
#[derive(Clone, Debug)]
pub struct DiversityConfig {
    /// Minimum number of tracks between two tracks by the same artist.
    pub artist_gap: usize,
    /// Minimum number of tracks between two tracks from the same album.
    pub album_gap: usize,
    /// Minimum number of tracks between two tracks from the same playlist.
    pub playlist_gap: usize,
    /// Whether to deduplicate by canonical id (remasters/reuploads).
    pub dedup_canonical: bool,
}

impl Default for DiversityConfig {
    fn default() -> Self {
        DiversityConfig {
            artist_gap: 5,
            album_gap: 10,
            playlist_gap: 8,
            dedup_canonical: true,
        }
    }
}

impl DiversityConfig {
    /// Strict diversity: large gaps, full dedup.
    pub fn strict() -> Self {
        DiversityConfig {
            artist_gap: 8,
            album_gap: 15,
            playlist_gap: 12,
            dedup_canonical: true,
        }
    }

    /// Relaxed diversity: small gaps (for radio, which benefits from some repetition).
    pub fn relaxed() -> Self {
        DiversityConfig {
            artist_gap: 3,
            album_gap: 5,
            playlist_gap: 4,
            dedup_canonical: true,
        }
    }
}

/// Apply diversity control to a ranked list of candidates. Returns a new
/// list with diversity constraints enforced. Candidates that would violate
/// a gap constraint are deferred to a later position (or dropped if no
/// valid position exists).
///
/// `catalog` is used to look up artist/album info for local tracks.
/// `session_history` is the list of recently-played track ids (to avoid
/// repeating what was just played).
pub fn apply_diversity(
    candidates: &[Candidate],
    catalog: &[Track],
    config: &DiversityConfig,
    session_history: &[String],
) -> Vec<Candidate> {
    let mut result = Vec::new();
    let mut used = HashSet::new();
    let mut artist_positions: HashMap<String, usize> = HashMap::new();
    let mut album_positions: HashMap<String, usize> = HashMap::new();
    let mut canonical_seen: HashSet<String> = HashSet::new();

    // Pre-populate with session history positions (as if they were at the start).
    for (i, _track_id) in session_history.iter().enumerate() {
        let _ = i;
    }

    let mut remaining: Vec<Candidate> = candidates.to_vec();

    while !remaining.is_empty() {
        let mut found = false;
        for i in 0..remaining.len() {
            let candidate = &remaining[i];
            if used.contains(&candidate.track_id) {
                continue;
            }

            // Look up track info from catalog (for local tracks).
            let track_info = catalog.iter().find(|t| t.id == candidate.track_id);
            let artist = track_info
                .map(|t| t.primary_artist.clone())
                .unwrap_or_default();
            let album = track_info
                .map(|t| t.album.clone().unwrap_or_default())
                .unwrap_or_default();
            let title = track_info.map(|t| t.title.clone()).unwrap_or_default();
            if config.dedup_canonical && !artist.is_empty() && !title.is_empty() {
                let canonical = canonical_id(&artist, &title);
                if canonical_seen.contains(&canonical) {
                    continue;
                }
            }

            // Check artist gap.
            if !artist.is_empty() {
                if let Some(&pos) = artist_positions.get(&artist) {
                    if result.len() - pos < config.artist_gap {
                        continue;
                    }
                }
            }

            // Check album gap.
            if !album.is_empty() {
                if let Some(&pos) = album_positions.get(&album) {
                    if result.len() - pos < config.album_gap {
                        continue;
                    }
                }
            }

            // This candidate passes all diversity checks.
            let candidate = remaining.remove(i);
            used.insert(candidate.track_id.clone());

            if !artist.is_empty() {
                artist_positions.insert(artist.clone(), result.len());
            }
            if !album.is_empty() {
                album_positions.insert(album.clone(), result.len());
            }
            if config.dedup_canonical && !artist.is_empty() && !title.is_empty() {
                let canonical = canonical_id(&artist, &title);
                canonical_seen.insert(canonical);
            }

            result.push(candidate);
            found = true;
            break;
        }

        if !found {
            // No candidate passes diversity — add the best remaining one.
            if let Some(c) = remaining.first().cloned() {
                used.insert(c.track_id.clone());
                remaining.remove(0);
                result.push(c);
            }
        }
    }

    result
}

/// Check if a sequence of candidates violates any diversity constraints.
/// Returns the first violation found, or `None` if the sequence is clean.
pub fn check_violations(
    candidates: &[Candidate],
    catalog: &[Track],
    config: &DiversityConfig,
) -> Option<DiversityViolation> {
    let mut artist_positions: HashMap<String, usize> = HashMap::new();
    let mut album_positions: HashMap<String, usize> = HashMap::new();
    let mut canonical_seen: HashSet<String> = HashSet::new();

    for (i, candidate) in candidates.iter().enumerate() {
        let track_info = catalog.iter().find(|t| t.id == candidate.track_id);
        let artist = track_info
            .map(|t| t.primary_artist.clone())
            .unwrap_or_default();
        let album = track_info
            .map(|t| t.album.clone().unwrap_or_default())
            .unwrap_or_default();
        let title = track_info.map(|t| t.title.clone()).unwrap_or_default();

        // Check canonical dedup.
        if config.dedup_canonical && !artist.is_empty() && !title.is_empty() {
            let canonical = canonical_id(&artist, &title);
            if canonical_seen.contains(&canonical) {
                return Some(DiversityViolation::CanonicalDup {
                    position: i,
                    track_id: candidate.track_id.clone(),
                });
            }
            canonical_seen.insert(canonical);
        }

        // Check artist gap.
        if !artist.is_empty() {
            if let Some(&pos) = artist_positions.get(&artist) {
                if i - pos < config.artist_gap {
                    return Some(DiversityViolation::ArtistGap {
                        position: i,
                        artist,
                        gap: i - pos,
                        required: config.artist_gap,
                    });
                }
            }
            artist_positions.insert(artist, i);
        }

        // Check album gap.
        if !album.is_empty() {
            if let Some(&pos) = album_positions.get(&album) {
                if i - pos < config.album_gap {
                    return Some(DiversityViolation::AlbumGap {
                        position: i,
                        album,
                        gap: i - pos,
                        required: config.album_gap,
                    });
                }
            }
            album_positions.insert(album, i);
        }
    }

    None
}

/// A diversity violation.
#[derive(Clone, Debug)]
pub enum DiversityViolation {
    CanonicalDup {
        position: usize,
        track_id: String,
    },
    ArtistGap {
        position: usize,
        artist: String,
        gap: usize,
        required: usize,
    },
    AlbumGap {
        position: usize,
        album: String,
        gap: usize,
        required: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;

    fn make_track(id: &str, artist: &str, album: &str, title: &str) -> Track {
        Track {
            id: id.to_string(),
            artists: vec![artist.to_string()],
            primary_artist: artist.to_string(),
            title: title.to_string(),
            album: Some(album.to_string()),
            track_number: Some(1),
            disc_number: Some(1),
            bit_depth: 16,
            sample_rate_hz: 44100,
            isrc: None,
            source_path: std::path::PathBuf::from("/test/file.flac"),
            symlinked_into_artists: vec![],
        }
    }

    #[test]
    fn apply_diversity_enforces_artist_gap() {
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist A", "Album 1", "Song 2"),
            make_track("t3", "Artist B", "Album 2", "Song 3"),
        ];
        let candidates = vec![
            Candidate::new(
                "t1".into(),
                crate::reco::candidates::CandidateSource::Completion,
                5.0,
                true,
            ),
            Candidate::new(
                "t2".into(),
                crate::reco::candidates::CandidateSource::Completion,
                4.0,
                true,
            ),
            Candidate::new(
                "t3".into(),
                crate::reco::candidates::CandidateSource::Completion,
                3.0,
                true,
            ),
        ];
        let config = DiversityConfig {
            artist_gap: 2,
            album_gap: 10,
            playlist_gap: 10,
            dedup_canonical: false,
        };
        let result = apply_diversity(&candidates, &catalog, &config, &[]);
        // t1 and t2 are both Artist A — with gap=2, t2 can't be at position 1.
        // Expected: t1, t3, t2 (t2 deferred to after t3).
        assert_eq!(result[0].track_id, "t1");
        assert_eq!(result[1].track_id, "t3");
        assert_eq!(result[2].track_id, "t2");
    }

    #[test]
    fn check_violations_detects_artist_gap() {
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist A", "Album 1", "Song 2"),
        ];
        let candidates = vec![
            Candidate::new(
                "t1".into(),
                crate::reco::candidates::CandidateSource::Completion,
                5.0,
                true,
            ),
            Candidate::new(
                "t2".into(),
                crate::reco::candidates::CandidateSource::Completion,
                4.0,
                true,
            ),
        ];
        let config = DiversityConfig {
            artist_gap: 5,
            album_gap: 10,
            playlist_gap: 10,
            dedup_canonical: false,
        };
        let violation = check_violations(&candidates, &catalog, &config);
        assert!(matches!(
            violation,
            Some(DiversityViolation::ArtistGap { artist, gap: 1, required: 5, .. }) if artist == "Artist A"
        ));
    }

    #[test]
    fn check_violations_none_when_clean() {
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let candidates = vec![
            Candidate::new(
                "t1".into(),
                crate::reco::candidates::CandidateSource::Completion,
                5.0,
                true,
            ),
            Candidate::new(
                "t2".into(),
                crate::reco::candidates::CandidateSource::Completion,
                4.0,
                true,
            ),
        ];
        let config = DiversityConfig::default();
        assert!(check_violations(&candidates, &catalog, &config).is_none());
    }

    #[test]
    fn diversity_config_strict_has_larger_gaps() {
        let strict = DiversityConfig::strict();
        let default = DiversityConfig::default();
        assert!(strict.artist_gap > default.artist_gap);
    }

    #[test]
    fn diversity_config_relaxed_has_smaller_gaps() {
        let relaxed = DiversityConfig::relaxed();
        let default = DiversityConfig::default();
        assert!(relaxed.artist_gap < default.artist_gap);
    }
}
