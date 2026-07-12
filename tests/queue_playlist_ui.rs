//! Queue & playlist UI wiring tests.
//!
//! These tests verify that the `e` (enqueue), `x` (remove from queue),
//! `:queue clear` (clear queue), `a` (add to playlist / create playlist),
//! and `d` (delete playlist) key/command bindings actually call the
//! `Transport` and `App` methods they're supposed to — and that playlist
//! mutations persist to the state DB.
//!
//! All tests isolate `XDG_CONFIG_HOME` so `save_playlists_db` writes to a
//! temp directory instead of the user's real config dir.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::queue::Transport;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A 3-track catalog under one artist/album, with real on-disk source files
/// (so `play_selected`'s `std::fs::metadata` check passes and playback starts).
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

/// Isolate `XDG_CONFIG_HOME` so `save_playlists_db` writes to a temp dir.
/// Returns the temp dir path (keep it alive for the test's duration).
fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-qpui-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

/// Focus the track column (col 2) and place the cursor on track 0 of the album.
fn focus_track_col(app: &mut App) {
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0; // 40mP
    app.cursors.album = 0; // Cosmic
    app.cursors.track = 0; // Song1 (t1)
}

/// Open the `:` command overlay, type `text`, and press Enter.
fn open_command(app: &mut App, text: &str) {
    handle_key(app, key(':'));
    for c in text.chars() {
        handle_key(app, key(c));
    }
    handle_key(app, key_code(KeyCode::Enter));
}

// ---------------------------------------------------------------------------
// Enqueue (e key)
// ---------------------------------------------------------------------------

#[test]
fn e_enqueues_selected_track_to_manual_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    assert!(app.transport.manual_queue.is_empty());
    handle_key(&mut app, key('e'));
    assert_eq!(app.transport.manual_queue, vec!["t1".to_string()]);
}

#[test]
fn e_enqueues_different_tracks_as_cursor_moves() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    // Cursor on track 0 (t1) → enqueue t1.
    handle_key(&mut app, key('e'));
    // Move down to track 1 (t2) → enqueue t2.
    handle_key(&mut app, key_code(KeyCode::Down));
    handle_key(&mut app, key('e'));
    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t2".to_string()]
    );
}

// ---------------------------------------------------------------------------
// Remove from queue (x key)
// ---------------------------------------------------------------------------

#[test]
fn x_removes_focused_track_from_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Enqueue two tracks.
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    // Switch to Queue view, cursor on row 0.
    app.view = View::Queue;
    app.cursors.queue = 0;
    handle_key(&mut app, key('x'));
    assert_eq!(app.transport.manual_queue, vec!["t2".to_string()]);
}

#[test]
fn x_removes_second_track_from_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    app.transport.enqueue("t3".into());
    app.view = View::Queue;
    app.cursors.queue = 1; // t2
    handle_key(&mut app, key('x'));
    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t3".to_string()]
    );
}

#[test]
fn x_noop_outside_queue_view() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.view = View::Artists;
    handle_key(&mut app, key('x'));
    assert_eq!(app.transport.manual_queue, vec!["t1".to_string()]);
}

// ---------------------------------------------------------------------------
// Clear queue (:queue clear command)
// ---------------------------------------------------------------------------

#[test]
fn queue_clear_command_empties_manual_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    assert_eq!(app.transport.manual_queue.len(), 2);
    open_command(&mut app, "queue clear");
    assert!(app.transport.manual_queue.is_empty());
}

#[test]
fn queue_clear_sets_status_message() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    open_command(&mut app, "queue clear");
    assert_eq!(app.yt_status.as_deref(), Some("queue cleared"));
}

// ---------------------------------------------------------------------------
// Add to playlist (a key + PlaylistPicker overlay)
// ---------------------------------------------------------------------------

#[test]
fn a_opens_playlist_picker_with_selected_track() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    match &app.overlay {
        Some(Overlay::PlaylistPicker { track_id, cursor }) => {
            assert_eq!(track_id, "t1");
            assert_eq!(*cursor, 0);
        }
        _ => panic!("expected PlaylistPicker overlay"),
    }
}

#[test]
fn a_enter_on_new_playlist_creates_playlist_with_track() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    // No existing playlists → the only entry is "+ new playlist..." at cursor 0.
    handle_key(&mut app, key_code(KeyCode::Enter));
    // DEF-014: Enter on "+ new playlist..." opens a text input for the name.
    assert!(
        matches!(app.overlay, Some(Overlay::TextInput { .. })),
        "Enter on '+ new playlist...' should open a text input overlay"
    );
    // Type a custom name.
    handle_key(&mut app, key('M'));
    handle_key(&mut app, key('y'));
    handle_key(&mut app, key(' '));
    handle_key(&mut app, key('J'));
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key('m'));
    handle_key(&mut app, key('s'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(app.overlay.is_none(), "Enter should close the text input");
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].track_ids, vec!["t1".to_string()]);
    assert_eq!(
        app.playlists[0].name, "My Jams",
        "playlist should use the typed name"
    );
}

#[test]
fn a_enter_on_existing_playlist_adds_track() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Pre-create a playlist with one track.
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t2".into()],
    });
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    // Cursor 0 = "Faves" (first entry). Enter adds t1 to it.
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(
        app.playlists[0].track_ids,
        vec!["t2".to_string(), "t1".to_string()]
    );
}

#[test]
fn a_arrow_down_then_enter_creates_new_playlist() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t2".into()],
    });
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    // Down to "+ new playlist..." (cursor 1).
    handle_key(&mut app, key_code(KeyCode::Down));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // DEF-014: text input opens. Type a name and confirm.
    assert!(
        matches!(app.overlay, Some(Overlay::TextInput { .. })),
        "should open text input for playlist name"
    );
    handle_key(&mut app, key('C'));
    handle_key(&mut app, key('h'));
    handle_key(&mut app, key('i'));
    handle_key(&mut app, key('l'));
    handle_key(&mut app, key('l'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(app.playlists.len(), 2);
    assert_eq!(app.playlists[1].track_ids, vec!["t1".to_string()]);
    assert_eq!(app.playlists[1].name, "Chill");
}

#[test]
fn a_esc_cancels_without_adding() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none());
    assert!(app.playlists.is_empty(), "Esc should not create a playlist");
}

#[test]
fn a_add_track_to_playlist_skips_duplicates() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // t1 is already in the playlist → should not be added twice.
    assert_eq!(app.playlists[0].track_ids, vec!["t1".to_string()]);
}

// ---------------------------------------------------------------------------
// Delete playlist (d key)
// ---------------------------------------------------------------------------

#[test]
fn d_deletes_focused_playlist_in_playlists_view() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.playlists.push(Playlist {
        name: "Night".into(),
        track_ids: vec!["t2".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0; // "Faves"
                              // DEF-001: `d` opens a confirmation dialog, not immediate deletion.
    handle_key(&mut app, key('d'));
    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "`d` should open a confirmation dialog"
    );
    assert_eq!(app.playlists.len(), 2, "no deletion until confirmed");
    // Confirm.
    handle_key(&mut app, key('y'));
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Night");
}

#[test]
fn d_noop_outside_playlists_view() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Artists;
    app.focus_col = 0;
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        1,
        "d should be a no-op outside Playlists view"
    );
}

#[test]
fn d_sets_status_message() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;
    // DEF-001: `d` opens confirmation; status is set only after confirming.
    handle_key(&mut app, key('d'));
    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "should open confirmation dialog"
    );
    handle_key(&mut app, key('y'));
    assert!(
        app.yt_status
            .as_deref()
            .unwrap_or("")
            .contains("deleted playlist"),
        "should set a 'deleted playlist' status, got {:?}",
        app.yt_status
    );
}

// ---------------------------------------------------------------------------
// Playlist persistence (save → load)
// ---------------------------------------------------------------------------

#[test]
fn playlists_persist_save_then_load() {
    // Use an explicit temp DB path (avoids the XDG_CONFIG_HOME env race
    // between parallel tests — the same pattern as tests/state_ext.rs).
    let path = tempfile::tempdir().unwrap().keep().join("state.db");
    use jukebox::tui::app::Playlist;
    let pls = vec![
        Playlist {
            name: "Faves".into(),
            track_ids: vec!["t1".into(), "t2".into()],
        },
        Playlist {
            name: "Night".into(),
            track_ids: vec!["t3".into()],
        },
    ];
    jukebox::state::save_playlists_at(&path, &pls).unwrap();
    let loaded = jukebox::state::load_playlists_at(&path).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].name, "Faves");
    assert_eq!(
        loaded[0].track_ids,
        vec!["t1".to_string(), "t2".to_string()]
    );
    assert_eq!(loaded[1].name, "Night");
    assert_eq!(loaded[1].track_ids, vec!["t3".to_string()]);
}

#[test]
fn delete_playlist_removes_from_app_state() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    use jukebox::tui::app::Playlist;
    app.playlists.push(Playlist {
        name: "Faves".into(),
        track_ids: vec!["t1".into()],
    });
    app.playlists.push(Playlist {
        name: "Night".into(),
        track_ids: vec!["t2".into()],
    });
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0; // "Faves"
                              // DEF-001: confirm before deletion.
    handle_key(&mut app, key('d'));
    handle_key(&mut app, key('y'));
    // Verify the in-memory state: only "Night" remains.
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Night");
    // The cursor should be valid (0, pointing at "Night" which shifted up).
    assert_eq!(app.cursors.playlist, 0);
}

#[test]
fn add_to_playlist_updates_app_state() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // DEF-014: text input opens. Type a name and confirm.
    assert!(
        matches!(app.overlay, Some(Overlay::TextInput { .. })),
        "should open text input for playlist name"
    );
    handle_key(&mut app, key('V'));
    handle_key(&mut app, key('i'));
    handle_key(&mut app, key('b'));
    handle_key(&mut app, key('e'));
    handle_key(&mut app, key('s'));
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Verify the in-memory state: one playlist with t1.
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].track_ids, vec!["t1".to_string()]);
    assert_eq!(app.playlists[0].name, "Vibes");
}

// ---------------------------------------------------------------------------
// Transport-level sanity (verify the underlying methods work)
// ---------------------------------------------------------------------------

#[test]
fn transport_enqueue_adds_to_manual_queue() {
    let mut t = Transport::new(jukebox::tui::context::Context::Queue);
    t.enqueue("t1".into());
    t.enqueue("t2".into());
    assert_eq!(t.manual_queue, vec!["t1".to_string(), "t2".to_string()]);
}

#[test]
fn transport_remove_from_queue_removes_matching_id() {
    let mut t = Transport::new(jukebox::tui::context::Context::Queue);
    t.enqueue("t1".into());
    t.enqueue("t2".into());
    t.enqueue("t3".into());
    t.remove_from_queue("t2");
    assert_eq!(t.manual_queue, vec!["t1".to_string(), "t3".to_string()]);
}

#[test]
fn transport_clear_queue_empties() {
    let mut t = Transport::new(jukebox::tui::context::Context::Queue);
    t.enqueue("t1".into());
    t.enqueue("t2".into());
    t.clear_queue();
    assert!(t.manual_queue.is_empty());
}
