//! End-to-end YouTube-integration tests against a *fake* Python sidecar.
//!
//! The fake sidecar reads a JSON map of `{cmd -> canned response line}` from a
//! per-test file (path passed via `JK_FAKE_MAP`) and echoes the canned line for
//! each request. Using a per-test map *file* (not a shared env var) avoids the
//! parallel-test env race that broke a shared `JK_FAKE_SIDECAR` var.

use jukebox::source::TrackSource;
use jukebox::tui::app::App;
use jukebox::tui::queue::ContinueMode;
use jukebox::yt::session::Session;
use std::io::Write;

/// Write a per-test fake sidecar script + its map file. The map *path* is
/// baked into the script itself (no env var) so parallel tests can't race on
/// a shared process env. Returns (script, map).
fn fake_sidecar(map_json: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("e2e-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("e2e-fake-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    // The map path is interpolated directly into the script — no env var, so
    // parallel `Session::spawn` calls can't race on a shared JK_FAKE_MAP.
    writeln!(
        f,
        "import sys, json\nm = json.load(open({map_path:?}))\n{body}",
        map_path = map.display(),
        body = r#"
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
"#
    )
    .unwrap();
    (p, map)
}

fn spawn_session(script: &std::path::Path, _map: &std::path::Path) -> Session {
    Session::spawn(std::path::Path::new("python3"), script, None).unwrap()
}

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
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, jukebox::catalog::Catalog::load(&p).unwrap())
}

#[test]
fn mixed_mode_matches_local_on_isrc_and_plays_local() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert!(
        matches!(app.now_playing, Some(TrackSource::Local { ref track_id }) if track_id == "t1"),
        "mixed should play local on match: {:?}",
        app.now_playing
    );
}

#[test]
fn cont_youtube_with_no_session_stops_cleanly_no_panic() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app.next();
    assert!(app.now_playing.is_none(), "no session → should stop, not panic");
}

#[test]
fn dead_remote_track_is_skipped_not_halt() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.play_in_context_ids(vec!["vidA".into(), "vidB".into()], "vidA");
    assert!(app.dead.contains("vidA") || app.now_playing.is_none());
}

#[test]
fn sidecar_spawn_and_search_round_trip() {
    let map_json = r#"{"search":"{\"ok\":true,\"data\":{\"search\":[{\"video_id\":\"v1\",\"title\":\"Hello\",\"artist\":\"Adele\",\"album\":null,\"dur\":null,\"isrc\":\"GBBKS1500123\"}]}}"}"#;
    let (script, map) = fake_sidecar(map_json);
    let mut s = spawn_session(&script, &map);
    let v = s.search("adele", 5).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].video_id, "v1");
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map);
}

#[test]
fn sidecar_resolve_url_round_trip() {
    let (script, map) = fake_sidecar("{}");
    let mut s = spawn_session(&script, &map);
    let u = s.resolve_url("vidZ").unwrap();
    assert_eq!(u.url, "https://x/vidZ");
    assert_eq!(u.abr, 256);
    assert!(u.premium);
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map);
}

#[test]
fn cont_youtube_advances_via_radio_cursor() {
    let wp = r#"{"get_watch_playlist":"{\"ok\":true,\"data\":{\"watch_playlist\":[{\"video_id\":\"yt1\",\"title\":\"A\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null},{\"video_id\":\"yt2\",\"title\":\"B\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null}]}}"}"#;
    let (script, map) = fake_sidecar(wp);
    let session = spawn_session(&script, &map);
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, Some(session));
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;
    app.play_in_context_ids(vec!["yt1".into()], "yt1");
    assert!(app.now_playing.is_some(), "yt1 should resolve+play via the fake sidecar");
    app.next();
    let after = app.now_playing.clone();
    assert!(
        matches!(after, Some(TrackSource::Remote { ref video_id }) if video_id == "yt2"),
        "CONT=YouTube should advance to yt2, got {after:?}"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map);
}

#[test]
fn refresh_then_on_tick_populates_yt_lists_and_clears_loading() {
    // Fake sidecar returns account playlists + suggestions. refresh_yt_lists
    // fires async; on_tick drains + folds results into yt_lists + clears
    // loading. This is the path the unit tests missed (on_tick wiring).
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":3}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD1\",\"name\":\"Focus\",\"count\":0}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(jukebox::player::StubPlayer::default()), None, Some(session));
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    assert!(app.yt_lists_loading, "refresh should set loading");
    // Pump on_tick until the lists land (the reader thread delivers async).
    let mut populated = false;
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists.is_empty() {
            populated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(populated, "on_tick should fold the fetched lists into yt_lists");
    assert!(!app.yt_lists_loading, "on_tick should clear loading");
    assert!(app.yt_lists.iter().any(|l| l.name == "Liked"), "account list missing");
    assert!(app.yt_lists.iter().any(|l| l.name == "Focus"), "suggested list missing");
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}
