//! Explanation generation — human-readable reasons for recommendations.
//!
//! Every recommendation must have explainable provenance. Explanations are
//! derived from the candidate's source and seed information, NOT fabricated.
//! Examples:
//! - "From your 'Night Drive' playlist"
//! - "Similar to the current track"
//! - "New from an artist you often finish"
//! - "A local favorite not played recently"
//! - "Included to increase discovery"
//! - "Supplied by the experimental YouTube Music provider"

use crate::reco::candidates::Candidate;

/// A human-readable explanation for why a track was recommended.
#[derive(Clone, Debug)]
pub struct Explanation {
    /// The main reason text (e.g., "From your liked tracks").
    pub reason: String,
    /// Optional detail (e.g., "You've played this track 5 times").
    pub detail: Option<String>,
}

impl Explanation {
    /// Generate an explanation from a candidate's provenance.
    pub fn from_candidate(candidate: &Candidate) -> Self {
        let reason = candidate.source.description().to_string();

        let detail = match &candidate.seed_track_id {
            Some(seed) => Some(format!("seeded by track {seed}")),
            None => candidate
                .seed_artist
                .as_ref()
                .map(|artist| format!("from artist {artist}")),
        };

        Explanation { reason, detail }
    }

    /// Format the explanation as a single string.
    pub fn as_string(&self) -> String {
        match &self.detail {
            Some(d) => format!("{} ({})", self.reason, d),
            None => self.reason.clone(),
        }
    }
}

impl std::fmt::Display for Explanation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_string())
    }
}

/// Generate explanations for a list of candidates.
pub fn explain_all(candidates: &[Candidate]) -> Vec<(String, Explanation)> {
    candidates
        .iter()
        .map(|c| (c.track_id.clone(), Explanation::from_candidate(c)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reco::candidates::{Candidate, CandidateSource};

    #[test]
    fn explanation_from_liked() {
        let candidate = Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true);
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.reason.contains("liked"));
    }

    #[test]
    fn explanation_from_completion() {
        let candidate = Candidate::new("t1".into(), CandidateSource::Completion, 5.0, true);
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.reason.contains("finish"));
    }

    #[test]
    fn explanation_from_rediscovery() {
        let candidate = Candidate::new("t1".into(), CandidateSource::Rediscovery, 5.0, true);
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.reason.contains("used to love"));
    }

    #[test]
    fn explanation_with_seed_track() {
        let candidate = Candidate::new("t1".into(), CandidateSource::PlaylistNeighbors, 5.0, true)
            .with_seed_track("t2".into());
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.detail.is_some());
        assert!(exp.detail.as_ref().unwrap().contains("t2"));
    }

    #[test]
    fn explanation_with_seed_artist() {
        let candidate = Candidate::new("t1".into(), CandidateSource::ArtistAffinity, 5.0, true)
            .with_seed_artist("Artist A".into());
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.detail.is_some());
        assert!(exp.detail.as_ref().unwrap().contains("Artist A"));
    }

    #[test]
    fn explanation_to_string_includes_detail() {
        let candidate = Candidate::new("t1".into(), CandidateSource::ArtistAffinity, 5.0, true)
            .with_seed_artist("Artist A".into());
        let exp = Explanation::from_candidate(&candidate);
        let s = exp.as_string();
        assert!(s.contains("artist"));
        assert!(s.contains("Artist A"));
    }

    #[test]
    fn explanation_from_experimental_home() {
        let candidate = Candidate::new("t1".into(), CandidateSource::ExperimentalHome, 5.0, false);
        let exp = Explanation::from_candidate(&candidate);
        assert!(exp.reason.contains("YouTube Music"));
    }

    #[test]
    fn explain_all_returns_all() {
        let candidates = vec![
            Candidate::new("t1".into(), CandidateSource::Liked, 5.0, true),
            Candidate::new("t2".into(), CandidateSource::Completion, 4.0, true),
        ];
        let explanations = explain_all(&candidates);
        assert_eq!(explanations.len(), 2);
        assert_eq!(explanations[0].0, "t1");
        assert_eq!(explanations[1].0, "t2");
    }
}
