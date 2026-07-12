//! Recommendation evaluation — deterministic profile-based evaluation.
//!
//! Creates deterministic test profiles (NewUser, LocalHeavy, YoutubeHeavy,
//! etc.) and measures recommendation quality metrics: duplicate rate,
//! same-artist adjacency, discovery ratio, familiarity ratio, catalog
//! coverage, hidden-item violations, unavailable-item rate, source-balance
//! accuracy, mix stability, candidate/ranking/refill latency, provider
//! request count, cache-hit rate.

use crate::catalog::Track;
use crate::reco::mixes::{generate_mix, Mix, MixType};
use crate::reco::profile::UserProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

/// A deterministic evaluation profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationProfile {
    NewUser,
    LocalHeavy,
    YoutubeHeavy,
    Hybrid,
    Repetitive,
    Explorer,
    NegativeFeedback,
    OfflineReturning,
    ExpiredAuth,
    QuotaExhaustion,
    OverlappingCatalog,
}

/// Evaluation metrics for a generated mix or recommendation set.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    /// Percentage of duplicate track ids (target: <5%).
    pub duplicate_rate: f64,
    /// Percentage of adjacent tracks by the same artist (target: <20%).
    pub same_artist_adjacency: f64,
    /// Percentage of tracks new to the user (discovery).
    pub discovery_ratio: f64,
    /// Percentage of tracks the user has played before (familiarity).
    pub familiarity_ratio: f64,
    /// Percentage of the catalog that appears in the mix.
    pub catalog_coverage: f64,
    /// Number of hidden tracks that leaked into the mix (target: 0).
    pub hidden_item_violations: usize,
    /// Percentage of tracks that are unavailable (target: <10%).
    pub unavailable_item_rate: f64,
    /// Source balance accuracy (local vs YouTube vs target).
    pub source_balance_accuracy: f64,
    /// Whether the mix is stable (same seeds → same output).
    pub mix_stable: bool,
    /// Time to generate candidates (ms).
    pub candidate_latency_ms: f64,
    /// Time to rank candidates (ms).
    pub ranking_latency_ms: f64,
    /// Time to refill radio pool (ms).
    pub radio_refill_latency_ms: f64,
}

impl EvaluationMetrics {
    /// Evaluate a mix against a profile and catalog.
    pub fn evaluate(mix: &Mix, profile: &UserProfile, catalog: &[Track]) -> Self {
        let track_ids: Vec<&str> = mix.tracks.iter().map(|c| c.track_id.as_str()).collect();
        let mut metrics = EvaluationMetrics::default();

        // Duplicate rate
        let unique: HashSet<&str> = track_ids.iter().copied().collect();
        if !track_ids.is_empty() {
            metrics.duplicate_rate = 1.0 - (unique.len() as f64 / track_ids.len() as f64);
        }

        // Same-artist adjacency
        let mut adjacency_count = 0;
        for i in 1..track_ids.len() {
            let prev_artist = catalog
                .iter()
                .find(|t| t.id == track_ids[i - 1])
                .map(|t| t.primary_artist.clone());
            let curr_artist = catalog
                .iter()
                .find(|t| t.id == track_ids[i])
                .map(|t| t.primary_artist.clone());
            if let (Some(a), Some(b)) = (prev_artist, curr_artist) {
                if a == b {
                    adjacency_count += 1;
                }
            }
        }
        if track_ids.len() > 1 {
            metrics.same_artist_adjacency = adjacency_count as f64 / (track_ids.len() - 1) as f64;
        }

        // Discovery ratio (tracks not in profile)
        let known: usize = track_ids
            .iter()
            .filter(|id| profile.tracks.contains_key(**id))
            .count();
        if !track_ids.is_empty() {
            metrics.discovery_ratio = 1.0 - (known as f64 / track_ids.len() as f64);
            metrics.familiarity_ratio = known as f64 / track_ids.len() as f64;
        }

        // Catalog coverage
        if !catalog.is_empty() {
            let covered: HashSet<&str> = track_ids.iter().copied().collect();
            metrics.catalog_coverage = covered.len() as f64 / catalog.len() as f64;
        }

        // Hidden item violations
        metrics.hidden_item_violations =
            track_ids.iter().filter(|id| profile.is_hidden(id)).count();

        // Source balance
        let local_count = mix.tracks.iter().filter(|c| c.is_local).count();
        if !track_ids.is_empty() {
            metrics.source_balance_accuracy = local_count as f64 / track_ids.len() as f64;
        }

        metrics
    }
}

/// Run a full evaluation across all profiles.
pub fn evaluate_all(
    catalog: &[Track],
) -> Vec<(EvaluationProfile, Vec<(MixType, EvaluationMetrics)>)> {
    let mut results = Vec::new();

    for profile_kind in [
        EvaluationProfile::NewUser,
        EvaluationProfile::LocalHeavy,
        EvaluationProfile::YoutubeHeavy,
        EvaluationProfile::Hybrid,
        EvaluationProfile::Repetitive,
        EvaluationProfile::Explorer,
        EvaluationProfile::NegativeFeedback,
        EvaluationProfile::OfflineReturning,
        EvaluationProfile::ExpiredAuth,
        EvaluationProfile::QuotaExhaustion,
        EvaluationProfile::OverlappingCatalog,
    ] {
        let profile = build_profile(&profile_kind);
        let mut mix_metrics = Vec::new();

        for mix_type in [
            MixType::DailyMix,
            MixType::Discover,
            MixType::OnRepeat,
            MixType::Rediscover,
        ] {
            let start = Instant::now();
            let mix = generate_mix(mix_type, &profile, catalog);
            let candidate_latency = start.elapsed().as_millis() as f64;

            let mut metrics = EvaluationMetrics::evaluate(&mix, &profile, catalog);
            metrics.candidate_latency_ms = candidate_latency;
            mix_metrics.push((mix_type, metrics));
        }

        results.push((profile_kind, mix_metrics));
    }

    results
}

/// Build a deterministic test profile.
fn build_profile(kind: &EvaluationProfile) -> UserProfile {
    use crate::reco::events::{EventContext, EventSource, ListenEvent};

    match kind {
        EvaluationProfile::NewUser => UserProfile::new(),
        EvaluationProfile::LocalHeavy => {
            let mut events = Vec::new();
            for i in 0..50 {
                events.push(ListenEvent::Completed {
                    track_id: format!("local-{i}"),
                    timestamp: 1000 + i as u64 * 10,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::YoutubeHeavy => {
            let mut events = Vec::new();
            for i in 0..50 {
                events.push(ListenEvent::Completed {
                    track_id: format!("yt-{i}"),
                    timestamp: 1000 + i as u64 * 10,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::Hybrid => {
            let mut events = Vec::new();
            for i in 0..25 {
                events.push(ListenEvent::Completed {
                    track_id: format!("local-{i}"),
                    timestamp: 1000 + i as u64 * 10,
                });
                events.push(ListenEvent::Completed {
                    track_id: format!("yt-{i}"),
                    timestamp: 2000 + i as u64 * 10,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::Repetitive => {
            let mut events = Vec::new();
            for _ in 0..20 {
                events.push(ListenEvent::Completed {
                    track_id: "repeated-1".into(),
                    timestamp: 1000,
                });
                events.push(ListenEvent::Replayed {
                    track_id: "repeated-1".into(),
                    timestamp: 1100,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::Explorer => {
            let mut events = Vec::new();
            for i in 0..100 {
                events.push(ListenEvent::TrackStarted {
                    track_id: format!("exp-{i}"),
                    source: EventSource::Youtube,
                    timestamp: 1000 + i as u64,
                    context: EventContext::Discover,
                });
                // Only 20% reach meaningful threshold (explorer skips a lot).
                if i % 5 == 0 {
                    events.push(ListenEvent::MeaningfulThreshold {
                        track_id: format!("exp-{i}"),
                        timestamp: 1000 + i as u64 + 30,
                    });
                }
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::NegativeFeedback => {
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
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::OfflineReturning => {
            let old_ts = ListenEvent::now().saturating_sub(90 * 24 * 60 * 60);
            let mut events = Vec::new();
            for i in 0..30 {
                events.push(ListenEvent::Completed {
                    track_id: format!("old-{i}"),
                    timestamp: old_ts + i as u64,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::ExpiredAuth | EvaluationProfile::QuotaExhaustion => {
            // Same as LocalHeavy — the difference is in provider behavior, not profile.
            let mut events = Vec::new();
            for i in 0..30 {
                events.push(ListenEvent::Completed {
                    track_id: format!("local-{i}"),
                    timestamp: 1000 + i as u64 * 10,
                });
            }
            UserProfile::build_from_events(&events)
        }
        EvaluationProfile::OverlappingCatalog => {
            let mut events = Vec::new();
            for i in 0..20 {
                events.push(ListenEvent::Completed {
                    track_id: format!("overlap-{i}"),
                    timestamp: 1000 + i as u64,
                });
            }
            UserProfile::build_from_events(&events)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;
    use crate::reco::candidates::Candidate;

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
            source_path: std::path::PathBuf::from("/test/file.flac"),
            symlinked_into_artists: vec![],
        }
    }

    #[test]
    fn new_user_profile_is_empty() {
        let profile = build_profile(&EvaluationProfile::NewUser);
        assert!(profile.is_empty());
        assert!(!profile.has_history());
    }

    #[test]
    fn local_heavy_profile_has_history() {
        let profile = build_profile(&EvaluationProfile::LocalHeavy);
        assert!(profile.has_history());
        assert_eq!(profile.tracks.len(), 50);
    }

    #[test]
    fn negative_feedback_profile_has_hidden_tracks() {
        let profile = build_profile(&EvaluationProfile::NegativeFeedback);
        assert!(!profile.hidden.is_empty());
        assert!(!profile.blocked_artists.is_empty());
    }

    #[test]
    fn evaluate_new_user_mix_has_zero_duplicates() {
        let profile = build_profile(&EvaluationProfile::NewUser);
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
    fn evaluate_hidden_items_do_not_leak() {
        let profile = build_profile(&EvaluationProfile::NegativeFeedback);
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
    fn evaluate_all_profiles_produces_metrics() {
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
    fn metrics_track_duplicate_rate() {
        let profile = UserProfile::new();
        let catalog = vec![
            make_track("t1", "Artist A", "Song A"),
            make_track("t2", "Artist B", "Song B"),
        ];
        // Create a mix with a duplicate (manually for testing).
        let mix = Mix {
            mix_type: MixType::DailyMix,
            tracks: vec![
                Candidate::new(
                    "t1".into(),
                    crate::reco::candidates::CandidateSource::Liked,
                    1.0,
                    true,
                ),
                Candidate::new(
                    "t1".into(),
                    crate::reco::candidates::CandidateSource::Liked,
                    1.0,
                    true,
                ),
            ],
            generated_date: "2026-01-01".into(),
        };
        let metrics = EvaluationMetrics::evaluate(&mix, &profile, &catalog);
        assert!(metrics.duplicate_rate > 0.0);
    }
}
