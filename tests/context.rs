use jukebox::catalog::Catalog;
use jukebox::tui::context::{Context, ContextResolver, build_albums_by_artist};

fn cat2() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"a1","artists":["40mP"],"primary_artist":"40mP","title":"Alpha","album":"Cosmic","track_number":1,"bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"a2","artists":["40mP"],"primary_artist":"40mP","title":"Beta","album":"Cosmic","track_number":2,"bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/02.flac","symlinked_into_artists":["40mP"]},
          {"id":"a3","artists":["40mP"],"primary_artist":"40mP","title":"Gamma","album":"Solo","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/40mP/03.flac","symlinked_into_artists":["40mP"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    (d, cat)
}

struct FakeResolver { playlists: Vec<(String, Vec<String>)>, queue: Vec<String> }
impl ContextResolver for FakeResolver {
    fn playlist_ids(&self, name: &str) -> Vec<String> {
        self.playlists.iter().find(|(n,_)| n==name).map(|(_,v)| v.clone()).unwrap_or_default()
    }
    fn queue_ids(&self) -> Vec<String> { self.queue.clone() }
}

#[test]
fn albums_grouped_by_artist_and_title() {
    let (_d, cat) = cat2();
    let albums = build_albums_by_artist(&cat);
    let forty = albums.get("40mP").unwrap();
    // Two distinct albums: "Cosmic" (2 tracks) and "Solo" (1 track)
    assert_eq!(forty.len(), 2);
    let cosmic = forty.iter().find(|a| a.title == "Cosmic").unwrap();
    assert_eq!(cosmic.track_indices.len(), 2);
}

#[test]
fn album_context_track_ids_preserve_album_order() {
    let (_d, _cat) = cat2();
    let resolver = FakeResolver { playlists: vec![], queue: vec![] };
    let ctx = Context::Album { album: "Cosmic".into(), artist: "40mP".into(), track_ids: vec!["a1".into(),"a2".into()] };
    assert_eq!(ctx.track_ids(&resolver), vec!["a1".to_string(), "a2".to_string()]);
}

#[test]
fn playlist_context_resolves_via_resolver() {
    let (_d, _cat) = cat2();
    let resolver = FakeResolver {
        playlists: vec![("Faves".into(), vec!["a3".into(),"a1".into()])],
        queue: vec![],
    };
    let ctx = Context::Playlist { name: "Faves".into() };
    assert_eq!(ctx.track_ids(&resolver), vec!["a3".to_string(), "a1".to_string()]);
}
