//! Integration tests for music-content validation (reco::validation).
//!
//! Verifies ContentPreference (balanced vs strict) filtering, music vs
//! non-music classification, short track rejection, and Shorts rejection.

use jukebox::reco::identity::TrackVariant;
use jukebox::reco::validation::{validate, ContentPreference};

#[test]
fn balanced_prefs_allow_live() {
    let prefs = ContentPreference::balanced();
    assert!(prefs.passes(TrackVariant::Live));
}

#[test]
fn balanced_prefs_allow_remixes() {
    let prefs = ContentPreference::balanced();
    assert!(prefs.passes(TrackVariant::Remix));
}

#[test]
fn balanced_prefs_allow_covers() {
    let prefs = ContentPreference::balanced();
    assert!(prefs.passes(TrackVariant::Cover));
}

#[test]
fn balanced_prefs_block_shorts() {
    let prefs = ContentPreference::balanced();
    assert!(!prefs.passes(TrackVariant::Short));
}

#[test]
fn balanced_prefs_block_commentary() {
    let prefs = ContentPreference::balanced();
    assert!(!prefs.passes(TrackVariant::Commentary));
}

#[test]
fn strict_prefs_block_live() {
    let prefs = ContentPreference::strict();
    assert!(!prefs.passes(TrackVariant::Live));
}

#[test]
fn strict_prefs_block_remixes() {
    let prefs = ContentPreference::strict();
    assert!(!prefs.passes(TrackVariant::Remix));
}

#[test]
fn strict_prefs_block_covers() {
    let prefs = ContentPreference::strict();
    assert!(!prefs.passes(TrackVariant::Cover));
}

#[test]
fn strict_prefs_block_acoustic() {
    let prefs = ContentPreference::strict();
    assert!(!prefs.passes(TrackVariant::Acoustic));
}

#[test]
fn strict_prefs_allow_original() {
    let prefs = ContentPreference::strict();
    assert!(prefs.passes(TrackVariant::Original));
}

#[test]
fn validate_music_track() {
    let result = validate(
        "Normal Song Title",
        Some(180.0),
        &ContentPreference::balanced(),
    );
    assert!(result.is_music);
    assert!(result.confidence > 0.5);
}

#[test]
fn validate_short_track_rejected() {
    let result = validate("Short Clip", Some(15.0), &ContentPreference::balanced());
    assert!(!result.is_music);
    assert!(result.reason.contains("too short"));
}

#[test]
fn validate_shorts_rejected() {
    let result = validate("Song #shorts", Some(60.0), &ContentPreference::balanced());
    assert!(!result.is_music);
}

#[test]
fn validate_commentary_rejected() {
    let result = validate(
        "Song Reaction Video",
        Some(300.0),
        &ContentPreference::balanced(),
    );
    assert!(!result.is_music);
}

#[test]
fn validate_live_allowed_in_balanced() {
    let result = validate("Song (Live)", Some(240.0), &ContentPreference::balanced());
    assert!(result.is_music);
}

#[test]
fn validate_live_filtered_in_strict() {
    let result = validate("Song (Live)", Some(240.0), &ContentPreference::strict());
    assert!(result.is_music);
    assert!(result.reason.contains("filtered"));
}

#[test]
fn content_preference_score_official_audio_preferred() {
    let prefs = ContentPreference::balanced();
    let official_score = prefs.score(TrackVariant::OfficialAudio);
    let video_score = prefs.score(TrackVariant::MusicVideo);
    assert!(official_score > video_score);
}

#[test]
fn content_preference_score_negative_infinity_for_blocked() {
    let prefs = ContentPreference::strict();
    let score = prefs.score(TrackVariant::Short);
    assert_eq!(score, f64::NEG_INFINITY);
}
