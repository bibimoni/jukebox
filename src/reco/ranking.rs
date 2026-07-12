//! Ranking — feature computation and scoring for candidate tracks.
//!
//! Computes features for each candidate (affinity, novelty, recency,
//! familiarity, source balance) and produces a final score via a weighted
//! sum. Ranking is deterministic with stable tie-breaking.

use crate::reco::candidates::{Candidate, CandidateSource};
use crate::reco::profile::UserProfile;
use std::cmp::Ordering;

/// A computed feature vector for a candidate.
#[derive(Clone, Debug, Default)]
pub struct Features {
    /// Affinity score from the user profile (likes, completions, etc.).
    pub affinity: f64,
    /// Novelty: how unfamiliar is this track? (0 = well-known, 1 = new).
    pub novelty: f64,
    /// Recency: how recently was this track played? (0 = just now, 1 = never).
    pub recency: f64,
    /// Familiarity: how well-known is the artist? (0 = unknown, 1 = top).
    pub familiarity: f64,
    /// Source balance: 0 = local, 1 = YouTube. Used to balance hybrid mode.
    pub source_balance: f64,
    /// Source strength: how strong is the recommendation source? (0-1).
    pub source_strength: f64,
}

impl Features {
    /// Compute features for a candidate given the user profile.
    pub fn compute(candidate: &Candidate, profile: &UserProfile) -> Self {
        let affinity = candidate.affinity;

        // Novelty: 0 if the track has been played, 1 if never seen.
        let novelty = if profile.tracks.contains_key(&candidate.track_id) {
            0.0
        } else {
            1.0
        };

        // Recency: 0 if just played, 1 if never played.
        let recency = profile
            .tracks
            .get(&candidate.track_id)
            .and_then(|t| t.last_played)
            .map(|_| 0.0)
            .unwrap_or(1.0);

        // Familiarity: based on play count (more plays = more familiar).
        let familiarity = profile
            .tracks
            .get(&candidate.track_id)
            .map(|t| (t.play_count as f64).min(10.0) / 10.0)
            .unwrap_or(0.0);

        // Source balance: 0 for local, 1 for YouTube.
        let source_balance = if candidate.is_local { 0.0 } else { 1.0 };

        // Source strength: how strong is the source signal?
        let source_strength = match candidate.source {
            CandidateSource::Liked => 1.0,
            CandidateSource::Completion => 0.9,
            CandidateSource::Repeated => 0.8,
            CandidateSource::PlaylistNeighbors => 0.7,
            CandidateSource::ArtistAffinity => 0.6,
            CandidateSource::LocalMetadata => 0.5,
            CandidateSource::ExistingPlaylists => 0.5,
            CandidateSource::NewFromArtists => 0.6,
            CandidateSource::Rediscovery => 0.4,
            CandidateSource::ExplicitSeed => 1.0,
            CandidateSource::ProviderSearch => 0.3,
            CandidateSource::ExperimentalHome => 0.2,
        };

        Features {
            affinity,
            novelty,
            recency,
            familiarity,
            source_balance,
            source_strength,
        }
    }
}

/// Weights for the scoring function. Higher weight = more important.
#[derive(Clone, Debug)]
pub struct RankingWeights {
    pub affinity: f64,
    pub novelty: f64,
    pub recency: f64,
    pub familiarity: f64,
    pub source_strength: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        // Default: balanced weighting. Affinity is the strongest signal,
        // followed by source strength and novelty (for discovery).
        RankingWeights {
            affinity: 3.0,
            novelty: 1.0,
            recency: 0.5,
            familiarity: 0.5,
            source_strength: 2.0,
        }
    }
}

/// Discover-mix weights: high novelty (discovery), low familiarity.
impl RankingWeights {
    pub fn discover() -> Self {
        RankingWeights {
            affinity: 1.0,
            novelty: 3.0,
            recency: 0.3,
            familiarity: 0.1,
            source_strength: 2.0,
        }
    }

    /// On-repeat weights: high familiarity, low novelty.
    pub fn on_repeat() -> Self {
        RankingWeights {
            affinity: 3.0,
            novelty: 0.1,
            recency: 1.0,
            familiarity: 3.0,
            source_strength: 2.0,
        }
    }

    /// Rediscover weights: high recency (old tracks), moderate affinity.
    pub fn rediscover() -> Self {
        RankingWeights {
            affinity: 2.0,
            novelty: 0.1,
            recency: 3.0,
            familiarity: 2.0,
            source_strength: 1.5,
        }
    }
}

/// Score a candidate using the feature vector and weights.
pub fn score(features: &Features, weights: &RankingWeights) -> f64 {
    features.affinity * weights.affinity
        + features.novelty * weights.novelty
        + features.recency * weights.recency
        + features.familiarity * weights.familiarity
        + features.source_strength * weights.source_strength
}

/// Rank candidates by score (descending). Ties are broken by track_id
/// (ascending) for deterministic output.
pub fn rank(candidates: &mut Vec<Candidate>, profile: &UserProfile, weights: &RankingWeights) {
    // Compute scores.
    let mut scored: Vec<(f64, String, Candidate)> = candidates
        .iter()
        .map(|c| {
            let features = Features::compute(c, profile);
            (score(&features, weights), c.track_id.clone(), c.clone())
        })
        .collect();

    // Sort by score descending, then by track_id ascending (deterministic tie-break).
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    // Reorder candidates by the sorted order.
    *candidates = scored.into_iter().map(|(_, _, c)| c).collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reco::events::ListenEvent;
    use crate::reco::profile::UserProfile;

    #[test]
    fn features_compute_for_known_track() {
        let events = vec![
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: crate::reco::events::EventSource::Local,
                timestamp: 100,
                context: crate::reco::events::EventContext::Album,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 200,
            },
        ];
        let profile = UserProfile::build_from_events(&events);
        let candidate = Candidate::new("t1".into(), CandidateSource::Completion, 5.0, true);
        let features = Features::compute(&candidate, &profile);
        assert!(features.affinity > 0.0);
        assert_eq!(features.novelty, 0.0); // known track
        assert!(features.familiarity > 0.0); // played (TrackStarted)
    }

    #[test]
    fn features_compute_for_unknown_track() {
        let profile = UserProfile::new();
        let candidate = Candidate::new("t1".into(), CandidateSource::ProviderSearch, 0.0, false);
        let features = Features::compute(&candidate, &profile);
        assert_eq!(features.novelty, 1.0); // unknown track
        assert_eq!(features.familiarity, 0.0);
        assert_eq!(features.source_balance, 1.0); // YouTube
    }

    #[test]
    fn score_higher_for_liked_than_search() {
        let profile = UserProfile::new();
        let liked = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
        let search = Candidate::new("t2".into(), CandidateSource::ProviderSearch, 0.0, false);
        let weights = RankingWeights::default();
        let f1 = Features::compute(&liked, &profile);
        let f2 = Features::compute(&search, &profile);
        assert!(score(&f1, &weights) > score(&f2, &weights));
    }

    #[test]
    fn discover_weights_prefer_novelty() {
        let events = vec![
            ListenEvent::TrackStarted {
                track_id: "t1".into(),
                source: crate::reco::events::EventSource::Local,
                timestamp: 100,
                context: crate::reco::events::EventContext::Album,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 200,
            },
        ];
        let profile = UserProfile::build_from_events(&events);
        // "t1" is known (in profile), "t2" is novel (not in profile).
        let known = Candidate::new("t1".into(), CandidateSource::Completion, 1.0, true);
        let novel = Candidate::new("t2".into(), CandidateSource::ProviderSearch, 0.0, false);
        let weights = RankingWeights::discover();
        let f1 = Features::compute(&known, &profile);
        let f2 = Features::compute(&novel, &profile);
        // With discover weights (novelty=3.0), the novel track (novelty=1.0)
        // should score higher than the known track (novelty=0.0).
        assert!(score(&f2, &weights) > score(&f1, &weights));
    }

    #[test]
    fn on_repeat_weights_prefer_familiarity() {
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 200,
            },
        ];
        let profile = UserProfile::build_from_events(&events);
        let familiar = Candidate::new("t1".into(), CandidateSource::Completion, 5.0, true);
        let novel = Candidate::new("t2".into(), CandidateSource::ProviderSearch, 0.0, false);
        let weights = RankingWeights::on_repeat();
        let f1 = Features::compute(&familiar, &profile);
        let f2 = Features::compute(&novel, &profile);
        // With on-repeat weights, familiar track should score higher.
        assert!(score(&f1, &weights) > score(&f2, &weights));
    }

    #[test]
    fn rank_sorts_by_score_descending() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t2".into(),
                timestamp: 100,
            },
        ];
        let profile = UserProfile::build_from_events(&events);
        let mut candidates = vec![
            Candidate::new("t2".into(), CandidateSource::Completion, 3.0, true),
            Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true),
        ];
        rank(&mut candidates, &profile, &RankingWeights::default());
        // t1 (liked, higher score) should be first.
        assert_eq!(candidates[0].track_id, "t1");
    }

    #[test]
    fn rank_ties_broken_deterministically() {
        let profile = UserProfile::new();
        let mut candidates = vec![
            Candidate::new("z".into(), CandidateSource::ProviderSearch, 0.0, false),
            Candidate::new("a".into(), CandidateSource::ProviderSearch, 0.0, false),
            Candidate::new("m".into(), CandidateSource::ProviderSearch, 0.0, false),
        ];
        rank(&mut candidates, &profile, &RankingWeights::default());
        // Same score → sorted by track_id ascending.
        assert_eq!(candidates[0].track_id, "a");
        assert_eq!(candidates[1].track_id, "m");
        assert_eq!(candidates[2].track_id, "z");
    }

    #[test]
    fn source_strength_liked_highest() {
        let profile = UserProfile::new();
        let liked = Candidate::new("t1".into(), CandidateSource::Liked, 0.0, true);
        let features = Features::compute(&liked, &profile);
        assert_eq!(features.source_strength, 1.0);
    }

    #[test]
    fn source_strength_experimental_lowest() {
        let profile = UserProfile::new();
        let exp = Candidate::new("t1".into(), CandidateSource::ExperimentalHome, 0.0, false);
        let features = Features::compute(&exp, &profile);
        assert_eq!(features.source_strength, 0.2);
    }
}
