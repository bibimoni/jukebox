//! Top-level TUI layout + responsive breakpoints.
//!
//! [`draw`] is the single entry the event loop calls. It handles the
//! "terminal too small" guard, then splits the screen into a main browse area
//! (Miller columns rendered by [`columns::render`]) and a 2-line persistent
//! player bar ([`player_bar::render`]); if an [`Overlay`] is active it is
//! painted on top via [`overlay::render`] (Task 11 fills in the real overlay
//! surface — for now it is a no-op stub so layout compiles standalone).
//!
//! Responsive breakpoints:
//! - At `width < 80` or `height < 24` we refuse to render the browse layout
//!   and instead show a centered "terminal too small" message. 80×24 is the
//!   minimum that comfortably fits the three-column Miller layout + the player
//!   bar; below that the columns compress past readability.
//! - At 80–120 cols the columns themselves compress via `Constraint` ratios
//!   inside [`columns::render`] (it reads `outer[0].width` and adjusts).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use super::{columns, footer, overlay, player_bar};

/// Minimum terminal size we'll attempt to render the full browse layout in.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
/// Below this the app refuses to render and shows a "terminal too small" message.
pub const NARROW_MIN_WIDTH: u16 = 60;
pub const NARROW_MIN_HEIGHT: u16 = 20;

/// Height of the persistent player bar at the bottom of the screen.
const PLAYER_BAR_HEIGHT: u16 = 2;
/// Height of the always-visible footer hint bar.
const FOOTER_HEIGHT: u16 = 1;

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

    // Narrow terminals (60–80 cols, or < 24 rows) collapse the Miller columns
    // to a single focused pane with a compressed 1-row player bar + short
    // footer (spec §5.6) — usable in a tmux split.
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_narrow(f, area, app);
        if app.overlay.is_some() {
            overlay::render(f, area, app);
        }
        return;
    }

    // Vertical split: main browse area gets the remainder; player bar gets a
    // fixed 2-line strip; footer gets a fixed 1-line hint strip. At 80×24
    // that's content 21 + bar 2 + footer 1.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(PLAYER_BAR_HEIGHT),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);

    columns::render(f, outer[0], app);
    player_bar::render(f, outer[1], app);
    footer::render(f, &outer[2], app);

    if app.overlay.is_some() {
        overlay::render(f, area, app);
    }
}

/// Narrow fallback: a single focused pane (Miller collapse — `h`/`l` drills
/// in/out), a 1-row compressed player bar (info+flags share a row, no gauge),
/// and a short 1-row footer. Below the columns we still render the rail so the
/// view letters stay visible.
fn render_narrow(f: &mut Frame, area: Rect, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1), // compressed player bar
            Constraint::Length(1), // short footer
        ])
        .split(area);

    // Single pane: render the focused column only via the columns module's
    // narrow path. The columns renderer already adapts to area.width; in a
    // narrow area it shows the focused column with a breadcrumb title.
    columns::render_narrow(f, outer[0], app);

    // Compressed 1-row player bar: now-playing + quality + flags on one line.
    player_bar::render_compact(f, outer[1], app);
    footer::render(f, &outer[2], app);
}

/// Render a centered "terminal too small" message and nothing else. The user
/// is told to resize the window or press `q` to quit — no browse chrome is
/// drawn in this state so a cramped terminal doesn't show garbage.
fn render_too_small(f: &mut Frame, area: Rect) {
    let msg = "terminal too small — resize or press q to quit";
    let paragraph = Paragraph::new(Line::from(msg))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(paragraph, area);
}
