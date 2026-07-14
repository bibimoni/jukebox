use jukebox::catalog::{Catalog, Track};
use jukebox::cli::NEXT_STEP_HINT;
use std::fs;
use tempfile::tempdir;

fn write_catalog(dir: &std::path::Path, json: &str) -> std::path::PathBuf {
    let p = dir.join("catalog.json");
    fs::write(&p, json).unwrap();
    p
}

fn track_json_with(source_path: &str) -> String {
    format!(
        r#"{{
      "id": "t1",
      "artists": ["Ado"],
      "primary_artist": "Ado",
      "title": "Freedom",
      "album": "Ado's Best",
      "track_number": 1,
      "disc_number": 1,
      "bit_depth": 24,
      "sample_rate_hz": 48000,
      "isrc": null,
      "source_path": {source_path:?},
      "symlinked_into_artists": ["Ado"]
    }}"#
    )
}

fn catalog_json(source_root: &str, tracks: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "built_at": "2026-07-14T00:00:00Z",
      "source_root": {source_root:?},
      "tracks": [{tracks}]
    }}"#
    )
}

#[test]
fn sync_zero_tracks_is_rejected_not_silent_success() {
    let d = tempdir().unwrap();
    let p = write_catalog(d.path(), &catalog_json("/tmp/src", ""));
    let cat = Catalog::load(&p).unwrap();
    let err = cat.require_tracks().unwrap_err().to_string();
    assert!(err.contains("0 tracks"), "got: {err}");
    assert!(err.contains("sync"), "got: {err}");
}

#[test]
fn sync_nonempty_catalog_passes_require_tracks() {
    let d = tempdir().unwrap();
    let p = write_catalog(
        d.path(),
        &catalog_json("/tmp/src", &track_json_with("src/Ado - Freedom.flac")),
    );
    let cat = Catalog::load(&p).unwrap();
    assert!(cat.require_tracks().is_ok());
}

#[test]
fn play_missing_catalog_returns_recovery_not_error() {
    let d = tempdir().unwrap();
    let p = d.path().join("catalog.json");
    assert!(!p.exists());
    assert!(Catalog::load_for_playback(&p).unwrap().is_none());
}

#[test]
fn play_empty_catalog_returns_recovery_not_error() {
    let d = tempdir().unwrap();
    let p = write_catalog(d.path(), &catalog_json("/tmp/src", ""));
    assert!(Catalog::load_for_playback(&p).unwrap().is_none());
}

#[test]
fn play_nonempty_catalog_returns_some() {
    let d = tempdir().unwrap();
    let p = write_catalog(
        d.path(),
        &catalog_json("/tmp/src", &track_json_with("src/Ado - Freedom.flac")),
    );
    assert!(Catalog::load_for_playback(&p).unwrap().is_some());
}

#[test]
fn play_corrupt_catalog_propagates_error_not_silent_recovery() {
    let d = tempdir().unwrap();
    let p = write_catalog(d.path(), "this is not json");
    assert!(Catalog::load_for_playback(&p).is_err());
}

#[test]
fn onboarding_hint_tells_user_to_sync_then_play() {
    assert!(NEXT_STEP_HINT.contains("sync"));
    assert!(NEXT_STEP_HINT.contains("jukebox"));
}

fn make_track(source_path: &str) -> Track {
    let d = tempdir().unwrap();
    let p = write_catalog(
        d.path(),
        &catalog_json("/tmp/src", &track_json_with(source_path)),
    );
    let cat = Catalog::load(&p).unwrap();
    cat.tracks[0].clone()
}

#[test]
fn non_lossless_source_dir_resolves_to_real_file() {
    let d = tempdir().unwrap();
    let src_dir = d.path().join("my_music").join("Ado");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("01 - Freedom.flac");
    fs::write(&file, "flac").unwrap();

    let source_root = d.path().join("my_music");
    let t = make_track("my_music/Ado/01 - Freedom.flac");
    let resolved = t.resolve_source(&source_root);
    assert!(
        resolved.exists(),
        "resolved path does not exist: {}",
        resolved.display()
    );
    assert_eq!(resolved, file);
}

#[test]
fn lossless_source_dir_still_resolves_to_real_file() {
    let d = tempdir().unwrap();
    let src_dir = d.path().join("lossless").join("Ado");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("01 - Freedom.flac");
    fs::write(&file, "flac").unwrap();

    let source_root = d.path().join("lossless");
    let t = make_track("lossless/Ado/01 - Freedom.flac");
    let resolved = t.resolve_source(&source_root);
    assert!(
        resolved.exists(),
        "resolved path does not exist: {}",
        resolved.display()
    );
    assert_eq!(resolved, file);
}

#[test]
fn hardcoded_lossless_prefix_does_not_resolve_for_non_lossless_dir() {
    let d = tempdir().unwrap();
    let src_dir = d.path().join("my_music").join("Ado");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("01 - Freedom.flac"), "flac").unwrap();

    let source_root = d.path().join("my_music");
    let t = make_track("lossless/Ado/01 - Freedom.flac");
    let resolved = t.resolve_source(&source_root);
    assert!(
        !resolved.exists(),
        "hardcoded lossless/ prefix should NOT resolve for a non-lossless dir, but found: {}",
        resolved.display()
    );
}
