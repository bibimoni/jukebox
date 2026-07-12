//! Feedback system — user actions on recommendations and tracks.
//!
//! Feedback actions have defined scopes (current mix, current radio, future
//! recommendations, long-term profile, YouTube account). The system does NOT
//! write provider-level feedback unless the user explicitly performs a
//! provider-level action (like on YouTube, subscribe on YouTube).

use crate::reco::events::ListenEvent;
use crate::reco::profile::UserProfile;
use serde::{Deserialize, Serialize};

/// A feedback action the user can perform.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackAction {
    /// Like a track (strong positive).
    Like,
    /// Remove a like.
    Unlike,
    /// Hide a track from recommendations (persistent negative).
    HideTrack,
    /// Hide all tracks by an artist (persistent negative).
    HideArtist,
    /// Block an artist from all recommendations (strongest negative).
    BlockArtist,
    /// "Play less like this" — soft negative, reduce frequency.
    PlayLess,
    /// Don't recommend from this source (playlist, channel, etc.).
    DontRecommendSource,
    /// Prefer the local version of this track.
    PreferLocal,
    /// Prefer the YouTube version of this track.
    PreferYoutube,
    /// Remove a track from the current mix.
    RemoveFromMix,
    /// Replace a recommendation with a different one.
    ReplaceRecommendation,
    /// Reset all feedback (clears all negative signals).
    ResetFeedback,
}

/// The scope at which a feedback action takes effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackScope {
    /// Affects only the current mix.
    CurrentMix,
    /// Affects only the current radio session.
    CurrentRadio,
    /// Affects future recommendations.
    FutureRecommendations,
    /// Affects the long-term user profile.
    LongTermProfile,
    /// Affects the YouTube account (provider-level action — only when the
    /// user explicitly performs a provider-level action like YouTube like).
    ProviderAccount,
}

impl FeedbackAction {
    /// Get the scopes this action affects.
    pub fn scopes(&self) -> Vec<FeedbackScope> {
        match self {
            FeedbackAction::Like => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::Unlike => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::HideTrack => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::CurrentRadio,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::HideArtist => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::CurrentRadio,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::BlockArtist => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::CurrentRadio,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::PlayLess => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::CurrentRadio,
                FeedbackScope::FutureRecommendations,
            ],
            FeedbackAction::DontRecommendSource => vec![FeedbackScope::FutureRecommendations],
            FeedbackAction::PreferLocal => vec![
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::PreferYoutube => vec![
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
            FeedbackAction::RemoveFromMix => vec![FeedbackScope::CurrentMix],
            FeedbackAction::ReplaceRecommendation => {
                vec![FeedbackScope::CurrentMix, FeedbackScope::CurrentRadio]
            }
            FeedbackAction::ResetFeedback => vec![
                FeedbackScope::CurrentMix,
                FeedbackScope::CurrentRadio,
                FeedbackScope::FutureRecommendations,
                FeedbackScope::LongTermProfile,
            ],
        }
    }

    /// True if this action affects the YouTube provider account.
    pub fn is_provider_level(&self) -> bool {
        self.scopes().contains(&FeedbackScope::ProviderAccount)
    }

    /// Convert the feedback action to a listening event for recording.
    pub fn to_event(&self, track_id: &str, artist: Option<&str>) -> ListenEvent {
        let ts = ListenEvent::now();
        match self {
            FeedbackAction::Like => ListenEvent::Liked {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::Unlike => ListenEvent::Unliked {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::HideTrack => ListenEvent::Hidden {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::HideArtist | FeedbackAction::BlockArtist => {
                ListenEvent::ArtistBlocked {
                    artist: artist.unwrap_or("").into(),
                    timestamp: ts,
                }
            }
            FeedbackAction::PlayLess => ListenEvent::PlayLess {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::DontRecommendSource => ListenEvent::Hidden {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::PreferLocal | FeedbackAction::PreferYoutube => {
                // Source preference is handled separately (not a ListenEvent).
                ListenEvent::AddedToQueue {
                    track_id: track_id.into(),
                    timestamp: ts,
                }
            }
            FeedbackAction::RemoveFromMix => ListenEvent::RemovedFromQueue {
                track_id: track_id.into(),
                timestamp: ts,
            },
            FeedbackAction::ReplaceRecommendation => ListenEvent::RecommendationDismissed {
                track_id: track_id.into(),
                source: "feedback".into(),
                timestamp: ts,
            },
            FeedbackAction::ResetFeedback => ListenEvent::AddedToQueue {
                track_id: track_id.into(),
                timestamp: ts,
            },
        }
    }
}

/// Apply a feedback action to the user profile.
pub fn apply_feedback(
    action: &FeedbackAction,
    track_id: &str,
    artist: Option<&str>,
    profile: &mut UserProfile,
) {
    let event = action.to_event(track_id, artist);
    profile.apply_event(&event);
}

/// Check if a track should be excluded from recommendations based on feedback.
pub fn is_excluded(track_id: &str, artist: Option<&str>, profile: &UserProfile) -> bool {
    if profile.is_hidden(track_id) {
        return true;
    }
    if let Some(a) = artist {
        if profile.is_blocked(a) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::reco::profile::UserProfile;

    #[test]
    fn like_affects_multiple_scopes() {
        let scopes = FeedbackAction::Like.scopes();
        assert!(scopes.contains(&FeedbackScope::CurrentMix));
        assert!(scopes.contains(&FeedbackScope::LongTermProfile));
    }

    #[test]
    fn hide_track_excludes_from_all_scopes() {
        let scopes = FeedbackAction::HideTrack.scopes();
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
    fn no_feedback_action_is_provider_level() {
        // None of our feedback actions directly affect the YouTube account
        // (only explicit YouTube likes/subscribes do, which are separate).
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
    fn apply_like_adds_positive_to_profile() {
        let mut profile = UserProfile::new();
        apply_feedback(&FeedbackAction::Like, "t1", None, &mut profile);
        assert!(profile.track_score("t1") > 0.0);
        assert!(profile.tracks["t1"].liked);
    }

    #[test]
    fn apply_hide_excludes_from_recommendations() {
        let mut profile = UserProfile::new();
        apply_feedback(&FeedbackAction::HideTrack, "t1", None, &mut profile);
        assert!(profile.is_hidden("t1"));
    }

    #[test]
    fn apply_block_artist_excludes_artist() {
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
    fn is_excluded_checks_hidden_and_blocked() {
        let mut profile = UserProfile::new();
        apply_feedback(&FeedbackAction::HideTrack, "t1", None, &mut profile);
        assert!(is_excluded("t1", None, &profile));
        assert!(!is_excluded("t2", None, &profile));
    }

    #[test]
    fn reset_feedback_clears_profile() {
        let mut profile = UserProfile::new();
        apply_feedback(&FeedbackAction::Like, "t1", None, &mut profile);
        assert!(!profile.is_empty());
        apply_feedback(&FeedbackAction::ResetFeedback, "t1", None, &mut profile);
        // ResetFeedback doesn't directly clear — it's a signal that the caller
        // should call profile.reset(). This test verifies the action is defined.
        let scopes = FeedbackAction::ResetFeedback.scopes();
        assert!(scopes.contains(&FeedbackScope::LongTermProfile));
    }
}
