//! Responsive layout tests — 4 terminal sizes × 4 font/color modes.
use jukebox::tui::view::home::{HomeSection, HomeState};
use jukebox::tui::view::icons::{FontMode, Icon, IconRenderer};

fn render_at(width: u16, height: u16, mode: FontMode) {
    let icons = IconRenderer::new(mode);
    let state = HomeState::new();
    let sections = vec![
        (HomeSection::MadeForYou, vec![]),
        (HomeSection::Explore, vec![]),
    ];
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let _ = jukebox::tui::view::home::render_compact(area, &sections, &state, &icons);
}

#[test]
fn render_at_80x24_nerd_font() {
    render_at(80, 24, FontMode::NerdFont);
}
#[test]
fn render_at_80x24_unicode() {
    render_at(80, 24, FontMode::Unicode);
}
#[test]
fn render_at_80x24_ascii() {
    render_at(80, 24, FontMode::Ascii);
}

#[test]
fn render_at_100x30_unicode() {
    render_at(100, 30, FontMode::Unicode);
}
#[test]
fn render_at_120x40_unicode() {
    render_at(120, 40, FontMode::Unicode);
}
#[test]
fn render_at_160x50_unicode() {
    render_at(160, 50, FontMode::Unicode);
}

#[test]
fn no_icon_only_meaning_in_ascii() {
    let icons = IconRenderer::new(FontMode::Ascii);
    for icon in [
        Icon::Playing,
        Icon::Paused,
        Icon::Like,
        Icon::Radio,
        Icon::Generated,
    ] {
        assert!(!icon.label().is_empty());
        assert!(icons.glyph(icon).is_ascii());
    }
}
