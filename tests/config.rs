use jukebox::config::{config_path, validate_source_dir, Config};
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

// We can't fully control $XDG_CONFIG_HOME here, so test save/load roundtrip
// via a direct path by using the public save/load that target config_path().
// Instead, test the bits we can control deterministically.

// Tests below mutate process-global env vars (HOME, XDG_CONFIG_HOME). Serialize
// them so they don't race under the default parallel test runner.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn default_for_sets_filtered_sibling() {
    let cfg = Config::default_for("/Users/distiled/Music/lossless".into());
    assert_eq!(
        cfg.source_dir,
        std::path::Path::new("/Users/distiled/Music/lossless")
    );
    assert_eq!(
        cfg.filtered_dir,
        std::path::Path::new("/Users/distiled/Music/filtered_lossless")
    );
    assert_eq!(cfg.version, 1);
}

#[test]
fn validate_rejects_missing_dir() {
    let r = validate_source_dir(std::path::Path::new("/nonexistent/xyz"));
    assert!(r.is_err());
}

#[test]
fn validate_rejects_dir_without_flac() {
    let d = tempdir().unwrap();
    fs::write(d.path().join("not-audio.txt"), b"x").unwrap();
    assert!(validate_source_dir(d.path()).is_err());
}

#[test]
fn save_then_load_roundtrip() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Use a temp HOME so config_path() lands in our tempdir.
    let tmp = tempdir().unwrap();
    // HOME-based fallback is what dirs::config_dir() uses on macOS when XDG unset.
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    let cfg = Config::default_for(tmp.path().join("lossless"));
    cfg.save().unwrap();
    let loaded = Config::load().unwrap().expect("config should exist");
    assert_eq!(loaded.source_dir, cfg.source_dir);
    assert_eq!(loaded.filtered_dir, cfg.filtered_dir);
    let p = config_path();
    assert!(
        p.starts_with(tmp.path()),
        "config_path {p:?} should be under temp HOME"
    );
    let meta = fs::metadata(p).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
}

#[test]
fn load_returns_none_when_missing() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempdir().unwrap();
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    assert!(Config::load().unwrap().is_none());
}
