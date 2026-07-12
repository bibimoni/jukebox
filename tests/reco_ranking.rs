//! Integration tests for ranking (reco::ranking).
//!
//! Verifies Features computation for known/unknown tracks, score ordering,
//! discover weights preferring novelty, on_repeat weights preferring
//! familiarity, rank sorting by score descending, and deterministic tie-breaking.

use jukebox::reco::candidates::{Candidate, CandidateSource};
use jukebox::reco::events::{EventContext, EventSource, ListenEvent};
use jukebox::reco::profile::UserProfile;
use jukebox::reco::ranking::{rank, score, Features, RankingWeights};

#[test]
fn features_compute_for_known_track() {
    let events = vec![
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: 100,
            context: EventContext::Album,
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
            source: EventSource::Local,
            timestamp: 100,
            context: EventContext::Album,
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 200,
        },
    ];
    let profile = UserProfile::build_from_events(&events);
    let known = Candidate::new("t1".into(), CandidateSource::Completion, 1.0, true);
    let novel = Candidate::new("t2".into(), CandidateSource::ProviderSearch, 0.0, false);
    let weights = RankingWeights::discover();
    let f1 = Features::compute(&known, &profile);
    let f2 = Features::compute(&novel, &profile);
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
    assert_eq!(candidates[0].track_id, "a");
    assert_eq!(candidates[1].track_id, "m");
    assert_eq!(candidates[2].track_id, "z");
}

#[test]
fn source_strength_liked_is_highest() {
    let profile = UserProfile::new();
    let liked = Candidate::new("t1".into(), CandidateSource::Liked, 0.0, true);
    let features = Features::compute(&liked, &profile);
    assert_eq!(features.source_strength, 1.0);
}

#[test]
fn source_strength_experimental_is_lowest() {
    let profile = UserProfile::new();
    let exp = Candidate::new("t1".into(), CandidateSource::ExperimentalHome, 0.0, false);
    let features = Features::compute(&exp, &profile);
    assert_eq!(features.source_strength, 0.2);
}

#[test]
fn discover_weights_have_higher_novelty_than_default() {
    let discover = RankingWeights::discover();
    let default = RankingWeights::default();
    assert!(discover.novelty > default.novelty);
}

#[test]
fn on_repeat_weights_have_higher_familiarity_than_default() {
    let on_repeat = RankingWeights::on_repeat();
    let default = RankingWeights::default();
    assert!(on_repeat.familiarity > default.familiarity);
}

#[test]
fn rediscover_weights_have_higher_recency_than_default() {
    let rediscover = RankingWeights::rediscover();
    let default = RankingWeights::default();
    assert!(rediscover.recency > default.recency);
}

#[test]
fn score_is_weighted_sum() {
    let features = Features {
        affinity: 1.0,
        novelty: 2.0,
        recency: 3.0,
        familiarity: 4.0,
        source_balance: 0.0,
        source_strength: 5.0,
    };
    let weights = RankingWeights {
        affinity: 1.0,
        novelty: 1.0,
        recency: 1.0,
        familiarity: 1.0,
        source_strength: 1.0,
    };
    // score = 1*1 + 2*1 + 3*1 + 4*1 + 5*1 = 15
    assert_eq!(score(&features, &weights), 15.0);
}
