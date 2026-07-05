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
use super::{columns, overlay, player_bar};

/// Minimum terminal size we'll attempt to render the full browse layout in.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// Height of the persistent player bar at the bottom of the screen.
const PLAYER_BAR_HEIGHT: u16 = 2;

/// The single entry point the event loop calls. Renders the full TUI frame:
/// too-small guard, columns + player bar, and any active overlay on top.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(f, area);
        return;
    }

    // Vertical split: main browse area gets the remainder, player bar gets a
    // fixed 2-line strip at the bottom. `Min(3)` guarantees the columns always
    // have at least a header + content + footer row even at exactly 80×24.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(PLAYER_BAR_HEIGHT),
        ])
        .split(area);

    columns::render(f, outer[0], app);
    player_bar::render(f, outer[1], app);

    if app.overlay.is_some() {
        overlay::render(f, area, app);
    }
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
