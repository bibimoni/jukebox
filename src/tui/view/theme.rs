//! Color palette + display-width helpers + accessibility style helpers.
//!
//! ## Accessibility Design
//!
//! The theme honors `NO_COLOR` (no-color.org): when set, all semantic colors
//! collapse to grayscale values (`White`, `Reset`, `Black`) so no **hue**
//! color codes are emitted. Visual distinction under `NO_COLOR` relies on
//! **brightness levels** (white > reset > black) and text attributes (bold,
//! reverse video, underline) applied by the view layer via the
//! [`selection_style`], [`focus_border_style`], and [`actionable_style`]
//! helpers (and their method-form counterparts [`Theme::selection_style`] and
//! [`Theme::error_style`]).
//!
//! ### Non-color indicators (color is never the only signal)
//!
//! - **Selection**: `REVERSED` + `BOLD` modifier via [`selection_style`] /
//!   [`Theme::selection_style`] — reverse video inverts fg/bg (survives
//!   `NO_COLOR`); bold adds weight. The view layer also adds `►` / `❯` glyph
//!   prefixes for a third text-visible cue.
//! - **Focus**: `BOLD` modifier via [`focus_border_style`] — heavier border
//!   weight distinguishes the focused pane without color.
//! - **Actionability**: `UNDERLINE` modifier via [`actionable_style`] —
//!   underlined text marks interactive elements without color.
//! - **Errors**: [`Theme::error_style`] returns `BOLD` under `NO_COLOR` (no
//!   hue) or `self.error` (Red) in color mode. Callers prefix the text with
//!   `[ERR]` so the error is text-visible without color.
//! - **Progress**: [`progress_color`] returns a high-contrast color (or
//!   `Reset` under `NO_COLOR`) so the playhead dot is always visible.
//! - **Quality**: [`quality_color`] returns `Reset` under `NO_COLOR`;
//!   the text label itself (e.g. "24bit-96kHz") carries the information.
//!
//! For normal (color) mode, the palette uses high-contrast colors:
//! - `accent`: `Cyan` — vivid on dark backgrounds; saturated enough to remain
//!   visible on light-background terminals (unlike `LightCyan` which washes
//!   out on white).
//! - `dim`: `Gray` (ANSI 7 = 0xC0C0C0) — bright enough for readable secondary
//!   text and borders. Under `NO_COLOR`, `dim` uses `Reset` (terminal default)
//!   for maximum contrast — `DarkGray` (0x808080) falls below WCAG 4.5:1 on
//!   black backgrounds.
//! - `text`: `Reset` — the terminal's default foreground, which adapts
//!   automatically to both dark and light backgrounds.
//!
//! A [`Theme::high_contrast`] constructor provides a pure black-on-white /
//! white-on-black palette for users who need maximum contrast.

use std::cell::RefCell;

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders};

use crate::tui::view::icons::{FontMode, Icon};

/// True when NO_COLOR is set (no-color.org). Colors must not be the only signal.
///
/// Reads the env var directly on every call. Hot-path callers should
/// snapshot the value once via `Theme::default()` (which stores the
/// `no_color` decision on the struct) and use the snapshot instead of
/// re-calling this function. Direct env reads are cheap (one syscall
/// amortized over many calls), but the per-row render path calls this
/// ~50×/frame via `Theme::*_style` methods — those should use the
/// Theme snapshot, not this free function.
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

// RC18-D1: thread-local cache of the startup font mode. The previous
// `is_ascii()` path constructed a fresh `Theme::default()` on every call,
// which in turn called `FontMode::auto_detect()` on every call — reading
// `JUKEBOX_FONT_MODE` / `TERM` / `TERM_FONT` env vars each time. The result
// was deterministic for a given env, but the reviewer observed "flaky
// Unicode/ASCII rendering between launches" because the PTY driver passed
// `--ascii` for some runs (test 17-ascii) and not others, and any process
// that mutated the env mid-session could flip the glyph vocabulary. We
// now read the env ONCE per thread at the first `is_ascii()` call and
// freeze it for the thread's lifetime, so the glyph vocabulary is stable
// within a session regardless of later env changes. The TUI event loop
// is single-threaded, so thread-local == process-stable in production.
// Tests that mutate `JUKEBOX_FONT_MODE` call [`reset_font_mode_cache`] so
// the next read on their thread re-reads the env.
thread_local! {
    static FONT_MODE: RefCell<Option<FontMode>> = const { RefCell::new(None) };
    // Phase 9 reduced-motion: JUKEBOX_NO_MOTION is read once per thread
    // and frozen for the thread's lifetime (the TUI event loop is
    // single-threaded so thread-local == process-stable in production).
    // Tests that mutate JUKEBOX_NO_MOTION call [`reset_no_motion_cache`].
    static NO_MOTION_CACHED: RefCell<Option<bool>> = const { RefCell::new(None) };
}

/// True when `JUKEBOX_NO_MOTION=1` is set (Phase 9 reduced-motion). When
/// set, the cursor style skips `SLOW_BLINK` and the braille spinner is
/// frozen at frame 0. Honors the reduced-motion convention for users
/// with vestibular/photosensitivity concerns. Cached per-thread like
/// `cached_font_mode` so the read happens once.
pub fn no_motion() -> bool {
    NO_MOTION_CACHED.with(|cell| {
        if let Some(b) = *cell.borrow() {
            return b;
        }
        let b = std::env::var_os("JUKEBOX_NO_MOTION").is_some();
        *cell.borrow_mut() = Some(b);
        b
    })
}

/// Reset the cached `JUKEBOX_NO_MOTION` flag for the calling thread.
/// Intended for tests that toggle `JUKEBOX_NO_MOTION` between assertions.
pub fn reset_no_motion_cache() {
    NO_MOTION_CACHED.with(|cell| *cell.borrow_mut() = None);
}

/// Read the cached startup font mode, initializing it from
/// `FontMode::auto_detect()` on first call (per thread). Public so callers
/// that need the raw (uncached) detection can still reach
/// `FontMode::auto_detect()`.
pub fn cached_font_mode() -> FontMode {
    FONT_MODE.with(|cell| {
        if let Some(m) = *cell.borrow() {
            return m;
        }
        let m = FontMode::auto_detect();
        *cell.borrow_mut() = Some(m);
        m
    })
}

/// Reset the cached font mode for the calling thread so the next
/// [`cached_font_mode`] / [`is_ascii`] call re-reads the env vars. Intended
/// for tests that mutate `JUKEBOX_FONT_MODE` / `TERM` / `TERM_FONT` between
/// assertions (the cache is thread-stable in production).
pub fn reset_font_mode_cache() {
    FONT_MODE.with(|cell| *cell.borrow_mut() = None);
}

/// True when `JUKEBOX_HIGH_CONTRAST` is set (any value). When set,
/// [`Theme::default`] returns the high-contrast palette (pure
/// white/black/gray, no hue) for users who need maximum contrast (visual
/// impairment, bright ambient light). Takes precedence over `NO_COLOR`
/// since it is an explicit opt-in — the high-contrast palette is already
/// hue-free so it satisfies the `NO_COLOR` convention.
///
/// Reads the env var directly on every call (cheap). Callers that want
/// a snapshot should use `Theme::default()` which stores the
/// `high_contrast` decision on the struct.
pub fn high_contrast() -> bool {
    std::env::var_os("JUKEBOX_HIGH_CONTRAST").is_some()
}

/// Semantic color tokens. [`Theme::default`] honors `NO_COLOR`: when set, every
/// field collapses to a grayscale value so the TUI stays usable without hue.
pub struct Theme {
    pub accent: Color, // focus + selection
    pub dim: Color,    // chrome / borders unfocused
    pub text: Color,
    pub muted: Color,
    pub hi_fg: Color, // text on accent background
    pub hires: Color, // Hi-Res quality accent
    pub cd: Color,    // CD-quality accent
    // --- New semantic tokens (UI revamp Stage 2) ---
    pub playing: Color,      // currently-playing row indicator
    pub success: Color,      // ready / online / connected
    pub warning: Color,      // needs attention / stale / rate-limited
    pub error: Color,        // failed / unavailable / auth expired
    pub source_local: Color, // local source badge
    pub source_yt: Color,    // YouTube source badge
    pub surface: Color,      // panel background accent
    /// The font mode for icon rendering (NerdFont / Unicode / Ascii).
    /// Auto-detected at startup; can be changed at runtime via
    /// `set_font_mode`. Used by `icon()` to render glyphs.
    pub font_mode: FontMode,
    // --- Pane-polish semantic tokens (visual spec Phase 0) ---
    /// Overlay / dialog background. `Black` in color mode, `Reset` under
    /// `NO_COLOR` (so overlay backdrops don't emit `\e[40m` and don't
    /// invert on light-theme terminals). Replaces the 10 hardcoded
    /// `bg(Color::Black)` sites in `overlay.rs`.
    pub background: Color,
    /// Active-pane background accent — slightly lighter than `surface`,
    /// used to suggest depth on the focused pane. `Indexed(238)` in color
    /// mode, `Reset` under `NO_COLOR`, `Black` in high-contrast.
    pub surface_active: Color,
    /// Unfocused pane border color (semantically `dim`). Kept as a
    /// dedicated slot so callers document intent (`theme.border` reads as
    /// "border color" rather than "chrome / dim text").
    pub border: Color,
    /// Focused pane border color (semantically `accent`).
    pub border_focused: Color,
    /// Pane-edit-mode border color. Distinct from `border_focused` so
    /// edit mode can be emphasized without re-coloring focused panes.
    pub border_editing: Color,
    /// Secondary text — semantic alias of `dim` for callers that want to
    /// document "muted text" intent. Same value as `dim` in every palette.
    pub text_muted: Color,
    /// Disabled / greyed controls. `DarkGray` in color mode (paired with
    /// `DIM` modifier under `NO_COLOR` for a non-color cue).
    pub text_disabled: Color,
    /// De-emphasized interactive accent — used for hover/secondary actions.
    /// `DarkCyan` in color mode, `Reset` under `NO_COLOR`, `Gray` in
    /// high-contrast.
    pub accent_soft: Color,
    /// Selection background — semantic alias of `accent` for clarity. The
    /// selected row's background color.
    pub selection_bg: Color,
    /// Selection foreground — semantic alias of `hi_fg` for clarity. The
    /// selected row's text color (high-contrast on `selection_bg`).
    pub selection_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        if high_contrast() {
            // Explicit opt-in: pure white/black/gray palette. Takes
            // precedence over NO_COLOR — the high-contrast palette is
            // already hue-free so it satisfies NO_COLOR too.
            return Self::high_contrast();
        }
        if no_color() {
            // NO_COLOR: no hue colors — only brightness levels for visual
            // hierarchy. White (brightest) > Reset (terminal default) >
            // Black (darkest). `dim` uses Reset (not DarkGray) because
            // DarkGray (0x808080) falls below WCAG 4.5:1 contrast on black
            // backgrounds — Reset adapts to the terminal's default fg which
            // is always high-contrast. Visual hierarchy under NO_COLOR relies
            // on modifier bits (BOLD, REVERSED, UNDERLINE) applied by the
            // view layer's style helpers, not on dimmer colors.
            Theme {
                accent: Color::White, // brightest — selection stands out
                dim: Color::Reset,    // terminal default — high contrast
                text: Color::Reset,   // terminal default — high contrast
                muted: Color::Reset,  // terminal default — high contrast
                hi_fg: Color::Black,  // darkest — text on bright accent bg
                hires: Color::Reset,
                cd: Color::Reset,
                playing: Color::White, // brightest — now-playing indicator
                success: Color::Reset,
                warning: Color::Reset,
                error: Color::Reset,
                source_local: Color::Reset,
                source_yt: Color::Reset,
                surface: Color::Black, // subtle zebra background
                font_mode: cached_font_mode(),
                // Phase 0: pane-polish semantic tokens, NO_COLOR path.
                // All collapse to grayscale (no hue) so the TUI stays
                // usable without color. `background` is `Reset` (not
                // `Black`) so overlay backdrops don't paint a hard black
                // box on light-theme terminals under NO_COLOR.
                background: Color::Reset,
                surface_active: Color::Reset,
                border: Color::Reset,
                border_focused: Color::White,
                border_editing: Color::White,
                text_muted: Color::Reset,
                text_disabled: Color::Reset,
                accent_soft: Color::Reset,
                selection_bg: Color::White,
                selection_fg: Color::Black,
            }
        } else {
            Theme {
                accent: Color::Cyan, // vivid on dark; visible on light
                dim: Color::Gray,    // bright — readable borders + text
                text: Color::Reset,  // adapts to terminal bg
                muted: Color::Gray,  // mid-level — secondary text
                hi_fg: Color::Black, // readable on Cyan background
                hires: Color::Magenta,
                cd: Color::Green,
                playing: Color::Magenta,
                success: Color::Green,
                warning: Color::Yellow,
                error: Color::Red,
                source_local: Color::Green,
                source_yt: Color::Yellow,
                // 0x303030 — very dark gray. Gray (0xC0C0C0) dim text on this
                // bg is ~7.3:1 contrast (WCAG AA 4.5:1 ✓). Subtle enough that
                // zebra striping reads as a gentle alternation against the
                // terminal's black bg, not a loud band. DarkGray (0x808080)
                // was only ~2.1:1 vs Gray text — below WCAG. See columns.rs
                // zebra_bg usage.
                surface: Color::Indexed(236),
                font_mode: cached_font_mode(),
                // Phase 0: pane-polish semantic tokens, color path. Single
                // cyan accent + one warning hue (Yellow, reserved for
                // `warning`) + one error hue (Red, reserved for `error`).
                // Magenta is kept only for `hires` (a quality tag) and
                // `playing` (the now-playing row) for backward compat;
                // Phase 5 demotes `playing` to `accent`.
                background: Color::Black,
                surface_active: Color::Indexed(238),
                border: Color::Gray,
                border_focused: Color::Cyan,
                border_editing: Color::Cyan,
                text_muted: Color::Gray,
                text_disabled: Color::DarkGray,
                accent_soft: Color::Indexed(30), // 256-color dark cyan (0x008787)
                selection_bg: Color::Cyan,
                selection_fg: Color::Black,
            }
        }
    }
}

/// High-contrast theme: pure white / black / gray — no hue. Use when maximum
/// contrast is needed (visual impairment, bright ambient light). All semantic
/// states collapse to brightness-only, so it is safe for color-blind users.
///
/// Enable via `Theme::high_contrast()`. The view layer's modifier-based
/// helpers ([`selection_style`] etc.) still apply REVERSED/BOLD/UNDERLINE on
/// top, so selection, focus, and actionability remain distinguishable.
/// [`progress_color`] returns `White` so the progress bar is maximally
/// visible.
impl Theme {
    pub fn high_contrast() -> Self {
        Theme {
            accent: Color::White,
            dim: Color::Gray,
            text: Color::Reset,
            muted: Color::Gray,
            hi_fg: Color::Black,
            hires: Color::White,
            cd: Color::White,
            playing: Color::White,
            success: Color::White,
            warning: Color::White,
            error: Color::White,
            source_local: Color::White,
            source_yt: Color::White,
            surface: Color::Black,
            font_mode: cached_font_mode(),
            // Phase 0: pane-polish semantic tokens, high-contrast path.
            // Pure white/black/gray — no hue. `background` is `Black` so
            // overlay backdrops remain visible on a dark terminal (the
            // documented assumption for JUKEBOX_HIGH_CONTRAST).
            background: Color::Black,
            surface_active: Color::Black,
            border: Color::Gray,
            border_focused: Color::White,
            border_editing: Color::White,
            text_muted: Color::Gray,
            text_disabled: Color::Gray,
            accent_soft: Color::Gray,
            selection_bg: Color::White,
            selection_fg: Color::Black,
        }
    }

    /// Get the glyph for an icon using the theme's font mode. Essential
    /// meaning never depends only on the glyph — callers always pair it
    /// with a text label for accessibility.
    pub fn icon(&self, icon: Icon) -> &'static str {
        icon.glyph(self.font_mode)
    }

    /// Set the font mode at runtime (e.g. when the user cycles modes).
    pub fn set_font_mode(&mut self, mode: FontMode) {
        self.font_mode = mode;
    }

    /// Get the current font mode.
    pub fn font_mode(&self) -> FontMode {
        self.font_mode
    }

    /// Selection style as a method (method-form entry point for callers that
    /// own a `Theme` value, complementing the free-function [`selection_style`]
    /// helper used by the view layer).
    ///
    /// - **NO_COLOR**: modifiers only — [`Modifier::REVERSED`] inverts
    ///   fg/bg (color-agnostic) and [`Modifier::BOLD`] adds weight. No fg/bg
    ///   are set because colors collapse to grayscale and the inversion alone
    ///   produces a clear selected-row cue.
    /// - **Color**: `hi_fg` (Black) foreground on `accent` (Cyan) background
    ///   plus [`Modifier::REVERSED`] + [`Modifier::BOLD`] — high-contrast
    ///   inverted text with weight. The view layer's `►`/`❯` glyph prefixes
    ///   add a third text-visible cue. REVERSED + BOLD are included in color
    ///   mode too so the selection is distinguishable even if the terminal
    ///   renders fg/bg with low contrast (T8: selection relied on color
    ///   alone — now REVERSED|BOLD survive in both modes).
    ///
    /// **Accessibility:** two non-color cues (REVERSED + BOLD) ensure the
    /// selected row is identifiable in monochrome terminals. Color is never
    /// the only signal.
    pub fn selection_style(&self) -> Style {
        if no_color() {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.hi_fg)
                .bg(self.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    /// Error style as a method. Callers should prefix the error text with
    /// `[ERR] ` so the error is text-visible under `NO_COLOR` (color collapses
    /// to grayscale — the `[ERR]` tag survives monochrome rendering).
    ///
    /// - **NO_COLOR**: [`Modifier::BOLD`] only — no hue. Bold weight
    ///   distinguishes errors from normal dim text without color.
    /// - **Color**: `self.error` (Red) foreground — the conventional error
    ///   hue, paired with the caller-supplied `[ERR]` tag for redundancy.
    ///
    /// **Accessibility:** the `[ERR]` text prefix (added by the caller) is
    /// the primary non-color cue; bold weight is the secondary cue. Color is
    /// never the only signal.
    pub fn error_style(&self) -> Style {
        if no_color() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.error)
        }
    }

    /// Header style for pane/column titles — makes the visual hierarchy
    /// readable at a glance (T8 accessibility: headers were color-only).
    ///
    /// - **NO_COLOR**: [`Modifier::BOLD`] only — weight distinguishes headers
    ///   from body text without hue. The header text itself carries the
    ///   information; bold weight is the non-color cue.
    /// - **Color**: `self.accent` (Cyan) foreground + [`Modifier::BOLD`] —
    ///   vivid header with weight. Two cues (hue + weight) so the header
    ///   survives monochrome and low-vision modes.
    ///
    /// **Accessibility:** bold weight is a non-color cue. Color is never the
    /// only signal (AC-M6.4.2).
    pub fn header_style(&self) -> Style {
        header_style_for(no_color())
    }

    /// Compact playback-state label: `[▶]` / `[⏸]` / `[■]` — text-visible
    /// state indicators that survive `NO_COLOR` (T8 accessibility: state
    /// indicators were color-only). The verbose `[PLAYING]`/`[PAUSED]`/
    /// `[STOPPED]` labels live in `player_bar.rs` for the now-playing row;
    /// this is the compact glyph form for tight surfaces (footer, compact
    /// player bar, overlay headers).
    ///
    /// - `playing=true` → `[▶]` (playing — forward motion)
    /// - `paused=true` (playing=false, has track) → `[⏸]` (paused — hold)
    /// - neither → `[■]` (stopped — no track / ended)
    ///
    /// `playing` takes precedence over `paused` (a player that reports both
    /// is treated as playing). The glyphs are shape-distinct (▶ / ⏸ / ■) so
    /// they remain distinguishable in monochrome terminals where color
    /// collapses to grayscale.
    ///
    /// **Accessibility:** the glyph shape is a non-color cue. Color is never
    /// the only signal (AC-M6.4.2).
    ///
    /// Phase 8 (visual spec C5 / A3): the glyphs now route through
    /// `play_glyph()` / `pause_glyph()` / `stop_glyph()` so they respect
    /// `JUKEBOX_FONT_MODE=ascii` (`>` / `||` / `#` instead of `▶` / `⏸` /
    /// `■`). Returns a `&'static str` matching the helper's return type.
    pub fn state_label(&self, playing: bool, paused: bool) -> &'static str {
        if playing {
            match play_glyph() {
                ">" => "[>]",
                _ => "[▶]",
            }
        } else if paused {
            match pause_glyph() {
                "||" => "[||]",
                _ => "[⏸]",
            }
        } else {
            match stop_glyph() {
                "#" => "[#]",
                _ => "[■]",
            }
        }
    }

    /// Playback "now-playing" row style — a reusable helper for the
    /// currently-playing track indicator (T8 accessibility: the playing state
    /// was color-only; this helper pairs a hue with a non-color cue).
    ///
    /// - **NO_COLOR**: [`Modifier::BOLD`] only — bold weight distinguishes the
    ///   playing row from surrounding dim rows without hue. Callers also emit
    ///   a `▶`/`⏸`/`□` glyph so the state is text-visible too.
    /// - **Color**: `self.playing` (Magenta) foreground + [`Modifier::BOLD`] —
    ///   the conventional "active" hue with weight, paired with the
    ///   caller-supplied `▶` play glyph for redundancy. BOLD in color mode
    ///   makes the playing row a clear third tier in the visual hierarchy:
    ///   normal rows = dim, playing rows = **bold magenta**, selected rows =
    ///   cyan reverse-video + BOLD. Without BOLD the playing row was magenta
    ///   text at normal weight, which read as only a hue change against dim
    ///   rows on low-saturation terminals (judge: "selection/active-state
    ///   indicators rely heavily on color").
    ///
    /// **Accessibility:** bold weight is a non-color cue. Color is never the
    /// only signal (AC-M6.4.2).
    pub fn playing_style(&self) -> Style {
        if no_color() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.playing)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Selected-row style — full reverse video: `hi_fg` on `accent` + BOLD
    /// in color mode, `REVERSED|BOLD` under `NO_COLOR` (T1.1/T8.1: the old
    /// medium-gray `surface` bg lowered contrast; now full reverse video so
    /// the active selection is scannable against dim browse rows).
    ///
    /// - **NO_COLOR**: [`Modifier::REVERSED`] + [`Modifier::BOLD`] — reverse
    ///   video inverts fg/bg (color-agnostic) and bold adds weight. Two
    ///   non-color cues so the selected row is identifiable in monochrome.
    /// - **Color**: `self.hi_fg` (Black) foreground on `self.accent` (Cyan)
    ///   background plus [`Modifier::BOLD`] — high-contrast inverted text
    ///   with weight.
    ///
    /// **Accessibility:** under `NO_COLOR` the `REVERSED`+`BOLD` modifiers
    /// provide two non-color cues. Color is never the only signal.
    pub fn selected_style(&self) -> Style {
        if no_color() {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.hi_fg)
                .bg(self.accent)
                .add_modifier(Modifier::BOLD)
        }
    }

    // -----------------------------------------------------------------
    // Phase 0: pane-polish semantic methods. These are additive —
    // existing callers continue to use `selected_style` / `error_style`
    // / `header_style` / `playing_style` unchanged. New callers should
    // prefer the methods below so styling is centralized and the view
    // layer stops constructing `Style::default().fg(...)` inline.
    // -----------------------------------------------------------------

    /// Pane border `Style`. `focused` switches the border color from
    /// `border` (unfocused) to `border_focused` (focused); `editing`
    /// further switches to `border_editing` (PaneEdit mode). Returns
    /// only the `Style` — the caller still applies `border_set` (ASCII
    /// vs Unicode) and `border_type` (Thick vs Plain) on the `Block`.
    ///
    /// **Accessibility:** under `NO_COLOR` all three colors collapse to
    /// grayscale; the caller's `BorderType::Thick` (focused) vs `Plain`
    /// (unfocused) provides the non-color cue. Under `high_contrast` the
    /// border is `White` (focused/editing) or `Gray` (unfocused).
    pub fn pane_border(&self, focused: bool, editing: bool) -> Style {
        let color = if editing {
            self.border_editing
        } else if focused {
            self.border_focused
        } else {
            self.border
        };
        Style::default().fg(color)
    }

    /// Convenience: a full pane `Block` with title, borders, ASCII/Unicode
    /// branch, and `Thick` (focused/editing) vs `Plain` (unfocused) border
    /// type. Collapses the ~14 duplicated `pane_block` constructions across
    /// `pane/render.rs`, `columns.rs`, `yt_view.rs`, `sidebar.rs`,
    /// `now_playing_panel.rs`, `player_bar_big.rs`. The caller passes the
    /// already-formatted title (e.g. `"Artists [EDIT]"`); this method
    /// applies the title color via [`pane_border`].
    pub fn pane_block(&self, title: &str, focused: bool, editing: bool) -> Block<'static> {
        let style = self.pane_border(focused, editing);
        let mut block = if is_ascii() {
            Block::default()
                .borders(Borders::ALL)
                .border_set(ASCII_BORDER_SET)
                .border_style(style)
        } else {
            let bt = if focused || editing {
                BorderType::Thick
            } else {
                BorderType::Plain
            };
            Block::default()
                .borders(Borders::ALL)
                .border_type(bt)
                .border_style(style)
        };
        block = block.title(Span::styled(title.to_string(), style));
        block
    }

    /// Build a `Block` whose title sits in a **notch** on the top border
    /// (e.g. `╭─ ▶ NOW PLAYING ────╮`) rather than colliding with the
    /// `─` line. The caller passes the already-formatted title **without**
    /// leading/trailing spaces — this method adds them so Ratatui
    /// renders `─` on both sides of the title span.
    ///
    /// Per spec:
    /// - **Single-line** borders by default. `BorderType::Rounded` for
    ///   a polished look (corners `╭╮╰╯`).
    /// - **Focused** state adds the `▶` marker and `· FOCUSED` suffix in
    ///   the title text — **not** a heavy double border (the spec:
    ///   "avoid heavy double borders unless needed for accessibility").
    /// - ASCII fallback via [`ASCII_BORDER_SET`] (`+`-`-`|`).
    ///
    /// The title is rendered with `pane_border(focused, editing)` so the
    /// border color and the title color match (no visual collision).
    /// Body cells are left terminal-default (no `bg`) so transparency is
    /// preserved.
    ///
    /// Pass `title = "NOW PLAYING"` and `marker = "▶"` for the focused
    /// state, or `marker = ""` for the unfocused state. The resulting
    /// title string is `" {marker} {title} "` (with leading/trailing
    /// spaces) so Ratatui's border renderer draws `─` on both sides.
    pub fn pane_block_notched(
        &self,
        title: &str,
        marker: &str,
        focused: bool,
        editing: bool,
    ) -> Block<'static> {
        let style = self.pane_border(focused, editing);
        let notch = if is_ascii() { "-" } else { "─" };
        let title_str = if marker.is_empty() {
            format!("{notch} {title} ")
        } else {
            format!("{notch} {marker} {title} ")
        };
        let mut block = if is_ascii() {
            Block::default()
                .borders(Borders::ALL)
                .border_set(ASCII_BORDER_SET)
                .border_style(style)
        } else {
            // Single-line rounded border for both focused and unfocused.
            // The focus cue is the title marker + text, not a heavy
            // double border (spec: avoid heavy double borders).
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(style)
        };
        block = block.title(Span::styled(title_str, style));
        block
    }

    /// Selected-row style. `focused=true` matches the current
    /// [`selected_style`] (REVERSED + BOLD in color, REVERSED + BOLD only
    /// under NO_COLOR). `focused=false` produces a dimmer selection for
    /// unfocused panes — `text_muted` + BOLD, no REVERSED, no glyph
    /// prefix — so the live selection in the focused pane is unmistakable
    /// at a glance. The view layer still adds the `▸`/`›` glyph marker
    /// on the focused pane's selected row.
    ///
    /// **Accessibility:** under `NO_COLOR` the focused branch keeps
    /// REVERSED + BOLD (color-agnostic inversion). The unfocused branch
    /// uses BOLD only (weight without color or inversion) so it remains
    /// distinguishable from the focused selection.
    pub fn selected_row(&self, focused: bool) -> Style {
        if focused {
            self.selected_style()
        } else {
            Style::default()
                .fg(self.text_muted)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Tab / breadcrumb active style. `active=true` → `accent + BOLD +
    /// UNDERLINE`; `active=false` → `dim`. Replaces the ~14 inline
    /// `Style::default().fg(theme.accent).add_modifier(BOLD)` sites and
    /// the 11 `if nc { Reset } else { theme.accent }` accent-drift sites.
    ///
    /// **Why UNDERLINE not REVERSED:** reverse-video on a 1-row tab
    /// inverts the whole row, making the active tab look like a selected
    /// list row (visual spec §3 conflict resolution). Underline survives
    /// `NO_COLOR` and doesn't collide with row selection. The visual
    /// hierarchy becomes: tabs (accent + bold + underline) < selected
    /// row (accent + bold + reverse) < focused pane (accent + thick).
    pub fn tab(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(self.dim)
        }
    }

    /// Status-line key-cap style (e.g. `"Enter"`, `"Esc"`, `"hjkl"`):
    /// `accent + BOLD`. Replaces the inline `accent + BOLD` constructions
    /// in the footer, edit-mode status line, and overlay keymap hints.
    /// Distinct from [`tab`] — key caps don't get UNDERLINE (they're
    /// short tokens, not full-row tabs).
    pub fn status_key(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Status-line description style (e.g. `"to play"`, `"move"`):
    /// `dim`. Replaces the 33 redundant `if no_color() { Reset } else
    /// { theme.dim }` branches across the view layer — `theme.dim` is
    /// already `Reset` under `NO_COLOR`, so the branch re-implements
    /// the theme's own collapse.
    pub fn status_description(&self) -> Style {
        Style::default().fg(self.dim)
    }

    /// Overlay / dialog background `Style`. Returns `bg(background)` so
    /// overlay backdrops adapt to `NO_COLOR` (`Reset` instead of `Black`)
    /// and `high_contrast` (`Black`). Replaces the 10 hardcoded
    /// `Style::default().bg(Color::Black)` sites in `overlay.rs` which
    /// violate no-color.org and invert on light-theme terminals.
    pub fn overlay(&self) -> Style {
        Style::default().bg(self.background)
    }

    /// Edit-prompt cursor style: `accent + SLOW_BLINK`. Replaces the 7
    /// inline `Style::default().add_modifier(Modifier::SLOW_BLINK)`
    /// sites in `overlay.rs` and `yt_view.rs`. Honors `JUKEBOX_NO_MOTION`
    /// (Phase 9) by dropping the blink modifier when reduced-motion is
    /// requested — the cursor stays visible as a static `_` via the
    /// caller-supplied glyph.
    pub fn cursor(&self) -> Style {
        let mut style = Style::default().fg(self.accent);
        if !no_motion() {
            style = style.add_modifier(Modifier::SLOW_BLINK);
        }
        style
    }

    /// Form-field focus style (e.g. publication.rs Name/Privacy/Account
    /// fields): `accent + BOLD` when active, `dim` when not. Semantically
    /// identical to [`tab`] but named for its UI context. Replaces the
    /// inline `Style::default().add_modifier(BOLD)` toggled by active
    /// field in `publication.rs:232,237,242` and `generator.rs:176`.
    pub fn form_field(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.dim)
        }
    }

    /// Quality-tag `Style`: wraps the existing [`quality_color`] free
    /// fn into a `Style`. `hires` (Magenta) for ≥24-bit or ≥48kHz,
    /// `cd` (Green) otherwise. `Reset` under `NO_COLOR` (the text label
    /// `"24bit-96kHz"` carries the information — color is decorative).
    /// Replaces the 5 inline `Span::styled(label, Style::default().fg(
    /// quality_color(...)))` sites in `player_bar.rs` and
    /// `player_bar_big.rs`.
    pub fn quality_style(&self, bit_depth: u32, sample_rate_hz: u32) -> Style {
        Style::default().fg(quality_color(bit_depth, sample_rate_hz))
    }

    /// Source-badge `Style` for the `[L]`/`[Y]` row badge in Mixed mode.
    /// Replaces the 2 inline constructions in `columns.rs:1519,1521,1679`
    /// which use `theme.source_local` / `theme.source_yt` on `zebra_bg`.
    /// The caller supplies the zebra background via the row's existing
    /// `bg(zebra_bg)` style; this method only returns the fg.
    pub fn source_badge(&self, yt: bool) -> Style {
        Style::default().fg(if yt {
            self.source_yt
        } else {
            self.source_local
        })
    }
}

/// Header styling with an explicit color-mode input, allowing deterministic
/// coverage without mutating process-global environment variables.
pub fn header_style_for(monochrome: bool) -> Style {
    let theme = Theme::default();
    Style::default()
        .fg(if monochrome {
            Color::Reset
        } else {
            theme.accent
        })
        .add_modifier(Modifier::BOLD)
}

// ---------------------------------------------------------------------------
// Accessibility style helpers (free-function form, used by the view layer)
// ---------------------------------------------------------------------------

/// Selection style for list items: accent foreground on hi-fg background,
/// plus [`Modifier::REVERSED`] and [`Modifier::BOLD`]. The `REVERSED`
/// modifier inverts foreground/background so the selected row is visually
/// inverted — this survives `NO_COLOR` (where colors collapse to grayscale
/// but the modifier still creates a clear visual inversion). `BOLD` adds
/// weight so the selected row is heavier than unselected rows even when
/// color is stripped.
///
/// **Accessibility:** color is NOT the only signal. `REVERSED` + `BOLD`
/// provide two non-color cues that work in monochrome terminals. The view
/// layer also adds `►` / `❯` glyph prefixes for a third text-visible cue.
pub fn selection_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.hi_fg)
        .bg(theme.accent)
        .add_modifier(Modifier::REVERSED)
        .add_modifier(Modifier::BOLD)
}

/// Focus border style: accent color plus [`Modifier::BOLD`]. The `BOLD`
/// modifier ensures the focused border is visually heavier — distinguishable
/// under `NO_COLOR` even when the color collapses to grayscale.
///
/// **Accessibility:** the bold weight is a non-color cue for focus.
pub fn focus_border_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD)
}

/// Actionable item style: accent color plus [`Modifier::UNDERLINE`]. The
/// `UNDERLINE` modifier marks interactive elements (key hints, clickable
/// text) so they are identifiable under `NO_COLOR`.
///
/// **Accessibility:** underline is a non-color cue for actionability.
pub fn actionable_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::UNDERLINED)
}

/// Accent color for the quality tag: Magenta for Hi-Res (24-bit or ≥48kHz),
/// Green for CD. Returns [`Color::Reset`] when `NO_COLOR` is set.
pub fn quality_color(bit_depth: u32, sample_rate_hz: u32) -> Color {
    if no_color() {
        return Color::Reset;
    }
    if bit_depth >= 24 || sample_rate_hz >= 48000 {
        Color::Magenta
    } else {
        Color::Green
    }
}

/// High-contrast color for the progress bar playhead and fill. Returns the
/// theme accent (`Cyan`) in color mode for a vivid bar; `Reset` (terminal
/// default foreground) under `NO_COLOR` so the bar stays visible without
/// hue. The progress bar's block characters (`●` / `▰` / `▱`) are
/// shape-distinct from the dashed line, providing a non-color cue in
/// addition to the color.
pub fn progress_color(theme: &Theme) -> Color {
    if no_color() {
        Color::Reset
    } else {
        theme.accent
    }
}

/// Terminal display width of a single character. Uses the same Unicode width
/// tables as Ratatui, including CJK, emoji, combining marks, and zero-width
/// characters.
pub fn char_disp_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Unicode terminal display width used for clipping and alignment.
///
/// Zero-width chars (U+200B–200F, U+FEFF) and combining diacritical marks
/// (U+0300–036F) count as 0 so they don't inflate alignment calculations
/// (AC-M6.4.1). Without this, a title like "A\u{0301}do" (Ádo with combining
/// acute) would measure as width 4 instead of 3, breaking right-alignment.
pub fn disp_width(s: &str) -> usize {
    s.chars().map(char_disp_width).sum()
}

/// Hard-truncate `s` to at most `width` display columns (no ellipsis). CJK /
/// wide characters are counted as 2 columns via [`char_disp_width`] so the
/// cut point respects terminal cell boundaries. Used to clip lyric / text
/// lines to the pane inner width so they never overflow into the right
/// `│` border (Issue 1: long lyric lines bled into the border, producing a
/// garbled `|||||...` artifact after the active line).
pub fn clip_to_width(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_disp_width(c);
        if w + cw > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

/// Right-pad `left` so that `right` sits flush against the pane's right edge.
/// `width` is the inner pane width (borders already subtracted). CJK/wide
/// characters are counted as 2 columns via [`disp_width`] so alignment holds for
/// Japanese titles (Ado / Aimer / etc.), not just ASCII.
pub fn pad_between(left: &str, right: &str, width: usize) -> String {
    let lw = disp_width(left);
    let rw = disp_width(right);
    let pad = width.saturating_sub(lw + rw);
    format!("{}{}{}", left, " ".repeat(pad), right)
}

// ---------------------------------------------------------------------------
// ASCII font mode helpers (DEF-006)
// ---------------------------------------------------------------------------

/// ASCII border set: `+`, `-`, `|` characters instead of Unicode box-drawing.
/// Used when `JUKEBOX_FONT_MODE=ascii` so the TUI is fully ASCII-compatible.
pub const ASCII_BORDER_SET: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// True when the active font mode is ASCII (`JUKEBOX_FONT_MODE=ascii`).
/// D2: `NO_COLOR` no longer triggers ASCII mode — it only disables colors
/// (see `theme::no_color`). Use `JUKEBOX_FONT_MODE=ascii` for ASCII glyphs.
/// RC18-D1: reads the cached startup font mode (see [`FONT_MODE`]) so the
/// glyph vocabulary is stable for the process lifetime — never flaky
/// between calls.
pub fn is_ascii() -> bool {
    cached_font_mode() == FontMode::Ascii
}

/// The horizontal line character for the current font mode: `─` (Unicode) or
/// `-` (ASCII). Used by separator rules and horizontal dividers.
pub fn h_line() -> &'static str {
    if is_ascii() {
        "-"
    } else {
        "─"
    }
}

/// The vertical separator character for the current font mode: `│` (Unicode)
/// or `|` (ASCII). Used by the tab bar between view labels.
pub fn v_sep() -> &'static str {
    if is_ascii() {
        "|"
    } else {
        "│"
    }
}

// --- Player bar glyph helpers (DEF-006 ASCII mode) ---

/// Play glyph: `▶` (Unicode) or `>` (ASCII).
pub fn play_glyph() -> &'static str {
    if is_ascii() {
        ">"
    } else {
        "▶"
    }
}
/// Pause glyph: `⏸` (Unicode) or `||` (ASCII).
pub fn pause_glyph() -> &'static str {
    if is_ascii() {
        "||"
    } else {
        "⏸"
    }
}
/// Stop glyph: `■` (Unicode) or `#` (ASCII).
pub fn stop_glyph() -> &'static str {
    if is_ascii() {
        "#"
    } else {
        "■"
    }
}
/// Filled block for progress/volume bars: `▰` or `#`.
pub fn filled_block() -> char {
    if is_ascii() {
        '#'
    } else {
        '▰'
    }
}
/// Empty block for progress/volume bars: `▱` or `-`.
pub fn empty_block() -> char {
    if is_ascii() {
        '-'
    } else {
        '▱'
    }
}
/// Heavy horizontal line for the progress bar's filled portion: `━` or `=`.
/// Per spec, the deck's progress uses `━` (heavy) for filled + `─` (light)
/// for empty so the playhead stands out against the track, rather than the
/// block characters `▰` / `▱` used by the legacy bars.
pub fn progress_fill() -> char {
    if is_ascii() {
        '='
    } else {
        '━'
    }
}
/// Light horizontal line for the progress bar's empty portion: `─` or `-`.
pub fn progress_track() -> char {
    if is_ascii() {
        '-'
    } else {
        '─'
    }
}
/// Playhead thumb for the progress bar: `●` or `#`.
pub fn progress_thumb() -> char {
    if is_ascii() {
        '#'
    } else {
        '●'
    }
}
/// Previous-track glyph: `◀◀` or `<<`.
pub fn prev_glyph() -> &'static str {
    if is_ascii() {
        "<<"
    } else {
        "◀◀"
    }
}
/// Next-track glyph: `▶▶` or `>>`.
pub fn next_glyph() -> &'static str {
    if is_ascii() {
        ">>"
    } else {
        "▶▶"
    }
}
/// Up-next marker glyph: `▸` or `>`.
pub fn marker_glyph() -> &'static str {
    if is_ascii() {
        ">"
    } else {
        "▸"
    }
}
/// Selection marker glyph for the currently-selected row: `▸` (Unicode) or
/// `>` (ASCII). Exposed as a dedicated helper (RB-6) so the view layer can
/// apply a visible non-color cue to the selected row without depending on
/// `REVERSED` or color. This is the same as [`marker_glyph`] but named to
/// convey its role in accessibility (selection visibility under `NO_COLOR`).
pub fn selection_marker() -> &'static str {
    marker_glyph()
}
/// Separator dot: `·` (Unicode) or `*` (ASCII). Used between status fields.
pub fn sep_dot() -> &'static str {
    if is_ascii() {
        "*"
    } else {
        "·"
    }
}
/// Em-dash: `—` (Unicode) or `--` (ASCII). Used in "title — artist" etc.
pub fn em_dash() -> &'static str {
    if is_ascii() {
        "--"
    } else {
        "—"
    }
}
/// Ellipsis: `…` (Unicode) or `...` (ASCII). Used in truncation/loading.
pub fn ellipsis() -> &'static str {
    if is_ascii() {
        "..."
    } else {
        "…"
    }
}
/// Right arrow: `→` (Unicode) or `->` (ASCII). Used in breadcrumbs and hints.
pub fn right_arrow() -> &'static str {
    if is_ascii() {
        "->"
    } else {
        "→"
    }
}
/// Left arrow: `←` (Unicode) or `<-` (ASCII). Used in breadcrumbs.
pub fn left_arrow() -> &'static str {
    if is_ascii() {
        "<-"
    } else {
        "←"
    }
}
/// Up arrow: `↑` (Unicode) or `^` (ASCII). Used in navigation hints.
pub fn up_arrow() -> &'static str {
    if is_ascii() {
        "^"
    } else {
        "↑"
    }
}
/// Down arrow: `↓` (Unicode) or `v` (ASCII). Used in navigation hints.
pub fn down_arrow() -> &'static str {
    if is_ascii() {
        "v"
    } else {
        "↓"
    }
}
/// Bullet: `•` (Unicode) or `*` (ASCII). Used for list items in overlays.
pub fn bullet() -> &'static str {
    if is_ascii() {
        "*"
    } else {
        "•"
    }
}
/// Replace all known Unicode decorative characters in `s` with their ASCII
/// equivalents when ASCII font mode is active. Used for strings that originate
/// outside the view layer (e.g. `YtState::human_label()`) where per-call
/// helpers like `em_dash()` can't be inserted at the source. When not in ASCII
/// mode, returns `s` unchanged.
pub fn ascii_sanitize(s: &str) -> String {
    if !is_ascii() {
        return s.to_string();
    }
    s.replace('—', "--")
        .replace('·', "*")
        .replace('…', "...")
        .replace('→', "->")
        .replace('←', "<-")
        .replace('↑', "^")
        .replace('↓', "v")
        .replace(['▸', '▶'], ">")
        .replace('♫', "#")
        .replace('✦', "*")
        .replace('≡', "#")
        .replace('⏸', "||")
        .replace('■', "#")
        .replace('•', "*")
}
