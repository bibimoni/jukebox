use jukebox::catalog::Catalog;
use std::fs;
use tempfile::tempdir;

fn sample() -> &'static str {
    r#"{
      "version": 1,
      "built_at": "2026-07-04T00:00:00Z",
      "source_root": "/Users/distiled/Music/lossless",
      "tracks": [
        {
          "id": "abc123",
          "artists": ["Ado"],
          "primary_artist": "Ado",
          "title": "Freedom",
          "album": "Ado's Best",
          "track_number": 15,
          "disc_number": 1,
          "bit_depth": 24,
          "sample_rate_hz": 48000,
          "isrc": "JPPO02105116",
          "source_path": "lossless/Ado discography/Ado - Freedom.flac",
          "symlinked_into_artists": ["Ado"]
        }
      ]
    }"#
}

#[test]
fn parses_catalog() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, sample()).unwrap();
    let c = Catalog::load(&p).unwrap();
    assert_eq!(c.version, 1);
    assert_eq!(c.tracks.len(), 1);
    let t = &c.tracks[0];
    assert_eq!(t.title, "Freedom");
    assert_eq!(t.artists, vec!["Ado".to_string()]);
    assert_eq!(t.bit_depth, 24);
    assert_eq!(t.sample_rate_hz, 48000);
    assert_eq!(t.isrc.as_deref(), Some("JPPO02105116"));
}

#[test]
fn resolve_source_joins_parent_of_source_root() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, sample()).unwrap();
    let c = Catalog::load(&p).unwrap();
    let t = &c.tracks[0];
    let abs = t.resolve_source(&c.source_root);
    assert!(abs.ends_with("Ado discography/Ado - Freedom.flac"));
    assert!(abs.is_absolute() || abs.starts_with("lossless/"));
}

#[test]
fn quality_label_formats_int_and_fractional_khz() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, sample()).unwrap();
    let c = Catalog::load(&p).unwrap();
    let t = &c.tracks[0];
    assert_eq!(t.quality_label(), "24bit-48kHz");

    // 44100 Hz -> 44.1kHz
    let mut t2 = t.clone();
    t2.sample_rate_hz = 44100;
    t2.bit_depth = 16;
    assert_eq!(t2.quality_label(), "16bit-44.1kHz");
}

#[test]
fn handles_null_optional_fields() {
    let json = r#"{
      "version": 1,
      "built_at": "2026-07-04T00:00:00Z",
      "source_root": "/Users/distiled/Music/lossless",
      "tracks": [
        {
          "id": "abc123",
          "artists": ["Ado"],
          "primary_artist": "Ado",
          "title": "Freedom",
          "album": null,
          "track_number": null,
          "disc_number": null,
          "bit_depth": 16,
          "sample_rate_hz": 44100,
          "isrc": null,
          "source_path": "lossless/Ado discography/Ado - Freedom.flac",
          "symlinked_into_artists": []
        }
      ]
    }"#;
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    fs::write(&p, json).unwrap();
    let c = Catalog::load(&p).unwrap();
    let t = &c.tracks[0];
    assert_eq!(t.album, None);
    assert_eq!(t.track_number, None);
    assert_eq!(t.disc_number, None);
    assert_eq!(t.isrc, None);
    assert_eq!(t.quality_label(), "16bit-44.1kHz");
}
