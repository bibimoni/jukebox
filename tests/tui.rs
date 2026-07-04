use jukebox::catalog::Catalog;
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
    let app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()));
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
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()));
    app.enqueue_artist("Ado");
    assert_eq!(app.queue().len(), 1);
}
