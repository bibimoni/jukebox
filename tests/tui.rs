use jukebox::catalog::Catalog;
use jukebox::search::Searcher;
use jukebox::tui::App;

fn mini_catalog_json() -> String {
    serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"]},
          {"id":"t2","artists":["Aimer"],"primary_artist":"Aimer","title":"Brave",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Aimer"]},
        ]
    }).to_string()
}

#[test]
fn app_builds_artist_index() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
    let artists = app.artists();
    assert!(artists.iter().any(|a| a == "Ado"));
    assert!(artists.iter().any(|a| a == "Aimer"));
}

#[test]
fn enqueue_artist_adds_their_tracks() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
    app.enqueue_artist("Ado");
    assert_eq!(app.queue().len(), 1);
}

fn build_catalog_and_index() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let idx = d.path().join("search-index");
    jukebox::search::build_index(&cat, &idx).unwrap();
    (d, cat)
}

#[test]
fn search_populates_results() {
    let (_d, _cat) = build_catalog_and_index();
    let s = Searcher::open(&_d.path().join("search-index")).unwrap();
    let hits = s.search("Freedom", 10).unwrap();
    assert!(!hits.is_empty());
}

#[test]
fn enqueue_results_then_next_advances() {
    let (_d, cat) = build_catalog_and_index();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
    app.queue.enqueue("t1".into());
    app.queue.enqueue("t2".into());
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("t1".to_string()));
    app.queue.next();
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("t2".to_string()));
}

#[test]
fn dead_source_marks_track_and_advances() {
    // source_root lives inside a tempdir; t1 points at a nonexistent file
    // (dead), t2 points at a real file we create. Playing should mark t1 dead
    // and advance the queue to t2 without loading t1.
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("b")).unwrap();
    // real file for t2
    std::fs::write(lossless.join("b/01.flac"), b"not really flac").unwrap();

    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"dead1","artists":["X"],"primary_artist":"X","title":"Gone",
           "bit_depth":16,"sample_rate_hz":44100,
           "source_path":"lossless/a/01.flac","symlinked_into_artists":["X"]},
          {"id":"alive2","artists":["Y"],"primary_artist":"Y","title":"Here",
           "bit_depth":16,"sample_rate_hz":44100,
           "source_path":"lossless/b/01.flac","symlinked_into_artists":["Y"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
    app.queue.enqueue("dead1".into());
    app.queue.enqueue("alive2".into());
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("dead1".to_string()));

    app.play_current_queue();

    // t1 marked dead
    assert!(app.dead.contains("dead1"), "dead1 should be marked dead");
    // queue advanced past dead1 to alive2
    assert_eq!(app.queue.current().map(|s| s.clone()), Some("alive2".to_string()));
}

#[test]
fn all_dead_queue_does_not_loop_forever() {
    // Every track is dead: play_current_queue must terminate (not recurse) and
    // leave the player unloaded.
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"d1","artists":["X"],"primary_artist":"X","title":"G1",
           "bit_depth":16,"sample_rate_hz":44100,
           "source_path":"lossless/a/01.flac","symlinked_into_artists":["X"]},
          {"id":"d2","artists":["Y"],"primary_artist":"Y","title":"G2",
           "bit_depth":16,"sample_rate_hz":44100,
           "source_path":"lossless/b/01.flac","symlinked_into_artists":["Y"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);
    app.queue.enqueue("d1".into());
    app.queue.enqueue("d2".into());

    // Should return promptly without infinite-looping.
    app.play_current_queue();

    assert!(app.dead.contains("d1"));
    assert!(app.dead.contains("d2"));
}

// --- auto-next + space-in-search tests ---

#[test]
fn enter_on_artist_browses_their_songs_without_enqueueing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    // Two artists, Ado with two tracks so we can verify browse lists both
    // and sorts by title, and that nothing is enqueued.
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"a1","artists":["Ado"],"primary_artist":"Ado","title":"Zebra",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"]},
          {"id":"a2","artists":["Ado"],"primary_artist":"Ado","title":"Alpha",
           "bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/a/02.flac","symlinked_into_artists":["Ado"]},
          {"id":"b1","artists":["Aimer"],"primary_artist":"Aimer","title":"Brave",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["Aimer"]},
        ]
    }).to_string();
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None);

    // Cursor on Ado (sorted: Ado, Aimer -> Ado is index 0).
    app.artist_cursor = 0;
    app.browse_artist();

    // Results are exactly Ado's two tracks, nothing from Aimer, queue empty.
    assert_eq!(app.results.len(), 2, "browse should list Ado's tracks only");
    let ids: Vec<String> = app.results.iter()
        .map(|(_, i)| app.catalog.tracks[*i].id.clone()).collect();
    assert!(ids.contains(&"a1".into()) && ids.contains(&"a2".into()));
    assert!(!ids.contains(&"b1".into()));
    assert_eq!(app.queue().len(), 0, "browse must not enqueue anything");
    assert!(matches!(app.focus, jukebox::tui::Pane::Search));

    // Sorted by title: Alpha (a2) before Zebra (a1).
    assert_eq!(app.results[0].1, 1, "Alpha (index 1) should sort first");
    assert_eq!(app.results[1].1, 0, "Zebra (index 0) should sort second");
}

/// A stub player whose `track_ended()` returns true after `end_after` loads,
/// simulating a track that finishes on its own. Lets us test auto-advance
/// without a real mpv.
use jukebox::player::Player;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct EndAfterN {
    loads: u32,
    end_after: u32,        // fire track_ended once after this many loads
    ended_fired: bool,
    loaded: Option<PathBuf>,
}
impl Player for EndAfterN {
    fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        self.loads += 1;
        self.ended_fired = false;
        self.loaded = Some(path.to_path_buf());
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> { Ok(()) }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> { Ok(()) }
    fn stop(&mut self) -> anyhow::Result<()> { Ok(()) }
    fn position(&self) -> Option<f64> { None }
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool { self.loaded.is_some() }
    fn track_ended(&mut self) -> bool {
        if !self.ended_fired && self.loads >= self.end_after {
            self.ended_fired = true;
            return true;
        }
        false
    }
}

#[test]
fn auto_next_advances_queue_on_track_end() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    // Player signals end after the first load.
    let player: Box<dyn Player> = Box::new(EndAfterN { end_after: 1, ..Default::default() });
    let mut app = App::new(cat, player, None);

    // Need real source files for play_current_queue to load them.
    std::fs::create_dir_all("/tmp/lossless/a").unwrap();
    std::fs::create_dir_all("/tmp/lossless/b").unwrap();
    std::fs::write("/tmp/lossless/a/01.flac", b"x").unwrap();
    std::fs::write("/tmp/lossless/b/01.flac", b"x").unwrap();

    app.queue.enqueue("t1".into());
    app.queue.enqueue("t2".into());
    app.play_current_queue();
    assert_eq!(app.queue.current().cloned(), Some("t1".to_string()));

    // Simulate the TUI loop detecting end-of-track: advance + play next.
    let ended = app.player.track_ended();
    assert!(ended, "track should report ended after first load");
    if ended {
        app.queue.next();
        app.play_current_queue();
    }
    assert_eq!(app.queue.current().cloned(), Some("t2".to_string()));
    assert_eq!(app.now_playing.as_deref(), Some("t2"));

    // cleanup
    let _ = std::fs::remove_dir_all("/tmp/lossless");
}

#[test]
fn consume_current_drops_finished_track_and_advances_to_next() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let player: Box<dyn Player> = Box::new(jukebox::player::StubPlayer::default());
    let mut app = App::new(cat, player, None);

    // Create real source files so play_current_queue can load them.
    std::fs::create_dir_all("/tmp/lossless/a").unwrap();
    std::fs::create_dir_all("/tmp/lossless/b").unwrap();
    std::fs::write("/tmp/lossless/a/01.flac", b"x").unwrap();
    std::fs::write("/tmp/lossless/b/01.flac", b"x").unwrap();

    app.queue.enqueue("t1".into());
    app.queue.enqueue("t2".into());
    app.play_current_queue();
    assert_eq!(app.queue.current().cloned(), Some("t1".to_string()));
    assert_eq!(app.queue.len(), 2);

    // peek_next reports t2 before consume.
    assert_eq!(app.queue.peek_next().cloned(), Some("t2".to_string()));

    // Consume the finished track (simulate track-end).
    app.queue.consume_current();
    // t1 removed, t2 is now current, queue length is 1.
    assert_eq!(app.queue.len(), 1);
    assert_eq!(app.queue.current().cloned(), Some("t2".to_string()));

    let _ = std::fs::remove_dir_all("/tmp/lossless");
}

#[test]
fn space_in_search_enqueues_highlighted_result_not_first() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let idx = d.path().join("search-index");
    jukebox::search::build_index(&cat, &idx).unwrap();
    let s = Searcher::open(&idx).unwrap();
    let player: Box<dyn Player> = Box::new(jukebox::player::StubPlayer::default());
    let mut app = App::new(cat, player, Some(s));

    // Create the source files so play_current_queue (auto-play on first enqueue)
    // can load them instead of marking them dead.
    std::fs::create_dir_all("/tmp/lossless/a").unwrap();
    std::fs::create_dir_all("/tmp/lossless/b").unwrap();
    std::fs::write("/tmp/lossless/a/01.flac", b"x").unwrap();
    std::fs::write("/tmp/lossless/b/01.flac", b"x").unwrap();

    app.search_input = "a".into();           // matches both (Ado, Aimer, Freedom, Brave)
    app.run_search();
    assert!(app.results.len() >= 2, "expected >=2 results, got {}", app.results.len());

    // Arrow down to the second result.
    app.result_cursor = 1;
    let highlighted_id = {
        let (_, tidx) = app.results[1];
        app.catalog.tracks[tidx].id.clone()
    };

    // Space should enqueue the HIGHLIGHTED result, not result #0.
    // (enqueue_current_result is what space now calls.)
    app.enqueue_current_result();
    assert_eq!(app.queue().items().first().cloned(), Some(highlighted_id),
        "space must enqueue the highlighted result, not the first");
    let _ = std::fs::remove_dir_all("/tmp/lossless");
}
