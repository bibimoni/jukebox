//! Extended persistence tests for `state.rs`: layout, volume, shuffle/repeat
//! modes, and playlists. Mirrors the brief's Step 1, with shuffle/repeat
//! asserted as the persisted string form (`"off"`/`"smart"`/`"random"`,
//! `"off"`/`"all"`/`"one"`) since `LayoutState` maps the enum modes to strings
//! (no serde derives on `ShuffleMode`/`RepeatMode`).

use jukebox::state::*;
use jukebox::tui::app::{ColumnWidths, Playlist};
use jukebox::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};

#[test]
fn layout_round_trips() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(loaded.volume, 70); // default
    assert_eq!(loaded.shuffle, "off");
    assert_eq!(loaded.repeat, "off");
    assert_eq!(loaded.continue_mode, "off");
    let widths = ColumnWidths {
        rail: 5,
        col1: 30,
        col2: 30,
        col3: 40,
    };
    save_layout_at(
        &path,
        &LayoutSave {
            focus: "playlists",
            widths: &widths,
            volume: 42,
            shuffle: ShuffleMode::Smart,
            repeat: RepeatMode::One,
            continue_mode: ContinueMode::Radio,
            source_mode: jukebox::mode::SourceMode::Mixed,
            yt_browser: "chrome",
            last_played_track_id: Some("t9"),
            last_played_position: 12.0,
            last_cursor_artist: 3,
            last_cursor_album: 1,
            last_cursor_track: 2,
            last_cursor_playlist: 0,
        },
    )
    .unwrap();
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(loaded.focus, "playlists");
    assert_eq!(loaded.widths.col1, 30);
    assert_eq!(loaded.volume, 42);
    assert_eq!(loaded.shuffle, "smart");
    assert_eq!(loaded.repeat, "one");
    assert_eq!(loaded.continue_mode, "radio");
    assert_eq!(loaded.source_mode, "mixed");
    assert_eq!(loaded.yt_browser, "chrome");
    // RC11-DEF-014: last-played track + position + cursors round-trip.
    assert_eq!(loaded.last_played_track_id.as_deref(), Some("t9"));
    assert!((loaded.last_played_position - 12.0).abs() < f64::EPSILON);
    assert_eq!(loaded.last_cursor_artist, 3);
    assert_eq!(loaded.last_cursor_album, 1);
    assert_eq!(loaded.last_cursor_track, 2);
    assert_eq!(loaded.last_cursor_playlist, 0);
}

#[test]
fn playlists_round_trip() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let pls = vec![
        Playlist {
            name: "Faves".into(),
            track_ids: vec!["a".into(), "b".into()],
        },
        Playlist {
            name: "Night".into(),
            track_ids: vec!["c".into()],
        },
    ];
    save_playlists_at(&path, &pls).unwrap();
    let loaded = load_playlists_at(&path).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].name, "Faves");
    assert_eq!(loaded[0].track_ids, vec!["a".to_string(), "b".to_string()]);
}
