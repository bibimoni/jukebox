//! Shared spinner + buffering state for the Now Playing Deck.
//!
//! The braille / ASCII spinner frames live here so the deck's minimal
//! layout and any future deck components share a single source of truth.
//! The frames are identical to `player_bar::SPINNER` /
//! `player_bar::SPINNER_ASCII` (`player_bar.rs:36,41`) — duplicated rather
//! than re-exported so `player_bar.rs` stays byte-identical (the ~40
//! existing player-bar tests assert on those constants directly).

use crate::tui::app::App;

/// Braille spinner frames (U+2800–28FF, width 1). Animated in
/// `App::on_tick` while a YouTube resolve is in flight.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII spinner frames — a fallback for minimal terminals (often paired
/// with `NO_COLOR`) where the braille dots may not render in the font.
pub const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

/// Pick the spinner glyph: ASCII when `JUKEBOX_FONT_MODE=ascii`, braille
/// otherwise. `spinner_frame` wraps modulo the active frame count. Mirrors
/// `player_bar::spinner_glyph` (`player_bar.rs:126`).
pub fn spinner_glyph(app: &App) -> &'static str {
    let frames = if crate::tui::view::theme::is_ascii() {
        &SPINNER_ASCII[..]
    } else {
        &SPINNER[..]
    };
    frames[app.spinner_frame as usize % frames.len()]
}

/// True when a YouTube track is being resolved (cold miss, `pending_play`
/// set) or loaded but not yet playing while a resolve is in flight.
/// Mirrors `player_bar::is_buffering` (`player_bar.rs:140`).
pub fn is_buffering(app: &App) -> bool {
    app.pending_play.is_some()
        || (app.now_playing.is_some() && !app.player.is_playing() && app.is_resolving())
}
