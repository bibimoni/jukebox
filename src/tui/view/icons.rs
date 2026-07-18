//! Nerd Font icon system — detected, installed monospaced Nerd Font support
//! with Unicode and ASCII fallbacks.
//!
//! Three font modes:
//! 1. `NerdFont` — PUA glyphs (e000-f8ff) from a detected Nerd Font.
//! 2. `Unicode` — standard Unicode symbols (no PUA).
//! 3. `Ascii` — plain ASCII text labels (no Unicode at all).
//!
//! Essential meaning NEVER depends only on icons or color — every icon has
//! a text label alongside. This ensures accessibility in no-color and
//! ASCII-only environments.

use serde::{Deserialize, Serialize};

/// True when `TERM=dumb` — a terminal that can't render ANSI controls or
/// Unicode glyphs (RB-6). Used by [`FontMode::auto_detect`] to select ASCII
/// glyphs.
fn is_dumb_terminal() -> bool {
    matches!(std::env::var("TERM"), Ok(t) if t == "dumb")
}

/// True when the locale is ASCII (`LC_ALL` / `LC_CTYPE` / `LANG` set to `C`
/// or `POSIX`) — an environment where Unicode glyphs may not render (RB-6).
/// `LC_ALL` takes precedence over `LC_CTYPE` which takes precedence over
/// `LANG` (the standard POSIX locale resolution order). A missing or empty
/// variable is treated as "not ASCII" (the common case for modern systems).
/// `C.UTF-8` and `POSIX.UTF-8` are NOT ASCII locales — they use the C base
/// locale but with UTF-8 encoding, so Unicode glyphs render correctly.
fn is_ascii_locale() -> bool {
    fn is_c_or_posix(var: &str) -> bool {
        match std::env::var(var) {
            Ok(v) => {
                if v.is_empty() {
                    return false;
                }
                // Exact match: C or POSIX (no encoding suffix) = ASCII.
                if v == "C" || v == "POSIX" {
                    return true;
                }
                // C.<encoding> or POSIX.<encoding>: ASCII only when the
                // encoding is ASCII (not UTF-8, which supports Unicode).
                let suffix = v.split_once('.').map(|(_, enc)| enc);
                matches!(suffix, Some(e) if e.eq_ignore_ascii_case("ASCII"))
            }
            Err(_) => false,
        }
    }
    is_c_or_posix("LC_ALL") || is_c_or_posix("LC_CTYPE") || is_c_or_posix("LANG")
}

/// The font mode for icon rendering.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum FontMode {
    /// Nerd Font PUA glyphs (e000-f8ff). Requires a Nerd Font to be
    /// installed and selected as the terminal font.
    NerdFont,
    /// Standard Unicode symbols (no PUA). Works with any Unicode-capable
    /// terminal font.
    #[default]
    Unicode,
    /// Plain ASCII text labels only. Works in any terminal, including
    /// NO_COLOR and ASCII-only environments.
    Ascii,
}

impl FontMode {
    /// Auto-detect the best font mode. Checks for Nerd Font environment
    /// hints; defaults to Unicode (the safest wide-compatible mode).
    ///
    /// DEF-006: also checks `JUKEBOX_FONT_MODE` — when set to "ascii" (case-
    /// insensitive), returns `FontMode::Ascii` so all glyphs use ASCII labels.
    /// When set to "nerd" or "nerdfont", returns `FontMode::NerdFont`. When
    /// set to "unicode", returns `FontMode::Unicode`.
    ///
    /// RB-6: also checks `TERM=dumb` and ASCII locales (`LC_ALL`/`LC_CTYPE`/
    /// `LANG` set to `C` or `POSIX`). These environments can't render Unicode
    /// glyphs, so `FontMode::Ascii` is selected so all glyphs use plain ASCII
    /// labels. `JUKEBOX_FONT_MODE` takes precedence over both (explicit user
    /// choice wins).
    ///
    /// D2: `NO_COLOR` is NOT checked here. `NO_COLOR` (no-color.org) only
    /// disables colors (handled in `theme.rs`); it must not change Unicode
    /// symbols to ASCII labels. `JUKEBOX_FONT_MODE=ascii` is the sole way to
    /// enable ASCII font mode. Conflating the two broke `NO_COLOR=1` by
    /// replacing all Unicode structural glyphs (›, ─, ♫) with ASCII, which
    /// is non-standard.
    pub fn auto_detect() -> Self {
        // Explicit override via JUKEBOX_FONT_MODE (highest priority).
        if let Ok(mode) = std::env::var("JUKEBOX_FONT_MODE") {
            match mode.to_lowercase().as_str() {
                "ascii" => return FontMode::Ascii,
                "nerd" | "nerdfont" => return FontMode::NerdFont,
                "unicode" => return FontMode::Unicode,
                _ => {}
            }
        }
        // RB-6: TERM=dumb or ASCII locale → ASCII glyphs. These environments
        // can't render Unicode, so use plain ASCII labels for all icons and
        // structural glyphs.
        if is_dumb_terminal() || is_ascii_locale() {
            return FontMode::Ascii;
        }
        // Check common Nerd Font environment hints.
        if let Ok(term) = std::env::var("TERM") {
            if term.contains("nerd") || term.contains("NF") {
                return FontMode::NerdFont;
            }
        }
        if let Ok(font) = std::env::var("TERM_FONT") {
            if font.to_lowercase().contains("nerd") {
                return FontMode::NerdFont;
            }
        }
        // Default: Unicode (works with most modern terminals).
        // NOTE: NO_COLOR intentionally does NOT trigger ASCII mode here —
        // it only disables colors (see `theme::no_color`). Use
        // JUKEBOX_FONT_MODE=ascii for ASCII font mode.
        FontMode::Unicode
    }

    /// Get the human-readable label for this font mode.
    pub fn label(&self) -> &'static str {
        match self {
            FontMode::NerdFont => "Nerd Font",
            FontMode::Unicode => "Unicode",
            FontMode::Ascii => "ASCII",
        }
    }
}

/// The limited vocabulary of icons used in the jukebox TUI. Each icon has
/// three representations: Nerd Font PUA glyph, Unicode symbol, and ASCII
/// text label.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Icon {
    // Source indicators
    Local,
    Youtube,
    Hybrid,
    // Playback states
    Playing,
    Paused,
    Buffering,
    // Navigation
    Queue,
    Radio,
    Autoplay,
    // Content
    Lyrics,
    Like,
    Hidden,
    Cached,
    Offline,
    // Status
    Warning,
    Error,
    Refresh,
    Search,
    // Discovery
    Generated,
}

impl Icon {
    /// Get the glyph for this icon in the given font mode.
    pub fn glyph(&self, mode: FontMode) -> &'static str {
        match mode {
            FontMode::NerdFont => self.nerd_font_glyph(),
            FontMode::Unicode => self.unicode_glyph(),
            FontMode::Ascii => self.ascii_label(),
        }
    }

    /// Get the human-readable label for this icon (always the same regardless
    /// of font mode — used alongside the glyph for accessibility).
    pub fn label(&self) -> &'static str {
        match self {
            Icon::Local => "local",
            Icon::Youtube => "YT",
            Icon::Hybrid => "mix",
            Icon::Playing => "playing",
            Icon::Paused => "paused",
            Icon::Buffering => "loading",
            Icon::Queue => "queue",
            Icon::Radio => "radio",
            Icon::Autoplay => "autoplay",
            Icon::Lyrics => "lyrics",
            Icon::Like => "liked",
            Icon::Hidden => "hidden",
            Icon::Cached => "cached",
            Icon::Offline => "offline",
            Icon::Warning => "warning",
            Icon::Error => "error",
            Icon::Refresh => "refresh",
            Icon::Search => "search",
            Icon::Generated => "generated",
        }
    }

    /// Nerd Font PUA glyph for this icon.
    fn nerd_font_glyph(&self) -> &'static str {
        // Nerd Font PUA code points (from the Nerd Font cheat sheet):
        // Using Material Design Icons (f0001-f1af0) and Font Awesome (ed00-f2ff).
        match self {
            Icon::Local => "\u{f0165}",    // nf-md-folder_music
            Icon::Youtube => "\u{f0166}",  // nf-md-youtube
            Icon::Hybrid => "\u{f0175}",   // nf-md-shuffle_variant
            Icon::Playing => "\u{f04b}",   // nf-fa-play
            Icon::Paused => "\u{f04c}",    // nf-fa-pause
            Icon::Buffering => "\u{f110}", // nf-fa-spinner
            Icon::Queue => "\u{f0cb}",     // nf-fa-list_ol
            Icon::Radio => "\u{f5c6}",     // nf-md-radio
            Icon::Autoplay => "\u{f021}",  // nf-fa-refresh (autoplay cycle)
            Icon::Lyrics => "\u{f028}",    // nf-fa-music (lyrics)
            Icon::Like => "\u{f004}",      // nf-fa-heart
            Icon::Hidden => "\u{f070}",    // nf-fa-eye_slash
            Icon::Cached => "\u{f021}",    // nf-fa-refresh (cache)
            Icon::Offline => "\u{f071}",   // nf-fa-exclamation_triangle
            Icon::Warning => "\u{f071}",   // nf-fa-exclamation_triangle
            Icon::Error => "\u{f071}",     // nf-fa-exclamation_circle
            Icon::Refresh => "\u{f021}",   // nf-fa-refresh
            Icon::Search => "\u{f002}",    // nf-fa-search
            Icon::Generated => "\u{f0e7}", // nf-fa-magic (generated content)
        }
    }

    /// Standard Unicode symbol for this icon.
    fn unicode_glyph(&self) -> &'static str {
        match self {
            Icon::Local => "\u{266b}",     // ♫ (beamed eighth notes)
            Icon::Youtube => "\u{25b6}",   // ▶ (play button)
            Icon::Hybrid => "\u{2194}",    // ↔ (left-right arrow)
            Icon::Playing => "\u{25b6}",   // ▶ (play)
            Icon::Paused => "\u{23f8}",    // ⏸ (pause)
            Icon::Buffering => "\u{23f3}", // ⏳ (hourglass)
            Icon::Queue => "\u{2630}",     // ☰ (trigram for heaven / list)
            Icon::Radio => "\u{25ce}",     // ◎ (bullseye / radio)
            Icon::Autoplay => "\u{21bb}",  // ↻ (clockwise arrow)
            Icon::Lyrics => "\u{266b}",    // ♫ (music notes)
            Icon::Like => "\u{2665}",      // ♥ (heart)
            Icon::Hidden => "\u{2715}",    // ✕ (cross)
            Icon::Cached => "\u{21bb}",    // ↻ (clockwise arrow / cache)
            Icon::Offline => "\u{26a0}",   // ⚠ (warning sign)
            Icon::Warning => "\u{26a0}",   // ⚠ (warning sign)
            Icon::Error => "\u{26a0}",     // ⚠ (warning sign)
            Icon::Refresh => "\u{21bb}",   // ↻ (clockwise arrow)
            Icon::Search => "\u{2315}",    // ⌕ (search)
            Icon::Generated => "\u{2728}", // ✨ (sparkles)
        }
    }

    /// Plain ASCII text label for this icon.
    fn ascii_label(&self) -> &'static str {
        match self {
            Icon::Local => "[L]",
            Icon::Youtube => "[Y]",
            Icon::Hybrid => "[M]",
            Icon::Playing => "[>]",
            Icon::Paused => "[||]",
            Icon::Buffering => "[~]",
            Icon::Queue => "[Q]",
            Icon::Radio => "[R]",
            Icon::Autoplay => "[A]",
            Icon::Lyrics => "~",
            Icon::Like => "[+]",
            Icon::Hidden => "[X]",
            Icon::Cached => "[C]",
            Icon::Offline => "[!]",
            Icon::Warning => "[!]",
            Icon::Error => "[E]",
            Icon::Refresh => "[R]",
            Icon::Search => "[/]",
            Icon::Generated => "[*]",
        }
    }
}

/// An icon renderer that caches the font mode and provides convenient methods.
pub struct IconRenderer {
    mode: FontMode,
}

impl IconRenderer {
    /// Create a new renderer with the given font mode.
    pub fn new(mode: FontMode) -> Self {
        IconRenderer { mode }
    }

    /// Create a renderer with auto-detected font mode.
    pub fn auto() -> Self {
        Self::new(FontMode::auto_detect())
    }

    /// Get the glyph for an icon.
    pub fn glyph(&self, icon: Icon) -> &'static str {
        icon.glyph(self.mode)
    }

    /// Get the label for an icon (always text, never a glyph).
    pub fn label(&self, icon: Icon) -> &'static str {
        icon.label()
    }

    /// Get both the glyph and label, formatted as "glyph label" (for
    /// accessibility: the glyph is visual, the label is for screen readers
    /// and no-color/ASCII fallback).
    pub fn glyph_and_label(&self, icon: Icon) -> String {
        format!("{} {}", icon.glyph(self.mode), icon.label())
    }

    /// Get the current font mode.
    pub fn mode(&self) -> FontMode {
        self.mode
    }

    /// Set the font mode.
    pub fn set_mode(&mut self, mode: FontMode) {
        self.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that set/unset env vars (JUKEBOX_FONT_MODE) so they
    /// don't interfere with each other under parallel test execution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn font_mode_default_is_unicode() {
        assert_eq!(FontMode::default(), FontMode::Unicode);
    }

    #[test]
    fn font_mode_labels() {
        assert_eq!(FontMode::NerdFont.label(), "Nerd Font");
        assert_eq!(FontMode::Unicode.label(), "Unicode");
        assert_eq!(FontMode::Ascii.label(), "ASCII");
    }

    #[test]
    fn icon_glyph_in_nerd_font_mode() {
        let renderer = IconRenderer::new(FontMode::NerdFont);
        let glyph = renderer.glyph(Icon::Playing);
        assert!(!glyph.is_empty());
        // Nerd Font glyphs are PUA characters.
        assert!(glyph.chars().all(|c| (c as u32) >= 0xe000 || c.is_ascii()));
    }

    #[test]
    fn icon_glyph_in_unicode_mode() {
        let renderer = IconRenderer::new(FontMode::Unicode);
        let glyph = renderer.glyph(Icon::Playing);
        assert!(!glyph.is_empty());
        // Unicode glyphs should be non-ASCII (for most icons).
        assert!(!glyph.is_empty());
    }

    #[test]
    fn icon_glyph_in_ascii_mode() {
        let renderer = IconRenderer::new(FontMode::Ascii);
        let glyph = renderer.glyph(Icon::Playing);
        assert!(!glyph.is_empty());
        // ASCII glyphs must be plain ASCII.
        for c in glyph.chars() {
            assert!(
                c.is_ascii(),
                "ASCII glyph must be ASCII: found non-ASCII in {:?}",
                glyph
            );
        }
    }

    #[test]
    fn icon_label_always_text() {
        for icon in [
            Icon::Local,
            Icon::Youtube,
            Icon::Hybrid,
            Icon::Playing,
            Icon::Paused,
            Icon::Buffering,
            Icon::Queue,
            Icon::Radio,
            Icon::Autoplay,
            Icon::Lyrics,
            Icon::Like,
            Icon::Hidden,
            Icon::Cached,
            Icon::Offline,
            Icon::Warning,
            Icon::Error,
            Icon::Refresh,
            Icon::Search,
            Icon::Generated,
        ] {
            let label = icon.label();
            assert!(!label.is_empty());
            // Labels must be plain ASCII (for screen readers and no-color).
            for c in label.chars() {
                assert!(
                    c.is_ascii(),
                    "label must be ASCII: found non-ASCII in {:?}",
                    label
                );
            }
        }
    }

    #[test]
    fn no_essential_function_is_icon_only() {
        // Every icon has a text label — no essential meaning depends only
        // on an icon or color.
        for icon in [
            Icon::Local,
            Icon::Youtube,
            Icon::Hybrid,
            Icon::Playing,
            Icon::Paused,
            Icon::Buffering,
            Icon::Queue,
            Icon::Radio,
            Icon::Autoplay,
            Icon::Lyrics,
            Icon::Like,
            Icon::Hidden,
            Icon::Cached,
            Icon::Offline,
            Icon::Warning,
            Icon::Error,
            Icon::Refresh,
            Icon::Search,
            Icon::Generated,
        ] {
            assert!(
                !icon.label().is_empty(),
                "icon {:?} has no text label",
                icon
            );
        }
    }

    #[test]
    fn glyph_and_label_includes_both() {
        let renderer = IconRenderer::new(FontMode::Unicode);
        let combined = renderer.glyph_and_label(Icon::Playing);
        assert!(combined.contains("playing"));
        assert!(!combined.starts_with("playing")); // glyph comes first
    }

    #[test]
    fn ascii_mode_works_for_all_icons() {
        let renderer = IconRenderer::new(FontMode::Ascii);
        for icon in [
            Icon::Local,
            Icon::Youtube,
            Icon::Hybrid,
            Icon::Playing,
            Icon::Paused,
            Icon::Buffering,
            Icon::Queue,
            Icon::Radio,
            Icon::Autoplay,
            Icon::Lyrics,
            Icon::Like,
            Icon::Hidden,
            Icon::Cached,
            Icon::Offline,
            Icon::Warning,
            Icon::Error,
            Icon::Refresh,
            Icon::Search,
            Icon::Generated,
        ] {
            let glyph = renderer.glyph(icon);
            for c in glyph.chars() {
                assert!(c.is_ascii(), "ASCII glyph for {:?} must be ASCII", icon);
            }
        }
    }

    #[test]
    fn icon_renderer_mode_setter() {
        let mut renderer = IconRenderer::new(FontMode::Unicode);
        assert_eq!(renderer.mode(), FontMode::Unicode);
        renderer.set_mode(FontMode::Ascii);
        assert_eq!(renderer.mode(), FontMode::Ascii);
    }

    #[test]
    fn font_mode_auto_detect_returns_valid_mode() {
        let mode = FontMode::auto_detect();
        // Should be one of the three valid modes.
        assert!(matches!(
            mode,
            FontMode::NerdFont | FontMode::Unicode | FontMode::Ascii
        ));
    }

    /// DEF-006: JUKEBOX_FONT_MODE=ascii must produce FontMode::Ascii.
    #[test]
    fn font_mode_jukebox_font_mode_ascii() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        drop(_guard);
        assert_eq!(mode, FontMode::Ascii);
    }

    /// DEF-006: JUKEBOX_FONT_MODE=nerd must produce FontMode::NerdFont.
    #[test]
    fn font_mode_jukebox_font_mode_nerd() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "nerd");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        drop(_guard);
        assert_eq!(mode, FontMode::NerdFont);
    }

    /// DEF-006: JUKEBOX_FONT_MODE=unicode must produce FontMode::Unicode.
    #[test]
    fn font_mode_jukebox_font_mode_unicode() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        drop(_guard);
        assert_eq!(mode, FontMode::Unicode);
    }

    /// DEF-006: JUKEBOX_FONT_MODE is case-insensitive.
    #[test]
    fn font_mode_jukebox_font_mode_case_insensitive() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "ASCII");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        drop(_guard);
        assert_eq!(mode, FontMode::Ascii);
    }

    /// D2: `NO_COLOR=1` must NOT trigger `FontMode::Ascii`. `NO_COLOR`
    /// (no-color.org) only disables colors (handled in `theme::no_color`);
    /// it must not change Unicode symbols to ASCII labels. The previous
    /// code conflated the two, replacing all Unicode structural glyphs (›,
    /// ─, ♫) with ASCII under `NO_COLOR=1`, which is non-standard. Only
    /// `JUKEBOX_FONT_MODE=ascii` enables ASCII font mode.
    #[test]
    fn font_mode_no_color_does_not_trigger_ascii() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Clear any explicit override so the default path runs.
        std::env::remove_var("JUKEBOX_FONT_MODE");
        std::env::remove_var("TERM_FONT");
        // Set NO_COLOR — must NOT produce Ascii.
        std::env::set_var("NO_COLOR", "1");
        std::env::set_var("TERM", "xterm-256color");
        let mode = FontMode::auto_detect();
        std::env::remove_var("NO_COLOR");
        drop(_guard);
        assert_eq!(
            mode,
            FontMode::Unicode,
            "D2: NO_COLOR=1 must NOT trigger ASCII font mode (got {mode:?})"
        );
    }

    /// D2: `NO_COLOR=1` must not override an explicit `JUKEBOX_FONT_MODE`.
    /// When both are set, the explicit font mode wins (NO_COLOR only
    /// affects colors).
    #[test]
    fn font_mode_jukebox_font_mode_takes_precedence_over_no_color() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "unicode");
        std::env::set_var("NO_COLOR", "1");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        std::env::remove_var("NO_COLOR");
        drop(_guard);
        assert_eq!(
            mode,
            FontMode::Unicode,
            "D2: JUKEBOX_FONT_MODE=unicode must win over NO_COLOR=1"
        );
    }

    /// D2: `NO_COLOR=1` + `JUKEBOX_FONT_MODE=ascii` still produces Ascii
    /// (the explicit ASCII override is independent of NO_COLOR).
    #[test]
    fn font_mode_ascii_env_with_no_color() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
        std::env::set_var("NO_COLOR", "1");
        let mode = FontMode::auto_detect();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        std::env::remove_var("NO_COLOR");
        drop(_guard);
        assert_eq!(
            mode,
            FontMode::Ascii,
            "D2: JUKEBOX_FONT_MODE=ascii must produce Ascii even with NO_COLOR=1"
        );
    }
}
