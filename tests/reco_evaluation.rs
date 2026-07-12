//! Integration tests for recommendation evaluation (reco::evaluation).
//!
//! Verifies profile construction (new user, local heavy, negative feedback),
//! duplicate-rate measurement, hidden-item non-leak, and evaluate_all across
//! all deterministic profiles.
//!
//! NOTE: `build_profile` is private to the evaluation module, so these tests
//! construct profiles manually via `UserProfile::build_from_events`. The
//! public `evaluate_all` and `EvaluationMetrics::evaluate` functions are used
//! for the end-to-end checks.

use jukebox::catalog::Track;
use jukebox::reco::candidates::{Candidate, CandidateSource};
use jukebox::reco::evaluation::{evaluate_all, EvaluationMetrics, EvaluationProfile};
use jukebox::reco::events::ListenEvent;
use jukebox::reco::mixes::{generate_mix, Mix, MixType};
use jukebox::reco::profile::UserProfile;
use std::path::PathBuf;

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

#[test]
fn new_user_profile_is_empty() {
    let profile = UserProfile::new();
    assert!(profile.is_empty());
    assert!(!profile.has_history());
}

#[test]
fn local_heavy_has_history() {
    let mut events = Vec::new();
    for i in 0..50 {
        events.push(ListenEvent::Completed {
            track_id: format!("local-{i}"),
            timestamp: 1000 + i as u64 * 10,
        });
    }
    let profile = UserProfile::build_from_events(&events);
    assert!(profile.has_history());
    assert_eq!(profile.tracks.len(), 50);
}

#[test]
fn negative_feedback_has_hidden() {
    let mut events = Vec::new();
    for i in 0..20 {
        events.push(ListenEvent::Completed {
            track_id: format!("good-{i}"),
            timestamp: 1000 + i as u64,
        });
    }
    for i in 0..10 {
        events.push(ListenEvent::Hidden {
            track_id: format!("bad-{i}"),
            timestamp: 2000 + i as u64,
        });
    }
    events.push(ListenEvent::ArtistBlocked {
        artist: "Blocked Artist".into(),
        timestamp: 3000,
    });
    let profile = UserProfile::build_from_events(&events);
    assert!(!profile.hidden.is_empty());
    assert!(!profile.blocked_artists.is_empty());
}

#[test]
fn evaluate_duplicate_rate() {
    let profile = UserProfile::new();
    let catalog = vec![
        make_track("t1", "Artist A", "Song A"),
        make_track("t2", "Artist B", "Song B"),
    ];
    let mix = Mix {
        mix_type: MixType::DailyMix,
        tracks: vec![
            Candidate::new("t1".into(), CandidateSource::Liked, 1.0, true),
            Candidate::new("t1".into(), CandidateSource::Liked, 1.0, true),
        ],
        generated_date: "2026-01-01".into(),
    };
    let metrics = EvaluationMetrics::evaluate(&mix, &profile, &catalog);
    assert!(metrics.duplicate_rate > 0.0);
}

#[test]
fn hidden_items_no_leak() {
    let mut events = Vec::new();
    for i in 0..20 {
        events.push(ListenEvent::Completed {
            track_id: format!("good-{i}"),
            timestamp: 1000 + i as u64,
        });
    }
    for i in 0..10 {
        events.push(ListenEvent::Hidden {
            track_id: format!("bad-{i}"),
            timestamp: 2000 + i as u64,
        });
    }
    let profile = UserProfile::build_from_events(&events);
    let catalog: Vec<Track> = (0..30)
        .map(|i| {
            make_track(
                &format!("good-{i}"),
                &format!("Artist {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let mix = generate_mix(MixType::DailyMix, &profile, &catalog);
    let metrics = EvaluationMetrics::evaluate(&mix, &profile, &catalog);
    assert_eq!(metrics.hidden_item_violations, 0);
}

#[test]
fn evaluate_all_profiles() {
    let catalog: Vec<Track> = (0..20)
        .map(|i| {
            make_track(
                &format!("t{i}"),
                &format!("Artist {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let results = evaluate_all(&catalog);
    assert!(results.len() >= 10);
}

#[test]
fn evaluate_new_user_mix_has_low_duplicate_rate() {
    let profile = UserProfile::new();
    let catalog: Vec<Track> = (0..20)
        .map(|i| {
            make_track(
                &format!("t{i}"),
                &format!("Artist {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let mix = generate_mix(MixType::DailyMix, &profile, &catalog);
    let metrics = EvaluationMetrics::evaluate(&mix, &profile, &catalog);
    assert!(metrics.duplicate_rate < 0.1);
}

#[test]
fn evaluate_metrics_default_is_zeroed() {
    let metrics = EvaluationMetrics::default();
    assert_eq!(metrics.duplicate_rate, 0.0);
    assert_eq!(metrics.hidden_item_violations, 0);
    assert_eq!(metrics.same_artist_adjacency, 0.0);
}

#[test]
fn evaluation_profile_variants_exist() {
    let _ = EvaluationProfile::NewUser;
    let _ = EvaluationProfile::LocalHeavy;
    let _ = EvaluationProfile::YoutubeHeavy;
    let _ = EvaluationProfile::Hybrid;
    let _ = EvaluationProfile::Repetitive;
    let _ = EvaluationProfile::Explorer;
    let _ = EvaluationProfile::NegativeFeedback;
    let _ = EvaluationProfile::OfflineReturning;
    let _ = EvaluationProfile::ExpiredAuth;
    let _ = EvaluationProfile::QuotaExhaustion;
    let _ = EvaluationProfile::OverlappingCatalog;
}
