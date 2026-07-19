//! Medium layout for the Now Playing Deck (80–119 cols, ≥ 24 rows).
//!
//! Single column. All groups stacked vertically. The title sits in a
//! notched border. The source status is right-aligned on the artist
//! row so it doesn't compete with the title (spec problem #11).
//!
//! ## Layout
//!
//! ```text
//! ╭─ ▶ NOW PLAYING ─────────────────────────────────────────────╮
//! │  Artist                                        ● YT Connected │
//! │  Track title                                                  │
//! │  Album                                                        │
//! │  ▶ PLAYING  [Space] Pause                                     │
//! │  0:00  ━━━━━━━━━━━━●──────────  --:--                          │
//! │  [←] Previous  [Space] Pause  [→] Next  Vol 70% ███████░       │
//! │  Shuffle: Random  Repeat: One  Continue: Off                  │
//! │  Up next: Nothing queued                                     │
//! ╰──────────────────────────────────────────────────────────────╯
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::{
    controls, metadata, modes, progress, source_status, state, up_next,
};
use crate::tui::view::theme::{is_ascii, play_glyph, Theme};

/// Render the medium layout into `area`. The caller must ensure
/// `area.width >= 80 && area.height >= 24`.
pub fn render_medium(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let theme = Theme::default();
    let marker = if focused { play_glyph() } else { "" };
    let title = if focused {
        if is_ascii() {
            "NOW PLAYING - FOCUSED"
        } else {
            "NOW PLAYING · FOCUSED"
        }
    } else {
        "NOW PLAYING"
    };
    let outer = theme.pane_block_notched(title, marker, focused, false);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Rows: metadata(4) + state(1) + progress(1) + controls(1) + source(1)
    // + modes(1) + up_next(1) = 10 rows inside the 12-row bordered deck.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // metadata
            Constraint::Length(1), // state
            Constraint::Length(1), // progress
            Constraint::Length(1), // controls + volume
            Constraint::Length(1), // source + connection
            Constraint::Length(1), // modes
            Constraint::Length(1), // up next
            Constraint::Min(0),    // trailing
        ])
        .split(inner);

    metadata::render_metadata(f, rows[0], app);
    state::render_state(f, rows[1], app);
    progress::render_progress_bar(f, rows[2], app);
    controls::render_controls(f, rows[3], app);
    source_status::render_source_status(f, rows[4], app);
    modes::render_modes(f, rows[5], app);
    up_next::render_up_next(f, rows[6], app);
}
