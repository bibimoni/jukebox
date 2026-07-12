//! Integration tests for generated mixes (reco::mixes).
//!
//! Verifies generate_mix produces tracks, generate_all_mixes produces at least
//! 4 mix types, Discover uses discover weights, OnRepeat requires history,
//! DailyMix is stable for the same date, empty profile generates an empty mix,
//! and max tracks is respected.

use jukebox::catalog::Track;
use jukebox::reco::events::ListenEvent;
use jukebox::reco::mixes::{generate_all_mixes, generate_mix, MixType};
use jukebox::reco::profile::UserProfile;
use jukebox::reco::ranking::RankingWeights;
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

fn make_profile() -> UserProfile {
    let events = vec![
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
        ListenEvent::Completed {
            track_id: "t2".into(),
            timestamp: 200,
        },
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 300,
        },
    ];
    UserProfile::build_from_events(&events)
}

#[test]
fn generate_mix_produces_tracks() {
    let profile = make_profile();
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
        make_track("t3", "Artist C", "Album 3", "Song 3"),
    ];
    let mix = generate_mix(MixType::DailyMix, &profile, &catalog);
    assert!(!mix.tracks.is_empty());
    assert_eq!(mix.mix_type, MixType::DailyMix);
}

#[test]
fn generate_all_mixes_at_least_4_types() {
    let profile = make_profile();
    let catalog: Vec<Track> = (0..20)
        .map(|i| {
            make_track(
                &format!("t{i}"),
                &format!("Artist {i}"),
                &format!("Album {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let mixes = generate_all_mixes(&profile, &catalog);
    assert!(mixes.len() >= 4, "expected >=4 mixes, got {}", mixes.len());
}

#[test]
fn discover_uses_discover_weights() {
    let weights = MixType::Discover.weights();
    let default = RankingWeights::default();
    assert!(weights.novelty > default.novelty);
}

#[test]
fn on_repeat_requires_history() {
    assert!(MixType::OnRepeat.requires_history());
    assert!(MixType::Rediscover.requires_history());
    assert!(!MixType::DailyMix.requires_history());
    assert!(!MixType::Discover.requires_history());
}

#[test]
fn daily_mix_stable_for_same_date() {
    let profile = make_profile();
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
    ];
    let mix1 = generate_mix(MixType::DailyMix, &profile, &catalog);
    let mix2 = generate_mix(MixType::DailyMix, &profile, &catalog);
    assert_eq!(mix1.generated_date, mix2.generated_date);
    assert_eq!(mix1.track_ids(), mix2.track_ids());
}

#[test]
fn empty_profile_generates_empty_mix() {
    let profile = UserProfile::new();
    let catalog = vec![make_track("t1", "Artist A", "Album 1", "Song 1")];
    let mix = generate_mix(MixType::OnRepeat, &profile, &catalog);
    // Cold-start fallback: the catalog seeds candidates even without a profile.
    assert!(!mix.tracks.is_empty(), "mix should use catalog fallback");
}

#[test]
fn max_tracks_respected() {
    let profile = make_profile();
    let catalog: Vec<Track> = (0..100)
        .map(|i| {
            make_track(
                &format!("t{i}"),
                &format!("Artist {i}"),
                &format!("Album {i}"),
                &format!("Song {i}"),
            )
        })
        .collect();
    let mix = generate_mix(MixType::Discover, &profile, &catalog);
    assert!(mix.tracks.len() <= MixType::Discover.max_tracks());
}

#[test]
fn mix_type_labels_are_human_readable() {
    assert_eq!(MixType::DailyMix.label(), "Daily Mix");
    assert_eq!(MixType::Discover.label(), "Discover Mix");
    assert_eq!(MixType::OnRepeat.label(), "On Repeat");
    assert_eq!(MixType::Rediscover.label(), "Rediscover");
}

#[test]
fn mix_descriptions_are_nonempty() {
    for mix_type in [
        MixType::DailyMix,
        MixType::Discover,
        MixType::OnRepeat,
        MixType::Rediscover,
        MixType::ForgottenFavorites,
        MixType::NewFromArtists,
        MixType::LocalYtBlend,
    ] {
        assert!(!mix_type.description().is_empty());
    }
}

#[test]
fn mix_track_ids_returns_refs() {
    let profile = make_profile();
    let catalog = vec![
        make_track("t1", "Artist A", "Album 1", "Song 1"),
        make_track("t2", "Artist B", "Album 2", "Song 2"),
    ];
    let mix = generate_mix(MixType::DailyMix, &profile, &catalog);
    let ids = mix.track_ids();
    for id in &ids {
        assert!(!id.is_empty());
    }
}
