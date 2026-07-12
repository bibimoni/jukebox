//! Autoplay modes — what happens when the current context ends.
//!
//! Autoplay modes:
//! - Stop: do nothing when context ends.
//! - Related: play related music (YouTube-style autoplay).
//! - ContextRadio: start radio from the current context.
//! - Familiar: play familiar music from the user's profile.
//! - Discovery: play discovery music (new to the user).

use serde::{Deserialize, Serialize};

/// The autoplay mode — what happens when the current context (album, playlist,
/// queue) ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoplayMode {
    /// Do nothing — stop playback when context ends.
    #[default]
    Stop,
    /// Play related music (YouTube-style autoplay radio).
    Related,
    /// Start radio from the current context (album/playlist/track).
    ContextRadio,
    /// Play familiar music from the user's profile.
    Familiar,
    /// Play discovery music (new to the user).
    Discovery,
}

impl AutoplayMode {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            AutoplayMode::Stop => "stop",
            AutoplayMode::Related => "related music",
            AutoplayMode::ContextRadio => "context radio",
            AutoplayMode::Familiar => "familiar music",
            AutoplayMode::Discovery => "discovery music",
        }
    }

    /// Short glyph for display (ASCII-safe).
    pub fn icon(&self) -> &'static str {
        match self {
            AutoplayMode::Stop => "[stop]",
            AutoplayMode::Related => "[rel]",
            AutoplayMode::ContextRadio => "[rad]",
            AutoplayMode::Familiar => "[fam]",
            AutoplayMode::Discovery => "[dis]",
        }
    }

    /// Cycle to the next mode.
    pub fn next(&self) -> Self {
        match self {
            AutoplayMode::Stop => AutoplayMode::Related,
            AutoplayMode::Related => AutoplayMode::ContextRadio,
            AutoplayMode::ContextRadio => AutoplayMode::Familiar,
            AutoplayMode::Familiar => AutoplayMode::Discovery,
            AutoplayMode::Discovery => AutoplayMode::Stop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_stop() {
        assert_eq!(AutoplayMode::default(), AutoplayMode::Stop);
    }

    #[test]
    fn cycle_through_all_modes() {
        let mut mode = AutoplayMode::Stop;
        assert_eq!(mode.next(), AutoplayMode::Related);
        mode = mode.next(); // Related
        assert_eq!(mode, AutoplayMode::Related);
        mode = mode.next(); // ContextRadio
        assert_eq!(mode, AutoplayMode::ContextRadio);
        mode = mode.next(); // Familiar
        assert_eq!(mode, AutoplayMode::Familiar);
        mode = mode.next(); // Discovery
        assert_eq!(mode, AutoplayMode::Discovery);
        mode = mode.next(); // Stop
        assert_eq!(mode, AutoplayMode::Stop);
    }

    #[test]
    fn labels_are_human_readable() {
        assert_eq!(AutoplayMode::Stop.label(), "stop");
        assert_eq!(AutoplayMode::Related.label(), "related music");
    }

    #[test]
    fn icons_are_ascii_safe() {
        for mode in [
            AutoplayMode::Stop,
            AutoplayMode::Related,
            AutoplayMode::ContextRadio,
            AutoplayMode::Familiar,
            AutoplayMode::Discovery,
        ] {
            for c in mode.icon().chars() {
                assert!(c.is_ascii(), "icon {:?} must be ASCII-safe", mode.icon());
            }
        }
    }
}
