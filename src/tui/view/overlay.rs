//! Modal overlays (help, search, playlist picker, command).
//!
//! Task 11 implements the real overlay surface. For now this module exposes a
//! no-op [`render`] so [`crate::tui::view::layout::draw`] compiles standalone;
//! Task 11 will replace the body with the per-`Overlay` rendering switch.

use ratatui::{layout::Rect, Frame};

use crate::tui::app::App;

/// No-op stub. Task 11 replaces this with the per-`Overlay` rendering switch.
pub fn render(_f: &mut Frame, _area: Rect, _app: &mut App) {}
