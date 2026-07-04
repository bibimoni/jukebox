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
