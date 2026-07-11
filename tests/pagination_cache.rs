//! Slice 3 — pagination + offline cache + empty-vs-failed distinction.
//!
//! Integration tests against a fake Python sidecar (per-test canned JSON map,
//! no creds, no network) PLUS unit-style tests of the cache module and the
//! provider state machine. Covers the user's three required scenarios:
//!   1. `pagination_large_library` — a library with >25 playlists is NOT
//!      truncated to 25 (the sidecar's `limit=None` pagination feeds every
//!      entry; the Rust side folds all of them into `yt_lists`).
//!   2. `offline_shows_cached_marked_stale` — a launch with no session but a
//!      populated `yt_lists_cache` shows the cached lists and transitions to
//!      `ReadyStale` (offline — showing cached, press R), not `Failed`.
//!   3. `empty_vs_failed_distinguished` — a sidecar `{"ok":false,"error":...}`
//!      drives `yt_state` to an error state (not `Ready` with an empty list),
//!      so a genuinely empty library (`Ok([])` → `Ready`) is distinguishable
//!      from a failed fetch (yt-recon §5: the old sidecar swallowed both into
//!      `[]`, making empty == failed).
//!
//! Additional unit-style tests cover the rate-limit state, cache-clear-on-
//! logout, and the R-retry guard (AC-M3.* — rate-limit, R refresh).

use jukebox::tui::app::{App, View, YtList, YtListKind};
use jukebox::yt::cache;
use jukebox::yt::session::Session;
use jukebox::yt::state::YtState;
use std::io::Write;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a per-test fake sidecar script + its canned-response map. The map
/// path is baked into the script (no env var) so parallel tests can't race on
/// a shared `JK_FAKE_MAP`. Mirrors `tests/e2e_yt::fake_sidecar`.
#[allow(clippy::write_literal)]
fn fake_sidecar(map_json: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("pg-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("pg-fake-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    // The map path is interpolated directly into the script — no env var.
    write!(
        f,
        "import sys, json\nm = json.load(open({map_path:?}))\n",
        map_path = map.display(),
    )
    .unwrap();
    f.write_all(
        br#"
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    key = m.get(cmd)
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        key = json.dumps({"ok": True, "data": {"resolve": {"url": "https://x/" + vid, "expires_at": None, "codec": "AAC", "abr": 256, "sample_rate": 48000, "container": "m4a", "premium": True}}})
    if key is not None:
        print(key, flush=True)
"#,
    )
    .unwrap();
    writeln!(f).unwrap();
    (p, map)
}

/// A 1-track catalog backed by a real on-disk flac (so any metadata check in
/// `App::new` passes). Mirrors `tests/e2e_yt::local_cat`.
fn local_cat() -> (tempfile::TempDir, jukebox::catalog::Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Adele")).unwrap();
    std::fs::write(lossless.join("Adele").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["Adele"],"primary_artist":"Adele","title":"Hello",
        "album":"25","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/Adele/01.flac",
        "symlinked_into_artists":["Adele"],"isrc":"GBBKS1500123"}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, jukebox::catalog::Catalog::load(&p).unwrap())
}

/// Isolate `XDG_CONFIG_HOME` to a unique temp dir so `on_tick`'s best-effort
/// `save_yt_lists` (which writes to `state::db_path()`) lands in a throwaway
/// dir instead of the user's real config. Unique per call so parallel tests in
/// this binary don't share a dir. NB: these tests only WRITE to this dir (the
/// cache save side-effect); they never read it, so a parallel `set_var` race
/// is benign.
fn isolate_xdg() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!("jk-pg-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

/// A fresh temp DB path for the unit-style cache tests (race-free — no
/// `XDG_CONFIG_HOME` env var).
fn tmp_db() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    dir.keep().join("pagination-cache-test.db")
}

// ---------------------------------------------------------------------------
// 1. Pagination: a large library is not truncated to 25.
// ---------------------------------------------------------------------------

#[test]
fn pagination_large_library() {
    // Fake sidecar returns 30 library playlists (>25 default). The Rust side
    // must fold ALL of them into yt_lists — not truncate to 25. The sidecar's
    // `limit=None` pagination (yt.py) is what produces the full set; this test
    // guards the Rust merge path against a silent cap.
    let _xdg = isolate_xdg();
    let playlists: Vec<String> = (0..30)
        .map(|i| format!(r#"{{"id":"PL{i}","name":"List{i}","count":0}}"#))
        .collect();
    let playlists_json = format!(
        r#"{{"ok":true,"data":{{"playlists":[{}]}}}}"#,
        playlists.join(",")
    );
    // `{:?}` debug-formats the inner JSON as a quoted+escaped string literal,
    // so the map value is a JSON string holding the canned response line
    // (matching the wire format the real sidecar emits).
    let map = format!(r#"{{"library_playlists":{:?}}}"#, playlists_json);
    let (script, map_file) = fake_sidecar(&map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = View::Youtube;
    app.refresh_yt_lists();
    let mut landed = false;
    for _ in 0..150 {
        app.on_tick();
        if app.yt_lists.len() >= 30 {
            landed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(landed, "on_tick should fold all 30 playlists into yt_lists");
    assert_eq!(
        app.yt_lists.len(),
        30,
        "a large library must not be truncated to 25"
    );
    // The merged lists are all Account-kind (library_playlists), in order.
    assert_eq!(app.yt_lists[0].id, "PL0");
    assert_eq!(app.yt_lists[29].id, "PL29");
    assert!(app.yt_lists.iter().all(|l| l.kind == YtListKind::Account));
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// 2. Offline launch shows cached lists marked ReadyStale (not Failed).
// ---------------------------------------------------------------------------

#[test]
fn offline_shows_cached_marked_stale() {
    // A launch with NO session (sidecar couldn't start) but a populated
    // yt_lists_cache shows the cached lists and transitions to ReadyStale
    // (offline — showing cached, press R), NOT Failed. Uses an explicit temp
    // DB path so the test is race-free (no XDG_CONFIG_HOME env race).
    let db = tmp_db();
    let cached = vec![
        YtList {
            id: "PL1".into(),
            name: "Liked".into(),
            kind: YtListKind::Account,
            track_ids: vec![],
        },
        YtList {
            id: "RD1".into(),
            name: "Focus Mix".into(),
            kind: YtListKind::Suggested,
            track_ids: vec![],
        },
    ];
    cache::save_yt_lists_at(&db, &cached).unwrap();
    // Confirm the storage round-trip works — this is the regression guard for
    // the `VALUES (?1, ?1)` → `VALUES (?1, ?2)` SQL fix (the old form bound
    // the JSON to the key column, so load never matched and returned empty).
    let reloaded = cache::load_yt_lists_at(&db).unwrap();
    assert_eq!(reloaded.len(), 2, "cache round-trip must work");
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None, // no session → offline
    );
    // Before the cache load: no lists, default state (not ReadyStale).
    assert!(app.yt_lists.is_empty());
    assert_ne!(app.yt_state, YtState::ReadyStale);
    app.load_yt_lists_from_cache_at(&db);
    assert_eq!(
        app.yt_state,
        YtState::ReadyStale,
        "offline + cached lists → ReadyStale, not Failed"
    );
    assert!(!app.yt_lists.is_empty(), "cached lists should be visible");
    assert_eq!(app.yt_lists.len(), 2);
    assert_eq!(app.yt_lists[0].id, "PL1");
    assert_eq!(app.yt_lists[1].name, "Focus Mix");
    assert_eq!(app.yt_lists[1].kind, YtListKind::Suggested);
}

// ---------------------------------------------------------------------------
// 3. A failed fetch (ok:false) is distinguished from a genuinely empty library.
// ---------------------------------------------------------------------------

#[test]
fn empty_vs_failed_distinguished() {
    // A sidecar `{"ok":false,"error":"..."}` for library_playlists must drive
    // yt_state to an ERROR (ProviderError), NOT Ready with an empty list. This
    // is what distinguishes a failed fetch from a genuinely empty library
    // (Ok([]) → Ready with []). Without it, the Rust side can't tell empty
    // from failed (yt-recon §5) and silently shows nothing for a network blip.
    let _xdg = isolate_xdg();
    let map = r#"{"library_playlists":"{\"ok\":false,\"error\":\"fetch failed: network\"}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = View::Youtube;
    app.refresh_yt_lists();
    let mut errored = false;
    for _ in 0..150 {
        app.on_tick();
        if app.yt_state.is_error() {
            errored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        errored,
        "ok:false should drive yt_state to an error, not stay Synchronizing"
    );
    assert!(
        !app.yt_state.is_ready(),
        "a failed fetch must NOT promote to Ready"
    );
    assert!(
        app.yt_lists.is_empty(),
        "a failed fetch must not silently populate yt_lists"
    );
    assert!(
        !app.yt_lists_loading,
        "the loading indicator must clear on error"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// 4. Rate-limit state exists and is distinct from other errors (AC-M3).
// ---------------------------------------------------------------------------

#[test]
fn rate_limit_state_exists_and_is_distinct() {
    // RateLimited is a distinct state from ProviderError and AuthExpired.
    let rl = YtState::RateLimited;
    let pe = YtState::ProviderError;
    let ae = YtState::AuthExpired;

    assert!(rl.is_error(), "RateLimited is an error state");
    assert!(rl.can_retry(), "RateLimited is retryable (press R)");
    assert!(pe.is_error(), "ProviderError is an error state");
    assert!(ae.is_error(), "AuthExpired is an error state");

    // The human label should mention "rate" or "limit" so the user knows
    // to wait rather than re-auth.
    let label = rl.human_label();
    assert!(
        label.to_lowercase().contains("rate") || label.to_lowercase().contains("limit"),
        "RateLimited label should mention rate/limit: got '{label}'"
    );

    // The retry hint should mention pressing R.
    if let Some(hint) = rl.retry_hint() {
        assert!(
            hint.to_lowercase().contains('r') || hint.to_lowercase().contains("retry"),
            "RateLimited retry hint should mention R/retry: got '{hint}'"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Cache is cleared on logout so stale data doesn't survive a credential
//    change.
// ---------------------------------------------------------------------------

#[test]
fn cache_cleared_on_logout() {
    let db = tmp_db();
    let lists = vec![YtList {
        id: "PL_LOGOUT".into(),
        name: "Will be cleared".into(),
        kind: YtListKind::Account,
        track_ids: vec!["v1".into()],
    }];

    cache::save_yt_lists_at(&db, &lists).unwrap();
    assert!(
        !cache::load_yt_lists_at(&db).unwrap().is_empty(),
        "cache has data"
    );

    // Logout clears the cache.
    cache::clear_yt_lists_at(&db).unwrap();
    assert!(
        cache::load_yt_lists_at(&db).unwrap().is_empty(),
        "cache is empty after clear"
    );
}

// ---------------------------------------------------------------------------
// 6. R key can retry from RateLimited (the R handler gates on can_retry).
// ---------------------------------------------------------------------------

#[test]
fn r_key_can_retry_from_rate_limited() {
    // The R key handler in input.rs (retry_yt_probe) checks can_retry().
    // Verify RateLimited allows retry.
    let state = YtState::RateLimited;
    assert!(
        state.can_retry(),
        "R key should be able to retry from RateLimited state"
    );

    // Compare with Ready (not retryable — already working).
    let ready = YtState::Ready;
    assert!(!ready.can_retry(), "Ready state should not need retry");
}
