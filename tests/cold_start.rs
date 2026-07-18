//! Cold start tests — new user with no history.
use jukebox::catalog::Track;
use jukebox::reco::candidates::CandidateGenerator;
use jukebox::reco::mixes::{generate_all_mixes, generate_mix, MixType};
use jukebox::reco::profile::UserProfile;
use jukebox::reco::radio::{RadioSeed, RadioSession};
use std::path::PathBuf;

fn mk(id: &str, artist: &str) -> Track {
    Track {
        id: id.into(),
        artists: vec![artist.into()],
        primary_artist: artist.into(),
        title: id.into(),
        album: Some("Album".into()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path: PathBuf::from("/t.flac"),
        symlinked_into_artists: vec![],
    }
}

#[test]
fn cold_start_empty_profile() {
    let p = UserProfile::new();
    assert!(p.is_empty());
    assert!(!p.has_history());
}

#[test]
fn cold_start_no_candidates() {
    let p = UserProfile::new();
    let catalog = vec![mk("t1", "A")];
    let gen = CandidateGenerator::new(&p, &catalog);
    // Cold-start fallback: empty profile seeds candidates from the catalog.
    let candidates = gen.generate();
    assert!(
        !candidates.is_empty(),
        "cold start should fall back to catalog"
    );
    assert!(candidates.iter().all(|c| c.is_local));
}

#[test]
fn cold_start_mix_empty() {
    let p = UserProfile::new();
    let catalog = vec![mk("t1", "A")];
    // OnRepeat requires history in generate_all_mixes, but generate_mix itself
    // uses the catalog fallback so the mix has tracks even on cold start.
    let mix = generate_mix(MixType::OnRepeat, &p, &catalog);
    assert!(!mix.tracks.is_empty(), "mix should use catalog fallback");
}

#[test]
fn cold_start_daily_mix_still_generated() {
    let p = UserProfile::new();
    let catalog = vec![mk("t1", "A")];
    // Daily Mix is always generated (even cold start).
    let mix = generate_mix(MixType::DailyMix, &p, &catalog);
    assert!(
        !mix.tracks.is_empty(),
        "DailyMix should have tracks via catalog fallback"
    );
}

#[test]
fn cold_start_all_mixes_still_has_daily_discover() {
    let p = UserProfile::new();
    let catalog: Vec<Track> = (0..10)
        .map(|i| mk(&format!("t{i}"), &format!("A{i}")))
        .collect();
    let mixes = generate_all_mixes(&p, &catalog);
    // Even cold start gets DailyMix + Discover (no history required).
    assert!(mixes.iter().any(|m| m.mix_type == MixType::DailyMix));
    assert!(mixes.iter().any(|m| m.mix_type == MixType::Discover));
    // Each generated mix should have tracks (catalog fallback).
    for mix in &mixes {
        assert!(
            !mix.tracks.is_empty(),
            "mix {:?} should have tracks",
            mix.mix_type
        );
    }
}

#[test]
fn cold_start_radio_from_track() {
    let p = UserProfile::new();
    let catalog = vec![mk("t1", "A"), mk("t2", "A")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    // Radio works even cold start (uses catalog for seed-based candidates).
    assert!(
        !r.candidate_pool.is_empty(),
        "radio should seed from catalog on cold start"
    );
}
