//! Slice 10 security hardening tests.
//!
//! 1. `cli_output_sanitizes_control_chars` — escape-sequence injection from
//!    malicious track metadata is neutralized by `sanitize_for_terminal`.
//! 2. `corrupt_db_recovers_to_defaults` — a garbage state.db is auto-removed
//!    and `load_layout_at` returns `Ok(LayoutState::default())`.
//! 3. `sidecar_spawn_failure_returns_err_not_panic` — `Sidecar::spawn` with a
//!    bad python path returns `Err` instead of panicking.

use jukebox::state;
use jukebox::yt::sidecar::Sidecar;
use std::path::PathBuf;

#[test]
fn cli_output_sanitizes_control_chars() {
    // A malicious track title containing a terminal-clear escape sequence
    // and a shell-injection attempt. `sanitize_for_terminal` must replace
    // all C0 control chars (except \t \n \r) and DEL with `?`.
    let evil = "\x1b[2J;rm -rf /";
    let clean = jukebox::sanitize_for_terminal(evil);
    // The ESC char (0x1b) must be replaced — no escape sequence survives.
    assert!(
        !clean.contains('\x1b'),
        "ESC char survived sanitization: {clean:?}"
    );
    // The literal text after the control chars is preserved.
    assert!(
        clean.contains("[2J;rm -rf /"),
        "non-control text was lost: {clean:?}"
    );
    // Every C0 control char (0x00–0x1F except \t \n \r) + DEL is replaced.
    let mixed = "a\x00b\x01c\x07d\x1be\x7ff\thi\nj\rk";
    let sanitized = jukebox::sanitize_for_terminal(mixed);
    assert_eq!(sanitized, "a?b?c?d?e?f\thi\nj\rk");
    // Empty string is a no-op.
    assert_eq!(jukebox::sanitize_for_terminal(""), "");
}

#[test]
fn corrupt_db_recovers_to_defaults() {
    // Write garbage bytes to a state.db path — simulating a crash-corrupted
    // DB. `open_at` (called by `load_layout_at`) should remove the corrupt
    // file and re-open fresh, returning the default layout.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    std::fs::write(&db_path, b"this is not a sqlite database").unwrap();
    assert!(db_path.exists(), "garbage file should exist before load");

    // load_layout_at calls open_at internally; open_at should detect the
    // corrupt DB, remove it, re-open fresh, and return Ok(default).
    let loaded = state::load_layout_at(&db_path).unwrap();
    assert_eq!(loaded.focus, state::ARTISTS);
    assert_eq!(loaded.volume, 70);
    assert_eq!(loaded.shuffle, "off");
    assert_eq!(loaded.repeat, "off");

    // The DB should now be a valid SQLite file (re-created by open_at).
    assert!(db_path.exists(), "valid DB should exist after recovery");

    // A second load should work normally (no recovery needed).
    let loaded2 = state::load_layout_at(&db_path).unwrap();
    assert_eq!(loaded2.focus, state::ARTISTS);
}

#[test]
fn sidecar_spawn_failure_returns_err_not_panic() {
    // A nonexistent python binary — Sidecar::spawn must return Err, not panic.
    let bad_python = PathBuf::from("/nonexistent/python-that-does-not-exist");
    let script = PathBuf::from("/nonexistent/script.py");
    match Sidecar::spawn(&bad_python, &script, None, None, None) {
        Err(e) => {
            // The error must mention the spawn failure (not a panic message).
            let msg = e.to_string();
            assert!(
                msg.contains("spawning sidecar") || msg.contains("sidecar"),
                "error should mention sidecar spawn, got: {msg}"
            );
        }
        Ok(_) => panic!("Sidecar::spawn with bad python should return Err"),
    }
}
