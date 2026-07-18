//! Tests for the YouTube Home view.

use jukebox::reco::mixes::MixType;
use jukebox::tui::view::home::{HomeItem, HomeItemKind, HomeSection, HomeState};

#[test]
fn home_section_titles() {
    assert_eq!(HomeSection::ContinueListening.title(), "Continue Listening");
    assert_eq!(HomeSection::QuickPicks.title(), "Quick Picks");
    assert_eq!(HomeSection::MadeForYou.title(), "Made for You");
    assert_eq!(HomeSection::StartRadio.title(), "Start Radio");
    assert_eq!(HomeSection::NewRelevant.title(), "New and Relevant");
    assert_eq!(HomeSection::Library.title(), "Your YouTube Library");
    assert_eq!(HomeSection::Explore.title(), "Explore");
}

#[test]
fn home_section_all_has_7_sections() {
    assert_eq!(HomeSection::all().len(), 7);
}

#[test]
fn home_section_requires_history() {
    assert!(HomeSection::ContinueListening.requires_history());
    assert!(HomeSection::QuickPicks.requires_history());
    assert!(!HomeSection::MadeForYou.requires_history());
    assert!(!HomeSection::Explore.requires_history());
}

#[test]
fn home_item_playlist() {
    let item = HomeItem::playlist("PL123".into(), "My Playlist".into(), false);
    assert_eq!(item.title, "My Playlist");
    assert!(matches!(
        item.kind,
        HomeItemKind::Playlist {
            is_local: false,
            ..
        }
    ));
}

#[test]
fn home_item_track() {
    let item = HomeItem::track("v123".into(), "Song".into(), "Artist".into(), true);
    assert!(matches!(
        item.kind,
        HomeItemKind::Track { is_local: true, .. }
    ));
}

#[test]
fn home_item_mix() {
    let item = HomeItem::mix(MixType::DailyMix);
    assert_eq!(item.title, "Daily Mix");
    assert!(matches!(item.kind, HomeItemKind::Mix { .. }));
}

#[test]
fn home_item_with_explanation() {
    let item = HomeItem::track("v1".into(), "Song".into(), "Artist".into(), false)
        .with_explanation("from your liked tracks".into());
    assert!(item.explanation.is_some());
}

#[test]
fn home_state_new_is_loading() {
    let state = HomeState::new();
    assert!(state.loading);
    assert!(!state.has_history);
}

#[test]
fn home_state_cursor_up() {
    let mut state = HomeState::new();
    state.cursor = 2;
    state.cursor_up();
    assert_eq!(state.cursor, 1);
    state.cursor_up();
    assert_eq!(state.cursor, 0);
}

#[test]
fn home_state_section_next() {
    let mut state = HomeState::new();
    state.section_next(7);
    assert_eq!(state.focused_section, 1);
    assert_eq!(state.cursor, 0);
}

#[test]
fn home_state_section_prev() {
    let mut state = HomeState::new();
    state.focused_section = 2;
    state.section_prev();
    assert_eq!(state.focused_section, 1);
    state.section_prev();
    assert_eq!(state.focused_section, 0);
}
