//! Integration tests for the playlist generator (reco::generator).
//!
//! Verifies natural-language parsing (energetic running, calm relaxing,
//! no live, 70% discovery, hybrid sources), the structured plan string,
//! generate producing tracks, and energy/source labels.

use jukebox::catalog::Track;
use jukebox::reco::events::ListenEvent;
use jukebox::reco::generator::{generate, Energy, GeneratorConstraints, SourcePreference};
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
fn parse_energetic_running() {
    let c =
        GeneratorConstraints::from_natural_language("Make a 45-minute energetic running playlist");
    assert_eq!(c.energy, Energy::High);
    assert!(c.duration_secs.is_some());
    assert_eq!(c.duration_secs.unwrap(), 45.0 * 60.0);
}

#[test]
fn parse_calm_relaxing() {
    let c = GeneratorConstraints::from_natural_language("calm relaxing playlist");
    assert_eq!(c.energy, Energy::Low);
}

#[test]
fn parse_no_live() {
    let c = GeneratorConstraints::from_natural_language("no live versions");
    assert!(!c.include_live);
}

#[test]
fn parse_70_percent_discovery() {
    let c = GeneratorConstraints::from_natural_language("70% discovery");
    assert!((c.familiarity - 0.3).abs() < 0.01);
}

#[test]
fn parse_hybrid_sources() {
    let c = GeneratorConstraints::from_natural_language("local and youtube hybrid playlist");
    assert_eq!(c.sources, SourcePreference::Hybrid);
}

#[test]
fn parse_local_only() {
    let c = GeneratorConstraints::from_natural_language("mostly local tracks");
    assert_eq!(c.sources, SourcePreference::Local);
}

#[test]
fn parse_youtube_only() {
    let c = GeneratorConstraints::from_natural_language("youtube playlist");
    assert_eq!(c.sources, SourcePreference::Youtube);
}

#[test]
fn constraints_to_plan_string() {
    let c = GeneratorConstraints {
        duration_secs: Some(2700.0),
        energy: Energy::High,
        sources: SourcePreference::Hybrid,
        familiarity: 0.7,
        include_live: false,
        artist_gap: 5,
        allow_explicit: true,
        max_tracks: 50,
        ..Default::default()
    };
    let plan = c.to_plan_string();
    assert!(plan.contains("Duration:"));
    assert!(plan.contains("45 minutes"));
    assert!(plan.contains("Energy:"));
    assert!(plan.contains("High"));
    assert!(plan.contains("Sources:"));
    assert!(plan.contains("Local + YouTube"));
    assert!(plan.contains("70% known"));
    assert!(plan.contains("30% discovery"));
    assert!(plan.contains("Live recordings:"));
    assert!(plan.contains("Excluded"));
    assert!(plan.contains("Artist repeat gap: 5 tracks"));
    assert!(plan.contains("Explicit content:  Allowed"));
}

#[test]
fn generate_playlist_produces_tracks() {
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
    let profile = UserProfile::build_from_events(&events);
    let catalog = vec![
        make_track("t1", "Artist A", "Song A"),
        make_track("t2", "Artist B", "Song B"),
        make_track("t3", "Artist C", "Song C"),
    ];
    let constraints = GeneratorConstraints::default();
    let playlist = generate(&constraints, &profile, &catalog);
    assert!(!playlist.tracks.is_empty());
    assert!(playlist.is_preview);
}

#[test]
fn energy_labels() {
    assert_eq!(Energy::Low.label(), "Low");
    assert_eq!(Energy::Medium.label(), "Medium");
    assert_eq!(Energy::High.label(), "High");
}

#[test]
fn source_preference_labels() {
    assert_eq!(SourcePreference::Local.label(), "Local");
    assert_eq!(SourcePreference::Youtube.label(), "YouTube");
    assert_eq!(SourcePreference::Hybrid.label(), "Local + YouTube");
}

#[test]
fn default_constraints_are_sensible() {
    let c = GeneratorConstraints::default();
    assert_eq!(c.energy, Energy::Medium);
    assert_eq!(c.sources, SourcePreference::Local);
    assert!(c.include_live);
    assert!(c.include_remixes);
    assert!(c.include_covers);
    assert!(c.allow_explicit);
    assert_eq!(c.max_tracks, 50);
    assert_eq!(c.artist_gap, 5);
}

#[test]
fn parse_no_remixes() {
    let c = GeneratorConstraints::from_natural_language("no remix versions");
    assert!(!c.include_remixes);
}

#[test]
fn generate_respects_max_tracks() {
    let events = vec![ListenEvent::Liked {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let profile = UserProfile::build_from_events(&events);
    let catalog: Vec<Track> = (0..100)
        .map(|i| {
            make_track(
                &format!("t{i}"),
                &format!("Artist {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let constraints = GeneratorConstraints {
        max_tracks: 10,
        ..Default::default()
    };
    let playlist = generate(&constraints, &profile, &catalog);
    assert!(playlist.tracks.len() <= 10);
}
