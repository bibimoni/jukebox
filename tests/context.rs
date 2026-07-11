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

/// Regression: a collaborator who is never a `primary_artist` must still see
/// the album they appear on. `symlinked_into_artists` includes Lilas Ikuta on
/// a milet-primary track, so the album must be filed under Lilas Ikuta too —
/// otherwise the Albums column is empty and "this artist has no songs".
#[test]
fn collaborator_album_appears_under_non_primary_artist() {
    let d = tempfile::tempdir().unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":"/tmp/lossless",
        "tracks":[
          {"id":"c1","artists":["milet","Aimer","Lilas Ikuta"],"primary_artist":"milet","title":"Omokage","album":"Omokage","track_number":1,"bit_depth":24,"sample_rate_hz":48000,"source_path":"lossless/milet/01.flac","symlinked_into_artists":["milet","Aimer","Lilas Ikuta"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let albums = build_albums_by_artist(&cat);
    // The album is filed under every collaborating artist, not just the primary.
    let lilas = albums.get("Lilas Ikuta").expect("collaborator must have the album");
    assert_eq!(lilas.len(), 1);
    assert_eq!(lilas[0].title, "Omokage");
    assert_eq!(lilas[0].track_indices, vec![0]);
    // The album's owner label stays the primary artist.
    assert_eq!(lilas[0].artist, "milet");
    let aimer = albums.get("Aimer").expect("Aimer must have the album");
    assert_eq!(aimer.len(), 1);
    let milet = albums.get("milet").expect("milet must have the album");
    assert_eq!(milet.len(), 1);
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

struct R2;
impl jukebox::tui::context::ContextResolver for R2 {
    fn playlist_ids(&self, _: &str) -> Vec<String> { vec![] }
    fn queue_ids(&self) -> Vec<String> { vec![] }
    fn yt_playlist_ids(&self, key: &str) -> Vec<String> {
        if key == "yt1" { vec!["v1".into(), "v2".into()] } else { vec![] }
    }
}

#[test]
fn yt_playlist_resolver_returns_video_ids() {
    let ctx = jukebox::tui::context::Context::Youtube { key: "yt1".into(), name: "Y1".into() };
    assert_eq!(ctx.track_ids(&R2), vec!["v1".to_string(), "v2".to_string()]);
}
