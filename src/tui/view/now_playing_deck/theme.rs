//! Semantic `PlayerTheme` for the Now Playing Deck.
//!
//! A pre-computed struct of `Style`s for every surface in the deck.
//! Option B from `research/03-theme-a11y.md`: a struct of `Style` fields
//! the view layer reads with `p.track_title` (no method call per span).
//! `no_color` / `high_contrast` are evaluated **once** when
//! `Theme::player()` is called, cutting ~50 `no_color()` calls per frame
//! to 1.
//!
//! ## Accessibility
//!
//! Every state has at least one non-color differentiator (icon, border
//! weight, marker, modifier, or label text) so the deck remains usable
//! under `NO_COLOR`, in 16-color terminals, and for color-blind users.
//! See `research/03-theme-a11y.md` §4 for the full differentiator table.

use ratatui::style::{Modifier, Style};

use crate::tui::view::theme::Theme;

/// Pre-computed semantic `Style`s for the Now Playing Deck. Construct via
/// [`Theme::player`]. Every field is a `Style` (cheap `Copy`), so callers
/// do `let p = theme.player(); Span::styled(title, p.track_title)` with
/// no per-span method call.
///
/// ## Transparency-safe by construction
///
/// `player_surface` is `Style::default()` — no `bg`. The deck leaves
/// ordinary cell backgrounds as terminal-default/reset so the user's
/// terminal transparency remains visible. Hierarchy comes from
/// foreground styling, borders, spacing, and modifiers — never from a
/// filled surface. Only `control_selected` / `up_next_selected` /
/// `progress_thumb` set a localized `bg(accent)` for the *selected*
/// control / row / playhead (per spec: "small solid background for
/// selected controls" is acceptable).
///
/// The existing method-form helpers (`selection_style` / `error_style`)
/// stay for callers outside the deck; `PlayerTheme` is additive.
#[derive(Clone, Debug)]
pub struct PlayerTheme {
    /// Player panel background. **Always `Style::default()`** — no `bg`.
    /// Terminal transparency is preserved by construction. Callers must
    /// not set `bg` here; use foreground styling + borders + spacing for
    /// structure.
    pub player_surface: Style,
    /// Unfocused player border. `Gray` fg + `Plain` border weight.
    pub player_border: Style,
    /// Focused player border. `Cyan` fg + `Thick` border weight (the
    /// non-color cue is the border weight, applied by the caller via
    /// `BorderType::Thick`).
    pub player_border_focused: Style,
    /// Track title — the visual hero. `accent` + `BOLD`.
    pub track_title: Style,
    /// Track artist. `text` (terminal default).
    pub track_artist: Style,
    /// Track album. `text` with a `·` separator (the separator is the
    /// non-color cue).
    pub track_album: Style,
    /// Quality / format badge. Color depends on the quality tier (hi-res
    /// / cd / yt); the text label (`"24bit-96kHz"` / `"YT 160k"`) is the
    /// non-color cue.
    pub metadata_badge_hi: Style,
    pub metadata_badge_cd: Style,
    pub metadata_badge_yt: Style,
    /// `[L]` / `[Y]` source badge. The bracketed text label is the
    /// non-color cue.
    pub source_badge_local: Style,
    pub source_badge_yt: Style,
    /// Progress `M:SS` elapsed time. `dim`.
    pub progress_elapsed: Style,
    /// Progress `M:SS` remaining / total. `dim`.
    pub progress_remaining: Style,
    /// Progress filled portion `▰▰▰`. `accent` + `BOLD` (the block-char
    /// shape is the non-color cue).
    pub progress_filled: Style,
    /// Progress empty portion `▱▱▱`. `dim` (the block-char shape is the
    /// non-color cue).
    pub progress_track: Style,
    /// Progress playhead `●`. `accent` + `REVERSED` + `BOLD` (reverse
    /// video is the non-color cue; the glyph shape is the second cue).
    pub progress_thumb: Style,
    /// `[PLAYING]` label + `▶` glyph. `playing` (Magenta) + `BOLD`. Three
    /// non-color cues: `[PLAYING]` text, `▶` glyph, BOLD.
    pub playback_playing: Style,
    /// `[PAUSED]` label + `⏸` glyph. `warning` (Yellow) + `BOLD`. Two
    /// non-color cues: `[PAUSED]` text, `⏸` glyph.
    pub playback_paused: Style,
    /// `[STOPPED]` label + `■` glyph. `dim`. Two non-color cues: text +
    /// glyph.
    pub playback_stopped: Style,
    /// `[BUFFERING]` label + spinner. `accent` + `BOLD`. Two non-color
    /// cues: text + spinner glyph.
    pub playback_loading: Style,
    /// `[ERR]` prefix + `!` glyph. `error` (Red) + `BOLD`. Two non-color
    /// cues: `[ERR]` text, `!` glyph.
    pub playback_error: Style,
    /// Transport glyph (prev/next), unfocused. `dim`.
    pub control_normal: Style,
    /// Transport glyph, focused deck. `accent` + `BOLD`.
    pub control_focused: Style,
    /// Active transport button (hover/pressed). `REVERSED` + `BOLD` (two
    /// non-color cues).
    pub control_selected: Style,
    /// `SHUF on` / `RPT all` / `CONT radio` (active flag). `accent` +
    /// `BOLD`. Two cues: BOLD + "on/all/radio" text.
    pub mode_enabled: Style,
    /// `SHUF off` / `RPT off` (inactive flag). `dim`. One cue: "off" text.
    pub mode_disabled: Style,
    /// `[ok]` success toast. `success` (Green) + `BOLD`. Two cues: `[ok]`
    /// text + BOLD.
    pub toast_success: Style,
    /// `[!]` warning toast. `warning` (Yellow) + `BOLD`. Two cues: `[!]`
    /// text + BOLD.
    pub toast_warning: Style,
    /// `[ERR]` error toast. `error` (Red) + `BOLD`. Two cues: `[ERR]` text
    /// + BOLD.
    pub toast_error: Style,
    /// `▸ Next: title` row. `dim`. One cue: `▸` marker glyph.
    pub up_next_item: Style,
    /// Selected up-next row. `REVERSED` + `BOLD` (two cues).
    pub up_next_selected: Style,
}

impl Theme {
    /// Build the `PlayerTheme` snapshot for this `Theme`. Evaluates
    /// `no_color` / `high_contrast` once and returns a struct of `Style`s
    /// the caller reads with `p.track_title` etc. Cuts ~50 `no_color()`
    /// calls per frame to 1.
    pub fn player(&self) -> PlayerTheme {
        let bold = Modifier::BOLD;
        let reversed_bold = Modifier::REVERSED | Modifier::BOLD;

        PlayerTheme {
            // Transparency-safe: no `bg`. Terminal-default cell
            // backgrounds preserve the user's wallpaper. Hierarchy
            // comes from foreground styling + borders + spacing.
            player_surface: Style::default(),
            player_border: Style::default().fg(self.border),
            player_border_focused: Style::default().fg(self.border_focused),
            track_title: Style::default().fg(self.accent).add_modifier(bold),
            track_artist: Style::default().fg(self.text),
            track_album: Style::default().fg(self.text),
            metadata_badge_hi: Style::default().fg(self.hires),
            metadata_badge_cd: Style::default().fg(self.cd),
            metadata_badge_yt: Style::default().fg(self.source_yt),
            source_badge_local: Style::default().fg(self.source_local),
            source_badge_yt: Style::default().fg(self.source_yt),
            progress_elapsed: Style::default().fg(self.dim),
            progress_remaining: Style::default().fg(self.dim),
            progress_filled: Style::default().fg(self.accent).add_modifier(bold),
            progress_track: Style::default().fg(self.dim),
            progress_thumb: Style::default()
                .fg(self.hi_fg)
                .bg(self.accent)
                .add_modifier(reversed_bold),
            playback_playing: Style::default().fg(self.playing).add_modifier(bold),
            playback_paused: Style::default().fg(self.warning).add_modifier(bold),
            playback_stopped: Style::default().fg(self.dim),
            playback_loading: Style::default().fg(self.accent).add_modifier(bold),
            playback_error: Style::default().fg(self.error).add_modifier(bold),
            control_normal: Style::default().fg(self.dim),
            control_focused: Style::default().fg(self.accent).add_modifier(bold),
            control_selected: Style::default()
                .fg(self.hi_fg)
                .bg(self.accent)
                .add_modifier(reversed_bold),
            mode_enabled: Style::default().fg(self.accent).add_modifier(bold),
            mode_disabled: Style::default().fg(self.dim),
            toast_success: Style::default().fg(self.success).add_modifier(bold),
            toast_warning: Style::default().fg(self.warning).add_modifier(bold),
            toast_error: Style::default().fg(self.error).add_modifier(bold),
            up_next_item: Style::default().fg(self.dim),
            up_next_selected: Style::default()
                .fg(self.hi_fg)
                .bg(self.accent)
                .add_modifier(reversed_bold),
        }
    }
}
