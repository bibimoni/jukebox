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
// `write_literal`: the fake sidecar bodies are raw strings containing Python
// with `{`/`}` (JSON dicts). Inlining them into the format string would
// re-interpret those braces as format args and break, so the literals must
// stay as args.
#[allow(clippy::write_literal)]
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
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        key = json.dumps({"ok": True, "data": {"resolve": {"url": "https://x/" + vid, "expires_at": None, "codec": "AAC", "abr": 256, "sample_rate": 48000, "container": "m4a", "premium": True}}})
    if key is not None:
        print(key, flush=True)
"#
        .as_bytes(),
    )
    .unwrap();
    writeln!(f).unwrap();
    (p, map)
}

fn spawn_session(script: &std::path::Path, _map: &std::path::Path) -> Session {
    Session::spawn(std::path::Path::new("python3"), script, None).unwrap()
}

/// Pump `on_tick` (with small sleeps for the sidecar reader thread to deliver
/// responses) until `cond(app)` is true, up to `max` iterations. Returns true
/// on success. A cold-miss pick no longer sets `now_playing` synchronously —
/// the URL lands on the next tick — so tests pump until it does.
fn tick_until<F>(app: &mut App, max: usize, cond: F) -> bool
where
    F: Fn(&App) -> bool,
{
    for _ in 0..max {
        app.on_tick();
        if cond(&*app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
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
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, jukebox::catalog::Catalog::load(&p).unwrap())
}

#[test]
fn mixed_mode_matches_local_on_isrc_and_plays_local() {
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None,
    );
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
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None,
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app.next();
    assert!(
        app.now_playing.is_none(),
        "no session → should stop, not panic"
    );
}

#[test]
fn dead_remote_track_is_skipped_not_halt() {
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None,
    );
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
    let u = s.resolve_url("vidZ", "fast").unwrap();
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
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;
    app.play_in_context_ids(vec!["yt1".into()], "yt1");
    assert!(
        tick_until(&mut app, 100, |a| a.now_playing.is_some()),
        "yt1 should resolve+play via the fake sidecar (cold miss lands on tick)"
    );
    app.next();
    // next() arms a cold-miss swap to yt2 (now_playing stays on yt1 until the
    // URL lands); pump until it swaps.
    assert!(
        tick_until(&mut app, 100, |a| matches!(
            a.now_playing,
            Some(TrackSource::Remote { ref video_id }) if video_id == "yt2"
        )),
        "CONT=YouTube should advance to yt2, got {:?}",
        app.now_playing
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map);
}

#[test]
fn refresh_then_on_tick_populates_yt_lists_and_clears_loading() {
    // Fake sidecar returns account playlists. refresh_yt_lists fires async;
    // on_tick drains + folds results into yt_lists + clears loading. This is
    // the path the unit tests missed (on_tick wiring). NOTE: send_refresh no
    // longer fetches home_suggestions (get_home() can hang in guest mode,
    // blocking the single-threaded sidecar). Only library_playlists is sent.
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":3}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD1\",\"name\":\"Focus\",\"count\":0}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
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
    assert!(
        populated,
        "on_tick should fold the fetched lists into yt_lists"
    );
    assert!(!app.yt_lists_loading, "on_tick should clear loading");
    assert!(
        app.yt_lists.iter().any(|l| l.name == "Liked"),
        "account list missing"
    );
    // No suggested list — send_refresh no longer fetches home_suggestions.
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn focused_yt_list_lazy_loads_its_tracks_on_tick() {
    // Regression: focusing a YT list never fetched its tracks — col2 stayed on
    // "select a list to load its tracks", and `s`/Enter no-oped. on_tick now
    // fire-and-forget sends get_playlist for the focused list with empty
    // track_ids, then folds the response into the list.
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":2}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"},{\"video_id\":\"v2\",\"title\":\"Other\",\"artist\":\"B\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // Wait for the lists to land.
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(app.yt_lists.len(), 1);
    assert!(
        app.yt_lists[0].track_ids.is_empty(),
        "list starts with no tracks"
    );
    // The focused list (cursor 0 → PL1) has empty tracks: on_tick should
    // fire-and-forget get_playlist, then a later tick folds the tracks in.
    let mut loaded = false;
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists[0].track_ids.is_empty() {
            loaded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(loaded, "on_tick should lazy-load the focused list's tracks");
    assert_eq!(
        app.yt_lists[0].track_ids,
        vec!["v1".to_string(), "v2".to_string()]
    );
    assert!(app.loaded_yt_lists.contains("PL1"), "list marked loaded");
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn refresh_yt_lists_releases_playlist_inflight_so_a_refocus_can_refetch() {
    // Regression: a successful get_playlist cleared pending_tracks but NOT
    // playlist_inflight (only the error arm cleared it). So after a refresh
    // replaced yt_lists + cleared loaded_yt_lists, re-focusing a list hit the
    // `!inflight` guard as false forever → col2 wedged on "select a list…"
    // for the rest of the session.
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":2}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // First load: PL1 tracks land.
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists.is_empty() && !app.yt_lists[0].track_ids.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !app.yt_lists.is_empty() && !app.yt_lists[0].track_ids.is_empty(),
        "first load"
    );
    // The inflight guard MUST have cleared on success.
    assert!(
        !app.yt_session.as_ref().unwrap().playlist_loading("PL1"),
        "playlist_inflight must clear on a successful get_playlist"
    );
    // Simulate a re-focus (Tab away and back, or re-entering Y view): refresh
    // replaces the lists + clears loaded_yt_lists. RC11-DEF-055: a re-entry
    // with already-loaded lists no longer sets `yt_lists_loading` (silent
    // background refresh), so wait for the refresh response to land by
    // watching `yt_lists[0].track_ids` go from non-empty → empty (the
    // refreshed YtList starts with empty track_ids until on_tick re-fetches).
    app.refresh_yt_lists();
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists.is_empty() && app.yt_lists[0].track_ids.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    app.loaded_yt_lists.clear(); // mirror the refresh's clear
    assert!(
        app.yt_lists[0].track_ids.is_empty(),
        "refresh replaced the list (empty tracks)"
    );
    // Now on_tick must re-fetch PL1 — which requires playlist_inflight to be
    // clear. If the guard was wedged, this loop never loads.
    let mut reloaded = false;
    for _ in 0..100 {
        app.on_tick();
        if !app.yt_lists[0].track_ids.is_empty() {
            reloaded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        reloaded,
        "re-focus must re-fetch the list's tracks (inflight cleared on success)"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Forever-loading fix: when a loaded playlist's tracks have metadata evicted
// from track_cache (e.g. the user browsed enough other playlists to push them
// out before pinning existed), on_tick must re-fetch the playlist so the
// "Loading…" rows fill in. Without the fix, the lazy-load guard (`!loaded`)
// blocked the re-fetch and the rows stayed "Loading…" forever.
// ---------------------------------------------------------------------------

#[test]
fn on_tick_refetches_loaded_playlist_when_track_metadata_evicted() {
    // Load a 3-track playlist (v1, v2, v3) — all cached, list marked loaded.
    // Then simulate eviction: remove v1 + v2 from track_cache. on_tick must
    // detect the missing metadata and re-fetch the playlist, restoring the
    // cached metadata so no row is stuck on "Loading…".
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":3}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song1\",\"artist\":\"A\"},{\"video_id\":\"v2\",\"title\":\"Song2\",\"artist\":\"B\"},{\"video_id\":\"v3\",\"title\":\"Song3\",\"artist\":\"C\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // Wait for lists + first track load to complete.
    for _ in 0..200 {
        app.on_tick();
        if !app.yt_lists.is_empty()
            && !app.yt_lists[0].track_ids.is_empty()
            && !app
                .yt_session
                .as_ref()
                .map(|s| s.playlist_loading("PL1"))
                .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(app.yt_lists[0].track_ids, vec!["v1", "v2", "v3"]);
    assert!(app.loaded_yt_lists.contains("PL1"));
    // All three tracks have metadata.
    for id in &["v1", "v2", "v3"] {
        assert!(
            app.yt_session.as_ref().unwrap().track_for(id).is_some(),
            "{id} should have metadata after first load"
        );
    }

    // Simulate eviction of v1 + v2 (as if they were pushed out of the
    // 256-entry cache before pinning existed). The pin set (set each tick
    // from the focused playlist's track_ids) prevents FUTURE eviction, but
    // already-evicted entries are gone — the re-fetch must restore them.
    app.yt_session.as_mut().unwrap().track_cache.remove("v1");
    app.yt_session.as_mut().unwrap().track_cache.remove("v2");
    assert!(app.yt_session.as_ref().unwrap().track_for("v1").is_none());
    assert!(app.yt_session.as_ref().unwrap().track_for("v2").is_none());

    // on_tick must detect the missing metadata and re-fetch the playlist.
    let mut refetched = false;
    for _ in 0..200 {
        app.on_tick();
        // The re-fetch restores v1 + v2 in track_cache.
        if app.yt_session.as_ref().unwrap().track_for("v1").is_some()
            && app.yt_session.as_ref().unwrap().track_for("v2").is_some()
        {
            refetched = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        refetched,
        "on_tick must re-fetch a loaded playlist when its track metadata was \
         evicted, so rows don't stay 'Loading…' forever"
    );
    // All three tracks restored.
    for id in &["v1", "v2", "v3"] {
        assert!(
            app.yt_session.as_ref().unwrap().track_for(id).is_some(),
            "{id} metadata restored after re-fetch"
        );
    }
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Forever-loading fix (latency): the premium-tier pre-warm (nsig-solver
// download, ~10-15s cold) must NOT fire at all during browsing. The sidecar
// is single-threaded/sequential — a premium resolve queued before a
// get_playlist blocks the track fetch for 10-15s, keeping the Y view on
// "Loading…". The pre-warm was removed entirely; the one-time solver
// download happens on the first real `preload_next_url` (during playback).
// ---------------------------------------------------------------------------

#[test]
fn no_premium_resolve_fires_during_browsing_or_refresh() {
    // refresh_yt_lists must NOT fire a premium resolve. on_tick must NOT fire
    // one either (no pre-warm). The sidecar stays free to process
    // get_playlist requests immediately — no 10-15s block.
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":1}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // Right after refresh: no premium resolve in flight.
    assert!(
        !app.yt_session
            .as_ref()
            .map(|s| s.premium_resolve_busy())
            .unwrap_or(false),
        "refresh_yt_lists must NOT fire a premium resolve"
    );
    // Pump on_tick until the playlist loads. At NO point should a premium
    // resolve be in flight (the pre-warm was removed).
    let mut loaded = false;
    for _ in 0..200 {
        app.on_tick();
        assert!(
            !app.yt_session
                .as_ref()
                .map(|s| s.premium_resolve_busy())
                .unwrap_or(false),
            "on_tick must NOT fire a premium resolve during browsing (no pre-warm)"
        );
        if !app.yt_lists.is_empty() && !app.yt_lists[0].track_ids.is_empty() {
            loaded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(loaded, "the focused playlist should load its tracks");
    // Even after loading: no premium resolve was ever fired.
    assert!(
        !app.yt_session
            .as_ref()
            .map(|s| s.premium_resolve_busy())
            .unwrap_or(false),
        "no premium resolve should be in flight after browsing (pre-warm removed)"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Retry (R) fix: retry_yt_probe must be non-blocking. The old implementation
// called the BLOCKING library_playlists() (3s roundtrip), freezing the TUI.
// The new implementation immediately sets Synchronizing (visible feedback) +
// fire-and-forgets send_refresh. on_tick promotes to Ready / classifies
// errors when the response lands.
// ---------------------------------------------------------------------------

#[test]
fn retry_yt_probe_is_non_blocking_with_immediate_feedback() {
    // R must immediately transition to Synchronizing (without waiting for the
    // sidecar) and fire-and-forget the refresh. The old blocking call would
    // hang this test for 3s.
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":1}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    // Start in an error state (R is only allowed from error/stale/syncing).
    app.yt_state = jukebox::yt::state::YtState::ProviderError;
    app.yt_error = Some("previous error".into());
    // Measure: retry_yt_probe must return immediately (well under the 3s the
    // old blocking call took). The fake sidecar doesn't even have the response
    // queued yet, so a blocking call would time out at 3s.
    let start = std::time::Instant::now();
    app.retry_yt_probe();
    let elapsed = start.elapsed();
    // Immediate feedback: state is Synchronizing, old error cleared.
    assert_eq!(
        app.yt_state,
        jukebox::yt::state::YtState::Synchronizing,
        "R must immediately transition to Synchronizing (visible feedback)"
    );
    assert!(
        app.yt_error.is_none(),
        "R must clear the old error immediately"
    );
    // Must return in well under 1s (the old blocking call took up to 3s).
    // 500ms is generous for spawn overhead; the real assertion is "not 3s".
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "retry_yt_probe must be non-blocking (took {elapsed:?}, should be <500ms)"
    );
    // The refresh is now in flight. Pump on_tick until it completes.
    let mut ready = false;
    for _ in 0..200 {
        app.on_tick();
        if app.yt_state == jukebox::yt::state::YtState::Ready {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        ready,
        "on_tick must promote to Ready when the refresh response lands"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn retry_yt_probe_classifies_auth_expired_error_asynchronously() {
    // When the refresh returns an auth error, on_tick (not retry_yt_probe)
    // must classify it as AuthExpired. The old blocking retry did this
    // synchronously; the non-blocking retry delegates to on_tick's existing
    // error-classification handler.
    let map = r#"{"library_playlists":"{\"ok\":false,\"error\":\"Unauthorized: 401 — login required\"}","home_suggestions":"{\"ok\":false,\"error\":\"Unauthorized: 401\"}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.yt_state = jukebox::yt::state::YtState::ProviderError;
    app.retry_yt_probe();
    // Immediate: Synchronizing.
    assert_eq!(app.yt_state, jukebox::yt::state::YtState::Synchronizing);
    // Pump on_tick until the error response lands and on_tick classifies it.
    let mut classified = false;
    for _ in 0..200 {
        app.on_tick();
        if app.yt_state == jukebox::yt::state::YtState::AuthExpired {
            classified = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        classified,
        "on_tick must classify the 401 error as AuthExpired (delegated from retry_yt_probe)"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// Serialization fix: only ONE playlist fetch in flight at a time. The sidecar
// is single-threaded/sequential; allowing multiple concurrent fetches queues
// them all and the user's currently-focused playlist sits at the back (long
// loading time). Serializing means: switch A→B while A loads → B waits → A
// completes → next tick fires B → B loads. No queue buildup.
// ---------------------------------------------------------------------------

#[test]
fn only_one_playlist_fetch_in_flight_at_a_time() {
    // Use a non-responding sidecar: requests are read but never answered,
    // so the inflight set stays populated.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("noreply.py");
    std::fs::write(&script, "import sys\nfor line in sys.stdin:\n pass\n").unwrap();
    let mut session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();

    // Focus A → send_get_playlist(A)
    session.send_get_playlist("PL_A".into()).unwrap();
    assert!(
        session.playlist_loading("PL_A"),
        "PL_A should be marked loading after send_get_playlist"
    );

    // Focus B while A is still in flight → BLOCKED (serialization).
    // The old code allowed B through (different id), queueing both.
    // The fix blocks B: only one in flight at a time.
    session.send_get_playlist("PL_B".into()).unwrap();
    assert!(
        session.playlist_loading("PL_A"),
        "PL_A should still be loading (B was blocked, A is still in flight)"
    );
    assert!(
        !session.playlist_loading("PL_B"),
        "PL_B should NOT be loading — serialization blocks it while PL_A is in flight"
    );
}

#[test]
fn tracks_response_clears_inflight_so_next_playlist_can_load() {
    // When a Tracks response lands, the inflight guard clears, allowing the
    // next playlist's fetch to proceed on the next tick.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("partial_reply.py");
    std::fs::write(
        &script,
        "import sys, json\nfor line in sys.stdin:\n    line = line.strip()\n    if not line: continue\n    req = json.loads(line)\n    cmd = req.get('cmd')\n    if cmd == 'get_playlist':\n        print(json.dumps({\"ok\": True, \"data\": {\"tracks\": [{\"video_id\": \"v1\", \"title\": \"Song\", \"artist\": \"A\"}]}}), flush=True)\n",
    )
    .unwrap();
    let mut session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();

    // Send PL_A → in flight.
    session.send_get_playlist("PL_A".into()).unwrap();
    assert!(session.playlist_loading("PL_A"));

    // PL_B is blocked (serialization).
    session.send_get_playlist("PL_B".into()).unwrap();
    assert!(!session.playlist_loading("PL_B"));

    // Wait for PL_A's response.
    std::thread::sleep(std::time::Duration::from_millis(200));
    session.drain_paired();

    // PL_A's guard cleared.
    assert!(
        !session.playlist_loading("PL_A"),
        "PL_A's guard should be cleared after its response lands"
    );

    // Now PL_B can be sent (inflight set is empty).
    session.send_get_playlist("PL_B".into()).unwrap();
    assert!(
        session.playlist_loading("PL_B"),
        "PL_B should now be loading — serialization released after PL_A completed"
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// pending_tracks Vec fix: when two get_playlist responses land in the same
// drain_paired cycle (user switched A→B rapidly), BOTH must survive. The old
// single-slot Option design overwrote the first with the second → PL_A never
// got its tracks ("wrong tracks per playlist").
// ---------------------------------------------------------------------------

#[test]
fn multiple_get_playlist_responses_in_one_drain_all_survive() {
    // Use a fake sidecar that returns DIFFERENT tracks per playlist id, so we
    // can verify the right tracks go to the right list.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("per_id.py");
    std::fs::write(
        &script,
        "import sys, json\nfor line in sys.stdin:\n    line = line.strip()\n    if not line: continue\n    req = json.loads(line)\n    cmd = req.get('cmd')\n    if cmd == 'get_playlist':\n        pid = req.get('id', '')\n        if pid == 'PL_A':\n            print(json.dumps({\"ok\": True, \"data\": {\"tracks\": [{\"video_id\": \"a1\", \"title\": \"TrackA1\", \"artist\": \"X\"}, {\"video_id\": \"a2\", \"title\": \"TrackA2\", \"artist\": \"X\"}]}}), flush=True)\n        elif pid == 'PL_B':\n            print(json.dumps({\"ok\": True, \"data\": {\"tracks\": [{\"video_id\": \"b1\", \"title\": \"TrackB1\", \"artist\": \"Y\"}, {\"video_id\": \"b2\", \"title\": \"TrackB2\", \"artist\": \"Y\"}]}}), flush=True)\n",
    )
    .unwrap();
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    // Set up two playlists (simulating a refresh having landed).
    app.yt_state = jukebox::yt::state::YtState::Ready;
    app.yt_lists = vec![
        jukebox::tui::app::YtList {
            id: "PL_A".into(),
            name: "Playlist A".into(),
            kind: jukebox::tui::app::YtListKind::Account,
            track_ids: vec![],
        },
        jukebox::tui::app::YtList {
            id: "PL_B".into(),
            name: "Playlist B".into(),
            kind: jukebox::tui::app::YtListKind::Account,
            track_ids: vec![],
        },
    ];

    // Focus PL_A → on_tick fires send_get_playlist(PL_A). PL_A is now in
    // flight (serialized: only one at a time).
    app.cursors.playlist = 0; // PL_A
    app.on_tick();

    // Switch to PL_B → on_tick tries send_get_playlist(PL_B) but PL_A is
    // still in flight → BLOCKED (serialization). PL_B stays empty.
    app.cursors.playlist = 1; // PL_B
    app.on_tick();
    assert!(
        app.yt_lists[1].track_ids.is_empty(),
        "PL_B should still be empty — serialization blocked it while PL_A was in flight"
    );

    // Wait for PL_A's response to land.
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.on_tick(); // drains PL_A's response → PL_A gets tracks, inflight clears.

    // PL_A must have its correct tracks.
    assert_eq!(
        app.yt_lists[0].track_ids,
        vec!["a1".to_string(), "a2".to_string()],
        "PL_A must have its OWN tracks (a1, a2)"
    );

    // Now PL_B's fetch can proceed (inflight is clear). on_tick lazy-loads
    // the focused PL_B.
    app.on_tick(); // fires send_get_playlist(PL_B)
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.on_tick(); // drains PL_B's response

    // PL_B must have its correct tracks.
    assert_eq!(
        app.yt_lists[1].track_ids,
        vec!["b1".to_string(), "b2".to_string()],
        "PL_B must have its OWN tracks (b1, b2) — no cross-contamination"
    );

    let _ = std::fs::remove_file(&script);
}

#[test]
fn yt_search_is_explicit_submit_and_lands_on_tick() {
    // Regression: the `/` overlay re-ran the YouTube search on every keystroke,
    // stalling the UI ~3s/char. Now Youtube scope is explicit-submit: typing
    // never sends, Enter fires one async request, on_tick folds the response
    // into the overlay. This test drives the submit + drain path directly.
    use jukebox::tui::app::{Overlay, SearchScope};
    let map = r#"{"search":"{\"ok\":true,\"data\":{\"search\":[{\"video_id\":\"v1\",\"title\":\"A\",\"artist\":\"X\"},{\"video_id\":\"v2\",\"title\":\"B\",\"artist\":\"Y\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    // Open the search overlay in Youtube scope with a typed query. Crucially,
    // NO request has been sent yet (typing must not search).
    app.overlay = Some(Overlay::Search {
        input: "adele".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: None,
        searching: false,
    });
    // Submit — this sends exactly one Request::Search and marks searching.
    app.submit_yt_search("adele".into());
    assert!(
        app.yt_session
            .as_ref()
            .and_then(|s| s.search_inflight())
            .is_some(),
        "submit should set the search in flight"
    );
    // Mirror the overlay state the key handler would set on Enter.
    if let Some(Overlay::Search {
        submitted,
        searching,
        ..
    }) = app.overlay.as_mut()
    {
        *submitted = Some("adele".into());
        *searching = true;
    }
    // Pump on_tick until the response lands and populates the overlay.
    let mut landed = false;
    for _ in 0..100 {
        app.on_tick();
        if let Some(Overlay::Search {
            results, searching, ..
        }) = &app.overlay
        {
            if !results.is_empty() && !*searching {
                landed = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        landed,
        "on_tick should fold the search response into the overlay"
    );
    if let Some(Overlay::Search {
        results, searching, ..
    }) = &app.overlay
    {
        assert_eq!(*results, vec!["v1".to_string(), "v2".to_string()]);
        assert!(!*searching, "searching should clear once results land");
    }
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// A fake sidecar that BRANCHES resolve_url on `quality`: fast → AAC 129k
/// premium=false, premium → AAC 256k premium=true. Used by the two-tier tests.
fn fake_sidecar_two_tier() -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-2tier-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"
import sys, json, time
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        q = (req.get("quality") or "fast")
        if q == "premium":
            # Model real timing: the premium (tv/web + nsig solver) resolve is
            # ~10-15s, well after the ~1.3s fast tier. A short delay lets the
            # fast URL land first so a cold-miss swap starts on the fast tier
            # and the premium land later drives the progressive upgrade.
            time.sleep(0.5)
            r = {"url": "https://pre/" + vid, "expires_at": None, "codec": "AAC", "abr": 256, "sample_rate": 48000, "container": "m4a", "premium": True}
        else:
            r = {"url": "https://fast/" + vid, "expires_at": None, "codec": "AAC", "abr": 129, "sample_rate": 48000, "container": "m4a", "premium": False}
        print(json.dumps({"ok": True, "data": {"resolve": r}}), flush=True); continue
"#).unwrap();
    writeln!(f).unwrap();
    (p.clone(), p) // second is unused; kept for symmetry with fake_sidecar
}

#[test]
fn two_tier_cache_holds_both_and_prefers_premium() {
    // Regression: the url_cache used to be Vec<(vid,url,exp)> with retain-by-vid,
    // so a premium resolve EVICTED the fast URL. Now it holds both tiers per vid
    // and url_for prefers premium. send_resolve_premium also signals
    // pending_premium_url so App can swap the live stream up to 256k.
    let (script, _map) = fake_sidecar_two_tier();
    let mut s = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    // Fast first: caches https://fast/v, premium not yet present.
    let fast = s.resolve_url("v", "fast").unwrap();
    assert_eq!(fast.abr, 129);
    assert!(!fast.premium);
    assert_eq!(
        s.url_for("v").as_deref(),
        Some("https://fast/v"),
        "only fast cached → url_for returns fast"
    );
    assert!(s.url_for_premium("v").is_none());
    // Premium: caches https://pre/v WITHOUT evicting fast.
    let prem = s.resolve_url("v", "premium").unwrap();
    assert_eq!(prem.abr, 256);
    assert!(prem.premium);
    assert_eq!(
        s.url_for("v").as_deref(),
        Some("https://pre/v"),
        "premium present → url_for prefers premium"
    );
    assert_eq!(s.url_for_premium("v").as_deref(), Some("https://pre/v"));
    // The signal for App's progressive-upgrade swap was set on the premium land.
    // (resolve_url is a sync roundtrip; apply_pair ran and set the signal.)
    let (vid, u) = s
        .pending_premium_url
        .take()
        .expect("premium land signals pending_premium_url");
    assert_eq!(vid, "v");
    assert!(u.premium);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn progressive_upgrade_swaps_player_to_premium_and_resumes() {
    // App plays the fast URL, then a premium URL lands mid-play → on_tick swaps
    // the player to the premium URL + resumes at the captured position via
    // load_at (mpv `start`), guarded by same-track / not-near-end /
    // not-already-premium. No from-0 replay.
    let (script, _map) = fake_sidecar_two_tier();
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    // Play a YT track via the fast tier. A cold miss lands on the next tick
    // (resolve_source arms both tiers fire-and-forget + returns Pending).
    app.play_in_context_ids(vec!["v".to_string()], "v");
    // The StubPlayer loads the fast URL and starts at pos 0, dur 180.
    assert!(
        tick_until(&mut app, 100, |a| a.now_playing.is_some()),
        "track should be playing (cold miss lands on tick)"
    );
    assert!(!app.playing_premium, "started on the fast (129k) tier");
    // Advance the stub's position a little (simulate playback) so the swap has a
    // non-zero resume point and isn't near the end.
    app.player.seek(12.0).ok();
    assert_eq!(app.player.position(), Some(12.0));
    // Fire-and-forget the premium resolve; pump on_tick until the swap lands.
    app.yt_session
        .as_mut()
        .unwrap()
        .send_resolve_premium("v".into())
        .unwrap();
    let mut swapped = false;
    for _ in 0..100 {
        app.on_tick();
        if app.playing_premium {
            swapped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(swapped, "premium land should swap the player up to 256k");
    assert!(app.playing_premium, "playing_premium set after swap");
    // The player resumed at the captured position (12s) via load_at (no replay).
    assert_eq!(
        app.player.position(),
        Some(12.0),
        "swap must resume at the captured position"
    );
    // A second premium land must NOT re-swap (already premium guard).
    app.yt_session
        .as_mut()
        .unwrap()
        .send_resolve_premium("v".into())
        .unwrap();
    for _ in 0..20 {
        app.on_tick();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Still premium, still at 12s (no redundant reload that restarts audio).
    assert!(app.playing_premium);
    let _ = std::fs::remove_file(&script);
}

/// A fake sidecar where `search` succeeds, `resolve_url` premium ERRORS, fast
/// succeeds. Used to prove a background premium-preload error does NOT clear
/// an in-flight search overlay's `searching` flag (the error-scope tag fix).
fn fake_sidecar_error_scope() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-errscope-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "search":
        print(json.dumps({"ok": True, "data": {"search": [
            {"video_id": "s1", "title": "A", "artist": "X"},
            {"video_id": "s2", "title": "B", "artist": "Y"}
        ]}}), flush=True); continue
    if cmd == "resolve_url":
        q = (req.get("quality") or "fast")
        if q == "premium":
            print(json.dumps({"ok": False, "error": "premium resolve rate-limited"}), flush=True)
        else:
            v = req.get("video_id", "")
            print(json.dumps({"ok": True, "data": {"resolve": {"url": "https://fast/" + v, "expires_at": None, "codec": "AAC", "abr": 129, "sample_rate": 48000, "container": "m4a", "premium": False}}}), flush=True)
        continue
"#).unwrap();
    writeln!(f).unwrap();
    p
}

#[test]
fn premium_preload_error_does_not_drop_an_in_flight_search() {
    // Regression: pending_error was a single Option<String> with no kind tag,
    // and on_tick's error handler assumed every error belonged to the open
    // search overlay. So a background premium-preload error (rate-limit) while
    // a real search was in flight cleared the overlay's `searching` flag → the
    // search response, when it landed, was silently dropped (searching=false
    // meant the search-fold branch didn't populate results).
    use jukebox::tui::app::{Overlay, SearchScope};
    let script = fake_sidecar_error_scope();
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;

    // 1. A background premium preload fires (e.g. preload_next_url) — sent
    //    FIRST so its error response lands before the search's success.
    app.yt_session
        .as_mut()
        .unwrap()
        .send_resolve_premium("nextVid".into())
        .unwrap();

    // 2. The user opens the search overlay in Youtube scope + submits a query.
    app.overlay = Some(Overlay::Search {
        input: "adele".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adele".into()),
        searching: true,
    });
    app.submit_yt_search("adele".into());

    // 3. Pump on_tick. The premium ERROR lands first; with the bug it would
    //    clear the overlay's `searching` flag. Then the search success lands.
    let mut populated = false;
    for _ in 0..100 {
        app.on_tick();
        if let Some(Overlay::Search {
            results, searching, ..
        }) = &app.overlay
        {
            if !results.is_empty() && !*searching {
                populated = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        populated,
        "the premium-preload error must NOT drop the in-flight search's results"
    );
    if let Some(Overlay::Search { results, .. }) = &app.overlay {
        assert_eq!(*results, vec!["s1".to_string(), "s2".to_string()]);
    }
    // The premium error was still surfaced (footer), just without touching the
    // overlay — confirm the error message reached yt_error.
    assert!(
        app.yt_error.is_some(),
        "premium error surfaced in the footer"
    );
    let _ = std::fs::remove_file(&script);
}

#[test]
fn stale_search_response_does_not_drop_a_non_search_overlay() {
    // Regression: on_tick's search-fold used self.overlay.take(), so a stale
    // search response landing while a non-Search overlay (Help/PlaylistPicker/
    // Command/YtAuth/Discover) was open would destruct it and never restore.
    // Now it clones + matches, leaving a non-Search overlay untouched.
    use jukebox::tui::app::Overlay;
    let map = r#"{"search":"{\"ok\":true,\"data\":{\"search\":[{\"video_id\":\"s1\",\"title\":\"A\",\"artist\":\"X\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    std::env::set_var("JK_FAKE_MAP", &map_file);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;

    // Fire a search (in flight), then close it + open Help — simulating the
    // user hitting Esc then `?` while the search response is still pending.
    app.submit_yt_search("adele".into());
    app.overlay = None; // Esc
    app.overlay = Some(Overlay::Help); // user opens help

    // Pump on_tick until the search response lands (it would, with the bug,
    // drop the Help overlay). The Help overlay must survive.
    let help_survived = (0..100).all(|_| {
        app.on_tick();
        matches!(app.overlay, Some(Overlay::Help))
    });
    assert!(
        help_survived,
        "a stale search response must NOT drop the Help overlay"
    );
    assert!(
        matches!(app.overlay, Some(Overlay::Help)),
        "Help still open after search lands"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn search_error_keeps_search_scope_when_other_error_drains_same_cycle() {
    // Regression: pending_error was a single slot; a Search error drained
    // before an Other error in one drain_paired cycle was overwritten, losing
    // the Search scope tag → the search overlay's `searching` flag wouldn't
    // clear on a Search failure. Now set_error keeps a pending Search error.
    use jukebox::yt::session::ErrorScope;
    let script = fake_sidecar_error_scope();
    let s = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    // This fake's search SUCCEEDS; for this test we want two ERRORS, so use a
    // fake that errors on BOTH search and premium resolve (below).
    drop(s);
    // Use a fake whose search AND premium both error.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-botherr-{}-{}.py", std::process::id(), n));
    std::fs::write(
        &p,
        r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "search":
        print(json.dumps({"ok": False, "error": "search failed"}), flush=True); continue
    if cmd == "resolve_url":
        print(json.dumps({"ok": False, "error": "premium failed"}), flush=True); continue
"#,
    )
    .unwrap();
    let mut s = Session::spawn(std::path::Path::new("python3"), &p, None).unwrap();
    // Fire a search, then a premium resolve (FIFO: search error first, then
    // premium error). Both responses are read by the sidecar's reader thread;
    // we wait until BOTH have landed, then drain them in ONE drain_paired
    // cycle (the bug only manifests when both apply in the same cycle: the
    // second/Other error used to overwrite the first/Search one).
    s.send_search("q".into()).unwrap();
    s.send_resolve_premium("v".into()).unwrap();
    // Poll drain_paired until both errors are staged (or timeout). The reader
    // thread buffers responses asynchronously; a fixed sleep is flaky under
    // parallel test load (SYNC-3). Poll up to ~5s.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut errs = Vec::new();
    while std::time::Instant::now() < deadline {
        s.drain_paired();
        errs = std::mem::take(&mut s.pending_errors);
        if errs
            .iter()
            .any(|(sc, _)| matches!(sc, ErrorScope::Search(_)))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // The pending_errors (Vec) must contain the SEARCH-scoped error (not
    // dropped by the Other error; both staged). The Search one is what tells
    // on_tick to clear the overlay's searching flag.
    assert!(
        errs.iter()
            .any(|(sc, _)| matches!(sc, ErrorScope::Search(_))),
        "Search error staged (not clobbered/dropped by the Other error): {:?}",
        errs
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn second_search_does_not_steal_the_first_searchs_results() {
    // Regression: search_inflight was a single slot, so a second search's
    // query overwrote the first's tag. apply_pair tagged the FIRST response
    // with the LATEST query → the user searched "adeles" but got "adele"'s
    // results, and "adeles"'s real results were silently dropped. Now the
    // query rides in Pending::Search(q), so each response is tagged correctly.
    use jukebox::tui::app::{Overlay, SearchScope};
    // Fake sidecar: search returns results keyed on the query so we can tell
    // which query's results landed.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-2search-{}-{}.py", std::process::id(), n));
    std::fs::write(
        &p,
        r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "search":
        q = req.get("q", "")
        # Echo the query into the first result's id so the test can tell which
        # query's results these are.
        print(json.dumps({"ok": True, "data": {"search": [
            {"video_id": "first:" + q, "title": q, "artist": "X"}
        ]}}), flush=True); continue
"#,
    )
    .unwrap();
    let session = Session::spawn(std::path::Path::new("python3"), &p, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;

    // Submit "adele", then — while it's still in flight — submit "adeles".
    app.overlay = Some(Overlay::Search {
        input: "adele".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adele".into()),
        searching: true,
    });
    app.submit_yt_search("adele".into());
    // Second search: change the query + submit.
    app.overlay = Some(Overlay::Search {
        input: "adeles".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adeles".into()),
        searching: true,
    });
    app.submit_yt_search("adeles".into());

    // Pump on_tick. Both responses land (FIFO: adele first, then adeles). With
    // the bug, adele's response would be tagged "adeles" and adeles's response
    // dropped (search_inflight taken by adele). Now each is tagged correctly:
    // the overlay (submitted="adeles") gets ADELES's results.
    let mut got_adeles = false;
    for _ in 0..100 {
        app.on_tick();
        if let Some(Overlay::Search { results, .. }) = &app.overlay {
            if results.iter().any(|r| r == "first:adeles") {
                got_adeles = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        got_adeles,
        "the overlay (submitted=adeles) must get ADELES's results, not adele's"
    );
    // And adele's results must NOT be shown as adeles's (the bug's symptom).
    if let Some(Overlay::Search { results, .. }) = &app.overlay {
        assert!(
            !results.iter().any(|r| r == "first:adele"),
            "adele's results must not be mislabeled as adeles's"
        );
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn prior_querys_search_error_does_not_drop_current_querys_results() {
    // Regression: a Search error carried no query tag, so on_tick cleared the
    // overlay's `searching` flag for ANY Search error. If the error was for an
    // ABANDONED prior query ("adele") while "adeles" was still in flight, the
    // flag was cleared → "adeles"'s success landed into searching=false and
    // was dropped. Now the error carries its query and on_tick only clears the
    // flag when it matches the overlay's `submitted` query.
    use jukebox::tui::app::{Overlay, SearchScope};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-errthenok-{}-{}.py", std::process::id(), n));
    // search for "adele" ERRORS; search for "adeles" SUCCEEDS with id "first:adeles".
    std::fs::write(
        &p,
        r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "search":
        q = req.get("q", "")
        if q == "adele":
            print(json.dumps({"ok": False, "error": "adele rate-limited"}), flush=True)
        else:
            print(json.dumps({"ok": True, "data": {"search": [
                {"video_id": "first:" + q, "title": q, "artist": "X"}
            ]}}), flush=True)
        continue
"#,
    )
    .unwrap();
    let session = Session::spawn(std::path::Path::new("python3"), &p, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;

    // Submit "adele" (will error), then abandon it + submit "adeles" (will succeed).
    app.submit_yt_search("adele".into());
    app.overlay = Some(Overlay::Search {
        input: "adele".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adele".into()),
        searching: true,
    });
    app.submit_yt_search("adeles".into());
    app.overlay = Some(Overlay::Search {
        input: "adeles".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adeles".into()),
        searching: true,
    });

    // Pump on_tick. FIFO: adele's ERROR lands first (would clear searching with
    // the bug), then adeles's SUCCESS. With the fix, the error (query="adele")
    // does NOT match submitted="adeles", so searching stays true and adeles's
    // results populate.
    let mut got_adeles = false;
    for _ in 0..100 {
        app.on_tick();
        if let Some(Overlay::Search { results, .. }) = &app.overlay {
            if results.iter().any(|r| r == "first:adeles") {
                got_adeles = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        got_adeles,
        "adeles's results must show despite adele's stale error clearing searching"
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn both_queries_erroring_in_one_cycle_clears_the_current_querys_searching() {
    // Regression: pending_error was a single slot and set_error kept the FIRST
    // Search error, dropping the second. So if BOTH "adele" and "adeles"
    // errored in one drain cycle, the second (adeles — the overlay's current
    // query) was dropped at staging time → on_tick never saw it → searching
    // stayed true forever (wedge) AND the footer showed adele's error while the
    // user searched adeles. Now pending_errors is a Vec (nothing dropped);
    // on_tick matches the overlay's submitted query and clears searching for it.
    use jukebox::tui::app::{Overlay, SearchScope};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("e2e-botherr2-{}-{}.py", std::process::id(), n));
    // BOTH searches error.
    std::fs::write(
        &p,
        r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True); continue
    if cmd == "search":
        q = req.get("q", "")
        print(json.dumps({"ok": False, "error": q + " rate-limited"}), flush=True); continue
"#,
    )
    .unwrap();
    let session = Session::spawn(std::path::Path::new("python3"), &p, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;

    // Submit "adele", then abandon it + submit "adeles". Both will error.
    app.submit_yt_search("adele".into());
    app.overlay = Some(Overlay::Search {
        input: "adele".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adele".into()),
        searching: true,
    });
    app.submit_yt_search("adeles".into());
    app.overlay = Some(Overlay::Search {
        input: "adeles".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("adeles".into()),
        searching: true,
    });

    // Pump on_tick until both errors land in one drain cycle (the reader thread
    // buffers both). With the bug, searching would stay true forever (the
    // adeles error was dropped). Now it must clear (adeles's error matches).
    let mut searching_cleared = false;
    for _ in 0..100 {
        app.on_tick();
        if let Some(Overlay::Search {
            submitted,
            searching,
            ..
        }) = &app.overlay
        {
            if submitted.as_deref() == Some("adeles") && !*searching {
                searching_cleared = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        searching_cleared,
        "the current query (adeles) error must clear searching, not wedge"
    );
    // And the footer must show ADELES's error (the relevant one), not adele's.
    assert!(
        app.yt_error
            .as_deref()
            .is_some_and(|e| e.contains("adeles")),
        "footer shows adeles's error: {:?}",
        app.yt_error
    );
    let _ = std::fs::remove_file(&p);
}
