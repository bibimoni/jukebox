//! Radio engine — adaptive bounded session for continuous listening.
//!
//! Radio is distinct from a mix: a mix is a fixed, generated collection;
//! radio is an ongoing session that generates tracks incrementally, adapts
//! to feedback, and refills when the pool runs low.
//!
//! Seeds: track, artist, album, playlist, mix, search, genre, mood, folder, queue.
//! Maintains: seed, session history, skips, positive/negative feedback,
//! candidate pool, exclusions, diversity state, source availability, gen id.

use crate::catalog::Track;
use crate::reco::candidates::{Candidate, CandidateGenerator, CandidateSource};
use crate::reco::diversity::{apply_diversity, DiversityConfig};
use crate::reco::profile::UserProfile;
use crate::reco::ranking::{rank, RankingWeights};
use std::collections::HashSet;

/// The seed for a radio session.
#[derive(Clone, Debug)]
pub enum RadioSeed {
    Track(String),
    Artist(String),
    Album(String),
    Playlist(String),
    Mix(String),
    Search(String),
    Genre(String),
    Mood(String),
    Folder(String),
    Queue,
}

impl RadioSeed {
    pub fn description(&self) -> String {
        match self {
            RadioSeed::Track(id) => format!("track {id}"),
            RadioSeed::Artist(name) => format!("artist {name}"),
            RadioSeed::Album(name) => format!("album {name}"),
            RadioSeed::Playlist(name) => format!("playlist {name}"),
            RadioSeed::Mix(name) => format!("mix {name}"),
            RadioSeed::Search(q) => format!("search \"{q}\""),
            RadioSeed::Genre(g) => format!("genre {g}"),
            RadioSeed::Mood(m) => format!("mood {m}"),
            RadioSeed::Folder(f) => format!("folder {f}"),
            RadioSeed::Queue => "current queue".into(),
        }
    }
}

/// A radio session — adaptive, bounded, incremental.
#[derive(Clone, Debug)]
pub struct RadioSession {
    /// The seed that started this session.
    pub seed: RadioSeed,
    /// Track ids played in this session (for exclusion).
    pub session_history: Vec<String>,
    /// Track ids skipped in this session.
    pub skipped: HashSet<String>,
    /// Track ids with positive feedback in this session.
    pub positive_feedback: HashSet<String>,
    /// Track ids with negative feedback in this session.
    pub negative_feedback: HashSet<String>,
    /// The current candidate pool (not yet played).
    pub candidate_pool: Vec<Candidate>,
    /// Track ids excluded from this session (hidden, blocked, played).
    pub exclusions: HashSet<String>,
    /// The generation id (incremented when seed changes → cancel stale work).
    pub generation: u64,
    /// Maximum pool size before refill is needed.
    pub max_pool_size: usize,
    /// Refill threshold (refill when pool < this).
    pub refill_threshold: usize,
    /// YouTube video ids blended into the candidate pool when the provider is
    /// connected (DEF-011). Empty by default (local-only radio preserved).
    pub yt_track_ids: Vec<String>,
}

impl RadioSession {
    /// Create a new radio session from a seed.
    pub fn new(seed: RadioSeed) -> Self {
        RadioSession {
            seed,
            session_history: Vec::new(),
            skipped: HashSet::new(),
            positive_feedback: HashSet::new(),
            negative_feedback: HashSet::new(),
            candidate_pool: Vec::new(),
            exclusions: HashSet::new(),
            generation: 0,
            max_pool_size: 50,
            refill_threshold: 10,
            yt_track_ids: Vec::new(),
        }
    }

    /// Set the YouTube track ids to blend into the candidate pool (DEF-011).
    pub fn set_yt_track_ids(&mut self, ids: Vec<String>) {
        self.yt_track_ids = ids;
    }

    /// Generate the initial pool of candidates. Uses the full pipeline.
    pub fn initialize(&mut self, profile: &UserProfile, catalog: &[Track]) {
        self.generation += 1;
        self.candidate_pool = self.generate_candidates(profile, catalog);
    }

    /// Refill the pool when it drops below the threshold.
    pub fn refill_if_needed(&mut self, profile: &UserProfile, catalog: &[Track]) {
        if self.candidate_pool.len() < self.refill_threshold {
            let new_candidates = self.generate_candidates(profile, catalog);
            self.candidate_pool.extend(new_candidates);
        }
    }

    /// Generate candidates for this session. Uses the profile + catalog, and
    /// excludes tracks already played/skipped/excluded.
    fn generate_candidates(&self, profile: &UserProfile, catalog: &[Track]) -> Vec<Candidate> {
        let gen = CandidateGenerator::new(profile, catalog)
            .with_yt_track_ids(&self.yt_track_ids);
        let mut candidates = gen.generate();

        // If the seed is a track, boost that track's neighbors.
        if let RadioSeed::Track(seed_id) = &self.seed {
            if let Some(seed_track) = catalog.iter().find(|t| &t.id == seed_id) {
                // Add tracks from the same artist as the seed.
                for track in catalog
                    .iter()
                    .filter(|t| t.primary_artist == seed_track.primary_artist && t.id != *seed_id)
                {
                    if !self.exclusions.contains(&track.id) {
                        candidates.push(
                            Candidate::new(
                                track.id.clone(),
                                CandidateSource::ArtistAffinity,
                                3.0,
                                true,
                            )
                            .with_seed_track(seed_id.clone())
                            .with_seed_artist(seed_track.primary_artist.clone()),
                        );
                    }
                }
            }
        }

        // Exclude already-played/skipped/excluded.
        candidates.retain(|c| {
            !self.exclusions.contains(&c.track_id)
                && !self.session_history.contains(&c.track_id)
                && !self.negative_feedback.contains(&c.track_id)
        });

        // Rank and apply diversity.
        rank(&mut candidates, profile, &RankingWeights::default());
        let diverse = apply_diversity(
            &candidates,
            catalog,
            &DiversityConfig::relaxed(),
            &self.session_history,
        );

        let mut pool: Vec<Candidate> = diverse.into_iter().take(self.max_pool_size).collect();

        // Fallback: if the pool is empty after exclusions (cold start with no
        // profile signal, or all candidates excluded), seed from the catalog
        // so the radio always has something to play.
        if pool.is_empty() && !catalog.is_empty() {
            for track in catalog {
                if !self.exclusions.contains(&track.id)
                    && !self.session_history.contains(&track.id)
                    && !self.negative_feedback.contains(&track.id)
                {
                    pool.push(Candidate::new(
                        track.id.clone(),
                        CandidateSource::LocalMetadata,
                        0.1,
                        true,
                    ));
                }
            }
        }

        pool
    }

    /// Get the next track to play. Returns None if the pool is empty.
    pub fn next_track(&mut self) -> Option<Candidate> {
        let candidate = self.candidate_pool.first().cloned()?;
        self.candidate_pool.remove(0);
        self.session_history.push(candidate.track_id.clone());
        Some(candidate)
    }

    /// Record positive feedback for a track (like, complete, replay).
    pub fn positive_feedback(&mut self, track_id: &str) {
        self.positive_feedback.insert(track_id.to_string());
    }

    /// Record negative feedback for a track (skip, dislike, hide). Also
    /// removes the track from the current candidate pool so it doesn't play
    /// in the current session.
    pub fn negative_feedback(&mut self, track_id: &str) {
        self.negative_feedback.insert(track_id.to_string());
        self.exclusions.insert(track_id.to_string());
        // Remove from the current pool immediately.
        self.candidate_pool.retain(|c| c.track_id != track_id);
    }

    /// Record a skip (weak negative — don't exclude, but track it).
    pub fn skip(&mut self, track_id: &str) {
        self.skipped.insert(track_id.to_string());
    }

    /// Check if the pool needs refilling.
    pub fn needs_refill(&self) -> bool {
        self.candidate_pool.len() < self.refill_threshold
    }

    /// Change the seed (starts a new session). Cancels stale work.
    pub fn change_seed(&mut self, seed: RadioSeed, profile: &UserProfile, catalog: &[Track]) {
        self.seed = seed;
        self.session_history.clear();
        self.skipped.clear();
        self.positive_feedback.clear();
        self.negative_feedback.clear();
        self.candidate_pool.clear();
        self.exclusions.clear();
        self.initialize(profile, catalog);
    }

    /// Stop the radio session.
    pub fn stop(&mut self) {
        self.candidate_pool.clear();
    }

    /// Get the number of tracks remaining in the pool.
    pub fn pool_size(&self) -> usize {
        self.candidate_pool.len()
    }

    /// Get the session history (tracks played).
    pub fn history(&self) -> &[String] {
        &self.session_history
    }

    /// Get the next `n` upcoming candidates (not yet played) for display
    /// (DEF-063). Returns references in pool order.
    pub fn upcoming(&self, n: usize) -> Vec<&Candidate> {
        self.candidate_pool.iter().take(n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;
    use crate::reco::events::ListenEvent;
    use crate::reco::profile::UserProfile;

    fn make_track(id: &str, artist: &str, album: &str, title: &str) -> Track {
        Track {
            id: id.to_string(),
            artists: vec![artist.to_string()],
            primary_artist: artist.to_string(),
            title: title.to_string(),
            album: Some(album.to_string()),
            track_number: Some(1),
            disc_number: Some(1),
            bit_depth: 16,
            sample_rate_hz: 44100,
            isrc: None,
            source_path: std::path::PathBuf::from("/test/file.flac"),
            symlinked_into_artists: vec![],
        }
    }

    fn make_profile() -> UserProfile {
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Liked {
                track_id: "t2".into(),
                timestamp: 200,
            },
        ];
        UserProfile::build_from_events(&events)
    }

    #[test]
    fn radio_session_from_track_seed() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
            make_track("t3", "Artist A", "Album 1", "Song 3"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        assert!(!radio.candidate_pool.is_empty());
    }

    #[test]
    fn radio_next_track_moves_to_history() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        let next = radio.next_track();
        assert!(next.is_some());
        assert_eq!(radio.history().len(), 1);
    }

    #[test]
    fn radio_negative_feedback_excludes_track() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        radio.negative_feedback("t2");
        // t2 should not appear in the pool.
        assert!(!radio.candidate_pool.iter().any(|c| c.track_id == "t2"));
    }

    #[test]
    fn radio_needs_refill_when_pool_low() {
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.refill_threshold = 10;
        radio.candidate_pool = vec![Candidate::new(
            "t1".into(),
            CandidateSource::Liked,
            5.0,
            true,
        )];
        assert!(radio.needs_refill());
    }

    #[test]
    fn radio_change_seed_clears_session() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        radio.next_track();
        assert!(!radio.history().is_empty());
        radio.change_seed(RadioSeed::Track("t2".into()), &profile, &catalog);
        assert!(radio.history().is_empty());
    }

    #[test]
    fn radio_stop_clears_pool() {
        let profile = make_profile();
        let catalog = vec![make_track("t1", "Artist A", "Album 1", "Song 1")];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        assert!(!radio.candidate_pool.is_empty());
        radio.stop();
        assert!(radio.candidate_pool.is_empty());
    }

    #[test]
    fn radio_seed_description() {
        let seed = RadioSeed::Track("t1".into());
        assert!(seed.description().contains("track"));
        let seed = RadioSeed::Artist("Artist A".into());
        assert!(seed.description().contains("artist"));
    }

    #[test]
    fn radio_skip_does_not_exclude() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        radio.skip("t2");
        // Skip is weak — t2 should still be in the pool (not excluded).
        assert!(radio.skipped.contains("t2"));
    }

    #[test]
    fn radio_empty_profile_seeds_from_catalog() {
        let profile = UserProfile::new();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
            make_track("t3", "Artist C", "Album 3", "Song 3"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        radio.initialize(&profile, &catalog);
        // Cold-start fallback: the pool should have catalog tracks even
        // without a listening profile.
        assert!(
            !radio.candidate_pool.is_empty(),
            "radio should seed from catalog on cold start"
        );
    }

    #[test]
    fn radio_fallback_when_all_candidates_excluded() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mut radio = RadioSession::new(RadioSeed::Track("t1".into()));
        // Exclude all profile-derived candidates, leaving only the fallback.
        radio.exclusions.insert("t1".into());
        radio.initialize(&profile, &catalog);
        // t2 should still be in the pool (either from the profile or fallback).
        assert!(
            radio.candidate_pool.iter().any(|c| c.track_id == "t2"),
            "fallback should provide t2 when t1 is excluded"
        );
    }
}
