//! Command history tests: Up/Down recall, draft preservation, dedup-adjacent,
//! bound at 100, unicode, and persistence (state.db round-trip).
//!
//! These exercise the `Overlay::Command` key handling in `input.rs` (via
//! `handle_key`, same pattern as `tests/input.rs`) and the `state.rs`
//! persistence helpers (same pattern as `tests/state_ext.rs`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::state;
use jukebox::tui::app::{App, Overlay};
use jukebox::tui::input::handle_key;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A 3-track catalog under one artist/album, with real on-disk source files
/// (so any playback path that checks `std::fs::metadata` passes). Mirrors the
/// helper in `tests/input.rs`.
fn cat_album() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3)
        .map(|n| {
            serde_json::json!({
                "id": format!("t{n}"),
                "artists": ["40mP"],
                "primary_artist": "40mP",
                "title": format!("Song{n}"),
                "album": "Cosmic",
                "track_number": n,
                "bit_depth": 24,
                "sample_rate_hz": 96000,
                "source_path": format!("lossless/40mP/{n:02}.flac"),
                "symlinked_into_artists": ["40mP"],
            })
        })
        .collect();
    let json = serde_json::json!({
        "version": 1,
        "built_at": "x",
        "source_root": lossless.to_str().unwrap(),
        "tracks": tracks,
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Open the `:` command overlay (without submitting).
fn open_command(app: &mut App) {
    handle_key(app, key(':'));
}

/// Type text into the currently-open command overlay.
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        handle_key(app, key(c));
    }
}

/// Press Enter (submit the command).
fn submit(app: &mut App) {
    handle_key(app, key_code(KeyCode::Enter));
}

/// Open `:`, type `text`, press Enter — a full command submission.
fn run_command(app: &mut App, text: &str) {
    open_command(app);
    type_text(app, text);
    submit(app);
}

/// Extract the current command overlay input (panics if the overlay isn't open).
fn command_input(app: &App) -> String {
    match &app.overlay {
        Some(Overlay::Command { input, .. }) => input.clone(),
        _ => panic!("expected Command overlay to be open"),
    }
}

// ---------------------------------------------------------------------------
// Up/Down recall + draft preservation
// ---------------------------------------------------------------------------

#[test]
fn up_recalls_last_command() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "yt setup");
    // Open a fresh command overlay and press Up — the last command should
    // appear in the input.
    open_command(&mut app);
    handle_key(&mut app, key_code(KeyCode::Up));
    assert_eq!(command_input(&app), "yt setup");
}

#[test]
fn up_traverses_multiple_commands_in_order() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "first");
    run_command(&mut app, "second");
    open_command(&mut app);
    // First Up → most recent ("second").
    handle_key(&mut app, key_code(KeyCode::Up));
    assert_eq!(command_input(&app), "second");
    // Second Up → older ("first").
    handle_key(&mut app, key_code(KeyCode::Up));
    assert_eq!(command_input(&app), "first");
}

#[test]
fn down_after_up_restores_draft() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "yt setup");
    // Open a new command, type a partial (unsaved draft), then Up to recall.
    open_command(&mut app);
    type_text(&mut app, "yt lo");
    handle_key(&mut app, key_code(KeyCode::Up));
    assert_eq!(command_input(&app), "yt setup");
    // Down past the end → the draft "yt lo" is restored.
    handle_key(&mut app, key_code(KeyCode::Down));
    assert_eq!(command_input(&app), "yt lo");
}

#[test]
fn down_at_bottom_stays_at_draft() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "yt setup");
    open_command(&mut app);
    type_text(&mut app, "half");
    // Up then Down → draft restored. Down again → still draft (no clamping
    // past the bottom).
    handle_key(&mut app, key_code(KeyCode::Up));
    handle_key(&mut app, key_code(KeyCode::Down));
    assert_eq!(command_input(&app), "half");
    handle_key(&mut app, key_code(KeyCode::Down));
    assert_eq!(command_input(&app), "half");
}

// ---------------------------------------------------------------------------
// Dedup adjacent
// ---------------------------------------------------------------------------

#[test]
fn dedup_adjacent_commands() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "yt setup");
    run_command(&mut app, "yt setup"); // adjacent duplicate
    assert_eq!(
        app.command_history,
        vec!["yt setup".to_string()],
        "adjacent duplicate must be deduped"
    );
}

#[test]
fn dedup_only_adjacent_not_all_duplicates() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "foo");
    run_command(&mut app, "bar");
    run_command(&mut app, "foo"); // non-adjacent duplicate — kept
    assert_eq!(
        app.command_history,
        vec!["foo".to_string(), "bar".to_string(), "foo".to_string()],
        "non-adjacent duplicates are kept (only adjacent dedup)"
    );
}

// ---------------------------------------------------------------------------
// Bound at 100
// ---------------------------------------------------------------------------

#[test]
fn history_bounded_at_100() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Submit 105 distinct commands.
    for i in 0..105u32 {
        run_command(&mut app, &format!("cmd{i}"));
    }
    assert_eq!(
        app.command_history.len(),
        100,
        "history must be bounded at 100 entries"
    );
    // Most recent 100 are kept: cmd104 (newest) down to cmd5 (oldest).
    assert_eq!(app.command_history[0], "cmd104");
    assert_eq!(app.command_history[99], "cmd5");
}

// ---------------------------------------------------------------------------
// Unicode
// ---------------------------------------------------------------------------

#[test]
fn unicode_command_recalled() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    run_command(&mut app, "検索");
    open_command(&mut app);
    handle_key(&mut app, key_code(KeyCode::Up));
    assert_eq!(command_input(&app), "検索");
}

// ---------------------------------------------------------------------------
// :q and :quit
// ---------------------------------------------------------------------------

#[test]
fn q_command_quits() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(!app.should_quit);
    run_command(&mut app, "q");
    assert!(app.should_quit, ":q must set should_quit");
}

#[test]
fn quit_command_quits() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(!app.should_quit);
    run_command(&mut app, "quit");
    assert!(app.should_quit, ":quit must set should_quit");
}

// ---------------------------------------------------------------------------
// Persistence (state.db round-trip)
// ---------------------------------------------------------------------------

#[test]
fn persistence_round_trip() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let history = vec![
        "yt setup".to_string(),
        "yt auth".to_string(),
        "検索".to_string(),
        "q".to_string(),
    ];
    state::save_command_history_at(&path, &history).unwrap();
    let loaded = state::load_command_history_at(&path).unwrap();
    assert_eq!(loaded, history);
}

#[test]
fn persistence_empty_db_returns_empty() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let loaded = state::load_command_history_at(&path).unwrap();
    assert!(loaded.is_empty(), "empty DB must return empty history");
}

#[test]
fn persistence_overwrite() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    state::save_command_history_at(&path, &["old".to_string()]).unwrap();
    state::save_command_history_at(&path, &["new".to_string()]).unwrap();
    let loaded = state::load_command_history_at(&path).unwrap();
    assert_eq!(loaded, vec!["new".to_string()]);
}
