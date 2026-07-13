//! Tests for radio UX.
use jukebox::catalog::Track;
use jukebox::reco::events::ListenEvent;
use jukebox::reco::profile::UserProfile;
use jukebox::reco::radio::{RadioSeed, RadioSession};
use jukebox::tui::view::icons::FontMode;
use jukebox::tui::view::radio;
use std::path::PathBuf;

fn mk(id: &str, artist: &str) -> Track {
    Track {
        id: id.into(),
        artists: vec![artist.into()],
        primary_artist: artist.into(),
        title: id.into(),
        album: Some("Album".into()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path: PathBuf::from("/t.flac"),
        symlinked_into_artists: vec![],
    }
}

#[test]
fn radio_from_track() {
    let events = vec![ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    assert!(!r.candidate_pool.is_empty());
}

#[test]
fn radio_next_track_to_history() {
    let events = vec![ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    let next = r.next_track();
    assert!(next.is_some());
    assert_eq!(r.history().len(), 1);
}

#[test]
fn radio_negative_feedback_excludes() {
    let events = vec![ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    r.negative_feedback("t2");
    assert!(!r.candidate_pool.iter().any(|c| c.track_id == "t2"));
}

#[test]
fn radio_change_seed_clears() {
    let events = vec![ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A"), mk("t2", "B")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    r.next_track();
    r.change_seed(RadioSeed::Track("t2".into()), &p, &catalog);
    assert!(r.history().is_empty());
}

#[test]
fn radio_stop_clears_pool() {
    let events = vec![ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    }];
    let p = UserProfile::build_from_events(&events);
    let catalog = vec![mk("t1", "A")];
    let mut r = RadioSession::new(RadioSeed::Track("t1".into()));
    r.initialize(&p, &catalog);
    r.stop();
    assert!(r.candidate_pool.is_empty());
}

#[test]
fn radio_overlay_renders() {
    let session = RadioSession::new(RadioSeed::Track("t1".into()));
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    // Sibling-batch change: `radio::render` now takes a resolved
    // `seed_title` + `upcoming` slice so the view layer doesn't own
    // catalog/track_cache lookups. Pass empty/minimal values — the test
    // only verifies render doesn't panic.
    let seed_title = "t1";
    let upcoming: Vec<String> = Vec::new();
    let played: Vec<String> = Vec::new();
    let para = radio::render(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        &session,
        &icons,
        seed_title,
        &upcoming,
        &played,
    );
    let _ = para;
}
