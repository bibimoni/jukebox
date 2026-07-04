use jukebox::catalog::Catalog;
use jukebox::search::{build_index, Searcher};
use std::fs;
use tempfile::tempdir;

fn mini_catalog_json() -> String {
    serde_json::json!({
        "version": 1, "built_at": "2026-07-04T00:00:00Z",
        "source_root": "/tmp/lossless",
        "tracks": [
          { "id":"t1","artists":["Ikimono-gakari"],"primary_artist":"Ikimono-gakari",
            "title":"ブルーバード","album":"My Song","bit_depth":16,"sample_rate_hz":44100,
            "source_path":"lossless/i/01.flac","symlinked_into_artists":["Ikimono-gakari"] },
          { "id":"t2","artists":["Ado"],"primary_artist":"Ado",
            "title":"Freedom","album":"Best","bit_depth":24,"sample_rate_hz":48000,
            "source_path":"lossless/a/01.flac","symlinked_into_artists":["Ado"] },
        ]
    }).to_string()
}

#[test]
fn build_index_writes_segments() {
    let d = tempdir().unwrap();
    let cat_path = d.path().join("catalog.json");
    fs::write(&cat_path, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&cat_path).unwrap();
    let idx = d.path().join("search-index");
    build_index(&cat, &idx).unwrap();
    assert!(idx.is_dir());
    assert!(fs::read_dir(&idx).unwrap().count() > 0);
}

fn build_then_open() -> (tempfile::TempDir, Searcher) {
    let d = tempdir().unwrap();
    let cat_path = d.path().join("catalog.json");
    std::fs::write(&cat_path, mini_catalog_json()).unwrap();
    let cat = Catalog::load(&cat_path).unwrap();
    let idx = d.path().join("search-index");
    build_index(&cat, &idx).unwrap();
    let s = Searcher::open(&idx).unwrap();
    (d, s)
}

#[test]
fn romaji_finds_katakana_title() {
    let (_d, s) = build_then_open();
    let hits = s.search("burubado", 10).unwrap();
    assert!(hits.iter().any(|h| h.track_id == "t1"), "romaji -> ブルーバード");
}

#[test]
fn ascii_title_exact_ranks_high() {
    let (_d, s) = build_then_open();
    let hits = s.search("Freedom", 10).unwrap();
    assert_eq!(hits[0].track_id, "t2");
}

#[test]
fn fuzzy_typo_tolerated() {
    let (_d, s) = build_then_open();
    let hits = s.search("Freedon", 10).unwrap();
    assert!(hits.iter().any(|h| h.track_id == "t2"));
}
