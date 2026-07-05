//! Extended persistence tests for `state.rs`: layout, volume, shuffle/repeat
//! modes, and playlists. Mirrors the brief's Step 1, with shuffle/repeat
//! asserted as the persisted string form (`"off"`/`"smart"`/`"random"`,
//! `"off"`/`"all"`/`"one"`) since `LayoutState` maps the enum modes to strings
//! (no serde derives on `ShuffleMode`/`RepeatMode`).

use jukebox::state::*;
use jukebox::tui::app::{ColumnWidths, Playlist};
use jukebox::tui::queue::{RepeatMode, ShuffleMode};

#[test]
fn layout_round_trips() {
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(loaded.volume, 70); // default
    assert_eq!(loaded.shuffle, "off");
    assert_eq!(loaded.repeat, "off");
    let widths = ColumnWidths {
        rail: 5,
        col1: 30,
        col2: 30,
        col3: 40,
    };
    save_layout_at(
        &path,
        "playlists",
        &widths,
        42,
        ShuffleMode::Smart,
        RepeatMode::One,
    )
    .unwrap();
    let loaded = load_layout_at(&path).unwrap();
    assert_eq!(loaded.focus, "playlists");
    assert_eq!(loaded.widths.col1, 30);
    assert_eq!(loaded.volume, 42);
    assert_eq!(loaded.shuffle, "smart");
    assert_eq!(loaded.repeat, "one");
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
    assert_eq!(
        loaded[0].track_ids,
        vec!["a".to_string(), "b".to_string()]
    );
}
