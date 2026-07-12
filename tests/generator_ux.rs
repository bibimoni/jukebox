//! Tests for generator + publication UX.

use jukebox::tui::view::generator::{GeneratorPhase, GeneratorState};
use jukebox::tui::view::icons::FontMode;
use jukebox::tui::view::publication::PublicationState;

#[test]
fn generator_input_phase() {
    let s = GeneratorState::new();
    assert_eq!(s.phase, GeneratorPhase::Input);
}

#[test]
fn generator_parse_moves_to_review() {
    let mut s = GeneratorState::new();
    s.input = "45-minute energetic running".into();
    s.parse_input();
    assert_eq!(s.phase, GeneratorPhase::ReviewPlan);
    assert!(s.constraints.is_some());
}

#[test]
fn generator_generate_moves_to_preview() {
    let mut s = GeneratorState::new();
    s.input = "calm mix".into();
    s.parse_input();
    s.generate();
    assert_eq!(s.phase, GeneratorPhase::Preview);
}

#[test]
fn generator_render_input() {
    let s = GeneratorState::new();
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    let para =
        jukebox::tui::view::generator::render(ratatui::layout::Rect::new(0, 0, 80, 24), &s, &icons);
    let _ = para;
}

#[test]
fn publication_defaults_private() {
    let s = PublicationState::new();
    assert_eq!(s.privacy, "PRIVATE");
}

#[test]
fn publication_not_ready_when_empty() {
    assert!(!PublicationState::new().is_ready());
}

#[test]
fn publication_ready_when_all_set() {
    let mut s = PublicationState::new();
    s.name = "Test".into();
    s.account = "user@gmail.com".into();
    s.publishable_ids = vec!["v1".into()];
    assert!(s.is_ready());
}

#[test]
fn publication_render() {
    let s = PublicationState::new();
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    let para = jukebox::tui::view::publication::render(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        &s,
        &icons,
    );
    let _ = para;
}
