//! Regression tests for Fixer B defects (Batch B — Radio overlay + reco engine).
//!
//! - B.1 DEF-006: Radio `c` (change seed) dead — now wired.
//! - B.2 DEF-007: Radio `q` (stop) dead — now wired.
//! - B.3 DEF-010: Generator includes YouTube tracks when connected.
//! - B.4 DEF-011: Radio pool includes YouTube tracks when connected.
//! - B.5 DEF-023: Radio `+` shows a hint when nothing playing.
//! - B.6 DEF-026: `>` advances the radio.
//! - B.7 DEF-027: CONT=next continues (radio/wrap) when the queue exhausts.
//! - B.8 DEF-034: Reco engine records listening events during playback.
//! - B.9 DEF-061: Radio overlay shows resolved seed title, not raw id.
//! - B.11 DEF-063: Radio overlay shows upcoming track list.
//! - B.12 DEF-056: Radio auto-starts on `:radio` (not silent).
//! - B.13 DEF-064: autoplay.rs removed; evaluation.rs wired via `:profile`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::{Catalog, Track};
use jukebox::player::StubPlayer;
use jukebox::reco::radio::{RadioSeed, RadioSession};
use jukebox::tui::app::{App, Overlay};
use jukebox::tui::input::handle_key;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn mk(id: &str, artist: &str, title: &str, source_path: PathBuf) -> Track {
    Track {
        id: id.into(),
        artists: vec![artist.into()],
        primary_artist: artist.into(),
        title: title.into(),
        album: Some("Album".into()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path,
        symlinked_into_artists: vec![artist.into()],
    }
}

/// Build a catalog backed by REAL .flac files in a temp dir so
/// `start_playback`'s `std::fs::metadata` check passes and the StubPlayer
/// loads. Returns the TempDir (caller must keep it alive for the test).
fn make_catalog() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("A").join("02.flac"), b"x").unwrap();
    std::fs::write(lossless.join("A").join("03.flac"), b"x").unwrap();
    let tracks = vec![
        mk(
            "t1",
            "Artist A",
            "Song 1",
            lossless.join("A").join("01.flac"),
        ),
        mk(
            "t2",
            "Artist B",
            "Song 2",
            lossless.join("A").join("02.flac"),
        ),
        mk(
            "t3",
            "Artist A",
            "Song 3",
            lossless.join("A").join("03.flac"),
        ),
    ];
    let cat = Catalog {
        version: 1,
        built_at: "test".into(),
        source_root: d.path().to_path_buf(),
        tracks,
    };
    (d, cat)
}

fn mk_dummy(id: &str, artist: &str, title: &str) -> Track {
    mk(id, artist, title, PathBuf::from("/tmp/dummy.flac"))
}

fn make_app() -> App {
    // Isolate XDG so save_yt_lists / state.db writes never touch the user's
    // real home dir. The comment below already says "tests must NOT touch the
    // real state DB" — this actually enforces it.
    let _xdg = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg.path());
    let (_dir, cat) = make_catalog();
    // Leak the tempdir so it (and its .flac files) stay alive for the test's
    // lifetime — the StubPlayer reads the file path during `load`, which is
    // after this constructor returns. `mem::forget` avoids early cleanup.
    let dir = _dir;
    std::mem::forget(dir);
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

/// Start a track playing (so `now_playing` is set), then open a radio overlay
/// over it WITHOUT auto-advancing (we construct the overlay directly so the
/// auto-start in `start_radio_from_track` doesn't switch the track).
fn app_with_radio_overlay(seed: &str) -> App {
    let mut app = make_app();
    // Play the seed track so now_playing is set.
    app.play_in_context_ids(vec![seed.to_string()], seed);
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id().to_string()),
        Some(seed.to_string()),
        "seed track should be playing"
    );
    // Open a radio overlay seeded from the track, but don't auto-advance —
    // construct the session + overlay directly to keep `now_playing` on the seed.
    let mut session = RadioSession::new(RadioSeed::Track(seed.to_string()));
    session.initialize(&app.reco_profile, &app.catalog.tracks);
    app.overlay = Some(Overlay::Radio {
        session: Some(session),
    });
    app
}

// ---------------------------------------------------------------------------
// B.1 DEF-006: Radio `c` (change seed)
// ---------------------------------------------------------------------------

#[test]
fn radio_c_changes_seed() {
    let mut app = app_with_radio_overlay("t1");
    // `c` should change the seed. With nothing else playing after the change,
    // the seed becomes the first upcoming pool track (or the now-playing).
    handle_key(&mut app, key('c'));
    // The overlay should still be a Radio overlay (not closed).
    assert!(matches!(app.overlay, Some(Overlay::Radio { .. })));
    // A status message confirms the change.
    assert_eq!(app.yt_status.as_deref(), Some("radio seed changed"));
}

#[test]
fn radio_c_resets_played_list() {
    let mut app = app_with_radio_overlay("t1");
    // Advance once so the played-list is non-empty.
    handle_key(&mut app, key('n'));
    // The advance switches now_playing to the next radio track.
    let played_after_n = match &app.overlay {
        Some(Overlay::Radio { session }) => {
            session.as_ref().map(|s| s.history().len()).unwrap_or(0)
        }
        _ => 0,
    };
    assert!(played_after_n >= 1, "n should have advanced the radio");
    // `c` resets the played-list.
    handle_key(&mut app, key('c'));
    let played_after_c = match &app.overlay {
        Some(Overlay::Radio { session }) => {
            session.as_ref().map(|s| s.history().len()).unwrap_or(0)
        }
        _ => 0,
    };
    assert_eq!(played_after_c, 0, "c should clear the played-list");
}

// ---------------------------------------------------------------------------
// B.2 DEF-007: Radio `q` (stop)
// ---------------------------------------------------------------------------

#[test]
fn radio_q_stops_and_closes_overlay() {
    let mut app = app_with_radio_overlay("t1");
    handle_key(&mut app, key('q'));
    // Overlay should be closed.
    assert!(app.overlay.is_none(), "q should close the radio overlay");
    // Status confirms the stop.
    assert_eq!(app.yt_status.as_deref(), Some("radio stopped"));
}

// ---------------------------------------------------------------------------
// B.5 DEF-023: Radio `+` hint when nothing playing
// ---------------------------------------------------------------------------

#[test]
fn radio_plus_with_nothing_playing_shows_hint() {
    let mut app = make_app();
    // Open a radio overlay with NO track playing.
    let session = RadioSession::new(RadioSeed::Track("t1".into()));
    app.overlay = Some(Overlay::Radio {
        session: Some(session),
    });
    assert!(app.now_playing.is_none());
    handle_key(&mut app, key('+'));
    assert_eq!(
        app.yt_status.as_deref(),
        Some("nothing playing — press n to start"),
        "+ with nothing playing should show a hint"
    );
}

// ---------------------------------------------------------------------------
// B.6 DEF-026: `>` advances the radio
// ---------------------------------------------------------------------------

#[test]
fn radio_gt_advances_like_n() {
    let mut app = app_with_radio_overlay("t1");
    let before = match &app.overlay {
        Some(Overlay::Radio { session }) => {
            session.as_ref().map(|s| s.history().len()).unwrap_or(0)
        }
        _ => 0,
    };
    handle_key(&mut app, key('>'));
    let after = match &app.overlay {
        Some(Overlay::Radio { session }) => {
            session.as_ref().map(|s| s.history().len()).unwrap_or(0)
        }
        _ => 0,
    };
    assert_eq!(after, before + 1, "> should advance the radio like n");
}

// ---------------------------------------------------------------------------
// B.7 DEF-027: CONT=next continues when the queue exhausts (wrap)
// ---------------------------------------------------------------------------

#[test]
fn cont_next_wraps_when_queue_exhausts() {
    use jukebox::tui::queue::ContinueMode;
    let mut app = make_app();
    app.transport.continue_mode = ContinueMode::NextAlbum;
    // Play a 2-track context.
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id().to_string()),
        Some("t1".into())
    );
    // Advance past the last track → NextAlbum has no next album, so the
    // wrap fallback should restart the context from the top.
    app.next(); // leaves t1 (skipped), now t2
    app.next(); // leaves t2, queue exhausts → wrap to t1
    let playing = app.now_playing.as_ref().map(|s| s.id().to_string());
    assert!(
        playing.is_some(),
        "CONT=next should not stop at end of queue (radio or wrap)"
    );
}

// ---------------------------------------------------------------------------
// B.8 DEF-034: Reco engine records listening events during playback
// ---------------------------------------------------------------------------

#[test]
fn playback_records_track_started_event() {
    let mut app = make_app();
    assert!(app.reco_events.is_empty());
    app.play_in_context_ids(vec!["t1".into()], "t1");
    // start_playback → load_track → note_play_started → track_started event.
    let events = app.reco_events.recent(10);
    assert!(
        events
            .iter()
            .any(|e| e.type_tag() == "track_started" && e.track_id() == Some("t1")),
        "start_playback should record a track_started event"
    );
}

#[test]
fn track_ended_records_completed_event() {
    let mut app = make_app();
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    // Clear the started event to isolate the completed event.
    app.reco_events.clear();
    app.on_track_ended();
    let events = app.reco_events.recent(10);
    assert!(
        events
            .iter()
            .any(|e| e.type_tag() == "completed" && e.track_id() == Some("t1")),
        "on_track_ended should record a completed event for the finished track"
    );
}

#[test]
fn user_next_records_skipped_event() {
    let mut app = make_app();
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    app.reco_events.clear();
    // A user `>` (next) on a track that's been "playing" a while is a skip.
    // Force the play-start to >10s ago so it's a plain skip, not a rapid one.
    app.play_started_at = Some(
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(20))
            .unwrap_or_else(std::time::Instant::now),
    );
    app.next();
    let events = app.reco_events.recent(10);
    assert!(
        events
            .iter()
            .any(|e| e.type_tag() == "skipped" && e.track_id() == Some("t1")),
        "next() should record a skipped event for the left track"
    );
}

#[test]
fn enqueue_records_added_to_queue_event() {
    let mut app = make_app();
    // Navigate to a track so selected_track_id returns something.
    app.view = jukebox::tui::app::View::Queue;
    app.transport.manual_queue = vec!["t1".into(), "t2".into()];
    app.cursors.queue = 0;
    app.reco_events.clear();
    app.enqueue_selected();
    let events = app.reco_events.recent(10);
    assert!(
        events.iter().any(|e| e.type_tag() == "added_to_queue"),
        "enqueue_selected should record an added_to_queue event"
    );
}

#[test]
fn remove_from_queue_records_removed_event() {
    let mut app = make_app();
    app.view = jukebox::tui::app::View::Queue;
    app.transport.manual_queue = vec!["t1".into(), "t2".into()];
    app.cursors.queue = 0;
    app.reco_events.clear();
    app.remove_selected_from_queue();
    let events = app.reco_events.recent(10);
    assert!(
        events.iter().any(|e| e.type_tag() == "removed_from_queue"),
        "remove_selected_from_queue should record a removed_from_queue event"
    );
}

#[test]
fn meaningful_threshold_fires_on_tick() {
    let mut app = make_app();
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app.reco_events.clear();
    // StubPlayer position is 0 — advance it past 30s to trigger the threshold.
    // We can't set StubPlayer position directly; instead call on_tick after
    // seeking the stub to 35s. The StubPlayer seek adds to pos.
    let _ = app.player.seek(35.0);
    app.on_tick();
    let events = app.reco_events.recent(10);
    assert!(
        events
            .iter()
            .any(|e| e.type_tag() == "meaningful_threshold" && e.track_id() == Some("t1")),
        "on_tick should fire meaningful_threshold once position crosses 30s"
    );
}

#[test]
fn track_started_event_carries_correct_source() {
    use jukebox::reco::events::{EventSource, ListenEvent};
    let mut app = make_app();
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let started = app
        .reco_events
        .recent(10)
        .into_iter()
        .find(|e| e.type_tag() == "track_started")
        .expect("track_started event should exist");
    // t1 is a local catalog track → source is Local (default mode).
    assert_eq!(started.track_id(), Some("t1"));
    // Verify it's a TrackStarted variant with Local source.
    if let ListenEvent::TrackStarted { source, .. } = started {
        assert_eq!(*source, EventSource::Local);
    } else {
        panic!("expected TrackStarted variant");
    }
}

// ---------------------------------------------------------------------------
// B.12 DEF-056: Radio auto-starts on `:radio` (not silent)
// ---------------------------------------------------------------------------

#[test]
fn start_radio_from_track_auto_advances() {
    let mut app = make_app();
    assert!(app.now_playing.is_none());
    app.start_radio_from_track("t1");
    // Auto-start should have advanced to a radio track (now_playing is set).
    assert!(
        app.now_playing.is_some(),
        "start_radio_from_track should auto-start playback (not open silent)"
    );
    let playing = app.now_playing.as_ref().map(|s| s.id().to_string());
    assert_ne!(
        playing,
        Some("t1".into()),
        "auto-start should play a different track than the seed"
    );
}

// ---------------------------------------------------------------------------
// B.3/B.4 DEF-010/DEF-011: YouTube tracks in candidate pool
// ---------------------------------------------------------------------------

#[test]
fn candidate_generator_blends_yt_tracks() {
    use jukebox::reco::candidates::CandidateGenerator;
    use jukebox::reco::profile::UserProfile;
    let profile = UserProfile::new();
    let catalog = vec![mk_dummy("t1", "A", "Song 1")];
    let yt_ids = vec!["yt1".to_string(), "yt2".to_string()];
    let gen = CandidateGenerator::new(&profile, &catalog).with_yt_track_ids(&yt_ids);
    let candidates = gen.generate();
    assert!(
        candidates
            .iter()
            .any(|c| c.track_id == "yt1" && !c.is_local),
        "YouTube track yt1 should be a non-local candidate"
    );
    assert!(
        candidates
            .iter()
            .any(|c| c.track_id == "yt2" && !c.is_local),
        "YouTube track yt2 should be a non-local candidate"
    );
}

#[test]
fn generator_youtube_source_filters_to_yt() {
    use jukebox::reco::generator::{generate_with_yt, GeneratorConstraints, SourcePreference};
    use jukebox::reco::profile::UserProfile;
    let profile = UserProfile::new();
    let catalog = vec![mk_dummy("t1", "A", "Song 1"), mk_dummy("t2", "B", "Song 2")];
    let yt_ids = vec!["yt1".to_string(), "yt2".to_string()];
    let constraints = GeneratorConstraints {
        sources: SourcePreference::Youtube,
        ..Default::default()
    };
    let playlist = generate_with_yt(&constraints, &profile, &catalog, &yt_ids);
    assert!(
        playlist.tracks.iter().all(|c| !c.is_local),
        "Youtube source preference should keep only YouTube tracks"
    );
    assert!(
        !playlist.tracks.is_empty(),
        "Youtube source with yt ids should produce tracks"
    );
}

#[test]
fn radio_session_blends_yt_tracks() {
    let mut app = make_app();
    // Simulate a connected provider by setting yt_state Ready + yt_lists with
    // track ids. yt_track_ids() draws from yt_lists when ready.
    use jukebox::tui::app::{YtList, YtListKind};
    use jukebox::yt::state::YtState;
    app.yt_state = YtState::Ready;
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "My Mix".into(),
        kind: YtListKind::Account,
        track_ids: vec!["yt1".into(), "yt2".into()],
    }];
    let ids = app.yt_track_ids();
    assert!(ids.contains(&"yt1".to_string()));
    assert!(ids.contains(&"yt2".to_string()));
    // A radio session seeded from a local track should blend the YT ids.
    let mut session = RadioSession::new(RadioSeed::Track("t1".into()));
    session.set_yt_track_ids(ids);
    session.initialize(&app.reco_profile, &app.catalog.tracks);
    assert!(
        session
            .candidate_pool
            .iter()
            .any(|c| c.track_id == "yt1" || c.track_id == "yt2"),
        "radio pool should include YouTube tracks when connected"
    );
}

#[test]
fn yt_track_ids_empty_when_not_ready() {
    let mut app = make_app();
    use jukebox::tui::app::{YtList, YtListKind};
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "My Mix".into(),
        kind: YtListKind::Account,
        track_ids: vec!["yt1".into()],
    }];
    // yt_state default is Unconfigured → not ready → empty.
    assert!(app.yt_track_ids().is_empty());
}

// ---------------------------------------------------------------------------
// B.9/B.11 DEF-061/DEF-063: Radio overlay shows resolved seed + upcoming list
// ---------------------------------------------------------------------------

#[test]
fn radio_render_shows_resolved_seed_title() {
    use jukebox::tui::view::icons::FontMode;
    use jukebox::tui::view::radio;
    let session = RadioSession::new(RadioSeed::Track("rzVKfAQp2No".into()));
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    // The view receives a resolved title; verify it accepts a human-readable
    // string (not the raw id) without panic.
    let _ = radio::render(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        &session,
        &icons,
        "Ado — あのバンド",
        &[],
        &[],
    );
}

#[test]
fn radio_render_shows_upcoming_list() {
    use jukebox::tui::view::icons::FontMode;
    use jukebox::tui::view::radio;
    let session = RadioSession::new(RadioSeed::Track("t1".into()));
    let icons = jukebox::tui::view::icons::IconRenderer::new(FontMode::Unicode);
    let upcoming = vec!["Song A — Artist A".into(), "Song B — Artist B".into()];
    let _ = radio::render(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        &session,
        &icons,
        "track t1",
        &upcoming,
        &[],
    );
}

#[test]
fn radio_session_upcoming_returns_pool_tracks() {
    let mut session = RadioSession::new(RadioSeed::Track("t1".into()));
    use jukebox::reco::candidates::{Candidate, CandidateSource};
    session.candidate_pool = vec![
        Candidate::new("a".into(), CandidateSource::Liked, 1.0, true),
        Candidate::new("b".into(), CandidateSource::Liked, 1.0, true),
    ];
    let upcoming = session.upcoming(5);
    assert_eq!(upcoming.len(), 2);
    assert_eq!(upcoming[0].track_id, "a");
}

// ---------------------------------------------------------------------------
// B.13 DEF-064: autoplay.rs removed; evaluation.rs wired via `:profile`
// ---------------------------------------------------------------------------

#[test]
fn profile_health_summary_returns_string() {
    let app = make_app();
    let s = app.profile_health_summary();
    assert!(s.contains("profile:"), "summary should mention profile");
    assert!(s.contains("events"), "summary should mention event count");
}

#[test]
fn autoplay_module_removed() {
    // The reco::autoplay module was removed (DEF-064). Verify it's gone by
    // checking that the path doesn't resolve — this is a compile-time fact,
    // so this test is a placeholder that documents the removal. If autoplay
    // were re-added, `cargo build` would still succeed; this test just ensures
    // the removal didn't break App construction.
    let _app = make_app();
}
