//! No-destructive-single-key audit (AC-M6.4.3).
//!
//! Verifies that no single key causes accidental irreversible data loss:
//! - `d` (delete playlist) is context-gated — only fires in the Playlists view
//!   with focus on column 0, so browsing artists and pressing `d` is a no-op.
//! - `x` (remove from queue) is context-gated — only fires in the Queue view.
//! - `q` (quit) does not wipe playlist data — playlists survive a quit keypress.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Playlist, View};
use jukebox::tui::input::handle_key;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn build_app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1",
           "album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,
           "source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);

    // Populate with 2 playlists so we can verify `d` doesn't accidentally
    // delete them when pressed outside the Playlists view.
    app.playlists.push(Playlist {
        name: "My Playlist".into(),
        track_ids: vec!["t1".into()],
    });
    app.playlists.push(Playlist {
        name: "Another".into(),
        track_ids: vec![],
    });

    // Put something in the queue so we can verify `x` is gated.
    app.transport.enqueue("t1".into());

    (d, app)
}

// ---------------------------------------------------------------------------
// `d` — delete focused playlist: context-gated to Playlists view, column 0
// ---------------------------------------------------------------------------

#[test]
fn d_does_not_delete_playlist_in_artists_view() {
    let (_d, mut app) = build_app();
    app.view = View::Artists;
    let before = app.playlists.len();
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        before,
        "`d` in Artists view must not delete a playlist"
    );
}

#[test]
fn d_does_not_delete_playlist_in_queue_view() {
    let (_d, mut app) = build_app();
    app.view = View::Queue;
    let before = app.playlists.len();
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        before,
        "`d` in Queue view must not delete a playlist"
    );
}

#[test]
fn d_does_not_delete_playlist_in_youtube_view() {
    let (_d, mut app) = build_app();
    app.view = View::Youtube;
    let before = app.playlists.len();
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        before,
        "`d` in Youtube view must not delete a playlist"
    );
}

#[test]
fn d_does_not_delete_when_focus_col_not_zero() {
    let (_d, mut app) = build_app();
    app.view = View::Playlists;
    app.focus_col = 1; // focused on the tracks column, not the playlist list
    let before = app.playlists.len();
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        before,
        "`d` in Playlists view with focus_col=1 must not delete a playlist"
    );
}

#[test]
fn d_deletes_in_playlists_view_col0() {
    let (_d, mut app) = build_app();
    app.view = View::Playlists;
    app.focus_col = 0;
    app.cursors.playlist = 0;
    let before = app.playlists.len();
    handle_key(&mut app, key('d'));
    assert_eq!(
        app.playlists.len(),
        before - 1,
        "`d` in Playlists view col 0 should delete the focused playlist"
    );
}

// ---------------------------------------------------------------------------
// `x` — remove from queue: context-gated to Queue view
// ---------------------------------------------------------------------------

#[test]
fn x_does_not_remove_from_queue_in_artists_view() {
    let (_d, mut app) = build_app();
    app.view = View::Artists;
    let before = app.transport.manual_queue.len();
    handle_key(&mut app, key('x'));
    assert_eq!(
        app.transport.manual_queue.len(),
        before,
        "`x` in Artists view must not remove from the queue"
    );
}

#[test]
fn x_does_not_remove_from_queue_in_playlists_view() {
    let (_d, mut app) = build_app();
    app.view = View::Playlists;
    let before = app.transport.manual_queue.len();
    handle_key(&mut app, key('x'));
    assert_eq!(
        app.transport.manual_queue.len(),
        before,
        "`x` in Playlists view must not remove from the queue"
    );
}

#[test]
fn x_removes_from_queue_in_queue_view() {
    let (_d, mut app) = build_app();
    app.view = View::Queue;
    app.cursors.queue = 0;
    let before = app.transport.manual_queue.len();
    handle_key(&mut app, key('x'));
    assert_eq!(
        app.transport.manual_queue.len(),
        before - 1,
        "`x` in Queue view should remove the selected item"
    );
}

// ---------------------------------------------------------------------------
// `q` — quit: does not wipe data
// ---------------------------------------------------------------------------

#[test]
fn q_does_not_wipe_playlists() {
    let (_d, mut app) = build_app();
    let before = app.playlists.len();
    handle_key(&mut app, key('q'));
    assert!(app.should_quit, "`q` should set should_quit");
    assert_eq!(
        app.playlists.len(),
        before,
        "`q` must not wipe playlists — data survives quit"
    );
}

#[test]
fn q_does_not_wipe_queue() {
    let (_d, mut app) = build_app();
    let before = app.transport.manual_queue.len();
    handle_key(&mut app, key('q'));
    assert_eq!(
        app.transport.manual_queue.len(),
        before,
        "`q` must not wipe the queue"
    );
}

// ---------------------------------------------------------------------------
// Summary audit: enumerate all destructive single-key handlers and verify
// each is properly gated.
// ---------------------------------------------------------------------------

#[test]
fn no_destructive_single_key() {
    // Audit: every single-char key that modifies persistent or semi-persistent
    // state (playlists, queue) must be context-gated so it cannot fire
    // accidentally from a different view.
    //
    // Known destructive keys:
    //   `d` → delete_focused_playlist — gated by View::Playlists + focus_col==0
    //   `x` → remove_selected_from_queue — gated by View::Queue
    //
    // Non-destructive keys (safe by design):
    //   `q` → quit (sets should_quit; data saved on exit by main.rs)
    //   `e` → enqueue (adds to queue; not destructive)
    //   `s` → instant_random (replaces context; not data-destructive)
    //
    // This test verifies the gating by pressing each destructive key from a
    // non-matching view and asserting no state change.

    let (_d, mut app) = build_app();

    // Start in Artists view — none of the destructive keys should fire.
    app.view = View::Artists;
    let playlists_before = app.playlists.len();
    let queue_before = app.transport.manual_queue.len();

    // Press every lowercase letter that has a handler in input.rs.
    for c in [
        'a', 'c', 'd', 'e', 'f', 'g', 'h', 'j', 'k', 'l', 'm', 'r', 's', 'x', 'z',
    ] {
        handle_key(&mut app, key(c));
    }

    // Playlists must be untouched (d is gated to Playlists view).
    assert_eq!(
        app.playlists.len(),
        playlists_before,
        "no key in Artists view should delete a playlist"
    );
    // Queue must be untouched (x is gated to Queue view; e adds but we started
    // with 1 item and `e` in Artists view enqueues the selected track — that's
    // additive, not destructive, so we only assert playlists are unchanged and
    // queue is not shrunk below its original count).
    assert!(
        app.transport.manual_queue.len() >= queue_before,
        "no key in Artists view should remove from the queue"
    );
}
