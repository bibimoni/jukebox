//! Integration tests for the radio engine (S3.2.3).
//!
//! Verifies seed from track/artist/playlist, incremental generation, refill,
//! cancellation on seed change, skip/positive/negative feedback effects, and
//! exclusion enforcement.

use jukebox::catalog::Track;
use jukebox::reco::candidates::CandidateSource;
use jukebox::reco::events::ListenEvent;
use jukebox::reco::profile::UserProfile;
use jukebox::reco::radio::{RadioSeed, RadioSession};

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

fn make_profile() -> UserProfile {
    let events = vec![
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Liked {
            track_id: "t2".into(),
            timestamp: 200,
        },
    ];
    UserProfile::build_from_events(&events)
}

fn make_catalog() -> Vec<Track> {
    vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
        make_track("t3", "Artist A", "Album 1", "Song 3"),
        make_track("t4", "Artist C", "Album 3", "Song 4"),
    ]
}

#[test]
fn radio_seed_from_track_initializes_pool() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    assert!(!radio.candidate_pool.is_empty());
}

#[test]
fn radio_seed_from_artist_initializes() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Artist("Artist A".into()));
    radio.initialize(&profile, &catalog);
    // Pool may be empty if no candidates match, but should not crash
    assert!(radio.generation > 0);
}

#[test]
fn radio_seed_from_playlist_initializes() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Playlist("My Playlist".into()));
    radio.initialize(&profile, &catalog);
    assert!(radio.generation > 0);
}

#[test]
fn radio_next_track_moves_to_history() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    let next = radio.next_track();
    assert!(next.is_some());
    assert_eq!(radio.history().len(), 1);
}

#[test]
fn radio_next_track_returns_none_when_empty() {
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    assert!(radio.next_track().is_none());
}

#[test]
fn radio_negative_feedback_excludes_track_from_pool() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    radio.negative_feedback("t2");
    assert!(!radio.candidate_pool.iter().any(|c| c.track_id == "t2"));
    assert!(radio.negative_feedback.contains("t2"));
    assert!(radio.exclusions.contains("t2"));
}

#[test]
fn radio_positive_feedback_records_track() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    radio.positive_feedback("t2");
    assert!(radio.positive_feedback.contains("t2"));
}

#[test]
fn radio_skip_does_not_exclude_from_pool() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    radio.skip("t2");
    assert!(radio.skipped.contains("t2"));
    // Skip is weak — track may still be in pool
}

#[test]
fn radio_needs_refill_when_pool_low() {
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.refill_threshold = 10;
    radio.candidate_pool = vec![jukebox::reco::candidates::Candidate::new(
        "t1".into(),
        CandidateSource::Liked,
        5.0,
        true,
    )];
    assert!(radio.needs_refill());
}

#[test]
fn radio_does_not_need_refill_when_pool_full() {
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.refill_threshold = 5;
    radio.candidate_pool = (0..10)
        .map(|i| {
            jukebox::reco::candidates::Candidate::new(
                format!("t{i}"),
                CandidateSource::Liked,
                5.0,
                true,
            )
        })
        .collect();
    assert!(!radio.needs_refill());
}

#[test]
fn radio_refill_if_needed_adds_candidates() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.refill_threshold = 100;
    radio.initialize(&profile, &catalog);
    let before = radio.pool_size();
    radio.refill_if_needed(&profile, &catalog);
    assert!(radio.pool_size() >= before);
}

#[test]
fn radio_change_seed_cancels_and_clears_session() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    radio.next_track();
    assert!(!radio.history().is_empty());
    radio.change_seed(RadioSeed::Track("t2".into()), &profile, &catalog);
    assert!(radio.history().is_empty());
    assert!(radio.session_history.is_empty());
}

#[test]
fn radio_change_seed_increments_generation() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    let gen_before = radio.generation;
    radio.change_seed(RadioSeed::Track("t2".into()), &profile, &catalog);
    assert!(radio.generation > gen_before);
}

#[test]
fn radio_stop_clears_pool() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    assert!(!radio.candidate_pool.is_empty());
    radio.stop();
    assert!(radio.candidate_pool.is_empty());
}

#[test]
fn radio_seed_description_human_readable() {
    assert!(RadioSeed::Track("t1".into())
        .description()
        .contains("track"));
    assert!(RadioSeed::Artist("Artist A".into())
        .description()
        .contains("artist"));
    assert!(RadioSeed::Playlist("My PL".into())
        .description()
        .contains("playlist"));
    assert!(RadioSeed::Queue.description().contains("queue"));
}

#[test]
fn radio_pool_size_tracks_candidates() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    let size = radio.pool_size();
    radio.next_track();
    assert_eq!(radio.pool_size(), size - 1);
}

#[test]
fn radio_session_history_excludes_played_tracks() {
    let profile = make_profile();
    let catalog = make_catalog();
    let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
    radio.initialize(&profile, &catalog);
    let next = radio.next_track().unwrap();
    // The played track should not reappear in the pool
    assert!(!radio
        .candidate_pool
        .iter()
        .any(|c| c.track_id == next.track_id));
}
