//! Candidate generation — the first stage of the recommendation pipeline.
//!
//! Generates candidates from multiple independent sources, each tagged with
//! its provenance so the explanation generator can answer "why this track?"
//!
//! ## Pipeline position
//!
//! ```text
//! Context → [Candidate Generation] → Normalization → Music-Content
//! Validation → Identity Resolution → Eligibility → Feature Computation →
//! Ranking → Diversity → Explanations → Mix/Radio Assembly → Feedback → Eval
//! ```
//!
//! ## Sources
//!
//! Each source is an independent generator. The pipeline runs all applicable
//! sources and merges their output (deduping by canonical id). Sources are
//! ordered by signal strength: liked > completion > repeated > playlist
//! neighbors > artist affinity > local neighbors > search > rediscovery >
//! experimental.

use crate::catalog::Track;
#[cfg(test)]
use crate::reco::events::EventContext;
use crate::reco::events::ListenEvent;
use crate::reco::profile::UserProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Which candidate source produced this track. Used for provenance and
/// explanation generation.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CandidateSource {
    /// From the user's liked tracks.
    Liked,
    /// From high-completion tracks (played >75%).
    Completion,
    /// From repeated tracks (multiple plays).
    Repeated,
    /// From tracks in the same playlist as a positive track.
    PlaylistNeighbors,
    /// From high-affinity artists.
    ArtistAffinity,
    /// From local catalog metadata neighbors (same genre, similar tags).
    LocalMetadata,
    /// From existing user playlists.
    ExistingPlaylists,
    /// From provider search results.
    ProviderSearch,
    /// New releases from high-affinity artists.
    NewFromArtists,
    /// Previously positive tracks not heard recently.
    Rediscovery,
    /// Explicitly provided by the user.
    ExplicitSeed,
    /// From the experimental provider (ytmusicapi get_home).
    ExperimentalHome,
}

impl CandidateSource {
    /// A human-readable description for explanation generation.
    pub fn description(&self) -> &'static str {
        match self {
            CandidateSource::Liked => "from your liked tracks",
            CandidateSource::Completion => "tracks you often finish",
            CandidateSource::Repeated => "tracks you play repeatedly",
            CandidateSource::PlaylistNeighbors => "from a playlist you enjoy",
            CandidateSource::ArtistAffinity => "from an artist you listen to often",
            CandidateSource::LocalMetadata => "similar to your local favorites",
            CandidateSource::ExistingPlaylists => "from your playlists",
            CandidateSource::ProviderSearch => "from a search you did",
            CandidateSource::NewFromArtists => "new from an artist you like",
            CandidateSource::Rediscovery => "a track you used to love",
            CandidateSource::ExplicitSeed => "a track you selected",
            CandidateSource::ExperimentalHome => "from YouTube Music's home feed",
        }
    }
}

/// A candidate track with provenance. The pipeline carries the source all the
/// way to the explanation generator so every recommendation can answer "why?"
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The track id — either a local catalog id or a YouTube video_id.
    pub track_id: String,
    /// Which source produced this candidate.
    pub source: CandidateSource,
    /// The affinity score from the user profile (used for ranking).
    pub affinity: f64,
    /// Whether this candidate is a local track or a YouTube track.
    pub is_local: bool,
    /// Optional: the seed track that led to this candidate (for playlist
    /// neighbors, artist affinity, etc.).
    pub seed_track_id: Option<String>,
    /// Optional: the seed artist that led to this candidate.
    pub seed_artist: Option<String>,
}

impl Candidate {
    /// Create a new candidate with the given source and affinity.
    pub fn new(track_id: String, source: CandidateSource, affinity: f64, is_local: bool) -> Self {
        Self {
            track_id,
            source,
            affinity,
            is_local,
            seed_track_id: None,
            seed_artist: None,
        }
    }

    /// Set the seed track that led to this candidate.
    pub fn with_seed_track(mut self, seed: String) -> Self {
        self.seed_track_id = Some(seed);
        self
    }

    /// Set the seed artist that led to this candidate.
    pub fn with_seed_artist(mut self, artist: String) -> Self {
        self.seed_artist = Some(artist);
        self
    }
}

/// The candidate generator. Runs all applicable sources and merges output.
pub struct CandidateGenerator<'a> {
    profile: &'a UserProfile,
    catalog: &'a [Track],
    /// YouTube video ids available as candidates when the provider is
    /// connected (DEF-010/DEF-011). Empty by default (local-only behavior
    /// preserved for existing callers); set via [`with_yt_track_ids`].
    yt_track_ids: &'a [String],
}

#[allow(clippy::wrong_self_convention)]
impl<'a> CandidateGenerator<'a> {
    /// Create a new generator. `profile` is the user's listening profile;
    /// `catalog` is the local music catalog (for local-track candidates).
    pub fn new(profile: &'a UserProfile, catalog: &'a [Track]) -> Self {
        Self {
            profile,
            catalog,
            yt_track_ids: &[],
        }
    }

    /// Blend YouTube video ids into the candidate pool (DEF-010/DEF-011).
    /// Each id becomes a non-local candidate tagged
    /// [`CandidateSource::ExperimentalHome`]. Call this only when the
    /// YouTube provider is connected (`YtState::is_ready`); the empty
    /// default keeps existing callers local-only.
    pub fn with_yt_track_ids(mut self, ids: &'a [String]) -> Self {
        self.yt_track_ids = ids;
        self
    }

    /// Generate all candidates from all applicable sources. Returns a deduped
    /// list (by track_id). Hidden tracks and blocked artists are excluded.
    pub fn generate(&self) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        // Run each source, dedup as we go.
        self.from_liked(&mut candidates, &mut seen);
        self.from_completion(&mut candidates, &mut seen);
        self.from_repeated(&mut candidates, &mut seen);
        self.from_artist_affinity(&mut candidates, &mut seen);
        self.from_rediscovery(&mut candidates, &mut seen);
        self.from_existing_playlists(&mut candidates, &mut seen);
        self.from_local_metadata(&mut candidates, &mut seen);
        // YouTube tracks (when connected) — blended after local sources so
        // local affinity still drives ranking, but the pool isn't local-only.
        self.from_yt_tracks(&mut candidates, &mut seen);

        // Cold-start fallback: when the profile is empty (no listening
        // history), seed candidates from the local catalog so the
        // recommendation pipeline always has material to rank and diversify.
        if candidates.is_empty() && self.profile.is_empty() && !self.catalog.is_empty() {
            self.from_catalog_fallback(&mut candidates, &mut seen);
        }

        candidates
    }

    /// Generate candidates from YouTube video ids (the connected provider's
    /// track_cache + playlist track_ids). Each id becomes a weak non-local
    /// candidate so the ranking/diversity stages can blend local + YouTube
    /// tracks (DEF-010/DEF-011). Eligibility reuses the profile's hidden /
    /// blocked-artist filters; YouTube tracks have no catalog entry so the
    /// blocked-artist check is skipped for them.
    fn from_yt_tracks(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        for id in self.yt_track_ids {
            if seen.contains(id) || self.profile.is_hidden(id) {
                continue;
            }
            seen.insert((*id).clone());
            candidates.push(Candidate::new(
                (*id).clone(),
                CandidateSource::ExperimentalHome,
                // Weak affinity so local profile signal still ranks above
                // unprofiled YouTube tracks; the diversity stage spreads them.
                self.profile.track_score(id).max(0.05),
                false,
            ));
        }
    }

    /// Cold-start fallback: seed candidates from the entire local catalog
    /// when the user has no listening history. Every eligible track becomes a
    /// weak candidate with a default affinity score so the ranking and
    /// diversity stages have material to work with.
    fn from_catalog_fallback(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        for track in self.catalog {
            if self.is_eligible(&track.id) && !seen.contains(&track.id) {
                seen.insert(track.id.clone());
                candidates.push(Candidate::new(
                    track.id.clone(),
                    CandidateSource::LocalMetadata,
                    0.1,
                    true,
                ));
            }
        }
    }

    /// Generate candidates from liked tracks. These are the strongest seeds.
    fn from_liked(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        for track_id in self.profile.top_liked(50) {
            if self.is_eligible(&track_id) && !seen.contains(&track_id) {
                seen.insert(track_id.clone());
                candidates.push(Candidate::new(
                    track_id.clone(),
                    CandidateSource::Liked,
                    self.profile.track_score(&track_id),
                    self.is_local_track(&track_id),
                ));
            }
        }
    }

    /// Generate candidates from high-completion tracks (played >75%).
    fn from_completion(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        for (track_id, tp) in self.profile.top_tracks(50) {
            if tp.completion_count > 0 && self.is_eligible(track_id) && !seen.contains(track_id) {
                seen.insert(track_id.to_string());
                candidates.push(Candidate::new(
                    track_id.to_string(),
                    CandidateSource::Completion,
                    tp.score,
                    self.is_local_track(track_id),
                ));
            }
        }
    }

    /// Generate candidates from repeated tracks (play_count > 2).
    fn from_repeated(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        for (track_id, tp) in self.profile.top_tracks(50) {
            if tp.play_count > 2 && self.is_eligible(track_id) && !seen.contains(track_id) {
                seen.insert(track_id.to_string());
                candidates.push(Candidate::new(
                    track_id.to_string(),
                    CandidateSource::Repeated,
                    tp.score,
                    self.is_local_track(track_id),
                ));
            }
        }
    }

    /// Generate candidates from high-affinity artists' other tracks.
    fn from_artist_affinity(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        // For each high-affinity artist, find their other tracks in the catalog.
        let mut artist_scores: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for track in self.catalog {
            let tp = self.profile.tracks.get(&track.id);
            if let Some(tp) = tp {
                if tp.score > 0.0 {
                    *artist_scores
                        .entry(track.primary_artist.clone())
                        .or_default() += tp.score;
                }
            }
        }
        for track in self.catalog {
            let artist = track.primary_artist.clone();
            if let Some(score) = artist_scores.get(&artist) {
                if *score > 5.0 && self.is_eligible(&track.id) && !seen.contains(&track.id) {
                    seen.insert(track.id.clone());
                    candidates.push(
                        Candidate::new(
                            track.id.clone(),
                            CandidateSource::ArtistAffinity,
                            *score * 0.5,
                            true,
                        )
                        .with_seed_artist(artist.clone()),
                    );
                }
            }
        }
    }

    /// Generate rediscovery candidates: positive tracks not played recently.
    fn from_rediscovery(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        let now = ListenEvent::now();
        let thirty_days_ago = now.saturating_sub(30u64 * 24 * 60 * 60);
        for (track_id, tp) in self.profile.top_tracks(100) {
            // Positive score but not played in 30+ days.
            if tp.score > 0.0
                && tp.last_played.map(|t| t < thirty_days_ago).unwrap_or(true)
                && self.is_eligible(track_id)
                && !seen.contains(track_id)
            {
                seen.insert(track_id.to_string());
                candidates.push(Candidate::new(
                    track_id.to_string(),
                    CandidateSource::Rediscovery,
                    tp.score * 0.7,
                    self.is_local_track(track_id),
                ));
            }
        }
    }

    /// Generate candidates from existing user playlists (local playlists).
    fn from_existing_playlists(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        // This source uses the local catalog to find tracks in playlists that
        // contain other positive tracks. The actual playlist data is in
        // App::playlists; for now we generate from catalog tracks that have
        // positive scores (a simplified heuristic).
        for track in self.catalog {
            if let Some(tp) = self.profile.tracks.get(&track.id) {
                if tp.score > 0.0 && self.is_eligible(&track.id) && !seen.contains(&track.id) {
                    seen.insert(track.id.clone());
                    candidates.push(Candidate::new(
                        track.id.clone(),
                        CandidateSource::ExistingPlaylists,
                        tp.score * 0.6,
                        true,
                    ));
                }
            }
        }
    }

    /// Generate candidates from local metadata neighbors (same primary artist).
    fn from_local_metadata(&self, candidates: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
        // Find artists with positive scores and add their other catalog tracks.
        let mut artist_scores: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for track in self.catalog {
            if let Some(tp) = self.profile.tracks.get(&track.id) {
                if tp.score > 0.0 {
                    *artist_scores
                        .entry(track.primary_artist.clone())
                        .or_default() += tp.score;
                }
            }
        }
        for track in self.catalog {
            let artist = track.primary_artist.clone();
            if let Some(score) = artist_scores.get(&artist) {
                if *score > 2.0 && self.is_eligible(&track.id) && !seen.contains(&track.id) {
                    seen.insert(track.id.clone());
                    candidates.push(
                        Candidate::new(
                            track.id.clone(),
                            CandidateSource::LocalMetadata,
                            *score * 0.4,
                            true,
                        )
                        .with_seed_artist(artist.clone()),
                    );
                }
            }
        }
    }

    /// Check if a track is eligible for recommendation (not hidden, not by a
    /// blocked artist). The caller is responsible for availability checks
    /// (local file existence, YouTube video availability).
    fn is_eligible(&self, track_id: &str) -> bool {
        if self.profile.is_hidden(track_id) {
            return false;
        }
        // Check if the track's artist is blocked.
        if let Some(track) = self.catalog.iter().find(|t| t.id == track_id) {
            if self.profile.is_blocked(&track.primary_artist) {
                return false;
            }
        }
        true
    }

    /// Check if a track id is a local catalog track (vs a YouTube video_id).
    fn is_local_track(&self, track_id: &str) -> bool {
        self.catalog.iter().any(|t| t.id == track_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;

    fn make_track(id: &str, artist: &str, title: &str) -> Track {
        Track {
            id: id.to_string(),
            artists: vec![artist.to_string()],
            primary_artist: artist.to_string(),
            title: title.to_string(),
            album: Some("Test Album".to_string()),
            track_number: Some(1),
            disc_number: Some(1),
            bit_depth: 16,
            sample_rate_hz: 44100,
            isrc: None,
            source_path: std::path::PathBuf::from("/test/file.flac"),
            symlinked_into_artists: vec![],
        }
    }

    fn make_profile_with_events(events: Vec<ListenEvent>) -> UserProfile {
        UserProfile::build_from_events(&events)
    }

    #[test]
    fn empty_profile_falls_back_to_catalog() {
        let profile = UserProfile::new();
        let catalog: Vec<Track> = vec![
            make_track("t1", "Artist A", "Song A"),
            make_track("t2", "Artist B", "Song B"),
        ];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        // Cold-start fallback: every catalog track is a weak candidate.
        assert_eq!(
            candidates.len(),
            2,
            "empty profile should seed from catalog"
        );
        assert!(candidates.iter().all(|c| c.is_local));
        assert!(candidates.iter().all(|c| c.affinity > 0.0));
    }

    #[test]
    fn empty_profile_with_empty_catalog_generates_nothing() {
        let profile = UserProfile::new();
        let catalog: Vec<Track> = vec![];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates.is_empty(), "no catalog → no fallback candidates");
    }

    #[test]
    fn liked_tracks_generate_candidates() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 200,
            },
        ];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates.iter().any(|c| c.track_id == "t1"));
        assert!(candidates
            .iter()
            .any(|c| c.source == CandidateSource::Liked));
    }

    #[test]
    fn completion_tracks_generate_candidates() {
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
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates
            .iter()
            .any(|c| c.source == CandidateSource::Completion));
    }

    #[test]
    fn hidden_tracks_are_excluded() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Hidden {
                track_id: "t1".into(),
                timestamp: 200,
            },
        ];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(!candidates.iter().any(|c| c.track_id == "t1"));
    }

    #[test]
    fn blocked_artist_tracks_are_excluded() {
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::ArtistBlocked {
                artist: "Artist A".into(),
                timestamp: 200,
            },
        ];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(!candidates.iter().any(|c| c.track_id == "t1"));
    }

    #[test]
    fn candidates_are_deduped_by_track_id() {
        let events = vec![
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 200,
            },
            ListenEvent::Replayed {
                track_id: "t1".into(),
                timestamp: 300,
            },
        ];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        let t1_count = candidates.iter().filter(|c| c.track_id == "t1").count();
        assert_eq!(t1_count, 1, "track should appear only once (deduped)");
    }

    #[test]
    fn artist_affinity_generates_candidates() {
        // Two tracks by same artist, both positive → other tracks by that
        // artist should be candidates.
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t2".into(),
                timestamp: 200,
            },
        ];
        let profile = make_profile_with_events(events);
        let catalog = vec![
            make_track("t1", "Artist A", "Song A"),
            make_track("t2", "Artist A", "Song B"),
            make_track("t3", "Artist A", "Song C"), // not played, but same artist
        ];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates.iter().any(|c| c.track_id == "t3"));
    }

    #[test]
    fn rediscovery_generates_for_old_positive_tracks() {
        let now = ListenEvent::now();
        let old_timestamp = now.saturating_sub(60u64 * 24 * 60 * 60); // 60 days ago
                                                                      // Use Replayed (not Liked or Completed) so from_liked/from_completion
                                                                      // don't catch it first. Replayed gives a positive score but doesn't
                                                                      // increment play_count or completion_count.
        let events = vec![
            ListenEvent::Replayed {
                track_id: "t1".into(),
                timestamp: old_timestamp,
            },
            ListenEvent::Replayed {
                track_id: "t1".into(),
                timestamp: old_timestamp + 100,
            },
        ];
        let profile = make_profile_with_events(events);
        // Track "t1" is NOT in the catalog, so from_existing_playlists and
        // from_local_metadata won't catch it. from_rediscovery will.
        let catalog = vec![
            make_track("other1", "Artist B", "Song B"),
            make_track("other2", "Artist C", "Song C"),
        ];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(
            candidates
                .iter()
                .any(|c| c.source == CandidateSource::Rediscovery),
            "old positive track should generate rediscovery candidate"
        );
    }

    #[test]
    fn candidate_source_description_is_human_readable() {
        assert!(CandidateSource::Liked.description().contains("liked"));
        assert!(CandidateSource::Rediscovery
            .description()
            .contains("used to love"));
        assert!(CandidateSource::ArtistAffinity
            .description()
            .contains("artist"));
    }

    #[test]
    fn candidate_carries_provenance() {
        let events = vec![ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        }];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        let c = candidates.iter().find(|c| c.track_id == "t1").unwrap();
        assert_eq!(c.source, CandidateSource::Liked);
        assert!(c.affinity > 0.0);
        assert!(c.is_local);
    }

    #[test]
    fn track_started_alone_does_not_generate() {
        // TrackStarted alone is not positive — no candidates should be generated.
        let events = vec![ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: crate::reco::events::EventSource::Local,
            timestamp: 100,
            context: EventContext::Album,
        }];
        let profile = make_profile_with_events(events);
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates.is_empty());
    }

    #[test]
    fn yt_track_ids_blend_into_pool() {
        // DEF-010/DEF-011: when YouTube track ids are provided, they appear
        // in the candidate pool as non-local candidates.
        let profile = UserProfile::new();
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let yt_ids = vec!["yt1".to_string(), "yt2".to_string()];
        let gen = CandidateGenerator::new(&profile, &catalog).with_yt_track_ids(&yt_ids);
        let candidates = gen.generate();
        let yt_candidates: Vec<_> = candidates.iter().filter(|c| !c.is_local).collect();
        assert_eq!(yt_candidates.len(), 2, "both YouTube ids should be candidates");
        assert!(yt_candidates.iter().all(|c| matches!(c.source, CandidateSource::ExperimentalHome)));
    }

    #[test]
    fn yt_tracks_default_empty_preserves_local_only() {
        // Existing callers (no with_yt_track_ids) get no YouTube candidates.
        let profile = UserProfile::new();
        let catalog = vec![make_track("t1", "Artist A", "Song A")];
        let gen = CandidateGenerator::new(&profile, &catalog);
        let candidates = gen.generate();
        assert!(candidates.iter().all(|c| c.is_local), "default pool is local-only");
    }

    #[test]
    fn yt_hidden_tracks_excluded() {
        let mut profile = UserProfile::new();
        crate::reco::feedback::apply_feedback(
            &crate::reco::feedback::FeedbackAction::HideTrack,
            "yt1",
            None,
            &mut profile,
        );
        let catalog: Vec<Track> = vec![];
        let yt_ids = vec!["yt1".to_string(), "yt2".to_string()];
        let gen = CandidateGenerator::new(&profile, &catalog).with_yt_track_ids(&yt_ids);
        let candidates = gen.generate();
        assert!(!candidates.iter().any(|c| c.track_id == "yt1"), "hidden yt1 excluded");
        assert!(candidates.iter().any(|c| c.track_id == "yt2"), "yt2 still present");
    }
}
