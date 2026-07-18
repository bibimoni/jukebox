//! Integration tests for diversity control (reco::diversity).
//!
//! Verifies apply_diversity enforces the artist gap, check_violations detects
//! artist-gap violations, check_violations returns None when clean, and
//! strict vs relaxed configs have different gap sizes.

use jukebox::catalog::Track;
use jukebox::reco::candidates::{Candidate, CandidateSource};
use jukebox::reco::diversity::{
    apply_diversity, check_violations, DiversityConfig, DiversityViolation,
};
use std::path::PathBuf;

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
        source_path: PathBuf::from("/test/file.flac"),
        symlinked_into_artists: vec![],
    }
}

fn make_candidate(id: &str, affinity: f64) -> Candidate {
    Candidate::new(id.into(), CandidateSource::Completion, affinity, true)
}

#[test]
fn apply_diversity_enforces_artist_gap() {
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist A", "Album 1", "Song 2"),
        make_track("t3", "Artist B", "Album 2", "Song 3"),
    ];
    let candidates = vec![
        make_candidate("t1", 5.0),
        make_candidate("t2", 4.0),
        make_candidate("t3", 3.0),
    ];
    let config = DiversityConfig {
        artist_gap: 2,
        album_gap: 10,
        playlist_gap: 10,
        dedup_canonical: false,
    };
    let result = apply_diversity(&candidates, &catalog, &config, &[]);
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
    let candidates = vec![make_candidate("t1", 5.0), make_candidate("t2", 4.0)];
    let config = DiversityConfig {
        artist_gap: 5,
        album_gap: 10,
        playlist_gap: 10,
        dedup_canonical: false,
    };
    let violation = check_violations(&candidates, &catalog, &config);
    assert!(matches!(
        violation,
        Some(DiversityViolation::ArtistGap {
            artist,
            gap: 1,
            required: 5,
            ..
        }) if artist == "Artist A"
    ));
}

#[test]
fn check_violations_none_when_clean() {
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
    ];
    let candidates = vec![make_candidate("t1", 5.0), make_candidate("t2", 4.0)];
    let config = DiversityConfig::default();
    assert!(check_violations(&candidates, &catalog, &config).is_none());
}

#[test]
fn strict_config_has_larger_gaps_than_default() {
    let strict = DiversityConfig::strict();
    let default = DiversityConfig::default();
    assert!(strict.artist_gap > default.artist_gap);
    assert!(strict.album_gap > default.album_gap);
}

#[test]
fn relaxed_config_has_smaller_gaps_than_default() {
    let relaxed = DiversityConfig::relaxed();
    let default = DiversityConfig::default();
    assert!(relaxed.artist_gap < default.artist_gap);
    assert!(relaxed.album_gap < default.album_gap);
}

#[test]
fn default_config_has_canonical_dedup() {
    let config = DiversityConfig::default();
    assert!(config.dedup_canonical);
}

#[test]
fn check_violations_detects_album_gap() {
    let catalog = vec![
        make_track("t1", "Artist A", "Same Album", "Song 1"),
        make_track("t2", "Artist B", "Same Album", "Song 2"),
    ];
    let candidates = vec![make_candidate("t1", 5.0), make_candidate("t2", 4.0)];
    let config = DiversityConfig {
        artist_gap: 10,
        album_gap: 5,
        playlist_gap: 10,
        dedup_canonical: false,
    };
    let violation = check_violations(&candidates, &catalog, &config);
    assert!(matches!(
        violation,
        Some(DiversityViolation::AlbumGap {
            album,
            gap: 1,
            required: 5,
            ..
        }) if album == "Same Album"
    ));
}

#[test]
fn apply_diversity_with_empty_candidates_returns_empty() {
    let catalog: Vec<Track> = vec![];
    let candidates: Vec<Candidate> = vec![];
    let config = DiversityConfig::default();
    let result = apply_diversity(&candidates, &catalog, &config, &[]);
    assert!(result.is_empty());
}

#[test]
fn apply_diversity_preserves_all_candidates() {
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
        make_track("t3", "Artist C", "Album 3", "Song 3"),
    ];
    let candidates = vec![
        make_candidate("t1", 5.0),
        make_candidate("t2", 4.0),
        make_candidate("t3", 3.0),
    ];
    let config = DiversityConfig::default();
    let result = apply_diversity(&candidates, &catalog, &config, &[]);
    assert_eq!(result.len(), candidates.len());
}
