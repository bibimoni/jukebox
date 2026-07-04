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
