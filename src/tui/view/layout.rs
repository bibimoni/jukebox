//! Top-level TUI layout + responsive breakpoints.
//!
//! [`draw`] is the single entry the event loop calls. It handles the
//! "terminal too small" guard, then splits the screen into a main browse area
//! (Miller columns rendered by [`columns::render`]) and a 2-line persistent
//! player bar ([`player_bar::render`]); if an [`Overlay`] is active it is
//! painted on top via [`overlay::render`] (Task 11 fills in the real overlay
//! surface — for now it is a no-op stub so layout compiles standalone).
//!
//! Responsive breakpoints (T3):
//! - At `width < 60` or `height < 20` we refuse to render and instead show a
//!   centered "terminal too small" message. Below 60×20 the columns compress
//!   past readability.
//! - At `60 ≤ width < 70` the narrow path compresses to a single readable
//!   column (rail + focused pane) via `render_narrow`.
//! - At `70 ≤ width ≤ 100` (or `height < 24`) the narrow path shows 2 columns
//!   (rail + focused pane) with a breadcrumb cue for off-screen panes.
//! - At `width > 100` and `height ≥ 24` the full 3-column Miller layout is
//!   rendered. At `height ≤ 24` the full layout collapses chrome: no tab bar
//!   line (breadcrumb lives in column titles) + 1-row compact player bar,
//!   saving 2 rows for content (T3).

use ratatui::{
    buffer::CellDiffOption,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{columns, footer, overlay, player_bar, player_bar_big};
use crate::tui::app::{App, View};
use crate::tui::view::theme::{self, h_line, v_sep, Theme};

/// Minimum terminal width for the full 3-column Miller layout. At
/// `width < MIN_WIDTH` we collapse to the 2-column narrow path (rail +
/// focused pane) so the three columns don't compress past readability.
///
/// **T3 breakpoints:** `< 70` = single column, `70–99` = 2 columns (rail +
/// focused), `>= 100` = 3 columns. The old threshold was 120 (Issue 2 fix),
/// then 101; lowered to 100 so the 3-column layout kicks in at 100 cols
/// (the standard test terminal size), making SHUF/RPT/volume indicators
/// visible at 100×30 (DEF-004). rail(4)+col1(24)+col2(28)+col3(44)=100
/// fits exactly at 100 cols.
pub const MIN_WIDTH: u16 = 100;
pub const MIN_HEIGHT: u16 = 24;
/// Below this the app refuses to render and shows a "terminal too small" message.
pub const NARROW_MIN_WIDTH: u16 = 60;
pub const NARROW_MIN_HEIGHT: u16 = 20;
/// At or below this width the narrow path compresses to a single readable
/// column (still via `render_narrow` — the rail + focused pane). This is the
/// tmux-split floor; below `NARROW_MIN_WIDTH` we refuse to render entirely.
pub const NARROW_SINGLE_COL_WIDTH: u16 = 70;

/// Height of the persistent player bar at the bottom of the screen.
const PLAYER_BAR_HEIGHT: u16 = 2;
/// Height of the thin dim separator rule drawn above the player bar so it's
/// visually distinct from the browse content. One line of chrome — the bar
/// itself still gets exactly `PLAYER_BAR_HEIGHT` content rows. Suppressed at
/// `width < 90` to reclaim 1 row for the browse area on narrow terminals.
const BAR_SEPARATOR_HEIGHT: u16 = 1;
/// Height of the always-visible footer. 2 lines (status + hints) when there's
/// vertical room; 1 line (status + compact hints) at ≤24 rows so the browse
/// area isn't squeezed on minimum-size terminals.
const FOOTER_HEIGHT_WIDE: u16 = 2;
const FOOTER_HEIGHT_NARROW: u16 = 1;

/// Return the exact player-bar rectangle used by [`draw`] for `area`.
/// Mouse input and overlays use this layout-owned contract instead of
/// reconstructing bottom chrome with fixed row guesses.
pub fn player_bar_area(area: Rect) -> Option<Rect> {
    if area.width < NARROW_MIN_WIDTH || area.height < NARROW_MIN_HEIGHT {
        return None;
    }
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return Some(Rect::new(
            area.x,
            area.bottom().saturating_sub(FOOTER_HEIGHT_NARROW + 1),
            area.width,
            1,
        ));
    }
    let compact = area.height <= MIN_HEIGHT;
    let footer_h = if compact {
        FOOTER_HEIGHT_NARROW
    } else {
        FOOTER_HEIGHT_WIDE
    };
    let bar_h = if compact { 1 } else { PLAYER_BAR_HEIGHT };
    Some(Rect::new(
        area.x,
        area.bottom().saturating_sub(footer_h + bar_h),
        area.width,
        bar_h,
    ))
}

/// Main content bounds above the responsive player/footer chrome.
pub fn overlay_content_area(area: Rect) -> Rect {
    let separator_h = if area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT && area.width >= 90 {
        BAR_SEPARATOR_HEIGHT
    } else {
        0
    };
    let height = player_bar_area(area)
        .map(|bar| bar.y.saturating_sub(area.y).saturating_sub(separator_h))
        .unwrap_or(area.height);
    Rect::new(area.x, area.y, area.width, height)
}

/// The single entry point the event loop calls. Renders the full TUI frame:
/// too-small guard, columns + player bar + footer, and any active overlay on top.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.width < NARROW_MIN_WIDTH || area.height < NARROW_MIN_HEIGHT {
        render_too_small(f, area);
        return;
    }

    // Clamp browse cursors to valid ranges before rendering, so a stale
    // album/track cursor (after an artist switch or view change) doesn't
    // leave the Tracks column empty.
    app.clamp_cursors();

    // Narrow terminals (width ≤ 100, or height < 24) collapse the Miller
    // columns to a 2-column "rail + focused pane" via `render_narrow` with a
    // tab bar, compressed 1-row player bar + short footer — usable in a tmux
    // split. At `width < 70` the collapse deepens to a single readable column.
    //
    // T3 breakpoints: <70 single col, 70–100 two cols, >100 three cols.
    // The threshold is `< MIN_WIDTH` (101): width ≤ 100 stays narrow with the
    // breadcrumb cue for off-screen panes; width > 100 restores the full
    // 3-column Miller layout.
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_narrow(f, area, app);
        if app.overlay.is_some() {
            overlay::render(f, area, app);
        }
        post_render_force(f, app);
        return;
    }

    // T3: At height ≤ 24 (MIN_HEIGHT) the full layout collapses chrome to
    // reclaim vertical space — no separate tab bar line (the breadcrumb lives
    // in the column titles) + 1-row compact player bar instead of 2. This
    // saves 2 rows for content: 5 chrome rows → 3. At height > 24 the full
    // chrome is restored (tab bar + 2-row player bar + 2-row footer).
    //
    // At width < 90 the separator is suppressed (0 height) to reclaim 1
    // row for the browse area on narrow terminals — the player bar's
    // distinct styling provides enough visual separation.
    let compact = area.height <= MIN_HEIGHT;
    let big_mode = app.player_bar_state.effective_mode(area.width, area.height)
        == player_bar_big::PlayerBarMode::Big;
    app.player_bar_state.mode = if big_mode {
        player_bar_big::PlayerBarMode::Big
    } else {
        player_bar_big::PlayerBarMode::Mini
    };
    let footer_h = if compact {
        FOOTER_HEIGHT_NARROW
    } else {
        FOOTER_HEIGHT_WIDE
    };
    let sep_h = if area.width >= 90 {
        BAR_SEPARATOR_HEIGHT
    } else {
        0
    };
    let bar_h = if compact {
        1u16
    } else if big_mode {
        player_bar_big::BIG_BAR_HEIGHT
    } else {
        PLAYER_BAR_HEIGHT
    };

    if compact {
        // ≤24 rows: no tab bar, 1-row compact player bar. Chrome = sep(1) +
        // bar(1) + footer(1) = 3 rows. Content gets height-3 rows.
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(sep_h),
                Constraint::Length(1),
                Constraint::Length(footer_h),
            ])
            .split(area);
        columns::render(f, outer[0], app);
        if sep_h > 0 {
            render_separator_rule(f, outer[1], app);
        }
        player_bar::render_compact(f, outer[2], app);
        footer::render(f, &outer[3], app);
    } else {
        // >24 rows: full chrome — tab bar(1) + sep(1) + bar(2) + footer(2).
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(sep_h),
                Constraint::Length(bar_h),
                Constraint::Length(footer_h),
            ])
            .split(area);
        render_tab_bar(f, outer[0], app);
        columns::render(f, outer[1], app);
        if sep_h > 0 {
            render_separator_rule(f, outer[2], app);
        }
        if big_mode {
            player_bar_big::render_big(f, outer[3], app);
        } else {
            player_bar::render(f, outer[3], app);
        }
        footer::render(f, &outer[4], app);
    }

    if app.overlay.is_some() {
        overlay::render(f, area, app);
    }
    post_render_force(f, app);
}

/// Post-render diff forcing. Called at the end of every `draw` after all
/// widgets (and any overlay) have rendered. Combines two diff-forcing passes:
///
/// 1. **Every-frame full redraw** (MOD-7 / DEF-002 persistent fix): marks
///    every non-empty cell as `AlwaysUpdate` so the diff emits it
///    unconditionally on EVERY frame. The persistent character-dropping bug
///    ("delete" → "del te", "Test Artist" → "Test Art st", "YouTube" →
///    "utube", "lossless" → "lossle s") happened on initial render, same-view
///    updates (j/k navigation, scrolling), and view switches — anywhere
///    ratatui's `Cell::eq` found a coincidental content+style match at the
///    same position and skipped the cell, leaving stale terminal content.
///    The previous view-change-only fix was insufficient because the same
///    diff-skip happens whenever content shifts within a view (scroll,
///    cursor move) or on the very first frame (where the cleared buffer can
///    coincidentally match a styled cell). Forcing every non-empty cell on
///    every frame guarantees the terminal always reflects the buffer.
///
/// 2. **`force_space_redraw`** (DEF-002 / MAJ-2): marks default-styled
///    inter-word spaces so the diff writes them, clearing stale content
///    at space positions.
fn post_render_force(f: &mut Frame, app: &mut App) {
    force_full_redraw(f);
    force_space_redraw(f);
    app.last_rendered_view = app.view;
}

/// Mark every non-empty cell in the buffer as `AlwaysUpdate` so the diff
/// emits it regardless of whether it matches the previous frame. Called on
/// EVERY frame by [`post_render_force`] to prevent the MOD-7
/// character-dropping bug where a cell whose content+style coincidentally
/// matches the previous frame at the same position is skipped by ratatui's
/// `Cell::eq`, leaving stale terminal content. This happens on initial
/// render, same-view updates (scroll/cursor move), and view switches.
///
/// "Non-empty" means the cell has a non-space symbol OR a non-default style
/// (fg/bg/modifier). Default-styled spaces are left to `force_space_redraw`,
/// which marks the inter-word ones.
fn force_full_redraw(f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if cell.diff_option != CellDiffOption::None {
                    continue;
                }
                let is_default_space = cell.symbol() == " "
                    && cell.fg == Color::Reset
                    && cell.bg == Color::Reset
                    && cell.modifier == Modifier::empty();
                if !is_default_space {
                    cell.set_diff_option(CellDiffOption::AlwaysUpdate);
                }
            }
        }
    }
}

/// Maximum number of cells `force_space_redraw` will mark as `AlwaysUpdate`
/// per frame. At small terminal sizes (80×24, 100×30) the cap is never
/// reached, so all eligible spaces are marked (DEF-002 stays fixed). At
/// large sizes (120×40, 160×50) the cap prevents the ~2100-cell flood of
/// erase/update commands that corrupts the display (MAJ-2).
const MAX_MARKED_CELLS: usize = 500;

/// Work around ratatui's `Cell::eq` treating `symbol: None` (empty) as equal to
/// `symbol: Some(" ")` (space). When text uses `Color::Reset` (the default
/// `theme.text`), styled spaces are indistinguishable from empty buffer cells.
/// The diff skips them, so old terminal content at space positions is never
/// refreshed — the pervasive character-dropping bug (DEF-002). On a fresh
/// terminal this is invisible (blank ≈ space), but after a view switch the
/// previous frame's content shows through at every space position, making
/// words run together ("WorkoutEnergy" instead of "Workout Energy").
///
/// Fix: after all rendering, mark default-styled space cells that are
/// **adjacent to a non-space cell** as `CellDiffOption::AlwaysUpdate` so the
/// diff always emits them. This limits the extra writes to spaces that are
/// actually between words (or between a word and the border), not the vast
/// empty background. A space in the middle of nowhere doesn't need a forced
/// write — nothing visually abuts it, so stale content there is harmless.
///
/// MAJ-2: At large terminal sizes (120×40, 160×50) the old code marked every
/// space adjacent to text (~2100 cells), flooding the terminal with erase
/// commands that corrupt the status bar and hint line. Now we prioritize
/// inter-word spaces (both neighbours non-space — the DEF-002 critical case
/// where words run together) and cap the total at [`MAX_MARKED_CELLS`].
/// Trailing/leading spaces (one neighbour) are filled in only if the cap
/// hasn't been reached. At small sizes (80×24, 100×30) the cap is never hit,
/// so DEF-002 stays fully fixed; at large sizes the flood is bounded.
fn force_space_redraw(f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    // Collect candidates in two priority tiers:
    // 1. Inter-word spaces: both left AND right neighbours are non-space.
    //    These are the DEF-002 critical case — without them, words run
    //    together ("WorkoutEnergy" instead of "Workout Energy").
    // 2. One-neighbour spaces: only one side has a non-space neighbour
    //    (trailing/leading spaces at text boundaries). Less visually
    //    critical but still needed to clear stale content at text edges.
    let mut inter_word: Vec<(u16, u16)> = Vec::new();
    let mut one_neighbour: Vec<(u16, u16)> = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let is_default_space = match buf.cell((x, y)) {
                Some(cell) => {
                    cell.diff_option == CellDiffOption::None
                        && cell.symbol() == " "
                        && cell.fg == Color::Reset
                        && cell.bg == Color::Reset
                        && cell.modifier == Modifier::empty()
                }
                None => false,
            };
            if !is_default_space {
                continue;
            }
            // Check horizontal neighbours — `symbol()` returns " " for both
            // empty and space cells, so this only fires next to actual
            // text/border content.
            let left = buf
                .cell((x.wrapping_sub(1), y))
                .map(|c| c.symbol() != " " && !c.symbol().is_empty())
                .unwrap_or(false);
            let right = buf
                .cell((x.wrapping_add(1), y))
                .map(|c| c.symbol() != " " && !c.symbol().is_empty())
                .unwrap_or(false);
            if left && right {
                inter_word.push((x, y));
            } else if left || right {
                one_neighbour.push((x, y));
            }
        }
    }
    // Mark inter-word spaces first (highest priority), then one-neighbour
    // spaces, up to the cap. This ensures the most visible DEF-002 cases
    // are always covered while bounding the total terminal output per
    // frame (MAJ-2).
    let mut marked = 0;
    for &(x, y) in &inter_word {
        if marked >= MAX_MARKED_CELLS {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
        marked += 1;
    }
    for &(x, y) in &one_neighbour {
        if marked >= MAX_MARKED_CELLS {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
        marked += 1;
    }
}

/// Narrow fallback: a tab bar showing the available columns, a column
/// breadcrumb + drill hint (so the user can see what other panes exist and
/// how to reach them — Issue 3: at 80×24 the narrow path showed only one pane
/// with no indication that Albums/Tracks columns existed), a single focused
/// pane (Miller collapse — `h`/`l` drills in/out), a 1-row compressed player
/// bar, and a short 1-row footer.
///
/// **Chrome budget (5 rows):** tab bar (1) · HR (1) · breadcrumb (1) · hint (1)
/// · player bar (1) · footer (1). The column breadcrumb shows the Miller
/// hierarchy with brackets around columns that aren't currently visible
/// (only the focused column is shown in narrow mode), with the focused
/// column highlighted. The drill hint tells the user which keys move between
/// columns. Single-column views (Queue) skip the breadcrumb and hint rows.
fn render_narrow(f: &mut Frame, area: Rect, app: &mut App) {
    let bc = narrow_column_breadcrumb(app);

    let outer = if bc.is_some() {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                    // tab bar (1 row)
                Constraint::Length(1),                    // horizontal rule below tabs
                Constraint::Length(1),                    // column breadcrumb
                Constraint::Length(1),                    // drill-in/out hint
                Constraint::Min(3),                       // browse area
                Constraint::Length(1),                    // compressed player bar (1 row)
                Constraint::Length(FOOTER_HEIGHT_NARROW), // footer (exactly 1 row)
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                    // tab bar (1 row)
                Constraint::Length(1),                    // horizontal rule below tabs
                Constraint::Min(3),                       // browse area
                Constraint::Length(1),                    // compressed player bar (1 row)
                Constraint::Length(FOOTER_HEIGHT_NARROW), // footer (exactly 1 row)
            ])
            .split(area)
    };

    // Tab bar: shows the available columns for the current view with the
    // focused one highlighted — gives the user context about what panes exist
    // and which is active, even when only one pane is visible.
    render_tab_bar(f, outer[0], app);

    // Horizontal rule below the tab bar — visually separates the tab bar
    // from the browse content so the tabs read as a distinct navigation bar,
    // not just another row of text (Issue 3: at 80×24 the tabs blended into
    // the content row below).
    {
        let theme = Theme::default();
        let dim = Style::default().fg(if theme::no_color() {
            Color::Reset
        } else {
            theme.dim
        });
        let rule = h_line().repeat(outer[1].width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(rule, dim)))
                .block(Block::default().borders(Borders::NONE)),
            outer[1],
        );
    }

    // Column breadcrumb + drill hint (multi-column views only). The
    // breadcrumb shows the Miller hierarchy with brackets around panes that
    // aren't visible in narrow mode; the hint tells the user how to drill
    // in/out. Skipped for single-column views (Queue) to reclaim the 2 rows.
    let mut idx = 2;
    if let Some((cols, hint)) = bc {
        render_narrow_breadcrumb(f, outer[idx], &cols);
        idx += 1;
        render_narrow_hint(f, outer[idx], hint);
        idx += 1;
    }

    // Single pane: render the focused column only via the columns module's
    // narrow path. The columns renderer already adapts to area.width; in a
    // narrow area it shows the focused column with a breadcrumb title.
    columns::render_narrow(f, outer[idx], app);
    idx += 1;

    // Compressed 1-row player bar: now-playing + quality + flags on one line.
    player_bar::render_compact(f, outer[idx], app);
    idx += 1;
    footer::render(f, &outer[idx], app);
}

/// Build the column-structure breadcrumb + drill hint for the narrow path
/// (Issue 3). Returns `Some((cols, hint))` for multi-column views, where
/// `cols` is a vec of `(label, is_active)` pairs representing the Miller
/// column hierarchy — `is_active` marks the currently-focused (visible)
/// column; the others are bracketed in the render to indicate they're not
/// visible in narrow mode. `hint` is a one-line key hint for drilling in/out.
/// Returns `None` for single-column views (Queue) where no breadcrumb is
/// meaningful.
fn narrow_column_breadcrumb(app: &App) -> Option<(Vec<(&'static str, bool)>, &'static str)> {
    match app.view {
        View::Artists => match app.focus_col {
            0 => Some((
                vec![("Artists", true), ("Albums", false), ("Tracks", false)],
                "l → Albums · Enter → Tracks",
            )),
            1 => Some((
                vec![("Artists", false), ("Albums", true), ("Tracks", false)],
                "h ← Artists · l → Tracks",
            )),
            _ => Some((
                vec![("Artists", false), ("Albums", false), ("Tracks", true)],
                "h ← Albums",
            )),
        },
        View::Playlists => match app.focus_col {
            0 => Some((vec![("Playlists", true), ("Tracks", false)], "l → Tracks")),
            _ => Some((
                vec![("Playlists", false), ("Tracks", true)],
                "h ← Playlists",
            )),
        },
        View::Youtube => match app.focus_col {
            0 => Some((vec![("YouTube", true), ("Tracks", false)], "l → Tracks")),
            _ => Some((vec![("YouTube", false), ("Tracks", true)], "h ← YouTube")),
        },
        View::Queue => None,
    }
}

/// Render the narrow column breadcrumb: the Miller column hierarchy with
/// brackets around columns that aren't currently visible (only the focused
/// column is shown in narrow mode). The active (focused) column is rendered
/// with accent + BOLD; bracketed columns are dim. The `›` separator is a
/// non-color structural cue (survives `NO_COLOR`), matching the wide-layout
/// breadcrumb convention.
fn render_narrow_breadcrumb(f: &mut Frame, area: Rect, cols: &[(&str, bool)]) {
    let theme = Theme::default();
    let nc = theme::no_color();
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });
    let active = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span> = Vec::new();
    for (i, (label, is_active)) in cols.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                format!(" {} ", if theme::is_ascii() { ">" } else { "›" }),
                dim,
            ));
        }
        if *is_active {
            spans.push(Span::styled(*label, active));
        } else {
            spans.push(Span::styled(format!("[{label}]"), dim));
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(spans).alignment(Alignment::Center))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// Render the drill-in/out hint line below the column breadcrumb. A dim,
/// centered one-liner telling the user which keys move between the columns
/// shown in the breadcrumb (e.g. `l → Albums · Enter → Tracks`).
fn render_narrow_hint(f: &mut Frame, area: Rect, hint: &str) {
    let theme = Theme::default();
    let dim = Style::default().fg(if theme::no_color() {
        Color::Reset
    } else {
        theme.dim
    });
    let hint = theme::ascii_sanitize(hint);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, dim)))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// Render a centered "terminal too small" message and nothing else. The user
/// is told to resize the window or press `q` to quit — no browse chrome is
/// drawn in this state so a cramped terminal doesn't show garbage.
fn render_too_small(f: &mut Frame, area: Rect) {
    let msg = format!(
        "terminal too small {} resize or press q to quit",
        theme::em_dash()
    );
    let paragraph = Paragraph::new(Line::from(msg))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(paragraph, area);
}

// ---------------------------------------------------------------------------
// Separator rule (full layout) + tab bar with breadcrumb (top bar)
// ---------------------------------------------------------------------------

/// The thin dim rule drawn directly above the player bar — a plain visual
/// separator between the browse content and the player bar. The breadcrumb
/// path now lives in the top tab bar ([`render_tab_bar`]) so the user sees
/// their location at a glance at the top of the screen (T4); this rule just
/// marks the boundary above the player bar.
///
/// Suppressed at `width < 90` to reclaim 1 row for the browse area on narrow
/// terminals (see [`draw`]); the narrow path has its own HR below the top bar.
fn render_separator_rule(f: &mut Frame, area: Rect, _app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = Theme::default();
    let dim = Style::default().fg(if theme::no_color() {
        Color::Reset
    } else {
        theme.dim
    });
    let rule = h_line().repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(rule, dim)))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// Build a breadcrumb string for the current navigation context. Shows the
/// path from the view root down to the current selection depth, so the user
/// always knows where they are in the hierarchy (GLM: no-breadcrumbs).
///
/// - Artists view: `Artists › {artist} › {album}` (depth follows `focus_col`)
/// - Playlists view: `Playlists › {playlist}`
/// - Queue view: `Queue`
/// - YouTube view: `YouTube › {list}`
///
/// The `›` separator is a non-color structural cue (survives `NO_COLOR`).
fn breadcrumb(app: &App) -> String {
    let sep = format!(" {} ", if theme::is_ascii() { ">" } else { "›" });
    match app.view {
        View::Artists => {
            let mut parts: Vec<String> = vec!["Artists".to_string()];
            if app.focus_col >= 1 {
                if let Some(artist) = app.artists.get(app.cursors.artist) {
                    parts.push(artist.clone());
                }
            }
            if app.focus_col >= 2 {
                if let Some(artist) = app.artists.get(app.cursors.artist) {
                    if let Some(albums) = app.albums_by_artist.get(artist) {
                        if let Some(album) = albums.get(app.cursors.album) {
                            parts.push(album.title.clone());
                        }
                    }
                }
            }
            parts.join(&sep)
        }
        View::Playlists => {
            let mut parts: Vec<String> = vec!["Playlists".to_string()];
            if app.focus_col >= 1 {
                if let Some(pl) = app.playlists.get(app.cursors.playlist) {
                    parts.push(pl.name.clone());
                }
            }
            parts.join(&sep)
        }
        View::Queue => "Queue".to_string(),
        View::Youtube => {
            let mut parts: Vec<String> = vec!["YouTube".to_string()];
            if app.focus_col >= 1 {
                if let Some(list) = app.yt_lists.get(app.cursors.playlist) {
                    parts.push(list.name.clone());
                }
            }
            parts.join(&sep)
        }
    }
}

/// Top bar: shows the four top-level views as tabs separated by `│`, with
/// the active view highlighted (accent + BOLD + REVERSED), AND the current
/// navigation breadcrumb right-aligned on the same row (T3 + T4). This gives
/// the user view-switch context + key hints (the `1`–`4` prefix matches the
/// actual view-switch keys) AND their location in the hierarchy — both at
/// the top of the screen, at all widths ≥ 80 (T3: previously the wide layout
/// had no top tab bar; T4: previously the breadcrumb was buried in the
/// separator above the player bar).
///
/// Tabs: `1:Artists │ 2:Playlists │ 3:Queue │ 4:YouTube`
/// Breadcrumb (right-aligned): `Artists › 40mP › Cosmic` (built by
/// [`breadcrumb`]). Dropped when the remaining width is too narrow for it
/// to fit cleanly so the tabs never get crowded out.
///
/// The active tab uses accent + BOLD + REVERSED (three non-color cues under
/// `NO_COLOR` — bold weight + reverse video survive monochrome); inactive
/// tabs are dim. The `│` separator is dim so the tabs read as a connected
/// bar, not disconnected labels.
fn render_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = Theme::default();
    let nc = theme::no_color();
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });
    let active = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::REVERSED);
    let text = Style::default().fg(if nc { Color::Reset } else { theme.text });

    // View-switch tabs — the `N:` prefix matches the actual view-switch keys
    // (1=Artists, 2=Playlists, 3=Queue, 4=YouTube). The full "YouTube" label is
    // used so the tab name matches the view name exactly (T3: previously
    // abbreviated to "YT" which was ambiguous).
    let tabs: [(&str, View); 4] = [
        ("1:Artists", View::Artists),
        ("2:Playlists", View::Playlists),
        ("3:Queue", View::Queue),
        ("4:YouTube", View::Youtube),
    ];

    let sep: &'static str = if v_sep() == "|" { " | " } else { " │ " };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut tabs_w: usize = 0;
    for (i, (label, view)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(sep, dim));
            tabs_w += theme::disp_width(sep);
        }
        let style = if app.view == *view { active } else { dim };
        spans.push(Span::styled((*label).to_string(), style));
        tabs_w += theme::disp_width(label);
    }

    // Breadcrumb right-aligned: `Artists › 40mP › Cosmic`. Only render when
    // there's room (tabs + 2-space gap + breadcrumb ≤ width) so the tabs
    // never get crowded out on narrower widths. At 80 cols the tabs (~38
    // cols) + a short breadcrumb fit comfortably.
    let bc = breadcrumb(app);
    let bc_w = theme::disp_width(&bc);
    let gap = 2usize;
    if !bc.is_empty() && tabs_w + gap + bc_w <= area.width as usize {
        let spaces = area.width as usize - tabs_w - bc_w;
        spans.push(Span::raw(" ".repeat(spaces)));
        spans.push(Span::styled(bc, text));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans).alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::CellDiffOption;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    /// DEF-002 regression: a space between two words (default style) must be
    /// marked `AlwaysUpdate` by `force_space_redraw` so the diff writes it
    /// to the terminal, overwriting stale content from the previous frame.
    #[test]
    fn force_space_redraw_marks_spaces_between_words() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("hello world"), f.area());
                force_space_redraw(f);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Position 5 is the space between "hello" and "world".
        assert_eq!(
            buf[(5, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "space between words must be AlwaysUpdate"
        );
        // Position 0 is 'h' (non-space) — should NOT be AlwaysUpdate.
        assert_ne!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "non-space characters should not be force-marked"
        );
    }

    /// DEF-002 regression: a space in the empty background (not adjacent to
    /// any text) should NOT be marked — it doesn't need a forced write.
    #[test]
    fn force_space_redraw_skips_isolated_spaces() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                // Render only 5 chars in a 20-wide area — cols 5-19 are empty.
                f.render_widget(Paragraph::new("hello"), f.area());
                force_space_redraw(f);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Position 10 is empty background (no text neighbour).
        assert_ne!(
            buf[(10, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "isolated background space should not be force-marked"
        );
    }

    /// DEF-004 regression: MIN_WIDTH must be 100 so 100×30 uses the full
    /// 3-column layout with the 2-row player bar (SHUF/RPT/volume visible).
    #[test]
    fn min_width_is_100() {
        assert_eq!(MIN_WIDTH, 100);
    }

    /// MAJ-2: At large terminal sizes (160×50), `force_space_redraw` must not
    /// mark more than `MAX_MARKED_CELLS` cells as `AlwaysUpdate`. Without the
    /// cap, ~2100 cells are marked at 120×40, flooding the terminal with erase
    /// commands that corrupt the status bar and hint line.
    #[test]
    fn force_space_redraw_caps_marked_cells_at_large_sizes() {
        let backend = TestBackend::new(160, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                // Render many short words with spaces on every line to create
                // a large number of eligible default-styled space cells.
                for y in 0..50u16 {
                    let text = "word ".repeat(32); // 160 chars, lots of spaces
                    f.render_widget(Paragraph::new(text), Rect::new(0, y, 160, 1));
                }
                force_space_redraw(f);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let count = (0..50u16)
            .flat_map(|y| (0..160u16).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].diff_option == CellDiffOption::AlwaysUpdate)
            .count();
        assert!(
            count <= MAX_MARKED_CELLS,
            "MAJ-2: too many cells marked at 160×50: {count} > {MAX_MARKED_CELLS}"
        );
    }

    /// MOD-7: `force_full_redraw` must mark every non-empty (non-default-space)
    /// cell as `AlwaysUpdate` so the diff emits it unconditionally on a view
    /// switch. Without this, a cell whose content+style coincidentally matches
    /// the previous frame at the same position is skipped (e.g. "i" in "Test
    /// Artist" → "i" in "Late Night Jazz"), leaving stale content.
    #[test]
    fn force_full_redraw_marks_non_empty_cells() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("hello"), f.area());
                force_full_redraw(f);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // 'h' at (0,0) is a non-space character → must be AlwaysUpdate.
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: non-space cell must be marked AlwaysUpdate by force_full_redraw"
        );
        // 'o' at (4,0) is the last char of "hello" → must be AlwaysUpdate.
        assert_eq!(
            buf[(4, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: non-space cell must be marked AlwaysUpdate by force_full_redraw"
        );
        // Cell at (10,0) is an empty/default space → must NOT be marked
        // (left to force_space_redraw to handle inter-word spaces).
        assert_ne!(
            buf[(10, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: default-styled empty cell should not be marked by force_full_redraw"
        );
    }

    /// MOD-7: `force_full_redraw` must mark styled (non-default fg/bg/modifier)
    /// cells even if their symbol is a space. A styled space (e.g. a colored
    /// background) is "non-empty" in the visual sense and must be re-emitted
    /// on a view switch so the old frame's styling doesn't linger.
    #[test]
    fn force_full_redraw_marks_styled_spaces() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    Paragraph::new(" ").style(Style::default().bg(Color::Blue)),
                    f.area(),
                );
                force_full_redraw(f);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // The styled space at (0,0) has bg=Blue → must be AlwaysUpdate.
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: styled space (non-default bg) must be marked AlwaysUpdate"
        );
    }

    /// MOD-7: `post_render_force` must trigger `force_full_redraw` on EVERY
    /// frame — not just on view change. This is the persistent
    /// character-dropping fix: ratatui's diff skips cells whose content+style
    /// coincidentally match the previous frame at the same position, which
    /// happens on initial render, same-view updates (j/k navigation,
    /// scrolling), AND view switches. Marking all non-empty cells as
    /// `AlwaysUpdate` on every frame guarantees the terminal always reflects
    /// the buffer.
    #[test]
    fn post_render_force_always_triggers_full_redraw() {
        use crate::catalog::Catalog;
        use crate::player::StubPlayer;
        use crate::tui::app::{App, View};

        // Minimal catalog for App::new.
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
               "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        let cat = Catalog::load(&p).unwrap();

        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.view = View::Artists;
        app.last_rendered_view = View::Artists;

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        // Frame 1: same view → full redraw NOW runs on every frame. 'h'
        // must be AlwaysUpdate (MOD-7 persistent fix).
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("hello"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: cell must be marked AlwaysUpdate on every frame, even when view is unchanged"
        );

        // Switch view → full redraw still runs.
        app.view = View::Youtube;

        // Frame 2: view changed → full redraw. 'w' must be AlwaysUpdate.
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("world"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: cell must be marked AlwaysUpdate after view change"
        );
        // last_rendered_view must be updated so other code can track it.
        assert_eq!(
            app.last_rendered_view,
            View::Youtube,
            "MOD-7: last_rendered_view must be updated after view change"
        );

        // Frame 3: same view again → full redraw STILL runs (the persistent
        // fix). 'a' must be AlwaysUpdate.
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("again"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: cell must be marked AlwaysUpdate on every frame, even after a view switch"
        );
    }

    /// MOD-7 persistent fix: the first frame (initial render) must mark all
    /// non-empty cells as `AlwaysUpdate`. The cleared initial buffer can
    /// coincidentally match a styled cell, causing the diff to skip it and
    /// drop a character on the very first render.
    #[test]
    fn post_render_force_marks_cells_on_initial_frame() {
        use crate::catalog::Catalog;
        use crate::player::StubPlayer;
        use crate::tui::app::{App, View};

        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
               "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        let cat = Catalog::load(&p).unwrap();

        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.view = View::Artists;
        app.last_rendered_view = View::Artists;

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        // The very first draw on a fresh terminal — every non-empty cell
        // must be marked so the diff emits it, even though the previous
        // buffer is the cleared all-spaces buffer.
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("delete"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // All 6 characters of "delete" must be AlwaysUpdate.
        for (i, _ch) in "delete".chars().enumerate() {
            assert_eq!(
                buf[(i as u16, 0)].diff_option,
                CellDiffOption::AlwaysUpdate,
                "MOD-7: initial frame must mark every char of 'delete' as AlwaysUpdate (char at {i})"
            );
        }
    }

    /// MOD-7 persistent fix: a same-view update (e.g. j/k navigation changes
    /// the rendered text) must mark all non-empty cells as `AlwaysUpdate`.
    /// Without this, a cell whose new character coincidentally matches the
    /// old character at the same position with the same style is skipped
    /// (e.g. "i" in "Test Artist" → "i" in "Best Artist" at the same col).
    #[test]
    fn post_render_force_marks_cells_on_same_view_update() {
        use crate::catalog::Catalog;
        use crate::player::StubPlayer;
        use crate::tui::app::{App, View};

        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
               "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        let cat = Catalog::load(&p).unwrap();

        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.view = View::Artists;
        app.last_rendered_view = View::Artists;

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        // Frame 1: "Test Artist".
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("Test Artist"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();

        // Frame 2: same view, different text where "i" is at the same
        // position. Without every-frame force_full_redraw, the "i" would
        // be skipped by the diff (same char + same style).
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new("Best Artist"), f.area());
                post_render_force(f, &mut app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // "Best Artist" — 'B' at (0,0) must be AlwaysUpdate (the old 'T'
        // was different, but force ensures it regardless). And the 'i' at
        // (5,0) must also be AlwaysUpdate even though the previous frame
        // also had 'i' at (5,0) with the same style.
        assert_eq!(
            buf[(0, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: same-view update must mark 'B' as AlwaysUpdate"
        );
        assert_eq!(
            buf[(5, 0)].diff_option,
            CellDiffOption::AlwaysUpdate,
            "MOD-7: same-view update must mark 'i' as AlwaysUpdate even though previous frame also had 'i' at the same position"
        );
    }
}
