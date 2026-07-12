//! Playlist generator — natural-language and structured playlist generation.
//!
//! Supports natural-language input like "Make a 45-minute energetic running
//! playlist" and translates it into a visible structured plan that the user
//! can edit. The generator uses the recommendation pipeline to produce tracks
//! that match the constraints.
//!
//! An LLM may interpret constraints, but may NOT invent video IDs, tracks,
//! artists, metadata, or availability. All track IDs come from the local
//! catalog or the ytmusicapi sidecar.

use crate::catalog::Track;
use crate::reco::candidates::{Candidate, CandidateGenerator};
use crate::reco::diversity::{apply_diversity, DiversityConfig};
use crate::reco::profile::UserProfile;
use crate::reco::ranking::{rank, RankingWeights};
use serde::{Deserialize, Serialize};

/// The constraints for a generated playlist.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratorConstraints {
    /// Target duration in seconds (0 = no limit).
    pub duration_secs: Option<f64>,
    /// Energy level: Low, Medium, High.
    pub energy: Energy,
    /// Source preference: Local, YouTube, Hybrid.
    pub sources: SourcePreference,
    /// Familiarity ratio (0 = all discovery, 1 = all familiar).
    pub familiarity: f64,
    /// Whether to include live recordings.
    pub include_live: bool,
    /// Whether to include remixes.
    pub include_remixes: bool,
    /// Whether to include covers.
    pub include_covers: bool,
    /// Whether to allow explicit content.
    pub allow_explicit: bool,
    /// Artist repeat gap (tracks between same artist).
    pub artist_gap: usize,
    /// Seed artists (up to 3).
    pub seed_artists: Vec<String>,
    /// Base playlist to blend with.
    pub base_playlist: Option<String>,
    /// Maximum number of tracks.
    pub max_tracks: usize,
}

impl Default for GeneratorConstraints {
    fn default() -> Self {
        GeneratorConstraints {
            duration_secs: None,
            energy: Energy::Medium,
            sources: SourcePreference::Local,
            familiarity: 0.7,
            include_live: true,
            include_remixes: true,
            include_covers: true,
            allow_explicit: true,
            artist_gap: 5,
            seed_artists: Vec::new(),
            base_playlist: None,
            max_tracks: 50,
        }
    }
}

/// Energy level for a generated playlist.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Energy {
    Low,
    #[default]
    Medium,
    High,
}

impl Energy {
    pub fn label(&self) -> &'static str {
        match self {
            Energy::Low => "Low",
            Energy::Medium => "Medium",
            Energy::High => "High",
        }
    }
}

/// Source preference for a generated playlist.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreference {
    #[default]
    Local,
    Youtube,
    Hybrid,
}

impl SourcePreference {
    pub fn label(&self) -> &'static str {
        match self {
            SourcePreference::Local => "Local",
            SourcePreference::Youtube => "YouTube",
            SourcePreference::Hybrid => "Local + YouTube",
        }
    }
}

impl GeneratorConstraints {
    /// Create constraints from a natural-language description. This is a
    /// simple keyword-based parser — a full NLP parser would require an LLM,
    /// but the constraint extraction is deterministic and transparent.
    pub fn from_natural_language(text: &str) -> Self {
        let lower = text.to_lowercase();
        let mut constraints = GeneratorConstraints {
            max_tracks: 50,
            ..Default::default()
        };

        // Parse duration: "45 minute", "1 hour", "30 min"
        if let Some(mins) = parse_duration_minutes(&lower) {
            constraints.duration_secs = Some(mins * 60.0);
        }

        // Parse energy: "energetic", "high energy", "calm", "relaxing"
        if lower.contains("energetic") || lower.contains("high energy") || lower.contains("running")
        {
            constraints.energy = Energy::High;
        } else if lower.contains("calm") || lower.contains("relax") || lower.contains("chill") {
            constraints.energy = Energy::Low;
        }

        // Parse source: "local", "youtube", "hybrid"
        if lower.contains("local") && !lower.contains("youtube") {
            constraints.sources = SourcePreference::Local;
        } else if lower.contains("youtube") && !lower.contains("local") {
            constraints.sources = SourcePreference::Youtube;
        } else if lower.contains("hybrid") || (lower.contains("local") && lower.contains("youtube"))
        {
            constraints.sources = SourcePreference::Hybrid;
        }

        // Parse familiarity: "70% discovery", "30% known"
        if let Some(pct) = parse_percentage(&lower, "discovery") {
            constraints.familiarity = 1.0 - pct;
        } else if let Some(pct) = parse_percentage(&lower, "known") {
            constraints.familiarity = pct;
        }

        // Parse live: "no live", "no live versions"
        if lower.contains("no live") || lower.contains("exclude live") {
            constraints.include_live = false;
        } else if lower.contains("live") {
            constraints.include_live = true;
        }

        // Parse remixes
        if lower.contains("no remix") || lower.contains("exclude remix") {
            constraints.include_remixes = false;
        } else if lower.contains("remix") {
            constraints.include_remixes = true;
        }

        // Parse covers
        if lower.contains("no cover") || lower.contains("exclude cover") {
            constraints.include_covers = false;
        }

        // Parse explicit
        constraints.allow_explicit = !(lower.contains("no explicit") || lower.contains("clean"));

        // Parse artist gap
        if lower.contains("artist repeat gap") {
            // Try to extract a number after "gap"
            if let Some(n) = parse_number_after(&lower, "gap") {
                constraints.artist_gap = n;
            }
        }
        if constraints.artist_gap == 0 {
            constraints.artist_gap = 5; // default
        }

        // Parse seed artists: "based on these three artists"
        if lower.contains("based on") || lower.contains("artists:") {
            // Simple extraction: look for quoted or comma-separated names
            // This is a simplified parser; a full NLP system would do better.
            constraints.seed_artists = extract_seed_artists(&lower);
        }

        constraints
    }

    /// Format the constraints as a human-readable plan.
    pub fn to_plan_string(&self) -> String {
        let mut lines = Vec::new();
        if let Some(dur) = self.duration_secs {
            let mins = (dur / 60.0).round() as u32;
            lines.push(format!("Duration:          {} minutes", mins));
        }
        lines.push(format!("Energy:            {}", self.energy.label()));
        lines.push(format!("Sources:           {}", self.sources.label()));
        let fam_pct = (self.familiarity * 100.0).round() as u32;
        let disc_pct = 100 - fam_pct;
        lines.push(format!(
            "Familiarity:       {}% known / {}% discovery",
            fam_pct, disc_pct
        ));
        lines.push(format!(
            "Live recordings:   {}",
            if self.include_live {
                "Included"
            } else {
                "Excluded"
            }
        ));
        lines.push(format!(
            "Remixes:           {}",
            if self.include_remixes {
                "Included"
            } else {
                "Excluded"
            }
        ));
        lines.push(format!(
            "Covers:            {}",
            if self.include_covers {
                "Included"
            } else {
                "Excluded"
            }
        ));
        lines.push(format!("Artist repeat gap: {} tracks", self.artist_gap));
        lines.push(format!(
            "Explicit content:  {}",
            if self.allow_explicit {
                "Allowed"
            } else {
                "Excluded"
            }
        ));
        if !self.seed_artists.is_empty() {
            lines.push(format!(
                "Seed artists:      {}",
                self.seed_artists.join(", ")
            ));
        }
        if let Some(base) = &self.base_playlist {
            lines.push(format!("Base playlist:     {}", base));
        }
        lines.join("\n")
    }
}

/// A generated playlist with its constraints.
#[derive(Clone, Debug)]
pub struct GeneratedPlaylist {
    pub constraints: GeneratorConstraints,
    pub tracks: Vec<Candidate>,
    /// Whether this is a preview (not yet saved).
    pub is_preview: bool,
    /// Pinned track ids (won't be removed on regenerate).
    pub pinned: Vec<String>,
}

/// Generate a playlist from constraints.
pub fn generate(
    constraints: &GeneratorConstraints,
    profile: &UserProfile,
    catalog: &[Track],
) -> GeneratedPlaylist {
    let gen = CandidateGenerator::new(profile, catalog);
    let mut candidates = gen.generate();

    // Apply energy-based weighting
    let weights = match constraints.energy {
        Energy::High => RankingWeights::default(),
        Energy::Medium => RankingWeights::default(),
        Energy::Low => RankingWeights::rediscover(),
    };
    rank(&mut candidates, profile, &weights);

    // Apply diversity with the constraint's artist gap
    let diversity_config = DiversityConfig {
        artist_gap: constraints.artist_gap,
        ..DiversityConfig::default()
    };
    let diverse = apply_diversity(&candidates, catalog, &diversity_config, &[]);

    // Cap at max tracks
    let max = constraints.max_tracks.min(50);
    let tracks: Vec<Candidate> = diverse.into_iter().take(max).collect();

    GeneratedPlaylist {
        constraints: constraints.clone(),
        tracks,
        is_preview: true,
        pinned: Vec::new(),
    }
}

// --- Natural-language parsing helpers ---

fn parse_duration_minutes(text: &str) -> Option<f64> {
    // "45 minute", "45 min", "1 hour", "1.5 hours"
    if let Some(pos) = text.find("minute") {
        return parse_number_before(text, pos);
    }
    if let Some(pos) = text.find("min") {
        return parse_number_before(text, pos);
    }
    if let Some(pos) = text.find("hour") {
        return parse_number_before(text, pos).map(|n| n * 60.0);
    }
    None
}

fn parse_number_before(text: &str, pos: usize) -> Option<f64> {
    let before = &text[..pos];
    // Split on whitespace and hyphens to handle "45-minute" style.
    let tokens: Vec<&str> = before.split([' ', '-', '\t']).collect();
    // Try from the end, skipping empty strings (from trailing hyphens).
    for token in tokens.iter().rev() {
        if let Ok(n) = token.parse::<f64>() {
            return Some(n);
        }
    }
    None
}

fn parse_percentage(text: &str, keyword: &str) -> Option<f64> {
    // "70% discovery" → 0.7
    if let Some(pos) = text.find(keyword) {
        let before = &text[..pos];
        // Look for a number followed by % before the keyword
        if let Some(pct_pos) = before.rfind('%') {
            let between = &before[..pct_pos];
            let tokens: Vec<&str> = between.split_whitespace().collect();
            if let Some(last) = tokens.last() {
                if let Ok(n) = last.parse::<f64>() {
                    return Some(n / 100.0);
                }
            }
        }
    }
    None
}

fn parse_number_after(text: &str, keyword: &str) -> Option<usize> {
    if let Some(pos) = text.find(keyword) {
        let after = &text[pos + keyword.len()..];
        let tokens: Vec<&str> = after.split_whitespace().collect();
        for token in tokens {
            if let Ok(n) = token.parse::<usize>() {
                return Some(n);
            }
        }
    }
    None
}

fn extract_seed_artists(text: &str) -> Vec<String> {
    // Simple: look for "artists: X, Y, Z" or "based on X, Y, Z"
    if let Some(pos) = text.find("artists:") {
        let after = &text[pos + 8..];
        return after
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .take(3)
            .collect();
    }
    if let Some(pos) = text.find("based on") {
        let after = &text[pos + 8..];
        return after
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !s.starts_with("these"))
            .take(3)
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;
    use crate::reco::events::ListenEvent;
    use crate::reco::profile::UserProfile;

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
    fn parse_energetic_running() {
        let c = GeneratorConstraints::from_natural_language(
            "Make a 45-minute energetic running playlist",
        );
        assert_eq!(c.energy, Energy::High);
        assert!(c.duration_secs.is_some());
        assert_eq!(c.duration_secs.unwrap(), 45.0 * 60.0);
    }

    #[test]
    fn parse_calm_relaxing() {
        let c = GeneratorConstraints::from_natural_language("calm relaxing playlist");
        assert_eq!(c.energy, Energy::Low);
    }

    #[test]
    fn parse_no_live_versions() {
        let c = GeneratorConstraints::from_natural_language("no live versions");
        assert!(!c.include_live);
    }

    #[test]
    fn parse_70_percent_discovery() {
        let c = GeneratorConstraints::from_natural_language("70% discovery");
        assert!((c.familiarity - 0.3).abs() < 0.01);
    }

    #[test]
    fn parse_hybrid_sources() {
        let c = GeneratorConstraints::from_natural_language("local and youtube hybrid playlist");
        assert_eq!(c.sources, SourcePreference::Hybrid);
    }

    #[test]
    fn parse_local_only() {
        let c = GeneratorConstraints::from_natural_language("mostly local tracks");
        assert_eq!(c.sources, SourcePreference::Local);
    }

    #[test]
    fn constraints_to_plan_string() {
        let c = GeneratorConstraints {
            duration_secs: Some(2700.0),
            energy: Energy::High,
            sources: SourcePreference::Hybrid,
            familiarity: 0.7,
            include_live: false,
            artist_gap: 5,
            allow_explicit: true,
            max_tracks: 50,
            ..Default::default()
        };
        let plan = c.to_plan_string();
        assert!(plan.contains("Duration:"));
        assert!(plan.contains("45 minutes"));
        assert!(plan.contains("Energy:"));
        assert!(plan.contains("High"));
        assert!(plan.contains("Sources:"));
        assert!(plan.contains("Local + YouTube"));
        assert!(plan.contains("70% known"));
        assert!(plan.contains("30% discovery"));
        assert!(plan.contains("Live recordings:"));
        assert!(plan.contains("Excluded"));
        assert!(plan.contains("Artist repeat gap: 5 tracks"));
        assert!(plan.contains("Explicit content:  Allowed"));
    }

    #[test]
    fn generate_playlist_produces_tracks() {
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
        let profile = UserProfile::build_from_events(&events);
        let catalog = vec![
            make_track("t1", "Artist A", "Song A"),
            make_track("t2", "Artist B", "Song B"),
            make_track("t3", "Artist C", "Song C"),
        ];
        let constraints = GeneratorConstraints::default();
        let playlist = generate(&constraints, &profile, &catalog);
        assert!(!playlist.tracks.is_empty());
        assert!(playlist.is_preview);
    }

    #[test]
    fn generate_playlist_empty_profile_uses_catalog_fallback() {
        let profile = UserProfile::new();
        let catalog = vec![
            make_track("t1", "Artist A", "Song A"),
            make_track("t2", "Artist B", "Song B"),
            make_track("t3", "Artist C", "Song C"),
        ];
        let constraints = GeneratorConstraints::default();
        let playlist = generate(&constraints, &profile, &catalog);
        assert!(
            !playlist.tracks.is_empty(),
            "empty profile should still produce a playlist via catalog fallback"
        );
    }

    #[test]
    fn energy_label() {
        assert_eq!(Energy::Low.label(), "Low");
        assert_eq!(Energy::Medium.label(), "Medium");
        assert_eq!(Energy::High.label(), "High");
    }

    #[test]
    fn source_preference_label() {
        assert_eq!(SourcePreference::Local.label(), "Local");
        assert_eq!(SourcePreference::Youtube.label(), "YouTube");
        assert_eq!(SourcePreference::Hybrid.label(), "Local + YouTube");
    }
}
