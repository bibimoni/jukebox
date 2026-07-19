//! Now Playing Deck — a polished, responsive Now Playing surface.
//!
//! The deck replaces the visually dense three-line footer player with a
//! four-breakpoint layout that scales from a wide "Now Playing +
//! Up Next" split down to a single-line minimal bar. The deck owns the
//! breakpoint dispatch, semantic theme, and four dedicated renderers.
//!
//! ## Breakpoints
//!
//! | Name | Width | Height | Renderer |
//! |------|-------|--------|----------|
//! | Wide | >= 120 | >= 30 | two-column metadata + Up Next (11 rows) |
//! | Medium | >= 80 | >= 24 | stacked single column (12 rows) |
//! | Compact | >= 60 | >= 20 | condensed single column (7 rows) |
//! | Minimal | < 60 OR < 20 | borderless playback shell (3 rows) |
//!
//! `layout::draw` uses the same breakpoint and height helpers, keeping
//! allocation, rendering, mouse bounds, and overlays on one contract.
//!
//! ## Module layout
//!
//! - [`mod`] (this file) — `Breakpoint`, `pick_breakpoint`, `render`,
//!   `geometry`, `DeckGeometry`, `DeckState`.
//! - [`progress`] — shared progress component with clamping + ASCII
//!   fallback + hi-res wall-clock fix.
//! - [`minimal`] — 1-2 line minimal layout for tiny terminals.
//! - [`spinner`] — shared spinner frames + `is_buffering`.
//! - [`theme`] — `PlayerTheme` struct of pre-computed semantic `Style`s.

pub mod compact;
pub mod controls;
pub mod medium;
pub mod metadata;
pub mod minimal;
pub mod modes;
pub mod progress;
pub mod source_status;
pub mod spinner;
pub mod state;
pub mod theme;
pub mod up_next;
pub mod wide;

use ratatui::{layout::Rect, Frame};

use crate::tui::app::App;

/// Rows reserved by the application layout for each deck class. These
/// include the outer border for bordered layouts.
pub const WIDE_HEIGHT: u16 = 11;
pub const MEDIUM_HEIGHT: u16 = 12;
pub const COMPACT_HEIGHT: u16 = 7;
pub const MINIMAL_HEIGHT: u16 = 3;

/// Choose the deck class from the full terminal size, then return the rows
/// the application layout must reserve for it. Keeping this next to
/// [`pick_breakpoint`] prevents the renderer and `layout.rs` from drifting.
pub fn height_for_terminal(area: Rect) -> u16 {
    match pick_breakpoint(area) {
        Breakpoint::Wide => WIDE_HEIGHT,
        Breakpoint::Medium => MEDIUM_HEIGHT,
        Breakpoint::Compact => COMPACT_HEIGHT,
        Breakpoint::Minimal => MINIMAL_HEIGHT.min(area.height),
    }
}

/// The four responsive breakpoints for the Now Playing Deck. Width is
/// the primary input; height is the secondary input. Thresholds follow
/// the spec's recommendations:
///
/// - **Wide**:    `>= 120` cols, `>= 30` rows
/// - **Medium**:  `>= 80` cols,  `>= 24` rows
/// - **Compact**: `>= 60` cols,  `>= 20` rows
/// - **Minimal**: `< 60` cols OR `< 20` rows (last-resort fallback)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Breakpoint {
    /// Wide layout: `>= 120` cols, `>= 30` rows. Two-column split with
    /// the Up Next column on the right.
    Wide,
    /// Medium layout: `80–119` cols, `>= 24` rows. Single column, all
    /// groups stacked.
    Medium,
    /// Compact layout: `60–79` cols, `>= 20` rows. Condensed single
    /// column.
    Compact,
    /// Minimal layout: `< 60` cols OR `< 20` rows. 1-3 lines, no border.
    #[default]
    Minimal,
}

impl Breakpoint {
    /// The minimum width for this breakpoint.
    pub fn min_width(self) -> u16 {
        match self {
            Breakpoint::Wide => 120,
            Breakpoint::Medium => 80,
            Breakpoint::Compact => 60,
            Breakpoint::Minimal => 0,
        }
    }

    /// The minimum height for this breakpoint (content rows, not
    /// including the border).
    pub fn min_height(self) -> u16 {
        match self {
            Breakpoint::Wide => 30,
            Breakpoint::Medium => 24,
            Breakpoint::Compact => 20,
            Breakpoint::Minimal => 1,
        }
    }

    /// Stable string for `LayoutState` persistence
    /// (`"wide"` / `"medium"` / `"compact"` / `"minimal"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Breakpoint::Wide => "wide",
            Breakpoint::Medium => "medium",
            Breakpoint::Compact => "compact",
            Breakpoint::Minimal => "minimal",
        }
    }

    /// Parse a persisted breakpoint string back into the enum. Falls
    /// back to `Minimal` for unknown / empty values so a corrupt DB
    /// never breaks the deck.
    pub fn parse(s: &str) -> Self {
        match s {
            "wide" => Breakpoint::Wide,
            "medium" => Breakpoint::Medium,
            "compact" => Breakpoint::Compact,
            _ => Breakpoint::Minimal,
        }
    }
}

/// Pick the breakpoint for a deck rect. Width is primary; height is the
/// secondary input. The deck always returns the largest breakpoint that
/// fits — `Minimal` is the floor.
///
/// Per the spec: Wide `>= 120`, Medium `>= 80`, Compact `>= 60`,
/// Minimal `< 60` or `< 20`.
pub fn pick_breakpoint(rect: Rect) -> Breakpoint {
    if rect.width >= 120 && rect.height >= 30 {
        Breakpoint::Wide
    } else if rect.width >= 80 && rect.height >= 24 {
        Breakpoint::Medium
    } else if rect.width >= 60 && rect.height >= 20 {
        Breakpoint::Compact
    } else {
        Breakpoint::Minimal
    }
}

/// Cell rectangles for every clickable control rendered by the deck.
/// Rendering and input both consume this value, so hit-testing cannot
/// drift from the visible controls as terminal dimensions change.
///
/// Geometry follows the same row and text-width calculations as the dedicated
/// layouts. Minimal exposes progress and Resume when those rows are visible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckGeometry {
    pub previous: Rect,
    pub play_pause: Rect,
    pub next: Rect,
    pub progress: Rect,
    pub volume: Rect,
    pub shuffle: Rect,
    pub repeat: Rect,
    pub continue_mode: Rect,
    pub mode: Rect,
}

/// Compute the exact clickable rectangles used by the rendered deck.
pub fn geometry(area: Rect, app: &App) -> DeckGeometry {
    if area.width == 0 || area.height == 0 {
        return DeckGeometry::default();
    }
    let breakpoint = if area.width >= Breakpoint::Wide.min_width() && area.height == WIDE_HEIGHT {
        Breakpoint::Wide
    } else if area.width >= Breakpoint::Medium.min_width() && area.height >= MEDIUM_HEIGHT {
        Breakpoint::Medium
    } else if area.width >= Breakpoint::Compact.min_width() && area.height >= COMPACT_HEIGHT {
        Breakpoint::Compact
    } else {
        Breakpoint::Minimal
    };
    let inner = match breakpoint {
        Breakpoint::Minimal => area,
        _ if area.width >= 2 && area.height >= 2 => {
            Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
        }
        _ => return DeckGeometry::default(),
    };
    let mut geometry = DeckGeometry::default();
    match breakpoint {
        Breakpoint::Wide | Breakpoint::Medium => {
            let content_width = if breakpoint == Breakpoint::Wide {
                inner.width.saturating_sub(30)
            } else {
                inner.width
            };
            let progress_area = Rect::new(inner.x, inner.y + 5, content_width, 1);
            geometry.progress = progress::progress_bar_rect(progress_area, app);
            let y = inner.y + 6;
            let previous = if crate::tui::view::theme::is_ascii() {
                "[<] Previous"
            } else {
                "[←] Previous"
            };
            geometry.previous = text_rect(inner.x, y, previous);
            let mut x = geometry.previous.right() + 3;
            if let Some(action) = controls::action_label(app) {
                let play = format!("[Space] {action}");
                geometry.play_pause = text_rect(x, y, &play);
                x = geometry.play_pause.right() + 3;
            }
            let next = if crate::tui::view::theme::is_ascii() {
                "[>] Next"
            } else {
                "[→] Next"
            };
            geometry.next = text_rect(x, y, next);
        }
        Breakpoint::Compact => {
            geometry.progress =
                progress::progress_bar_rect(Rect::new(inner.x, inner.y + 1, inner.width, 1), app);
            let y = inner.y + 2;
            let mut x = inner.x;
            if let Some(action) = controls::action_label(app) {
                let play = format!("[Space] {action}");
                geometry.play_pause = text_rect(x, y, &play);
                x = geometry.play_pause.right() + 3;
            }
            geometry.previous = Rect::new(x, y, 3, 1);
            geometry.next = Rect::new(x + 4, y, 3, 1);
        }
        Breakpoint::Minimal => {
            geometry.progress =
                progress::progress_bar_rect(Rect::new(inner.x, inner.y + 1, inner.width, 1), app);
            if app.resume_hint.is_some() && app.now_playing.is_none() && inner.height >= 3 {
                geometry.play_pause = text_rect(inner.x, inner.y + 2, "[Space] Resume");
            }
        }
    }
    geometry
}

fn text_rect(x: u16, y: u16, text: &str) -> Rect {
    Rect::new(x, y, crate::tui::view::theme::disp_width(text) as u16, 1)
}

/// Persistent state for the Now Playing Deck. Owned by `App`; persisted
/// via `LayoutState`. Additive — does not replace the existing
/// `PlayerBarState` (which continues to drive the bottom-bar dispatch).
/// `DeckState` is reserved for future pane-embedded Now Playing modules
/// and for the planned `Shift+P` breakpoint-cycle key.
#[derive(Clone, Debug, Default)]
pub struct DeckState {
    /// The user's breakpoint preference. `None` = auto (use
    /// `pick_breakpoint`). `Some(bp)` pins the deck to `bp` when it fits;
    /// otherwise falls back to `pick_breakpoint`.
    pub breakpoint_pref: Option<Breakpoint>,
    /// True when the user has hidden the deck via `S` (in pane edit mode
    /// or after `Ctrl+w`). Mirrors `PlayerBarState.hidden`.
    pub hidden: bool,
    /// Reserved for optional album artwork (off by default per
    /// RC19-D15; the spec says "do not make graphics protocol support
    /// mandatory"). The field is here so a future `A` toggle can flip it
    /// without a state-shape change.
    pub artwork: bool,
}

/// The single entry point for the Now Playing Deck. Dispatches to the
/// breakpoint's renderer. The deck owns its own wide/medium/compact/
/// minimal layouts — no delegation to the legacy `player_bar_big` /
/// `player_bar` renderers. The legacy files stay byte-identical (so
/// the ~40 existing player-bar tests continue to pass when called
/// directly), but `layout::draw` calls the deck exclusively.
///
/// Per spec: terminal transparency is preserved — no `bg` is set on
/// any cell. Hierarchy comes from foreground styling + borders +
/// spacing.
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    render_for_breakpoint_with_focus(f, area, app, pick_breakpoint(area), false);
}

/// Render the deck as a pane module. Pane allocation is already complete, so
/// breakpoints use each renderer's actual required height rather than the full
/// terminal thresholds. Chrome is capped to the content height so a tall pane
/// does not become a mostly empty bordered panel.
pub fn render_pane(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let breakpoint = if area.width >= Breakpoint::Wide.min_width() && area.height >= WIDE_HEIGHT {
        Breakpoint::Wide
    } else if area.width >= Breakpoint::Medium.min_width() && area.height >= MEDIUM_HEIGHT {
        Breakpoint::Medium
    } else if area.width >= Breakpoint::Compact.min_width() && area.height >= COMPACT_HEIGHT {
        Breakpoint::Compact
    } else {
        Breakpoint::Minimal
    };
    let height = match breakpoint {
        Breakpoint::Wide => WIDE_HEIGHT,
        Breakpoint::Medium => MEDIUM_HEIGHT,
        Breakpoint::Compact => COMPACT_HEIGHT,
        Breakpoint::Minimal => MINIMAL_HEIGHT.min(area.height),
    };
    let deck_area = Rect::new(area.x, area.y, area.width, height);
    render_for_breakpoint_with_focus(f, deck_area, app, breakpoint, focused);
}

/// Render the deck at an explicit breakpoint. Used by `layout::draw`
/// so the layout's existing `big_mode` / `compact` decision can be
/// preserved (callers may pass a specific breakpoint rather than
/// relying on `pick_breakpoint`).
pub fn render_for_breakpoint(f: &mut Frame, area: Rect, app: &App, bp: Breakpoint) {
    render_for_breakpoint_with_focus(f, area, app, bp, false);
}

/// Render a specific breakpoint with an explicit focus state. Focus changes
/// the title marker and border foreground only; it never fills the panel.
pub fn render_for_breakpoint_with_focus(
    f: &mut Frame,
    area: Rect,
    app: &App,
    bp: Breakpoint,
    focused: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match bp {
        Breakpoint::Wide => wide::render_wide(f, area, app, focused),
        Breakpoint::Medium => medium::render_medium(f, area, app, focused),
        Breakpoint::Compact => compact::render_compact(f, area, app, focused),
        Breakpoint::Minimal => minimal::render(f, area, app, focused),
    }
}
