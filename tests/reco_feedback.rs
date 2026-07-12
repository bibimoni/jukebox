//! Integration tests for the feedback system (reco::feedback).
//!
//! Verifies that each feedback action has defined scopes, apply_feedback with
//! Like adds a positive signal, HideTrack excludes a track, BlockArtist
//! excludes an artist, is_excluded checks both hidden and blocked, and no
//! action is provider-level.

use jukebox::reco::feedback::{apply_feedback, is_excluded, FeedbackAction, FeedbackScope};
use jukebox::reco::profile::UserProfile;

#[test]
fn like_has_defined_scopes() {
    let scopes = FeedbackAction::Like.scopes();
    assert!(scopes.contains(&FeedbackScope::CurrentMix));
    assert!(scopes.contains(&FeedbackScope::FutureRecommendations));
    assert!(scopes.contains(&FeedbackScope::LongTermProfile));
}

#[test]
fn hide_track_has_all_scopes() {
    let scopes = FeedbackAction::HideTrack.scopes();
    assert!(scopes.contains(&FeedbackScope::CurrentMix));
    assert!(scopes.contains(&FeedbackScope::CurrentRadio));
    assert!(scopes.contains(&FeedbackScope::FutureRecommendations));
    assert!(scopes.contains(&FeedbackScope::LongTermProfile));
}

#[test]
fn hide_artist_has_all_scopes() {
    let scopes = FeedbackAction::HideArtist.scopes();
    assert!(scopes.contains(&FeedbackScope::CurrentMix));
    assert!(scopes.contains(&FeedbackScope::CurrentRadio));
    assert!(scopes.contains(&FeedbackScope::FutureRecommendations));
    assert!(scopes.contains(&FeedbackScope::LongTermProfile));
}

#[test]
fn block_artist_affects_all_scopes() {
    let scopes = FeedbackAction::BlockArtist.scopes();
    assert!(scopes.len() >= 4);
}

#[test]
fn remove_from_mix_only_affects_current_mix() {
    let scopes = FeedbackAction::RemoveFromMix.scopes();
    assert_eq!(scopes, vec![FeedbackScope::CurrentMix]);
}

#[test]
fn play_less_has_defined_scopes() {
    let scopes = FeedbackAction::PlayLess.scopes();
    assert!(scopes.contains(&FeedbackScope::CurrentMix));
    assert!(scopes.contains(&FeedbackScope::CurrentRadio));
    assert!(scopes.contains(&FeedbackScope::FutureRecommendations));
}

#[test]
fn apply_like_adds_positive() {
    let mut profile = UserProfile::new();
    apply_feedback(&FeedbackAction::Like, "t1", None, &mut profile);
    assert!(profile.track_score("t1") > 0.0);
    assert!(profile.is_liked("t1"));
}

#[test]
fn apply_hide_excludes() {
    let mut profile = UserProfile::new();
    apply_feedback(&FeedbackAction::HideTrack, "t1", None, &mut profile);
    assert!(profile.is_hidden("t1"));
}

#[test]
fn apply_block_artist_excludes() {
    let mut profile = UserProfile::new();
    apply_feedback(
        &FeedbackAction::BlockArtist,
        "t1",
        Some("Bad Artist"),
        &mut profile,
    );
    assert!(profile.is_blocked("Bad Artist"));
}

#[test]
fn is_excluded_checks_hidden() {
    let mut profile = UserProfile::new();
    apply_feedback(&FeedbackAction::HideTrack, "t1", None, &mut profile);
    assert!(is_excluded("t1", None, &profile));
    assert!(!is_excluded("t2", None, &profile));
}

#[test]
fn is_excluded_checks_blocked_artist() {
    let mut profile = UserProfile::new();
    apply_feedback(
        &FeedbackAction::BlockArtist,
        "t1",
        Some("Bad Artist"),
        &mut profile,
    );
    assert!(is_excluded("t1", Some("Bad Artist"), &profile));
    assert!(!is_excluded("t1", Some("Good Artist"), &profile));
}

#[test]
fn no_action_is_provider_level() {
    for action in [
        FeedbackAction::Like,
        FeedbackAction::Unlike,
        FeedbackAction::HideTrack,
        FeedbackAction::HideArtist,
        FeedbackAction::BlockArtist,
        FeedbackAction::PlayLess,
        FeedbackAction::DontRecommendSource,
        FeedbackAction::PreferLocal,
        FeedbackAction::PreferYoutube,
        FeedbackAction::RemoveFromMix,
        FeedbackAction::ReplaceRecommendation,
        FeedbackAction::ResetFeedback,
    ] {
        assert!(
            !action.is_provider_level(),
            "{:?} should not be provider-level",
            action
        );
    }
}

#[test]
fn apply_unlike_neutralizes_like() {
    let mut profile = UserProfile::new();
    apply_feedback(&FeedbackAction::Like, "t1", None, &mut profile);
    assert!(profile.track_score("t1") > 0.0);
    apply_feedback(&FeedbackAction::Unlike, "t1", None, &mut profile);
    assert!(!profile.is_liked("t1"));
}

#[test]
fn apply_play_less_sets_flag() {
    let mut profile = UserProfile::new();
    apply_feedback(&FeedbackAction::PlayLess, "t1", None, &mut profile);
    assert!(profile.is_play_less("t1"));
}
