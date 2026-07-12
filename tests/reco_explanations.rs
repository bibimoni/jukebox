//! Integration tests for explanation generation (S2.3.6).
//!
//! Verifies explanation generation from provenance.

use jukebox::reco::candidates::{Candidate, CandidateSource};
use jukebox::reco::explanations::{explain_all, Explanation};

#[test]
fn explanation_from_liked_contains_liked_text() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.reason.contains("liked"));
}

#[test]
fn explanation_from_completion_contains_finish_text() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Completion, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.reason.contains("finish"));
}

#[test]
fn explanation_from_rediscovery_contains_used_to_love() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Rediscovery, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.reason.contains("used to love"));
}

#[test]
fn explanation_from_artist_affinity_contains_artist() {
    let candidate = Candidate::new("t1".into(), CandidateSource::ArtistAffinity, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.reason.contains("artist"));
}

#[test]
fn explanation_from_experimental_home_contains_youtube() {
    let candidate = Candidate::new("t1".into(), CandidateSource::ExperimentalHome, 5.0, false);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.reason.contains("YouTube Music"));
}

#[test]
fn explanation_with_seed_track_has_detail() {
    let candidate = Candidate::new("t1".into(), CandidateSource::PlaylistNeighbors, 5.0, true)
        .with_seed_track("t2".into());
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.detail.is_some());
    assert!(exp.detail.as_ref().unwrap().contains("t2"));
}

#[test]
fn explanation_with_seed_artist_has_detail() {
    let candidate = Candidate::new("t1".into(), CandidateSource::ArtistAffinity, 5.0, true)
        .with_seed_artist("Artist A".into());
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.detail.is_some());
    assert!(exp.detail.as_ref().unwrap().contains("Artist A"));
}

#[test]
fn explanation_without_seed_has_no_detail() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    assert!(exp.detail.is_none());
}

#[test]
fn explanation_as_string_includes_detail_when_present() {
    let candidate = Candidate::new("t1".into(), CandidateSource::ArtistAffinity, 5.0, true)
        .with_seed_artist("Artist A".into());
    let exp = Explanation::from_candidate(&candidate);
    let s = exp.as_string();
    assert!(s.contains("artist"));
    assert!(s.contains("Artist A"));
    assert!(s.contains("("));
}

#[test]
fn explanation_as_string_is_just_reason_without_detail() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    let s = exp.as_string();
    assert!(s.contains("liked"));
    assert!(!s.contains("("));
}

#[test]
fn explanation_display_trait_works() {
    let candidate = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
    let exp = Explanation::from_candidate(&candidate);
    let s = format!("{exp}");
    assert!(s.contains("liked"));
}

#[test]
fn explain_all_returns_explanation_for_each_candidate() {
    let candidates = vec![
        Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true),
        Candidate::new("t2".into(), CandidateSource::Completion, 4.0, true),
        Candidate::new("t3".into(), CandidateSource::Rediscovery, 3.0, true),
    ];
    let explanations = explain_all(&candidates);
    assert_eq!(explanations.len(), 3);
    assert_eq!(explanations[0].0, "t1");
    assert_eq!(explanations[1].0, "t2");
    assert_eq!(explanations[2].0, "t3");
}

#[test]
fn explain_all_empty_returns_empty() {
    let explanations = explain_all(&[]);
    assert!(explanations.is_empty());
}
