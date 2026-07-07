//! Integration-level tests for the new context-play `App`.
//!
//! These exercise the whole `App` (catalog + player + transport wiring) end to
//! end, complementing the unit-level `tests/transport.rs` (engine only) and
//! `tests/app.rs` (single-artist album view). Focus here: multi-artist catalog
//! indexing, `play_in_context_ids` + `next`/`prev` round-trips through a Search
//! context, auto-advance via `on_track_ended`, and all-dead termination.

use jukebox::catalog::Catalog;
use jukebox::player::Player;
use jukebox::tui::app::App;
use jukebox::tui::context::Context;
use jukebox::tui::queue::ShuffleMode;

/// A catalog with two artists (Ado, Aimer), one track each, with real on-disk
/// source files so playback actually loads instead of marking tracks dead.
fn two_artist_catalog() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("a")).unwrap();
    std::fs::create_dir_all(lossless.join("b")).unwrap();
    std::fs::write(lossless.join("a/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("b/01.flac"), b"x").unwrap();
    // NOTE: `Track::resolve_source` joins `source_root.parent()` with
    // `source_path`, so source_root is the lossless dir and source_path keeps
    // its "lossless/..." prefix (resolved under the tempdir, lossless's parent).
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"]},
          {"id":"t2","artists":["Aimer"],"primary_artist":"Aimer","title":"Brave",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Aimer"]},
        ]
    }).to_string();
    let p = root.join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

#[test]
fn app_builds_artist_index() {
    let (_d, cat) = two_artist_catalog();
    let app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    // The artist index is keyed by symlinked_into_artists and surfaced as a
    // sorted Vec<String>; both artists must appear.
    assert!(app.artists.iter().any(|a| a == "Ado"));
    assert!(app.artists.iter().any(|a| a == "Aimer"));
    assert!(!app.artists.is_empty());
    // The index maps each artist to its track indices.
    let ado_tracks = app.artist_index.get("Ado").unwrap();
    assert_eq!(ado_tracks.len(), 1);
    assert_eq!(app.catalog.tracks[ado_tracks[0]].id, "t1");
}

#[test]
fn play_in_context_ids_sets_now_playing_and_context() {
    let (_d, cat) = two_artist_catalog();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    // Play t1 from a Search context spanning both tracks.
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    assert!(matches!(app.transport.context, Context::Search { .. }));

    // next advances to t2 within the same context.
    app.next();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));

    // prev walks back to t1.
    app.prev();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
}

/// A stub player whose `track_ended()` returns true after `end_after` loads,
/// simulating a track that finishes on its own. Lets us test auto-advance
/// without a real audio backend.
#[derive(Default)]
struct EndAfterN {
    loads: u32,
    end_after: u32,
    ended_fired: bool,
}
impl Player for EndAfterN {
    fn load(&mut self, _path: &std::path::Path) -> anyhow::Result<()> {
        self.loads += 1;
        self.ended_fired = false;
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> { Ok(()) }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> { Ok(()) }
    fn stop(&mut self) -> anyhow::Result<()> { Ok(()) }
    fn position(&self) -> Option<f64> { None }
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool { true }
    fn track_ended(&mut self) -> bool {
        if !self.ended_fired && self.loads >= self.end_after {
            self.ended_fired = true;
            return true;
        }
        false
    }
}

#[test]
fn on_track_ended_auto_advances() {
    let (_d, cat) = two_artist_catalog();
    let player: Box<dyn Player> = Box::new(EndAfterN { end_after: 1, ..Default::default() });
    let mut app = App::new(cat, player, None, None);

    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));

    // The TUI loop polls the player for end-of-track; when it fires, App
    // auto-advances via on_track_ended (which delegates to next).
    assert!(app.player.track_ended(), "stub should report end after first load");
    app.on_track_ended();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));
}

#[test]
fn all_dead_context_terminates_without_looping() {
    // Every track points at a nonexistent file: start_playback must mark each
    // dead and return promptly without infinite-looping, leaving the player
    // unloaded and now_playing cleared.
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"d1","artists":["X"],"primary_artist":"X","title":"G1",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/a/01.flac","symlinked_into_artists":["X"]},
          {"id":"d2","artists":["Y"],"primary_artist":"Y","title":"G2",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Y"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["d1".into(), "d2".into()], "d1");

    // Both marked dead; nothing loaded.
    assert!(app.dead.contains("d1"));
    assert!(app.dead.contains("d2"));
    assert!(app.now_playing.is_none(), "all-dead context must not leave a track playing");
}

#[test]
fn cycle_shuffle_via_app_persists_into_transport() {
    // Integration: App.cycle_shuffle drives Transport.set_shuffle, so the mode
    // change survives and the play order is rebuilt for the live context.
    let (_d, cat) = two_artist_catalog();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);

    app.cycle_shuffle();
    assert_eq!(app.transport.shuffle, ShuffleMode::Smart);
    // Order rebuilt to span the context (still 2 entries).
    assert_eq!(app.transport.order.len(), 2);

    app.cycle_shuffle();
    assert_eq!(app.transport.shuffle, ShuffleMode::Random);
    app.cycle_shuffle();
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);
}
