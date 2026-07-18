//! Integration tests for identity resolution (reco::identity).
//!
//! Verifies canonical_id normalization (case, punctuation, featuring,
//! parentheticals), TrackVariant detection from titles, and IdentityResolver
//! grouping of variants.

use jukebox::reco::identity::{canonical_id, IdentityResolver, TrackVariant};

#[test]
fn canonical_id_normalizes_case() {
    let id1 = canonical_id("The Beatles", "Hey Jude");
    let id2 = canonical_id("the beatles", "hey jude");
    assert_eq!(id1, id2);
}

#[test]
fn canonical_id_strips_punctuation() {
    let id1 = canonical_id("Artist!", "Song?");
    let id2 = canonical_id("Artist", "Song");
    assert_eq!(id1, id2);
}

#[test]
fn canonical_id_strips_featuring() {
    let id1 = canonical_id("Artist", "Song feat. Other Artist");
    let id2 = canonical_id("Artist", "Song");
    assert_eq!(id1, id2);
    let id3 = canonical_id("Artist", "Song featuring Other");
    let id4 = canonical_id("Artist", "Song");
    assert_eq!(id3, id4);
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
fn canonical_id_different_artists_not_equal() {
    let id1 = canonical_id("Artist A", "Song");
    let id2 = canonical_id("Artist B", "Song");
    assert_ne!(id1, id2);
}

#[test]
fn track_variant_detects_live() {
    assert_eq!(TrackVariant::from_title("Song (Live)"), TrackVariant::Live);
    assert_eq!(
        TrackVariant::from_title("Song (Live at Wembley)"),
        TrackVariant::Live
    );
}

#[test]
fn track_variant_detects_remix() {
    assert_eq!(
        TrackVariant::from_title("Song (Remix)"),
        TrackVariant::Remix
    );
}

#[test]
fn track_variant_detects_remaster() {
    assert_eq!(
        TrackVariant::from_title("Song (Remastered)"),
        TrackVariant::Remaster
    );
}

#[test]
fn track_variant_detects_cover() {
    assert_eq!(
        TrackVariant::from_title("Song (Cover)"),
        TrackVariant::Cover
    );
}

#[test]
fn track_variant_detects_shorts() {
    assert_eq!(
        TrackVariant::from_title("Song #shorts"),
        TrackVariant::Short
    );
    assert!(!TrackVariant::Short.is_music());
}

#[test]
fn track_variant_detects_commentary() {
    assert_eq!(
        TrackVariant::from_title("Song Reaction"),
        TrackVariant::Commentary
    );
    assert!(!TrackVariant::Commentary.is_music());
}

#[test]
fn track_variant_detects_acoustic() {
    assert_eq!(
        TrackVariant::from_title("Song (Acoustic)"),
        TrackVariant::Acoustic
    );
}

#[test]
fn track_variant_detects_instrumental() {
    assert_eq!(
        TrackVariant::from_title("Song (Instrumental)"),
        TrackVariant::Instrumental
    );
}

#[test]
fn track_variant_detects_official_audio() {
    assert_eq!(
        TrackVariant::from_title("Song (Official Audio)"),
        TrackVariant::OfficialAudio
    );
    assert!(TrackVariant::OfficialAudio.is_preferred());
}

#[test]
fn track_variant_detects_music_video() {
    assert_eq!(
        TrackVariant::from_title("Song (Official Video)"),
        TrackVariant::MusicVideo
    );
    assert!(TrackVariant::MusicVideo.is_music());
    assert!(!TrackVariant::MusicVideo.is_preferred());
}

#[test]
fn track_variant_original_is_default() {
    assert_eq!(
        TrackVariant::from_title("Just A Regular Song"),
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
    let mut resolver = IdentityResolver::new();
    resolver.add_track("a1", "Artist A", "Song");
    resolver.add_track("b1", "Artist B", "Song");
    let variants_a = resolver.variants_of("a1", "Artist A", "Song");
    assert!(variants_a.contains(&"a1".to_string()));
    assert!(!variants_a.contains(&"b1".to_string()));
}
