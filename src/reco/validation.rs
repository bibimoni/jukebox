//! Music-content validation — classifies YouTube results as music or non-music.
//!
//! YouTube results may contain commentary, reactions, interviews, tutorials,
//! shorts, covers, remixes, live recordings, fan uploads, lyrics videos,
//! reuploads, and non-music content. This module uses available metadata
//! (category, duration, channel identity, title, description, artist mapping)
//! to classify and filter content.
//!
//! Supports user preferences: prefer official audio, prefer music video,
//! allow live, allow remixes, allow covers, avoid lyric videos, avoid shorts,
//! hide likely non-music content.

use crate::reco::identity::TrackVariant;
use serde::{Deserialize, Serialize};

/// User preferences for content filtering.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContentPreference {
    /// Prefer official audio over music videos (for audio-only listening).
    pub prefer_official_audio: bool,
    /// Prefer music video over official audio.
    pub prefer_music_video: bool,
    /// Allow live recordings.
    pub allow_live: bool,
    /// Allow remixes.
    pub allow_remixes: bool,
    /// Allow covers.
    pub allow_covers: bool,
    /// Allow acoustic versions.
    pub allow_acoustic: bool,
    /// Avoid lyric videos (not official music).
    pub avoid_lyric_videos: bool,
    /// Avoid YouTube Shorts.
    pub avoid_shorts: bool,
    /// Hide likely non-music content (commentary, reactions, tutorials).
    pub hide_non_music: bool,
}

impl ContentPreference {
    /// Default preferences: allow most music variants, avoid non-music.
    pub fn balanced() -> Self {
        ContentPreference {
            prefer_official_audio: true,
            prefer_music_video: false,
            allow_live: true,
            allow_remixes: true,
            allow_covers: true,
            allow_acoustic: true,
            avoid_lyric_videos: true,
            avoid_shorts: true,
            hide_non_music: true,
        }
    }

    /// Strict preferences: only official audio/originals.
    pub fn strict() -> Self {
        ContentPreference {
            prefer_official_audio: true,
            prefer_music_video: false,
            allow_live: false,
            allow_remixes: false,
            allow_covers: false,
            allow_acoustic: false,
            avoid_lyric_videos: true,
            avoid_shorts: true,
            hide_non_music: true,
        }
    }

    /// Check if a track variant passes the preference filter.
    pub fn passes(&self, variant: TrackVariant) -> bool {
        if !variant.is_music() && self.hide_non_music {
            return false;
        }
        match variant {
            TrackVariant::Live => self.allow_live,
            TrackVariant::Remix => self.allow_remixes,
            TrackVariant::Cover => self.allow_covers,
            TrackVariant::Acoustic => self.allow_acoustic,
            TrackVariant::LyricsVideo => !self.avoid_lyric_videos,
            TrackVariant::Short => !self.avoid_shorts,
            TrackVariant::Original
            | TrackVariant::Remaster
            | TrackVariant::Explicit
            | TrackVariant::Clean
            | TrackVariant::OfficialAudio
            | TrackVariant::MusicVideo => true,
            TrackVariant::Instrumental => true,
            TrackVariant::Reupload => true,
            TrackVariant::Other => true,
            TrackVariant::Commentary => !self.hide_non_music,
        }
    }

    /// Score a variant by preference (higher = more preferred).
    pub fn score(&self, variant: TrackVariant) -> f64 {
        if !self.passes(variant) {
            return f64::NEG_INFINITY;
        }
        let mut score = 1.0;
        if variant == TrackVariant::OfficialAudio && self.prefer_official_audio {
            score += 1.0;
        }
        if variant == TrackVariant::MusicVideo && self.prefer_music_video {
            score += 1.0;
        }
        if variant.is_preferred() {
            score += 0.5;
        }
        score
    }
}

/// Validate a YouTube track based on available metadata. Returns a
/// `ValidationResult` indicating whether the track is likely music and
/// what variant it is.
#[derive(Clone, Debug)]
pub struct ValidationResult {
    pub is_music: bool,
    pub variant: TrackVariant,
    pub confidence: f64,
    pub reason: String,
}

/// Validate a track using its title and optional metadata.
pub fn validate(
    title: &str,
    duration_secs: Option<f64>,
    prefs: &ContentPreference,
) -> ValidationResult {
    let variant = TrackVariant::from_title(title);
    let is_music = variant.is_music();
    let passes = prefs.passes(variant);

    // Duration heuristic: very short (<30s) is likely not a full track.
    let mut confidence = 0.7;
    if let Some(dur) = duration_secs {
        if dur < 30.0 {
            return ValidationResult {
                is_music: false,
                variant,
                confidence: 0.9,
                reason: format!("too short ({:.0}s)", dur),
            };
        }
        if dur > 600.0 {
            confidence -= 0.2; // very long might be a mix/compilation
        }
    }

    if !is_music {
        return ValidationResult {
            is_music: false,
            variant,
            confidence: 0.8,
            reason: "title indicates non-music content".into(),
        };
    }

    if !passes {
        return ValidationResult {
            is_music: true,
            variant,
            confidence: 0.9,
            reason: "filtered by user preference".into(),
        };
    }

    ValidationResult {
        is_music: true,
        variant,
        confidence,
        reason: "passes content validation".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_prefs_allow_live() {
        let prefs = ContentPreference::balanced();
        assert!(prefs.passes(TrackVariant::Live));
    }

    #[test]
    fn strict_prefs_block_live() {
        let prefs = ContentPreference::strict();
        assert!(!prefs.passes(TrackVariant::Live));
    }

    #[test]
    fn balanced_prefs_block_shorts() {
        let prefs = ContentPreference::balanced();
        assert!(!prefs.passes(TrackVariant::Short));
    }

    #[test]
    fn validate_music_track() {
        let result = validate(
            "Normal Song Title",
            Some(180.0),
            &ContentPreference::balanced(),
        );
        assert!(result.is_music);
        assert!(result.confidence > 0.5);
    }

    #[test]
    fn validate_short_track_rejected() {
        let result = validate("Short Clip", Some(15.0), &ContentPreference::balanced());
        assert!(!result.is_music);
        assert!(result.reason.contains("too short"));
    }

    #[test]
    fn validate_shorts_rejected() {
        let result = validate("Song #shorts", Some(60.0), &ContentPreference::balanced());
        assert!(!result.is_music);
    }

    #[test]
    fn validate_live_allowed_in_balanced() {
        let result = validate("Song (Live)", Some(240.0), &ContentPreference::balanced());
        assert!(result.is_music);
    }

    #[test]
    fn validate_live_blocked_in_strict() {
        let result = validate("Song (Live)", Some(240.0), &ContentPreference::strict());
        assert!(result.is_music); // it IS music, but filtered
        assert!(!result.passes_validation());
    }

    impl ValidationResult {
        fn passes_validation(&self) -> bool {
            self.is_music && self.confidence > 0.0 && !self.reason.contains("filtered")
        }
    }

    #[test]
    fn content_preference_score_official_audio_preferred() {
        let prefs = ContentPreference::balanced();
        let official_score = prefs.score(TrackVariant::OfficialAudio);
        let video_score = prefs.score(TrackVariant::MusicVideo);
        assert!(official_score > video_score);
    }

    #[test]
    fn content_preference_score_negative_infinity_for_blocked() {
        let prefs = ContentPreference::strict();
        let score = prefs.score(TrackVariant::Short);
        assert_eq!(score, f64::NEG_INFINITY);
    }
}
