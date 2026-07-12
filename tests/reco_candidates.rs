//! Integration tests for candidate generation (reco::candidates).
//!
//! Verifies that each candidate generator produces candidates with the correct
//! provenance (CandidateSource), that candidates are deduped across sources,
//! hidden tracks and blocked artists are excluded, and an empty profile
//! produces no candidates.

use jukebox::catalog::Track;
use jukebox::reco::candidates::{CandidateGenerator, CandidateSource};
use jukebox::reco::events::ListenEvent;
use jukebox::reco::profile::UserProfile;
use std::path::PathBuf;

/// Build a local Track with the real struct fields used across the codebase.
fn make_track(id: &str, artist: &str, title: &str) -> Track {
    Track {
        id: id.to_string(),
        artists: vec![artist.to_string()],
        primary_artist: artist.to_string(),
        title: title.to_string(),
        album: Some("Album".to_string()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path: PathBuf::from("/test/file.flac"),
        symlinked_into_artists: vec![],
    }
}

fn make_profile_with_events(events: Vec<ListenEvent>) -> UserProfile {
    UserProfile::build_from_events(&events)
}

#[test]
fn liked_tracks_generate_candidates() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 200,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(candidates.iter().any(|c| c.track_id == "t1"));
    assert!(candidates
        .iter()
        .any(|c| c.source == CandidateSource::Liked));
}

#[test]
fn completion_tracks_generate_candidates() {
    let events = vec![
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 200,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(candidates
        .iter()
        .any(|c| c.source == CandidateSource::Completion));
}

#[test]
fn hidden_tracks_excluded() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Hidden {
            track_id: "t1".into(),
            timestamp: 200,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(!candidates.iter().any(|c| c.track_id == "t1"));
}

#[test]
fn blocked_artist_excluded() {
    let events = vec![
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::ArtistBlocked {
            artist: "Artist A".into(),
            timestamp: 200,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(!candidates.iter().any(|c| c.track_id == "t1"));
}

#[test]
fn candidates_deduped() {
    // t1 is liked, completed, and replayed — three sources could claim it.
    // The generator dedups by track_id, keeping the first (strongest) source.
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 200,
        },
        ListenEvent::Replayed {
            track_id: "t1".into(),
            timestamp: 300,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    let t1_count = candidates.iter().filter(|c| c.track_id == "t1").count();
    assert_eq!(t1_count, 1, "track should appear only once (deduped)");
}

#[test]
fn artist_affinity_generates() {
    // Two completed tracks by Artist A → Artist A has high affinity.
    // A third track by Artist A (never played) should be generated via
    // ArtistAffinity or LocalMetadata source.
    let events = vec![
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Completed {
            track_id: "t2".into(),
            timestamp: 200,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![
        make_track("t1", "Artist A", "Song A"),
        make_track("t2", "Artist A", "Song B"),
        make_track("t3", "Artist A", "Song C"),
    ];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(candidates.iter().any(|c| c.track_id == "t3"));
}

#[test]
fn empty_profile_no_candidates() {
    let profile = UserProfile::new();
    let catalog = vec![
        make_track("t1", "Artist A", "Song A"),
        make_track("t2", "Artist B", "Song B"),
    ];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(candidates.is_empty());
}

#[test]
fn candidate_carries_provenance_and_affinity() {
    let events = vec![ListenEvent::Liked {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    let c = candidates.iter().find(|c| c.track_id == "t1").unwrap();
    assert_eq!(c.source, CandidateSource::Liked);
    assert!(c.affinity > 0.0);
    assert!(c.is_local);
}

#[test]
fn candidate_source_description_is_human_readable() {
    assert!(CandidateSource::Liked.description().contains("liked"));
    assert!(CandidateSource::Rediscovery
        .description()
        .contains("used to love"));
    assert!(CandidateSource::ArtistAffinity
        .description()
        .contains("artist"));
}

#[test]
fn rediscovery_generates_for_old_positive_tracks() {
    let now = ListenEvent::now();
    let old_timestamp = now.saturating_sub(60u64 * 24 * 60 * 60);
    // Use Replayed so from_liked/from_completion don't catch it first.
    // Track "t1" is NOT in the catalog, so from_existing_playlists and
    // from_local_metadata won't catch it. from_rediscovery will.
    let events = vec![
        ListenEvent::Replayed {
            track_id: "t1".into(),
            timestamp: old_timestamp,
        },
        ListenEvent::Replayed {
            track_id: "t1".into(),
            timestamp: old_timestamp + 100,
        },
    ];
    let profile = make_profile_with_events(events);
    let catalog = vec![
        make_track("other1", "Artist B", "Song B"),
        make_track("other2", "Artist C", "Song C"),
    ];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(
        candidates
            .iter()
            .any(|c| c.source == CandidateSource::Rediscovery),
        "old positive track should generate rediscovery candidate"
    );
}

#[test]
fn track_started_alone_does_not_generate() {
    let events = vec![ListenEvent::TrackStarted {
        track_id: "t1".into(),
        source: jukebox::reco::events::EventSource::Local,
        timestamp: 100,
        context: jukebox::reco::events::EventContext::Album,
    }];
    let profile = make_profile_with_events(events);
    let catalog = vec![make_track("t1", "Artist A", "Song A")];
    let gen = CandidateGenerator::new(&profile, &catalog);
    let candidates = gen.generate();
    assert!(candidates.is_empty());
}
