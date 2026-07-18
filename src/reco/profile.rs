//! User profile computed from listening events.
//!
//! [`UserProfile`] is the sole input to the local recommendation engine
//! (Level 2 personalization). It aggregates listening events into:
//!
//! - **Track profiles** ([`TrackProfile`]): per-track affinity score, play
//!   count, completion count, skip count, last-played timestamp.
//! - **Artist profiles** ([`ArtistProfile`]): per-artist affinity score, play
//!   count, blocked/hide flags.
//! - **Feedback sets**: liked track ids, hidden track ids, blocked artist
//!   names, "play less" track ids.
//!
//! ## Scoring model
//!
//! The affinity score is a weighted sum of positive and negative signals:
//!
//! | Signal | Weight |
//! |--------|--------|
//! | `MeaningfulThreshold` | +1.0 |
//! | `Completed` | +2.0 |
//! | `Replayed` | +0.5 |
//! | `Liked` | +5.0 |
//! | `AddedToQueue` | +0.3 |
//! | `AddedToPlaylist` | +0.5 |
//! | `SearchResultSelected` | +0.5 |
//! | `RecommendationSelected` | +0.3 |
//! | `Skipped` | -0.5 |
//! | `RapidlySkipped` | -2.0 |
//! | `Disliked` | -5.0 |
//! | `RemovedFromQueue` | -0.2 |
//! | `RemovedFromPlaylist` | -0.3 |
//! | `PlaybackFailed` | -0.1 |
//!
//! `TrackStarted` alone is **not** a positive signal (weak — the user might
//! immediately skip). Only `MeaningfulThreshold` and above contribute positive
//! affinity. This is the core design principle from `events.rs`.
//!
//! ## Persistence
//!
//! The profile is serialized to JSON and stored in `state.db` under the
//! `'profile'` key. On launch, the profile is loaded; on shutdown (or
//! periodically), the profile is saved. The profile can also be built from
//! the event log via `build_from_events`.

use crate::reco::events::ListenEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Per-track aggregated data derived from listening events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrackProfile {
    /// Weighted affinity score. Positive = liked, negative = disliked.
    pub score: f64,
    /// How many times the track was played past the meaningful threshold.
    pub completion_count: u32,
    /// How many times the track was started (any play attempt).
    pub play_count: u32,
    /// How many times the track was skipped (any skip).
    pub skip_count: u32,
    /// How many times the track was rapidly skipped (<10s). Strong negative.
    pub rapid_skip_count: u32,
    /// Unix-epoch seconds of the last play attempt. `None` if never played.
    pub last_played: Option<u64>,
    /// True if the user has explicitly liked this track.
    pub liked: bool,
    /// True if the user has explicitly disliked this track.
    pub disliked: bool,
    /// True if the user has hidden this track from recommendations.
    pub hidden: bool,
    /// True if the user has requested "play less" of this track.
    pub play_less: bool,
}

/// Per-artist aggregated data derived from listening events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArtistProfile {
    /// Sum of all track scores for this artist.
    pub score: f64,
    /// Total play count across all tracks by this artist.
    pub play_count: u32,
    /// True if the user has blocked this artist.
    pub blocked: bool,
}

/// The user's listening profile, computed from [`ListenEvent`]s.
///
/// This is the sole input to the local recommendation engine. It aggregates
/// raw events into per-track and per-artist scores, plus explicit-feedback
/// sets (likes, hides, blocks).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserProfile {
    /// Per-track profiles, keyed by track id.
    pub tracks: HashMap<String, TrackProfile>,
    /// Per-artist profiles, keyed by artist name.
    pub artists: HashMap<String, ArtistProfile>,
    /// Set of liked track ids (for fast membership tests).
    pub liked: HashSet<String>,
    /// Set of hidden track ids (excluded from all recommendations).
    pub hidden: HashSet<String>,
    /// Set of "play less" track ids (reduced frequency, not excluded).
    pub play_less: HashSet<String>,
    /// Set of blocked artist names (all tracks by these artists excluded).
    pub blocked_artists: HashSet<String>,
    /// Set of disliked track ids (strong negative).
    pub disliked: HashSet<String>,
    /// Total number of events processed to build this profile.
    pub event_count: u64,
}

// --- Scoring weights -------------------------------------------------------

/// Weight for a `MeaningfulThreshold` event (first positive signal).
const W_MEANINGFUL: f64 = 1.0;
/// Weight for a `Completed` event (strong positive).
const W_COMPLETED: f64 = 2.0;
/// Weight for a `Replayed` event (positive).
const W_REPLAYED: f64 = 0.5;
/// Weight for a `Liked` event (strongest positive).
const W_LIKED: f64 = 5.0;
/// Weight for an `AddedToQueue` event (mild positive).
const W_ADDED_TO_QUEUE: f64 = 0.3;
/// Weight for an `AddedToPlaylist` event (positive intent).
const W_ADDED_TO_PLAYLIST: f64 = 0.5;
/// Weight for a `SearchResultSelected` event (positive).
const W_SEARCH_SELECTED: f64 = 0.5;
/// Weight for a `RecommendationSelected` event (positive for rec source).
const W_REC_SELECTED: f64 = 0.3;
/// Weight for a `Skipped` event (weak negative).
const W_SKIPPED: f64 = -0.5;
/// Weight for a `RapidlySkipped` event (strong negative).
const W_RAPID_SKIPPED: f64 = -2.0;
/// Weight for a `Disliked` event (strongest negative).
const W_DISLIKED: f64 = -5.0;
/// Weight for a `RemovedFromQueue` event (mild negative).
const W_REMOVED_FROM_QUEUE: f64 = -0.2;
/// Weight for a `RemovedFromPlaylist` event (mild negative).
const W_REMOVED_FROM_PLAYLIST: f64 = -0.3;
/// Weight for a `PlaybackFailed` event (availability signal, mild negative).
const W_PLAYBACK_FAILED: f64 = -0.1;

impl UserProfile {
    /// Create a new empty profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a profile from a slice of listening events. Events are processed
    /// in order; later events can modify earlier state (e.g., `Unliked`
    /// removes a track from the liked set).
    pub fn build_from_events(events: &[ListenEvent]) -> Self {
        let mut profile = Self::new();
        for event in events {
            profile.apply_event(event);
        }
        profile.event_count = events.len() as u64;
        profile
    }

    /// Apply a single event to the profile, updating track/artist data.
    pub fn apply_event(&mut self, event: &ListenEvent) {
        self.event_count += 1;
        match event {
            ListenEvent::TrackStarted {
                track_id,
                timestamp,
                ..
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.play_count += 1;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
                // TrackStarted alone is NOT a positive signal — no score change.
            }
            ListenEvent::MeaningfulThreshold {
                track_id,
                timestamp,
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_MEANINGFUL;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
            }
            ListenEvent::Completed {
                track_id,
                timestamp,
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_COMPLETED;
                tp.completion_count += 1;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
            }
            ListenEvent::Skipped {
                track_id,
                timestamp,
                ..
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_SKIPPED;
                tp.skip_count += 1;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
            }
            ListenEvent::RapidlySkipped {
                track_id,
                timestamp,
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_RAPID_SKIPPED;
                tp.rapid_skip_count += 1;
                tp.skip_count += 1;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
            }
            ListenEvent::Replayed {
                track_id,
                timestamp,
            } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_REPLAYED;
                tp.last_played = Some(tp.last_played.map_or(*timestamp, |t| t.max(*timestamp)));
            }
            ListenEvent::Sought { track_id, .. } => {
                // Neutral — informational only. Ensure the track exists.
                self.tracks.entry(track_id.clone()).or_default();
            }
            ListenEvent::AddedToQueue { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_ADDED_TO_QUEUE;
            }
            ListenEvent::RemovedFromQueue { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_REMOVED_FROM_QUEUE;
            }
            ListenEvent::Liked { track_id, .. } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_LIKED;
                tp.liked = true;
                self.liked.insert(track_id.clone());
            }
            ListenEvent::Unliked { track_id, .. } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score -= W_LIKED;
                tp.liked = false;
                self.liked.remove(track_id);
            }
            ListenEvent::Disliked { track_id, .. } => {
                let tp = self.tracks.entry(track_id.clone()).or_default();
                tp.score += W_DISLIKED;
                tp.disliked = true;
                self.disliked.insert(track_id.clone());
            }
            ListenEvent::Hidden { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().hidden = true;
                self.hidden.insert(track_id.clone());
            }
            ListenEvent::ArtistBlocked { artist, .. } => {
                self.artists.entry(artist.clone()).or_default().blocked = true;
                self.blocked_artists.insert(artist.clone());
            }
            ListenEvent::PlayLess { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().play_less = true;
                self.play_less.insert(track_id.clone());
            }
            ListenEvent::AddedToPlaylist { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_ADDED_TO_PLAYLIST;
            }
            ListenEvent::RemovedFromPlaylist { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_REMOVED_FROM_PLAYLIST;
            }
            ListenEvent::RadioStarted { .. }
            | ListenEvent::MixOpened { .. }
            | ListenEvent::MixPlayed { .. } => {
                // No per-track impact; informational for session analysis.
            }
            ListenEvent::RecommendationShown { .. } => {
                // No score impact; used for recommendation quality measurement.
            }
            ListenEvent::RecommendationSelected { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_REC_SELECTED;
            }
            ListenEvent::RecommendationDismissed { .. } => {
                // Mild negative for the recommendation source, not the track.
            }
            ListenEvent::SearchPerformed { .. } => {
                // No per-track impact; used for search-context candidate generation.
            }
            ListenEvent::SearchResultSelected { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_SEARCH_SELECTED;
            }
            ListenEvent::SourceFallback { track_id, .. } => {
                // Ensure the track exists; no score impact (availability signal).
                self.tracks.entry(track_id.clone()).or_default();
            }
            ListenEvent::PlaybackFailed { track_id, .. } => {
                self.tracks.entry(track_id.clone()).or_default().score += W_PLAYBACK_FAILED;
            }
        }
    }

    /// Get the affinity score for a track. Returns 0.0 if the track has no
    /// profile (never been played).
    pub fn track_score(&self, track_id: &str) -> f64 {
        self.tracks.get(track_id).map(|t| t.score).unwrap_or(0.0)
    }

    /// Get the `n` most-liked track ids, sorted by score descending.
    /// Returns owned `String`s for convenience (callers often need owned values
    /// for candidate construction).
    pub fn top_liked(&self, n: usize) -> Vec<String> {
        let mut liked: Vec<&String> = self.liked.iter().collect();
        liked.sort_by(|a, b| {
            self.track_score(b)
                .partial_cmp(&self.track_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        liked.into_iter().take(n).cloned().collect()
    }

    /// Get the `n` top tracks by score, returning references to the track id
    /// and its [`TrackProfile`]. Sorted by score descending.
    pub fn top_tracks(&self, n: usize) -> Vec<(&str, &TrackProfile)> {
        let mut entries: Vec<(&String, &TrackProfile)> = self.tracks.iter().collect();
        entries.sort_by(|a, b| {
            b.1.score
                .partial_cmp(&a.1.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
            .into_iter()
            .take(n)
            .map(|(id, tp)| (id.as_str(), tp))
            .collect()
    }

    /// Get the top artists by score, returning references to the artist name
    /// and its [`ArtistProfile`]. Sorted by score descending.
    pub fn top_artists(&self, n: usize) -> Vec<(&str, &ArtistProfile)> {
        let mut entries: Vec<(&String, &ArtistProfile)> = self.artists.iter().collect();
        entries.sort_by(|a, b| {
            b.1.score
                .partial_cmp(&a.1.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
            .into_iter()
            .take(n)
            .map(|(id, ap)| (id.as_str(), ap))
            .collect()
    }

    /// Check if a track is hidden from recommendations.
    pub fn is_hidden(&self, track_id: &str) -> bool {
        self.hidden.contains(track_id)
    }

    /// Check if an artist is blocked from recommendations.
    pub fn is_blocked(&self, artist: &str) -> bool {
        self.blocked_artists.contains(artist)
    }

    /// Check if a track has been disliked.
    pub fn is_disliked(&self, track_id: &str) -> bool {
        self.disliked.contains(track_id)
    }

    /// Check if a track has "play less" feedback.
    pub fn is_play_less(&self, track_id: &str) -> bool {
        self.play_less.contains(track_id)
    }

    /// Check if a track has been liked.
    pub fn is_liked(&self, track_id: &str) -> bool {
        self.liked.contains(track_id)
    }

    /// Compute the skip rate for a track: `skip_count / play_count`.
    /// Returns 0.0 if the track has never been played.
    pub fn skip_rate(&self, track_id: &str) -> f64 {
        self.tracks
            .get(track_id)
            .filter(|t| t.play_count > 0)
            .map(|t| t.skip_count as f64 / t.play_count as f64)
            .unwrap_or(0.0)
    }

    /// Reset the profile to empty (privacy: user requests profile reset).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// True if the user has any meaningful listening history (at least one
    /// completed track or liked track). Used by the mix generator to decide
    /// whether to generate history-dependent mixes (OnRepeat, Rediscover).
    pub fn has_history(&self) -> bool {
        self.tracks.values().any(|t| t.completion_count > 0) || !self.liked.is_empty()
    }

    /// Total number of events processed to build this profile. Alias for
    /// `event_count` for compatibility with modules that use `total_events`.
    pub fn total_events(&self) -> u64 {
        self.event_count
    }

    /// True if the profile is empty (cold start — no tracks, no artists).
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.artists.is_empty()
    }

    /// Serialize the profile to a JSON string for persistence.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserialize a profile from a JSON string.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reco::events::{EventContext, EventSource};

    fn ts(n: u64) -> u64 {
        n
    }

    #[test]
    fn new_profile_is_empty() {
        let p = UserProfile::new();
        assert!(p.tracks.is_empty());
        assert!(p.artists.is_empty());
        assert!(p.liked.is_empty());
        assert!(p.hidden.is_empty());
        assert!(p.blocked_artists.is_empty());
        assert_eq!(p.event_count, 0);
    }

    #[test]
    fn build_from_empty_events() {
        let p = UserProfile::build_from_events(&[]);
        assert!(p.tracks.is_empty());
        assert_eq!(p.event_count, 0);
    }

    #[test]
    fn track_started_alone_is_not_positive() {
        let events = vec![ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(100),
            context: EventContext::Album,
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert_eq!(tp.play_count, 1, "play_count should be 1");
        assert_eq!(
            tp.score, 0.0,
            "TrackStarted alone must not add positive score"
        );
        assert_eq!(tp.completion_count, 0);
    }

    #[test]
    fn meaningful_threshold_adds_positive_score() {
        let events = vec![ListenEvent::MeaningfulThreshold {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.tracks.get("t1").unwrap().score > 0.0);
    }

    #[test]
    fn completed_adds_strong_positive_score() {
        let events = vec![ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(tp.score > 0.0);
        assert_eq!(tp.completion_count, 1);
    }

    #[test]
    fn liked_adds_strongest_positive_and_sets_flag() {
        let events = vec![ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(tp.liked);
        assert!(tp.score > 0.0);
        assert!(p.is_liked("t1"));
        assert!(p.liked.contains("t1"));
    }

    #[test]
    fn unliked_removes_liked_flag_and_neutralizes_score() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Unliked {
                track_id: "t1".into(),
                timestamp: ts(200),
            },
        ];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(!tp.liked);
        assert!(!p.is_liked("t1"));
        // The like added +5, the unliked subtracted 5 → net 0.
        assert_eq!(tp.score, 0.0);
    }

    #[test]
    fn disliked_adds_strong_negative() {
        let events = vec![ListenEvent::Disliked {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(tp.disliked);
        assert!(tp.score < 0.0);
        assert!(p.is_disliked("t1"));
    }

    #[test]
    fn skipped_adds_weak_negative() {
        let events = vec![ListenEvent::Skipped {
            track_id: "t1".into(),
            timestamp: ts(100),
            position_sec: 30.0,
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(tp.score < 0.0, "skipped should be negative");
        assert_eq!(tp.skip_count, 1);
    }

    #[test]
    fn rapidly_skipped_adds_strong_negative() {
        let events = vec![ListenEvent::RapidlySkipped {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        let tp = p.tracks.get("t1").unwrap();
        assert!(
            tp.score < -1.0,
            "rapidly skipped should be strongly negative"
        );
        assert_eq!(tp.rapid_skip_count, 1);
        assert_eq!(tp.skip_count, 1);
    }

    #[test]
    fn hidden_track_is_excluded() {
        let events = vec![ListenEvent::Hidden {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.is_hidden("t1"));
        assert!(p.hidden.contains("t1"));
    }

    #[test]
    fn artist_blocked_is_excluded() {
        let events = vec![ListenEvent::ArtistBlocked {
            artist: "Bad Artist".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.is_blocked("Bad Artist"));
        assert!(p.blocked_artists.contains("Bad Artist"));
    }

    #[test]
    fn play_less_tracks_soft_negative() {
        let events = vec![ListenEvent::PlayLess {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.is_play_less("t1"));
        assert!(p.play_less.contains("t1"));
    }

    #[test]
    fn top_liked_returns_liked_tracks_sorted_by_score() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Liked {
                track_id: "t2".into(),
                timestamp: ts(100),
            },
            ListenEvent::Completed {
                track_id: "t2".into(),
                timestamp: ts(200),
            },
        ];
        let p = UserProfile::build_from_events(&events);
        let top = p.top_liked(10);
        assert_eq!(top.len(), 2);
        // t2 has Liked (5.0) + Completed (2.0) = 7.0; t1 has Liked (5.0) only.
        assert_eq!(top[0], "t2", "higher-scored track should be first");
        assert_eq!(top[1], "t1");
    }

    #[test]
    fn top_tracks_returns_all_tracks_sorted_by_score() {
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Liked {
                track_id: "t2".into(),
                timestamp: ts(100),
            },
        ];
        let p = UserProfile::build_from_events(&events);
        let top = p.top_tracks(10);
        assert_eq!(top.len(), 2);
        // t2 (Liked=5.0) > t1 (Completed=2.0)
        assert_eq!(top[0].0, "t2");
        assert_eq!(top[1].0, "t1");
    }

    #[test]
    fn track_score_returns_zero_for_unknown() {
        let p = UserProfile::new();
        assert_eq!(p.track_score("unknown"), 0.0);
    }

    #[test]
    fn skip_rate_computation() {
        let events = vec![
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: EventSource::Local,
                timestamp: ts(100),
                context: EventContext::Album,
            },
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: EventSource::Local,
                timestamp: ts(200),
                context: EventContext::Album,
            },
            ListenEvent::Skipped {
                track_id: "t1".into(),
                timestamp: ts(250),
                position_sec: 30.0,
            },
        ];
        let p = UserProfile::build_from_events(&events);
        // 1 skip / 2 plays = 0.5
        assert!((p.skip_rate("t1") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn skip_rate_zero_for_never_played() {
        let p = UserProfile::new();
        assert_eq!(p.skip_rate("unknown"), 0.0);
    }

    #[test]
    fn reset_clears_all_data() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Hidden {
                track_id: "t2".into(),
                timestamp: ts(100),
            },
            ListenEvent::ArtistBlocked {
                artist: "Bad".into(),
                timestamp: ts(100),
            },
        ];
        let mut p = UserProfile::build_from_events(&events);
        assert!(!p.tracks.is_empty());
        assert!(!p.liked.is_empty());
        assert!(!p.hidden.is_empty());
        assert!(!p.blocked_artists.is_empty());

        p.reset();
        assert!(p.tracks.is_empty());
        assert!(p.liked.is_empty());
        assert!(p.hidden.is_empty());
        assert!(p.blocked_artists.is_empty());
        assert_eq!(p.event_count, 0);
    }

    #[test]
    fn json_roundtrip_preserves_data() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: ts(200),
            },
            ListenEvent::Hidden {
                track_id: "t2".into(),
                timestamp: ts(300),
            },
        ];
        let p = UserProfile::build_from_events(&events);
        let json = p.to_json().unwrap();
        let back = UserProfile::from_json(&json).unwrap();
        assert_eq!(back.tracks.len(), 2);
        assert!(back.is_liked("t1"));
        assert!(back.is_hidden("t2"));
        assert!((back.track_score("t1") - p.track_score("t1")).abs() < 1e-9);
    }

    #[test]
    fn replayed_adds_positive() {
        let events = vec![ListenEvent::Replayed {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.track_score("t1") > 0.0);
    }

    #[test]
    fn added_to_queue_adds_small_positive() {
        let events = vec![ListenEvent::AddedToQueue {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.track_score("t1") > 0.0);
    }

    #[test]
    fn removed_from_queue_adds_small_negative() {
        let events = vec![ListenEvent::RemovedFromQueue {
            track_id: "t1".into(),
            timestamp: ts(100),
        }];
        let p = UserProfile::build_from_events(&events);
        assert!(p.track_score("t1") < 0.0);
    }

    #[test]
    fn last_played_updates_on_play() {
        let events = vec![
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: EventSource::Local,
                timestamp: 100,
                context: EventContext::Album,
            },
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: EventSource::Local,
                timestamp: 300,
                context: EventContext::Album,
            },
        ];
        let p = UserProfile::build_from_events(&events);
        assert_eq!(p.tracks.get("t1").unwrap().last_played, Some(300));
    }

    #[test]
    fn event_count_tracks_total_events() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: ts(100),
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: ts(200),
            },
            ListenEvent::Skipped {
                track_id: "t2".into(),
                timestamp: ts(300),
                position_sec: 5.0,
            },
        ];
        let p = UserProfile::build_from_events(&events);
        assert_eq!(p.event_count, 3);
    }
}
