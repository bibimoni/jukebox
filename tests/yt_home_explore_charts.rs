//! Integration tests for the YouTube Home/Explore/Charts cross-layer flow
//! (Task 7 of the YouTube Home/Explore/Charts feature).
//!
//! These tests verify the FLOW across layers (proto → session → app → view):
//!   1. Session lifecycle: `send_home`/`send_explore`/`send_charts` fires →
//!      fake sidecar responds → `drain_paired` routes the response → the
//!      matching `pending_*` field is populated → the inflight guard clears.
//!   2. App `on_tick` consumption: `pending_*` set → `on_tick` takes it →
//!      the matching `*_cached` field is set → (Home only) `home.sections`
//!      has the YouTube shelves appended with the right `HomeSection` variant
//!      from the title mapping.
//!   3. Cache lifecycle: logout clears all three caches.
//!   4. Error handling: a sidecar error response frees the inflight guard and
//!      surfaces the error.
//!
//! The per-task unit tests cover individual layers in isolation; these tests
//! cross the boundaries (a real `Session` driving a fake Python sidecar + an
//! `App` consuming the drained responses via `on_tick`).
//!
//! All tests are OFFLINE — the fake sidecar responds with pre-baked JSON, no
//! real YouTube network calls. Flake-avoidance: polling loops with generous
//! timeouts (up to ~5s) instead of fixed sleeps.

use jukebox::tui::app::{App, View, YtTab};
use jukebox::tui::view::home::{HomeItem, HomeSection};
use jukebox::yt::session::Session;
use jukebox::yt::state::YtState;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Helpers (self-contained — Rust integration tests are separate crates, so
// helpers from tests/e2e_yt.rs etc. are COPIED, not imported).
// ---------------------------------------------------------------------------

/// Build the JSON map file content for `fake_sidecar` from a single
/// `{cmd: response}` pair. The response is serialized to a JSON string (the
/// canned sidecar line). Keeps the test bodies readable (no hand-escaped
/// nested JSON).
fn one_cmd_map(cmd: &str, response: serde_json::Value) -> String {
    serde_json::json!({ cmd: response.to_string() }).to_string()
}

/// Write a per-test fake sidecar script + its map file. The map *path* is
/// baked into the script itself (no env var) so parallel tests can't race on
/// a shared process env. Returns (script, map).
///
/// Auto-responds to `home`/`explore`/`charts` with empty payloads when the
/// map doesn't provide them (mirrors tests/e2e_yt.rs::fake_sidecar) so the
/// fetch-on-first-visit on the Home tab never blocks the pending queue.
// `write_literal`: the fake sidecar body is a raw string containing Python
// with `{`/`}` (JSON dicts). Inlining it into the format string would
// re-interpret those braces as format args, so the literal stays as an arg.
#[allow(clippy::write_literal)]
fn fake_sidecar(map_json: &str) -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("yt-home-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("yt-home-fake-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        "import sys, json\nm = json.load(open({map_path:?}))\n",
        map_path = map.display(),
    )
    .unwrap();
    f.write_all(
        r#"
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    key = m.get(cmd)
    # Auto-respond to home/explore/charts with empty payloads when the map
    # doesn't provide them, so the fetch-on-first-visit on the Home tab
    # doesn't block the pending queue.
    if cmd == "home" and key is None:
        key = json.dumps({"ok": True, "data": {"home_sections": []}})
    if cmd == "explore" and key is None:
        key = json.dumps({"ok": True, "data": {"explore_playlists": []}})
    if cmd == "charts" and key is None:
        key = json.dumps({"ok": True, "data": {"charts": []}})
    if key is not None:
        print(key, flush=True)
"#
        .as_bytes(),
    )
    .unwrap();
    writeln!(f).unwrap();
    (p, map)
}

/// Spawn a `Session` against the fake sidecar script.
fn spawn_session(script: &Path, _map: &Path) -> Session {
    Session::spawn(Path::new("python3"), script, None).unwrap()
}

/// Isolate `XDG_CONFIG_HOME` to a temp dir so `on_tick`'s `save_yt_lists()`
/// (which writes to `state::db_path()`) can't leak fake playlist stubs into
/// the user's real state.db. Must be kept alive for the test's duration.
fn isolate_xdg() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", d.path());
    d
}

/// A one-track local catalog so `App::new` succeeds.
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

/// Construct an `App` with a session backed by `script`, on `tab`, in
/// `Ready` state. The XDG temp dir is kept alive for the test's duration
/// (returned as the first tuple element — keep it bound).
fn yt_app_on_tab(script: &Path, tab: YtTab) -> (tempfile::TempDir, App) {
    let _xdg = isolate_xdg();
    let session = Session::spawn(Path::new("python3"), script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = View::Youtube;
    app.yt_view.tab = tab;
    app.yt_state = YtState::Ready;
    // Keep both tempdirs alive for the app's lifetime. The catalog borrows
    // `source_root` (inside `_d`), and `on_tick`'s `save_yt_lists` writes
    // into `_xdg`. Leaking them avoids use-after-free; the test process is
    // short-lived.
    std::mem::forget(_xdg);
    std::mem::forget(_d);
    (tempfile::tempdir().unwrap(), app)
}

/// Pump `on_tick` (with small sleeps for the sidecar reader thread to deliver
/// responses) until `cond(app)` is true, up to `max` iterations. Returns true
/// on success. Mirrors tests/e2e_yt.rs::tick_until.
fn tick_until<F>(app: &mut App, max: usize, cond: F) -> bool
where
    F: Fn(&App) -> bool,
{
    for _ in 0..max {
        app.on_tick();
        if cond(&*app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Poll `session.drain_paired()` until `cond(&session)` is true, up to ~5s.
/// Used by the Session-only tests (no App). The sidecar reader thread delivers
/// responses asynchronously; a fixed sleep is flaky under parallel test load,
/// so we poll.
fn poll_session<F>(session: &mut Session, cond: F) -> bool
where
    F: Fn(&Session) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        session.drain_paired();
        if cond(session) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A fake sidecar that logs every received `cmd` to a file (one per line) and
/// responds to `home`/`explore`/`charts` with a non-empty payload. Used by the
/// inflight-guard test to COUNT received `home` commands (the second
/// `send_home()` must not produce a second `home` line).
fn counting_sidecar(log_path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("yt-home-count-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        "import sys, json\nlog = open({log_path:?}, \"w\")\n",
        log_path = log_path.display(),
    )
    .unwrap();
    f.write_all(
        r#"
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    log.write(cmd + "\n")
    log.flush()
    if cmd == "home":
        print(json.dumps({"ok": True, "data": {"home_sections": [{"title": "Listen again", "items": [{"title": "Mix 1", "playlist_id": "PL1"}]}]}}), flush=True)
    elif cmd == "explore":
        print(json.dumps({"ok": True, "data": {"explore_playlists": []}}), flush=True)
    elif cmd == "charts":
        print(json.dumps({"ok": True, "data": {"charts": []}}), flush=True)
"#
        .as_bytes(),
    )
    .unwrap();
    writeln!(f).unwrap();
    p
}

/// A minimal fake sidecar that just drains stdin (no responses). Used by the
/// logout test as the `yt_script` target so `clear_cookies`' respawn succeeds
/// without a real ytmusicapi install.
fn draining_sidecar() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("yt-home-drain-{}-{}.py", std::process::id(), n));
    std::fs::write(&p, "import sys\nfor line in sys.stdin: pass\n").unwrap();
    p
}

// ---------------------------------------------------------------------------
// Session lifecycle: send_* → drain_paired → pending_* populated → guard clears
// ---------------------------------------------------------------------------

/// `send_home` fires → the fake sidecar responds → `drain_paired` routes the
/// response → `pending_home_sections` is populated with the section + item →
/// `home_loading()` (inflight guard) is cleared.
#[test]
fn send_home_populates_pending_home_sections() {
    let map = one_cmd_map(
        "home",
        serde_json::json!({
            "ok": true,
            "data": {
                "home_sections": [
                    {
                        "title": "Listen again",
                        "items": [
                            {"title": "Mix 1", "playlist_id": "PL1"}
                        ]
                    }
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let mut s = spawn_session(&script, &map_file);

    s.send_home().unwrap();
    assert!(
        s.home_loading(),
        "home_inflight should be true right after send_home"
    );

    let landed = poll_session(&mut s, |s| s.pending_home_sections.is_some());
    assert!(
        landed,
        "pending_home_sections should be populated within 5s"
    );

    let sections = s.pending_home_sections.as_ref().unwrap();
    assert_eq!(sections.len(), 1, "one section in the response");
    assert_eq!(sections[0].title, "Listen again");
    assert_eq!(sections[0].items.len(), 1, "one item in the section");
    assert_eq!(sections[0].items[0].title, "Mix 1");
    assert_eq!(
        sections[0].items[0].playlist_id.as_deref(),
        Some("PL1"),
        "playlist_id should flow through from the sidecar response"
    );
    assert!(
        !s.home_loading(),
        "home_inflight must be cleared after the response lands"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// `send_explore` fires → the fake sidecar responds → `pending_explore_playlists`
/// is populated → `explore_loading()` is cleared.
#[test]
fn send_explore_populates_pending_explore_playlists() {
    let map = one_cmd_map(
        "explore",
        serde_json::json!({
            "ok": true,
            "data": {
                "explore_playlists": [
                    {"id": "PL1", "title": "Chill", "subtitle": "mood", "count": 7}
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let mut s = spawn_session(&script, &map_file);

    s.send_explore().unwrap();
    assert!(
        s.explore_loading(),
        "explore_inflight true after send_explore"
    );

    let landed = poll_session(&mut s, |s| s.pending_explore_playlists.is_some());
    assert!(
        landed,
        "pending_explore_playlists should be populated within 5s"
    );

    let playlists = s.pending_explore_playlists.as_ref().unwrap();
    assert_eq!(playlists.len(), 1, "one playlist in the response");
    assert_eq!(playlists[0].id, "PL1");
    assert_eq!(playlists[0].title, "Chill");
    assert_eq!(playlists[0].subtitle.as_deref(), Some("mood"));
    assert_eq!(playlists[0].count, Some(7));
    assert!(
        !s.explore_loading(),
        "explore_inflight must be cleared after the response lands"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// `send_charts` fires → the fake sidecar responds → `pending_charts` is
/// populated → `charts_loading()` is cleared.
#[test]
fn send_charts_populates_pending_charts() {
    let map = one_cmd_map(
        "charts",
        serde_json::json!({
            "ok": true,
            "data": {
                "charts": [
                    {"title": "Song A", "subtitle": "Artist A", "video_id": "v1", "chart": "Top songs"}
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let mut s = spawn_session(&script, &map_file);

    s.send_charts().unwrap();
    assert!(s.charts_loading(), "charts_inflight true after send_charts");

    let landed = poll_session(&mut s, |s| s.pending_charts.is_some());
    assert!(landed, "pending_charts should be populated within 5s");

    let charts = s.pending_charts.as_ref().unwrap();
    assert_eq!(charts.len(), 1, "one chart entry in the response");
    assert_eq!(charts[0].title, "Song A");
    assert_eq!(charts[0].chart, "Top songs");
    assert_eq!(charts[0].video_id.as_deref(), Some("v1"));
    assert!(
        !s.charts_loading(),
        "charts_inflight must be cleared after the response lands"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// App on_tick consumption: pending_* → cache + section mapping
// ---------------------------------------------------------------------------

/// `pending_home_sections` set → `on_tick` takes it → `home_sections_cached`
/// set → `home.sections` has the YouTube shelves APPENDED after the local
/// cold-start sections, with the right `HomeSection` variant from the title
/// mapping ("Listen again" → `ContinueListening`, unknown → `YtFeed(title)`).
#[test]
fn on_tick_consumes_pending_home_sections_and_sets_cache() {
    let map = one_cmd_map(
        "home",
        serde_json::json!({
            "ok": true,
            "data": {
                "home_sections": [
                    {
                        "title": "Listen again",
                        "items": [{"title": "Mix 1", "playlist_id": "PL1"}]
                    },
                    {
                        "title": "Today's hits",
                        "items": [{"title": "Song A", "video_id": "v1"}]
                    }
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let (_xdg, mut app) = yt_app_on_tab(&script, YtTab::Home);

    // Pre-populate the local cold-start sections so we can verify the YouTube
    // shelves are APPENDED (not replacing). `populate_home_state` is what
    // `render_yt_home` calls on first entry; we call it directly so the test
    // doesn't depend on the render path.
    let local_state = app.populate_home_state();
    app.yt_view.home = local_state;
    let local_section_count = app.yt_view.home.sections.len();
    assert!(
        local_section_count > 0,
        "populate_home_state should produce local cold-start sections"
    );

    // on_tick's fetch-on-first-visit fires send_home (cache is None). Pump
    // until the response lands + on_tick folds it into home_sections_cached.
    let landed = tick_until(&mut app, 250, |a| a.yt_view.home_sections_cached.is_some());
    assert!(
        landed,
        "on_tick should consume pending_home_sections into home_sections_cached within ~5s"
    );

    // The cache holds the raw proto list (2 sections).
    let cached = app.yt_view.home_sections_cached.as_ref().unwrap();
    assert_eq!(cached.len(), 2, "two YouTube sections in the cache");
    assert_eq!(cached[0].title, "Listen again");
    assert_eq!(cached[1].title, "Today's hits");

    // home.sections has the 2 YouTube shelves PREPENDED before the local
    // sections (YouTube content first, like the real YouTube Music app).
    let total = app.yt_view.home.sections.len();
    assert_eq!(
        total,
        local_section_count + 2,
        "home.sections should have 2 YouTube shelves + local sections"
    );

    // Section title mapping: "Listen again" → ContinueListening,
    // "Today's hits" (unknown) → YtFeed("Today's hits").
    // YouTube shelves are prepended (first 2 entries).
    let yt0 = &app.yt_view.home.sections[0];
    assert_eq!(
        yt0.0,
        HomeSection::ContinueListening,
        "'Listen again' must map to HomeSection::ContinueListening"
    );
    let yt1 = &app.yt_view.home.sections[1];
    assert!(
        matches!(yt1.0, HomeSection::YtFeed(ref t) if t == "Today's hits"),
        "unknown title must map to HomeSection::YtFeed carrying the raw title, got {:?}",
        yt1.0
    );

    // The first YouTube section's item is a playlist (playlist_id was set).
    assert_eq!(yt0.1.len(), 1, "one item in the Listen again shelf");

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// `pending_explore_playlists` set → `on_tick` takes it → `explore_cached`
/// set. The Explore tab renders directly from this cache (Task 5).
#[test]
fn on_tick_consumes_pending_explore_playlists_and_sets_cache() {
    let map = one_cmd_map(
        "explore",
        serde_json::json!({
            "ok": true,
            "data": {
                "explore_playlists": [
                    {"id": "PL1", "title": "Chill Vibes", "subtitle": "mood", "count": 42},
                    {"id": "PL2", "title": "Workout Beats"}
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let (_xdg, mut app) = yt_app_on_tab(&script, YtTab::Explore);

    let landed = tick_until(&mut app, 250, |a| a.yt_view.explore_cached.is_some());
    assert!(
        landed,
        "on_tick should consume pending_explore_playlists into explore_cached within ~5s"
    );

    let cached = app.yt_view.explore_cached.as_ref().unwrap();
    assert_eq!(cached.len(), 2, "two explore playlists in the cache");
    assert_eq!(cached[0].id, "PL1");
    assert_eq!(cached[0].title, "Chill Vibes");
    assert_eq!(cached[0].subtitle.as_deref(), Some("mood"));
    assert_eq!(cached[0].count, Some(42));
    assert_eq!(cached[1].id, "PL2");
    assert_eq!(cached[1].title, "Workout Beats");

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// `pending_charts` set → `on_tick` takes it → `charts_cached` set. The
/// Charts tab renders directly from this cache (Task 5).
#[test]
fn on_tick_consumes_pending_charts_and_sets_cache() {
    let map = one_cmd_map(
        "charts",
        serde_json::json!({
            "ok": true,
            "data": {
                "charts": [
                    {"title": "Song A", "subtitle": "Artist A", "video_id": "v1", "chart": "Top songs"},
                    {"title": "Video B", "subtitle": "Channel B", "video_id": "v2", "chart": "Top videos"}
                ]
            }
        }),
    );
    let (script, map_file) = fake_sidecar(&map);
    let (_xdg, mut app) = yt_app_on_tab(&script, YtTab::Charts);

    let landed = tick_until(&mut app, 250, |a| a.yt_view.charts_cached.is_some());
    assert!(
        landed,
        "on_tick should consume pending_charts into charts_cached within ~5s"
    );

    let cached = app.yt_view.charts_cached.as_ref().unwrap();
    assert_eq!(cached.len(), 2, "two chart entries in the cache");
    assert_eq!(cached[0].title, "Song A");
    assert_eq!(cached[0].chart, "Top songs");
    assert_eq!(cached[0].video_id.as_deref(), Some("v1"));
    assert_eq!(cached[1].chart, "Top videos");

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Inflight guard + error handling
// ---------------------------------------------------------------------------

/// A second `send_home()` while the first is in flight is a no-op (the
/// inflight guard blocks it). Verified by a counting fake sidecar that logs
/// every received `cmd` to a file — after two rapid `send_home()` calls, the
/// log must contain exactly ONE `home` line (the second was guarded and never
/// sent to the sidecar).
#[test]
fn inflight_guard_prevents_duplicate_send_home() {
    let log_dir = tempfile::tempdir().unwrap();
    let log_path = log_dir.path().join("cmds.log");
    let script = counting_sidecar(&log_path);
    let mut s = Session::spawn(Path::new("python3"), &script, None).unwrap();

    // First send fires (home_inflight was false → now true, Pending::Home
    // pushed, sidecar command sent).
    s.send_home().unwrap();
    assert!(s.home_loading(), "home_inflight true after first send_home");

    // Second send immediately after — must be a no-op (home_inflight is
    // already true → early return, no sidecar command sent).
    s.send_home().unwrap();
    assert!(
        s.home_loading(),
        "home_inflight still true after second send_home"
    );

    // Wait for the single response to land + be drained.
    let landed = poll_session(&mut s, |s| s.pending_home_sections.is_some());
    assert!(landed, "the one home response should land within 5s");
    assert!(
        !s.home_loading(),
        "home_inflight cleared after the response lands"
    );

    // Give the sidecar a moment to finish writing the log, then read + count.
    std::thread::sleep(Duration::from_millis(100));
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let home_count = log.lines().filter(|l| l.trim() == "home").count();
    assert_eq!(
        home_count, 1,
        "the sidecar must receive exactly ONE 'home' command (the second send_home was guarded): log={log:?}"
    );

    let _ = std::fs::remove_file(&script);
}

/// A sidecar error response frees the inflight guard so a later retry isn't
/// wedged, and surfaces the error message so the UI can exit its "loading…"
/// state. `pending_home_sections` stays `None` (no content staged); the
/// error lands in `pending_errors`.
#[test]
fn error_response_frees_inflight_guard() {
    let map = one_cmd_map(
        "home",
        serde_json::json!({"ok": false, "error": "test error from sidecar"}),
    );
    let (script, map_file) = fake_sidecar(&map);
    let mut s = spawn_session(&script, &map_file);

    s.send_home().unwrap();
    assert!(s.home_loading(), "home_inflight true after send_home");

    // Poll until the error response lands + is drained. The error arm in
    // apply_pair frees home_inflight + stages the error in pending_errors.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut errored = false;
    while Instant::now() < deadline {
        s.drain_paired();
        if !s.pending_errors.is_empty() {
            errored = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(errored, "the error response should land within 5s");

    assert!(
        !s.home_loading(),
        "home_inflight must be cleared by the error arm (so a later retry isn't wedged)"
    );
    assert!(
        s.pending_home_sections.is_none(),
        "no content staged on error (pending_home_sections stays None)"
    );
    assert!(
        !s.pending_errors.is_empty(),
        "the error must be surfaced in pending_errors"
    );
    let (scope, msg) = &s.pending_errors[0];
    assert!(
        msg.contains("test error from sidecar"),
        "the error message must be the sidecar's, got {msg:?}"
    );
    // home errors are Other-scoped (not Search-scoped).
    assert!(
        matches!(scope, jukebox::yt::session::ErrorScope::Other),
        "home errors are Other-scoped, got {scope:?}"
    );

    // A later send_home can now fire (the guard was freed by the error arm).
    s.send_home().unwrap();
    assert!(
        s.home_loading(),
        "send_home must fire again after the error freed the guard"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Cache lifecycle: logout clears all three caches
// ---------------------------------------------------------------------------

/// `yt_logout` clears `home_sections_cached`, `explore_cached`, and
/// `charts_cached` so a re-login under a different account never shows the
/// prior account's YouTube shelves. Uses a draining sidecar as the
/// `yt_script` target so `clear_cookies`' respawn succeeds without a real
/// ytmusicapi install.
#[test]
fn logout_clears_all_three_caches() {
    let _xdg = isolate_xdg();
    let script = draining_sidecar();
    let session = Session::spawn(Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    // Point yt_script at the drainer so `clear_cookies`' respawn succeeds
    // (the default `scripts/yt/yt.py` needs ytmusicapi installed).
    app.yt_script = script.clone();

    // Pre-populate all three caches with non-empty content.
    app.yt_view.home_sections_cached = Some(vec![jukebox::yt::proto::HomeSectionProto::default()]);
    app.yt_view.explore_cached = Some(vec![jukebox::yt::proto::PlaylistProto::default()]);
    app.yt_view.charts_cached = Some(vec![jukebox::yt::proto::ChartEntryProto::default()]);
    assert!(app.yt_view.home_sections_cached.is_some());
    assert!(app.yt_view.explore_cached.is_some());
    assert!(app.yt_view.charts_cached.is_some());

    app.yt_logout();

    assert!(
        app.yt_view.home_sections_cached.is_none(),
        "yt_logout must clear home_sections_cached"
    );
    assert!(
        app.yt_view.explore_cached.is_none(),
        "yt_logout must clear explore_cached"
    );
    assert!(
        app.yt_view.charts_cached.is_none(),
        "yt_logout must clear charts_cached"
    );
    assert_eq!(
        app.yt_state,
        YtState::SignedOut,
        "yt_logout transitions to SignedOut"
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// R refresh + logout must clear home.sections (prevent shelf duplication)
// ---------------------------------------------------------------------------

/// Build a realistic pre-refresh `home.sections` snapshot: one local
/// cold-start shelf (Quick Picks with a local track) + one YouTube shelf
/// (the "Listen again" shelf mapped to `ContinueListening` with a YouTube
/// playlist item). This is the state after the first fetch lands — the
/// shape `R` refresh must NOT accumulate onto.
fn local_plus_yt_sections() -> Vec<(HomeSection, Vec<HomeItem>)> {
    vec![
        (
            HomeSection::QuickPicks,
            vec![HomeItem::track(
                "t1".into(),
                "Hello".into(),
                "Adele".into(),
                true,
            )],
        ),
        (
            HomeSection::ContinueListening,
            vec![HomeItem::playlist("PL1".into(), "Mix 1".into(), false)],
        ),
    ]
}

/// `R` on the Home tab must NOT duplicate YouTube shelves in `home.sections`.
/// The refresh clears `home_sections_cached` AND `home.sections`, so the next
/// `render_yt_home` re-runs `populate_home_state` (rebuilding local-only
/// sections) and the `on_tick` consumer appends the new YouTube shelves onto
/// a fresh base. Without the `home.sections` clear, the consumer's `.push()`
/// would append the new YouTube shelves onto the PREVIOUS shelves, doubling
/// them on every `R`. Verified here by: pre-populating `home.sections` with
/// local + YouTube shelves + a non-empty cache, calling
/// `refresh_yt_home_explore_charts`, then asserting both the cache and
/// `home.sections` are cleared (and that a `home` re-fire was sent to the
/// sidecar).
#[test]
fn r_refresh_on_home_does_not_duplicate_sections() {
    let log_dir = tempfile::tempdir().unwrap();
    let log_path = log_dir.path().join("cmds.log");
    let script = counting_sidecar(&log_path);
    let (_xdg, mut app) = yt_app_on_tab(&script, YtTab::Home);

    // Simulate the post-first-fetch state: local + YouTube shelves already
    // in `home.sections`, and the raw proto list stashed in the cache.
    app.yt_view.home.sections = local_plus_yt_sections();
    app.yt_view.home_sections_cached = Some(vec![jukebox::yt::proto::HomeSectionProto::default()]);
    let pre_count = app.yt_view.home.sections.len();
    assert_eq!(
        pre_count, 2,
        "pre-condition: home.sections has local + YouTube shelves"
    );
    assert!(
        app.yt_view.home_sections_cached.is_some(),
        "pre-condition: home_sections_cached is populated"
    );

    // Simulate the `R` key on the Home tab.
    app.refresh_yt_home_explore_charts();

    // The cache is cleared (the existing behavior — fetch-on-next-visit
    // re-fires `send_home`).
    assert!(
        app.yt_view.home_sections_cached.is_none(),
        "R refresh must clear home_sections_cached"
    );
    // The fix: `home.sections` is also cleared so the `on_tick` consumer's
    // `.push()` doesn't append onto the previous shelves (duplicating them).
    // The next `render_yt_home` re-runs `populate_home_state` to rebuild the
    // local-only base, then the new YouTube shelves append when the response
    // lands.
    assert!(
        app.yt_view.home.sections.is_empty(),
        "R refresh must clear home.sections so the next fetch appends onto a \
         fresh base (not the previous shelves): got {:?}",
        app.yt_view.home.sections
    );

    // The refresh re-fired `send_home` — verify the sidecar received a
    // `home` command (poll the log for up to 2s — the sidecar's reader
    // thread may need a moment to flush, especially under parallel test
    // load).
    let mut home_count = 0;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        home_count = log.lines().filter(|l| l.trim() == "home").count();
        if home_count >= 1 {
            break;
        }
    }
    assert_eq!(
        home_count, 1,
        "the sidecar must receive a 'home' command from the R refresh"
    );

    let _ = std::fs::remove_file(&script);
}

/// `yt_logout` must clear `home.sections` (not just the three cache fields).
/// Without this clear, `populate_home_state` would be skipped on the next
/// `render_yt_home` (it only runs when `home.sections.is_empty()`) and the
/// prior account's YouTube shelves would linger on the Home tab until
/// `open_home` (H key) or restart — so a re-login under a different account
/// would show the prior account's shelves (stale data). Verified by:
/// pre-populating `home.sections` with local + YouTube shelves + the three
/// caches, calling `yt_logout`, then asserting all four fields are cleared.
#[test]
fn yt_logout_clears_home_sections() {
    let _xdg = isolate_xdg();
    let script = draining_sidecar();
    let session = Session::spawn(Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    // Point yt_script at the drainer so `clear_cookies`' respawn succeeds
    // (the default `scripts/yt/yt.py` needs ytmusicapi installed).
    app.yt_script = script.clone();

    // Pre-populate `home.sections` with local + YouTube shelves (the state
    // after the first fetch landed) and all three caches with non-empty
    // content.
    app.yt_view.home.sections = local_plus_yt_sections();
    app.yt_view.home_sections_cached = Some(vec![jukebox::yt::proto::HomeSectionProto::default()]);
    app.yt_view.explore_cached = Some(vec![jukebox::yt::proto::PlaylistProto::default()]);
    app.yt_view.charts_cached = Some(vec![jukebox::yt::proto::ChartEntryProto::default()]);
    assert_eq!(
        app.yt_view.home.sections.len(),
        2,
        "pre-condition: home.sections has local + YouTube shelves"
    );
    assert!(app.yt_view.home_sections_cached.is_some());
    assert!(app.yt_view.explore_cached.is_some());
    assert!(app.yt_view.charts_cached.is_some());

    app.yt_logout();

    // All three cache fields are cleared (the existing behavior).
    assert!(
        app.yt_view.home_sections_cached.is_none(),
        "yt_logout must clear home_sections_cached"
    );
    assert!(
        app.yt_view.explore_cached.is_none(),
        "yt_logout must clear explore_cached"
    );
    assert!(
        app.yt_view.charts_cached.is_none(),
        "yt_logout must clear charts_cached"
    );
    // The fix: `home.sections` is also cleared so the next `render_yt_home`
    // re-runs `populate_home_state` (rebuilding local-only sections, no
    // YouTube shelves since the user is now signed out).
    assert!(
        app.yt_view.home.sections.is_empty(),
        "yt_logout must clear home.sections so the prior account's YouTube \
         shelves don't linger until open_home/restart: got {:?}",
        app.yt_view.home.sections
    );
    assert_eq!(
        app.yt_state,
        YtState::SignedOut,
        "yt_logout transitions to SignedOut"
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Tab cycling (integration-level smoke test for the YtTab cycle)
// ---------------------------------------------------------------------------

/// `YtTab::next()` cycles through all 7 tabs in order and returns to Home.
/// This is a unit-style test but verifies the cycle at the integration level
/// (the tab cycle is the backbone of the fetch-on-first-visit wiring).
#[test]
fn yt_tab_cycling_reaches_all_seven_tabs() {
    let mut tab = YtTab::Home;
    let expected = [
        YtTab::Library,
        YtTab::Search,
        YtTab::Discover,
        YtTab::Radio,
        YtTab::Explore,
        YtTab::Charts,
        YtTab::Home,
    ];
    for (i, want) in expected.iter().enumerate() {
        tab = tab.next();
        assert_eq!(
            tab, *want,
            "next() step {}: expected {:?}, got {:?}",
            i, want, tab
        );
    }
    // After 7 next() calls we're back at Home (full cycle).
    assert_eq!(tab, YtTab::Home, "7 next() calls must return to Home");

    // prev() is the inverse: Home → Charts → Explore → Radio → ...
    let tab = YtTab::Home;
    assert_eq!(tab.prev(), YtTab::Charts);
    assert_eq!(tab.prev().prev(), YtTab::Explore);

    // all() returns 7 tabs with the documented labels.
    let all = YtTab::all();
    assert_eq!(all.len(), 7, "YtTab::all() must return 7 tabs");
    let labels: Vec<&str> = all.iter().map(|(l, _)| *l).collect();
    assert_eq!(
        labels,
        ["Home", "Library", "Search", "Discover", "Radio", "Explore", "Charts"],
        "YtTab::all() must return the 7 tabs in order with no number prefixes"
    );
}

// ---------------------------------------------------------------------------
// Loading-state render (optional Task 11): with a delayed sidecar, the
// Explore tab renders the "Loading…" state while the fetch is in flight.
// ---------------------------------------------------------------------------

/// A fake sidecar that DELAYS the `explore` response by `delay_ms` (so the
/// loading state is observable) but responds instantly to `home`/`charts`
/// with empty payloads. Used by the loading-state render test.
fn delayed_explore_sidecar(delay_ms: u64) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("yt-home-delayed-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        "import sys, json, time\ndelay = {delay_ms}\n",
        delay_ms = delay_ms,
    )
    .unwrap();
    f.write_all(
        r#"
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "home":
        print(json.dumps({"ok": True, "data": {"home_sections": []}}), flush=True)
    elif cmd == "explore":
        time.sleep(delay / 1000.0)
        print(json.dumps({"ok": True, "data": {"explore_playlists": [{"id": "PL1", "title": "Chill"}]}}), flush=True)
    elif cmd == "charts":
        print(json.dumps({"ok": True, "data": {"charts": []}}), flush=True)
"#
        .as_bytes(),
    )
    .unwrap();
    writeln!(f).unwrap();
    p
}

/// Render the YT view into a TestBackend buffer and return the joined cell
/// text (rows separated by `\n`). Mirrors the private `yt_view_text` helper
/// in src/tui/view/yt_view.rs's tests — replicated here because it's private.
fn render_yt_view_text(app: &mut App, width: u16, height: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            jukebox::tui::view::yt_view::render_yt_view(f, area, app);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        out.push('\n');
    }
    out
}

/// With a delayed `explore` response, the Explore tab renders the "Loading…"
/// state (Task 5's loading branch) while the fetch is in flight. This was
/// skipped in Task 5 because it needed a real session; the fake sidecar makes
/// it testable. Verifies the cross-layer flow: tab-switch → on_tick fires
/// send_explore → explore_loading() true → render shows "Loading…" → (later)
/// response lands → cache set → render shows content.
#[test]
fn render_yt_explore_shows_loading_state_with_session() {
    // 2s delay: long enough to observe the loading state before the response
    // lands, short enough to keep the test snappy.
    let script = delayed_explore_sidecar(2000);
    let (_xdg, mut app) = yt_app_on_tab(&script, YtTab::Explore);

    // on_tick fires send_explore (cache is None) → explore_inflight = true.
    app.on_tick();
    assert!(
        app.yt_session.as_ref().unwrap().explore_loading(),
        "on_tick must fire send_explore (explore_inflight true)"
    );
    assert!(
        app.yt_view.explore_cached.is_none(),
        "cache still None before the response lands"
    );

    // Render while the fetch is in flight → must show the "Loading…" state.
    let text = render_yt_view_text(&mut app, 100, 12);
    assert!(
        text.contains("Loading"),
        "Explore tab must show 'Loading…' while the fetch is in flight: {text:?}"
    );
    // The empty-state ("No content available") must NOT show while loading.
    assert!(
        !text.contains("No content available"),
        "Explore tab must not show the empty state while loading: {text:?}"
    );

    // Now pump on_tick until the delayed response lands + the cache is set.
    let landed = tick_until(&mut app, 250, |a| a.yt_view.explore_cached.is_some());
    assert!(
        landed,
        "the delayed explore response should land + set explore_cached within ~5s"
    );

    // Re-render → now the content state must show (not loading, not empty).
    let text2 = render_yt_view_text(&mut app, 100, 12);
    assert!(
        text2.contains("Chill"),
        "Explore tab must show the playlist title after the response lands: {text2:?}"
    );
    assert!(
        !text2.contains("Loading"),
        "Explore tab must not show 'Loading…' after the response lands: {text2:?}"
    );

    let _ = std::fs::remove_file(&script);
}
