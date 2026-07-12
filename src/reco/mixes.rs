//! Generated mixes — affinity-clustered, stable collections of tracks.
//!
//! Mix types:
//! - DailyMix: affinity-clustered, stable for the day (seeded by date)
//! - Discover: taste-adjacent unfamiliar music
//! - OnRepeat: based on genuine replay and completion behavior
//! - Rediscover: previously positive music not heard recently
//! - ForgottenFavorites: only when enough history exists
//! - NewFromArtists: new releases from high-affinity artists
//! - LocalYtBlend: uses both local and YouTube sources
//! - Focus/Calm/Energy/LateNight: mood/activity (when metadata supports)
//!
//! Only display collections supported by adequate evidence. Do not create
//! many nearly identical mixes with different names.

use crate::catalog::Track;
use crate::reco::candidates::{Candidate, CandidateGenerator};
use crate::reco::diversity::{apply_diversity, DiversityConfig};
use crate::reco::profile::UserProfile;
use crate::reco::ranking::{rank, RankingWeights};
use serde::{Deserialize, Serialize};

/// The type of generated mix. Each type uses different ranking weights and
/// diversity settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MixType {
    /// Affinity-clustered, stable for the day.
    DailyMix,
    /// Taste-adjacent unfamiliar music.
    Discover,
    /// Based on genuine replay and completion behavior.
    OnRepeat,
    /// Previously positive music not heard recently.
    Rediscover,
    /// Only when enough history exists.
    ForgottenFavorites,
    /// New releases from high-affinity artists.
    NewFromArtists,
    /// Uses both local and YouTube sources.
    LocalYtBlend,
    /// Focus/activity mix.
    Focus,
    /// Calm/activity mix.
    Calm,
    /// Energy/activity mix.
    Energy,
    /// Late night/activity mix.
    LateNight,
}

impl MixType {
    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            MixType::DailyMix => "Daily Mix",
            MixType::Discover => "Discover Mix",
            MixType::OnRepeat => "On Repeat",
            MixType::Rediscover => "Rediscover",
            MixType::ForgottenFavorites => "Forgotten Favorites",
            MixType::NewFromArtists => "New from Artists You Like",
            MixType::LocalYtBlend => "Local + YouTube Blend",
            MixType::Focus => "Focus Mix",
            MixType::Calm => "Calm Mix",
            MixType::Energy => "Energy Mix",
            MixType::LateNight => "Late Night Mix",
        }
    }

    /// A short description for display.
    pub fn description(&self) -> &'static str {
        match self {
            MixType::DailyMix => "Tracks clustered by your recent listening, refreshed daily",
            MixType::Discover => "Taste-adjacent music you haven't heard",
            MixType::OnRepeat => "Tracks you've been playing on repeat",
            MixType::Rediscover => "Music you used to love, not heard recently",
            MixType::ForgottenFavorites => "Deep cuts from your history",
            MixType::NewFromArtists => "New releases from artists you listen to",
            MixType::LocalYtBlend => "A blend of your local library and YouTube",
            MixType::Focus => "Music to help you focus",
            MixType::Calm => "Relaxing tracks from your library",
            MixType::Energy => "High-energy tracks to get you going",
            MixType::LateNight => "Mellow tracks for late-night listening",
        }
    }

    /// Ranking weights for this mix type.
    pub fn weights(&self) -> RankingWeights {
        match self {
            MixType::DailyMix => RankingWeights::default(),
            MixType::Discover => RankingWeights::discover(),
            MixType::OnRepeat => RankingWeights::on_repeat(),
            MixType::Rediscover => RankingWeights::rediscover(),
            MixType::ForgottenFavorites => RankingWeights::rediscover(),
            MixType::NewFromArtists => RankingWeights::default(),
            MixType::LocalYtBlend => RankingWeights::default(),
            MixType::Focus => RankingWeights::default(),
            MixType::Calm => RankingWeights::default(),
            MixType::Energy => RankingWeights::default(),
            MixType::LateNight => RankingWeights::default(),
        }
    }

    /// Diversity config for this mix type.
    pub fn diversity(&self) -> DiversityConfig {
        match self {
            MixType::DailyMix => DiversityConfig::default(),
            MixType::Discover => DiversityConfig::default(),
            MixType::OnRepeat => DiversityConfig::relaxed(),
            MixType::Rediscover => DiversityConfig::default(),
            MixType::ForgottenFavorites => DiversityConfig::default(),
            MixType::NewFromArtists => DiversityConfig::default(),
            MixType::LocalYtBlend => DiversityConfig::default(),
            MixType::Focus => DiversityConfig::relaxed(),
            MixType::Calm => DiversityConfig::relaxed(),
            MixType::Energy => DiversityConfig::relaxed(),
            MixType::LateNight => DiversityConfig::relaxed(),
        }
    }

    /// Maximum number of tracks in this mix type.
    pub fn max_tracks(&self) -> usize {
        match self {
            MixType::DailyMix => 50,
            MixType::Discover => 30,
            MixType::OnRepeat => 30,
            MixType::Rediscover => 40,
            MixType::ForgottenFavorites => 25,
            MixType::NewFromArtists => 20,
            MixType::LocalYtBlend => 50,
            MixType::Focus => 40,
            MixType::Calm => 40,
            MixType::Energy => 40,
            MixType::LateNight => 40,
        }
    }

    /// Whether this mix type requires history to be meaningful.
    pub fn requires_history(&self) -> bool {
        matches!(
            self,
            MixType::OnRepeat
                | MixType::Rediscover
                | MixType::ForgottenFavorites
                | MixType::NewFromArtists
        )
    }
}

/// A generated mix: a list of candidates with provenance.
#[derive(Clone, Debug)]
pub struct Mix {
    pub mix_type: MixType,
    pub tracks: Vec<Candidate>,
    /// The date this mix was generated (for daily stability).
    pub generated_date: String,
}

impl Mix {
    /// Get the track ids in this mix.
    pub fn track_ids(&self) -> Vec<&str> {
        self.tracks.iter().map(|c| c.track_id.as_str()).collect()
    }
}

/// Generate a mix of the given type. Uses the full recommendation pipeline:
/// candidate generation → ranking → diversity.
pub fn generate_mix(mix_type: MixType, profile: &UserProfile, catalog: &[Track]) -> Mix {
    let gen = CandidateGenerator::new(profile, catalog);
    let mut candidates = gen.generate();

    // Rank using the mix-type-specific weights.
    rank(&mut candidates, profile, &mix_type.weights());

    // Apply diversity.
    let diverse = apply_diversity(&candidates, catalog, &mix_type.diversity(), &[]);

    // Cap at max tracks.
    let tracks = diverse.into_iter().take(mix_type.max_tracks()).collect();

    Mix {
        mix_type,
        tracks,
        generated_date: today_date_string(),
    }
}

/// Generate multiple mixes. Returns only the types that have enough evidence.
pub fn generate_all_mixes(profile: &UserProfile, catalog: &[Track]) -> Vec<Mix> {
    let mut mixes = Vec::new();

    // Daily Mix is always generated (even cold start uses local catalog).
    mixes.push(generate_mix(MixType::DailyMix, profile, catalog));

    // Discover is always generated.
    mixes.push(generate_mix(MixType::Discover, profile, catalog));

    // OnRepeat requires history.
    if profile.has_history() {
        mixes.push(generate_mix(MixType::OnRepeat, profile, catalog));
    }

    // Rediscover requires history.
    if profile.has_history() {
        mixes.push(generate_mix(MixType::Rediscover, profile, catalog));
    }

    // ForgottenFavorites requires a lot of history.
    if profile.total_events() > 100 {
        mixes.push(generate_mix(MixType::ForgottenFavorites, profile, catalog));
    }

    // LocalYtBlend always generated.
    mixes.push(generate_mix(MixType::LocalYtBlend, profile, catalog));

    mixes
}

/// Get today's date as a YYYY-MM-DD string (for daily stability).
fn today_date_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Simple date computation from Unix timestamp.
    let days = now / 86400;
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Convert days since epoch to (year, month, day). Simple algorithm —
/// not a full calendar library, but sufficient for daily stability.
fn days_to_date(days: u64) -> (u32, u32, u32) {
    // 1970-01-01 is day 0.
    let mut remaining = days;
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year as u64 {
            break;
        }
        remaining -= days_in_year as u64;
        year += 1;
    }
    let leap = is_leap_year(year);
    let month_days = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u32;
    for &md in &month_days {
        if remaining < md as u64 {
            break;
        }
        remaining -= md as u64;
        month += 1;
    }
    let day = remaining as u32 + 1;
    (year, month, day)
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Track;
    use crate::reco::events::ListenEvent;

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
            ListenEvent::Completed {
                track_id: "t2".into(),
                timestamp: 200,
            },
            ListenEvent::Liked {
                track_id: "t1".into(),
                timestamp: 300,
            },
        ];
        UserProfile::build_from_events(&events)
    }

    #[test]
    fn mix_type_labels_are_human_readable() {
        assert_eq!(MixType::DailyMix.label(), "Daily Mix");
        assert_eq!(MixType::Discover.label(), "Discover Mix");
        assert_eq!(MixType::OnRepeat.label(), "On Repeat");
    }

    #[test]
    fn generate_daily_mix_produces_tracks() {
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
            make_track("t3", "Artist C", "Album 3", "Song 3"),
        ];
        let mix = generate_mix(MixType::DailyMix, &profile, &catalog);
        assert!(!mix.tracks.is_empty());
        assert_eq!(mix.mix_type, MixType::DailyMix);
    }

    #[test]
    fn generate_all_mixes_at_least_4_types() {
        let profile = make_profile();
        let catalog: Vec<Track> = (0..20)
            .map(|i| {
                make_track(
                    &format!("t{i}"),
                    &format!("Artist {i}"),
                    &format!("Album {i}"),
                    &format!("Song {i}"),
                )
            })
            .collect();
        let mixes = generate_all_mixes(&profile, &catalog);
        // Should have at least 4 distinct mix types.
        assert!(mixes.len() >= 4, "expected ≥4 mixes, got {}", mixes.len());
    }

    #[test]
    fn discover_mix_uses_discover_weights() {
        let weights = MixType::Discover.weights();
        // Discover weights should have higher novelty than default.
        let default = RankingWeights::default();
        assert!(weights.novelty > default.novelty);
    }

    #[test]
    fn on_repeat_requires_history() {
        assert!(MixType::OnRepeat.requires_history());
        assert!(!MixType::DailyMix.requires_history());
    }

    #[test]
    fn daily_mix_stable_for_same_date() {
        // Same profile + catalog → same mix on the same day (deterministic).
        let profile = make_profile();
        let catalog = vec![
            make_track("t1", "Artist A", "Album 1", "Song 1"),
            make_track("t2", "Artist B", "Album 2", "Song 2"),
        ];
        let mix1 = generate_mix(MixType::DailyMix, &profile, &catalog);
        let mix2 = generate_mix(MixType::DailyMix, &profile, &catalog);
        assert_eq!(mix1.generated_date, mix2.generated_date);
        assert_eq!(mix1.track_ids(), mix2.track_ids());
    }

    #[test]
    fn empty_profile_generates_empty_mix() {
        let profile = UserProfile::new();
        let catalog = vec![make_track("t1", "Artist A", "Album 1", "Song 1")];
        let mix = generate_mix(MixType::OnRepeat, &profile, &catalog);
        assert!(mix.tracks.is_empty());
    }

    #[test]
    fn mix_max_tracks_respected() {
        let profile = make_profile();
        let catalog: Vec<Track> = (0..100)
            .map(|i| {
                make_track(
                    &format!("t{i}"),
                    &format!("Artist {i}"),
                    &format!("Album {i}"),
                    &format!("Song {i}"),
                )
            })
            .collect();
        let mix = generate_mix(MixType::Discover, &profile, &catalog);
        assert!(mix.tracks.len() <= MixType::Discover.max_tracks());
    }

    #[test]
    fn days_to_date_basic() {
        let (y, m, d) = days_to_date(0);
        assert_eq!((y, m, d), (1970, 1, 1));
        let (y, m, d) = days_to_date(31);
        assert_eq!((y, m, d), (1970, 2, 1));
    }

    #[test]
    fn is_leap_year_correct() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
    }
}
