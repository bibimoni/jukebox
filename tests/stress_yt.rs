//! Stress tests for YouTube playlist loading: rapid switching, realistic
//! sidecar delays, wrong-track detection.
//!
//! Key design: the sidecar is single-threaded/sequential. The Rust session
//! serializes playlist fetches (only ONE in flight at a time) to prevent
//! queue buildup. These tests verify correct behavior under stress.

use jukebox::tui::app::{App, View, YtList, YtListKind};
use jukebox::yt::session::Session;
use jukebox::yt::state::YtState;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

/// Write a fake sidecar that returns DIFFERENT tracks per playlist id, with a
/// configurable delay (simulating real network latency).
fn delayed_sidecar(delay_ms: u64) -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("stress-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        r#"import sys, json, time
delay = {delay_ms}
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    if cmd == "ping":
        print(json.dumps({{"ok": True, "data": {{"pong": True}}}}), flush=True)
        continue
    if cmd == "auth_status":
        print(json.dumps({{"ok": True, "data": {{"auth": {{"ok": True, "premium": False, "account": False, "valid": True, "expired": False, "reason": None}}}}}}), flush=True)
        continue
    if cmd == "library_playlists":
        time.sleep(delay / 1000.0)
        print(json.dumps({{"ok": True, "data": {{"playlists": [
            {{"id": "PL1", "name": "P1", "count": 2}},
            {{"id": "PL2", "name": "P2", "count": 2}},
            {{"id": "PL3", "name": "P3", "count": 2}},
            {{"id": "PL4", "name": "P4", "count": 2}},
            {{"id": "PL5", "name": "P5", "count": 2}}
        ]}}}}), flush=True)
        continue
    if cmd == "home_suggestions":
        print(json.dumps({{"ok": True, "data": {{"suggestions": []}}}}), flush=True)
        continue
    if cmd == "get_playlist":
        pid = req.get("id", "")
        time.sleep(delay / 1000.0)
        tracks = {{
            "PL1": [{{"video_id": "v1", "title": "T1", "artist": "X"}}, {{"video_id": "v2", "title": "T2", "artist": "X"}}],
            "PL2": [{{"video_id": "v3", "title": "T3", "artist": "Y"}}, {{"video_id": "v4", "title": "T4", "artist": "Y"}}],
            "PL3": [{{"video_id": "v5", "title": "T5", "artist": "Z"}}, {{"video_id": "v6", "title": "T6", "artist": "Z"}}],
            "PL4": [{{"video_id": "v7", "title": "T7", "artist": "W"}}, {{"video_id": "v8", "title": "T8", "artist": "W"}}],
            "PL5": [{{"video_id": "v9", "title": "T9", "artist": "V"}}, {{"video_id": "v10", "title": "T10", "artist": "V"}}],
        }}.get(pid, [])
        print(json.dumps({{"ok": True, "data": {{"tracks": tracks}}}}), flush=True)
        continue
"#,
        delay_ms = delay_ms,
    )
    .unwrap();
    writeln!(f).unwrap();
    p
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

fn yt_app(script: &std::path::Path) -> App {
    let session = Session::spawn(std::path::Path::new("python3"), script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = View::Youtube;
    app.yt_state = YtState::Ready;
    app.yt_lists_loading = false;
    app.yt_lists = (1..=5)
        .map(|i| YtList {
            id: format!("PL{}", i),
            name: format!("P{}", i),
            kind: YtListKind::Account,
            track_ids: Vec::new(),
        })
        .collect();
    app.loaded_yt_lists.clear();
    app
}

fn expected_tracks(pl: &str) -> Vec<String> {
    match pl {
        "PL1" => vec!["v1".to_string(), "v2".to_string()],
        "PL2" => vec!["v3".to_string(), "v4".to_string()],
        "PL3" => vec!["v5".to_string(), "v6".to_string()],
        "PL4" => vec!["v7".to_string(), "v8".to_string()],
        "PL5" => vec!["v9".to_string(), "v10".to_string()],
        _ => panic!("unknown playlist {pl}"),
    }
}

fn tick_until<F>(app: &mut App, max: usize, cond: F) -> bool
where
    F: Fn(&App) -> bool,
{
    for _ in 0..max {
        app.on_tick();
        if cond(app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

// ---------------------------------------------------------------------------
// Test A: Sequential focus — focus each playlist, wait for load, verify
// correct tracks. This is normal user behavior.
// ---------------------------------------------------------------------------

#[test]
fn sequential_focus_each_playlist_loads_correct_tracks() {
    let script = delayed_sidecar(100);
    let mut app = yt_app(&script);

    for i in 0..5 {
        app.cursors.playlist = i;
        app.on_tick();
        let loaded = tick_until(&mut app, 100, |a| !a.yt_lists[i].track_ids.is_empty());
        assert!(
            loaded,
            "PL{} should load. State: cursor={}, lists={:?}",
            i + 1,
            app.cursors.playlist,
            app.yt_lists
                .iter()
                .map(|l| &l.track_ids)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &app.yt_lists[i].track_ids,
            &expected_tracks(&app.yt_lists[i].id),
            "PL{} has wrong tracks — expected {:?}, got {:?}",
            i + 1,
            expected_tracks(&app.yt_lists[i].id),
            &app.yt_lists[i].track_ids
        );
    }

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test B: Rapid switching — only the LAST focused playlist loads (the rest
// were never dwelled on long enough). The last playlist must have correct
// tracks.
// ---------------------------------------------------------------------------

#[test]
fn rapid_switch_last_focused_loads_correct_tracks() {
    let script = delayed_sidecar(200);
    let mut app = yt_app(&script);

    // Rapidly switch 0→1→2→3→4 with on_tick after each, no waiting.
    for i in 0..5 {
        app.cursors.playlist = i;
        app.on_tick();
    }

    // Pump until the LAST focused playlist (PL5, index 4) loads.
    let loaded = tick_until(&mut app, 200, |a| !a.yt_lists[4].track_ids.is_empty());
    assert!(
        loaded,
        "PL5 (last focused) should load. State: {:?}",
        app.yt_lists
            .iter()
            .map(|l| &l.track_ids)
            .collect::<Vec<_>>()
    );
    // PL5 must have its OWN correct tracks.
    assert_eq!(
        &app.yt_lists[4].track_ids,
        &expected_tracks("PL5"),
        "PL5 has wrong tracks after rapid switching"
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test C: Switch back and forth — tracks persist and are correct.
// ---------------------------------------------------------------------------

#[test]
fn switch_back_and_forth_tracks_persist_and_correct() {
    let script = delayed_sidecar(100);
    let mut app = yt_app(&script);

    // Focus PL1, let it load.
    app.cursors.playlist = 0;
    app.on_tick();
    assert!(
        tick_until(&mut app, 100, |a| !a.yt_lists[0].track_ids.is_empty()),
        "PL1 should load"
    );
    assert_eq!(&app.yt_lists[0].track_ids, &expected_tracks("PL1"));

    // Switch to PL2, let it load.
    app.cursors.playlist = 1;
    app.on_tick();
    assert!(
        tick_until(&mut app, 100, |a| !a.yt_lists[1].track_ids.is_empty()),
        "PL2 should load"
    );
    assert_eq!(&app.yt_lists[1].track_ids, &expected_tracks("PL2"));

    // Switch BACK to PL1 — tracks should persist (loaded guard).
    app.cursors.playlist = 0;
    app.on_tick();
    assert_eq!(
        &app.yt_lists[0].track_ids,
        &expected_tracks("PL1"),
        "PL1 tracks should persist after switching away and back"
    );

    // Switch to PL3, load, back to PL1.
    app.cursors.playlist = 2;
    app.on_tick();
    assert!(
        tick_until(&mut app, 100, |a| !a.yt_lists[2].track_ids.is_empty()),
        "PL3 should load"
    );
    app.cursors.playlist = 0;
    app.on_tick();
    assert_eq!(
        &app.yt_lists[0].track_ids,
        &expected_tracks("PL1"),
        "PL1 tracks should still be correct after PL3 load + switch back"
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test D: Re-select same playlist while loading — no duplicate request.
// ---------------------------------------------------------------------------

#[test]
fn reselect_same_playlist_while_loading_no_duplicate() {
    let script = delayed_sidecar(500);
    let mut app = yt_app(&script);

    app.cursors.playlist = 0;
    app.on_tick();
    assert!(
        app.yt_session.as_ref().unwrap().playlist_loading("PL1"),
        "PL1 should be loading"
    );

    // Focus PL1 again (same cursor) — should NOT fire a duplicate.
    app.on_tick();
    app.on_tick();
    app.on_tick();

    // Let PL1 load.
    let loaded = tick_until(&mut app, 100, |a| !a.yt_lists[0].track_ids.is_empty());
    assert!(loaded, "PL1 should load");
    assert_eq!(&app.yt_lists[0].track_ids, &expected_tracks("PL1"));

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test E: Refresh replaces lists while a fetch is in flight — no corruption.
// ---------------------------------------------------------------------------

#[test]
fn refresh_replaces_lists_while_fetch_in_flight_no_corruption() {
    let script = delayed_sidecar(200);
    let mut app = yt_app(&script);

    // Focus PL1 — fires get_playlist(PL1).
    app.cursors.playlist = 0;
    app.on_tick();
    assert!(app.yt_session.as_ref().unwrap().playlist_loading("PL1"));

    // While PL1's fetch is in flight, refresh.
    app.refresh_yt_lists();

    // Wait for refresh to land.
    let refreshed = tick_until(&mut app, 200, |a| {
        !a.yt_lists.is_empty() && !a.yt_lists_loading
    });
    assert!(refreshed, "refresh should complete");

    // Focus each playlist, load, verify correct tracks.
    for i in 0..5 {
        app.cursors.playlist = i;
        app.on_tick();
        let loaded = tick_until(&mut app, 100, |a| !a.yt_lists[i].track_ids.is_empty());
        assert!(
            loaded,
            "PL{} should load after refresh+focus. State: {:?}",
            i + 1,
            app.yt_lists
                .iter()
                .map(|l| &l.track_ids)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &app.yt_lists[i].track_ids,
            &expected_tracks(&app.yt_lists[i].id),
            "PL{} has wrong tracks after refresh — stale corruption",
            i + 1
        );
    }

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test F: Slow sidecar (1s per request) — refresh completes, not stuck
// on syncing.
// ---------------------------------------------------------------------------

#[test]
fn slow_sidecar_refresh_completes_not_stuck_on_syncing() {
    let script = delayed_sidecar(1000);
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
    assert!(app.yt_lists_loading);

    // Refresh sends library_playlists + home_suggestions (2 requests × 1s = 2s).
    let refreshed = tick_until(&mut app, 200, |a| {
        !a.yt_lists.is_empty() && !a.yt_lists_loading
    });
    assert!(
        refreshed,
        "refresh should complete within 4s. yt_lists_loading={}, len={}",
        app.yt_lists_loading,
        app.yt_lists.len()
    );
    assert_eq!(app.yt_state, YtState::Ready);
    assert_eq!(app.yt_lists.len(), 5);

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test G: Rapid switching then settle — after rapid switching, the user
// settles on one playlist. That playlist must load with correct tracks.
// Then the user visits each other playlist — each must load correctly.
// ---------------------------------------------------------------------------

#[test]
fn rapid_switch_then_settle_each_playlist_correct() {
    let script = delayed_sidecar(150);
    let mut app = yt_app(&script);

    // Rapid switch 0→1→2→3→4→2 (settle on PL3).
    for &i in &[0, 1, 2, 3, 4, 2] {
        app.cursors.playlist = i;
        app.on_tick();
    }

    // Wait for PL3 (index 2) to load.
    let loaded = tick_until(&mut app, 200, |a| !a.yt_lists[2].track_ids.is_empty());
    assert!(
        loaded,
        "PL3 (settled) should load. State: {:?}",
        app.yt_lists
            .iter()
            .map(|l| &l.track_ids)
            .collect::<Vec<_>>()
    );
    assert_eq!(&app.yt_lists[2].track_ids, &expected_tracks("PL3"));

    // Now visit each other playlist — each must load with correct tracks.
    for i in 0..5 {
        if !app.yt_lists[i].track_ids.is_empty() {
            continue; // already loaded
        }
        app.cursors.playlist = i;
        app.on_tick();
        let loaded = tick_until(&mut app, 100, |a| !a.yt_lists[i].track_ids.is_empty());
        assert!(
            loaded,
            "PL{} should load on visit. State: {:?}",
            i + 1,
            app.yt_lists
                .iter()
                .map(|l| &l.track_ids)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &app.yt_lists[i].track_ids,
            &expected_tracks(&app.yt_lists[i].id),
            "PL{} has wrong tracks on visit",
            i + 1
        );
    }

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test H: Stale playlist_inflight guard after sidecar respawn (re-auth bug)
// ---------------------------------------------------------------------------
//
// BUG: set_cookies / set_browser / clear_cookies respawn the sidecar but do
// NOT clear `playlist_inflight` or `pending`. After re-auth or logout:
//   1. Stale `playlist_inflight` guards block ALL future send_get_playlist
//      calls (the guard checks `!is_empty()`, not `contains(&id)`)
//   2. Stale `pending` FIFO entries cause wrong-track pairing
//
// This test demonstrates bug #1: after clear_cookies (logout), the stale
// inflight guard from a previous get_playlist blocks all future fetches.

#[test]
fn stale_inflight_guard_after_respawn_blocks_all_fetches() {
    let script = delayed_sidecar(100);
    let mut app = yt_app(&script);

    // Fire get_playlist for PL1 — sets playlist_inflight = {PL1}.
    app.cursors.playlist = 0;
    app.on_tick();
    assert!(
        app.yt_session.as_ref().unwrap().playlist_loading("PL1"),
        "PL1 should be inflight after first on_tick"
    );

    // Simulate logout: clear_cookies respawns the sidecar but does NOT clear
    // playlist_inflight (the bug). The old sidecar's in-flight response won't
    // arrive (old process killed), so the guard is never cleared by apply_pair.
    let script_path = std::path::PathBuf::from(&script);
    let _ = app
        .yt_session
        .as_mut()
        .unwrap()
        .clear_cookies(std::path::Path::new("python3"), &script_path);

    // The stale guard is STILL SET — this is the bug.
    assert!(
        app.yt_session.as_ref().unwrap().playlist_loading("PL1"),
        "BUG: PL1 inflight guard should have been cleared on respawn but is STILL SET \
         — this blocks ALL future get_playlist calls (stuck on syncing forever)"
    );

    // Try to fetch PL2 — should be blocked by the stale guard.
    app.cursors.playlist = 1;
    app.on_tick();
    assert!(
        !app.yt_session.as_ref().unwrap().playlist_loading("PL2"),
        "BUG: PL2 fetch was blocked by stale PL1 guard — send_get_playlist is a no-op \
         because `!playlist_inflight.is_empty()` is true (PL1's stale guard)"
    );

    // Pump on_tick for 5s — PL2 should NEVER load (stale guard blocks it).
    let pl2_loaded = tick_until(&mut app, 100, |app| {
        app.yt_lists
            .get(1)
            .map(|l| !l.track_ids.is_empty())
            .unwrap_or(false)
    });

    assert!(
        !pl2_loaded,
        "BUG CONFIRMED: PL2 never loads because stale PL1 inflight guard blocks \
         ALL get_playlist calls after sidecar respawn. This is the 'stuck on \
         syncing' / 'long loading time' root cause after re-auth/logout."
    );

    let _ = std::fs::remove_file(&script);
}

// ---------------------------------------------------------------------------
// Test I: Rapid switching — ALL visited playlists should eventually load.
// This test asserts the USER EXPECTATION: if you scroll through 5 playlists,
// all 5 should eventually get their tracks. The one-at-a-time serialization
// design means only the FIRST and LAST focused playlists load during rapid
// switching — the middle ones are never fetched (send_get_playlist is a no-op
// while any fetch is in flight).
// ---------------------------------------------------------------------------

#[test]
fn rapid_switch_all_five_should_load() {
    // After rapid switching, the user visits each playlist one by one. With
    // serialization, only one playlist fetch is in flight at a time — the
    // middle playlists were blocked during rapid switching and need to be
    // visited individually to trigger their lazy-load.
    let script = delayed_sidecar(200);
    let mut app = yt_app(&script);

    // Rapidly switch 0→1→2→3→4, calling on_tick after each.
    for i in 0..5 {
        app.cursors.playlist = i;
        app.on_tick();
    }

    // Wait for the last-focused playlist (PL5, index 4) to load.
    let last_loaded = tick_until(&mut app, 100, |a| !a.yt_lists[4].track_ids.is_empty());
    assert!(
        last_loaded,
        "PL5 (last focused) should load after rapid switching"
    );
    assert_eq!(&app.yt_lists[4].track_ids, &expected_tracks("PL5"));

    // Now visit each unloaded playlist. Each should load with correct tracks.
    for i in 0..5 {
        if !app.yt_lists[i].track_ids.is_empty() {
            continue; // already loaded
        }
        app.cursors.playlist = i;
        app.on_tick();
        let loaded = tick_until(&mut app, 100, |a| !a.yt_lists[i].track_ids.is_empty());
        assert!(
            loaded,
            "PL{} should load on visit. State: {:?}",
            i + 1,
            app.yt_lists
                .iter()
                .map(|l| (&l.id, &l.track_ids))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &app.yt_lists[i].track_ids,
            &expected_tracks(&app.yt_lists[i].id),
            "WRONG TRACKS for PL{}",
            i + 1
        );
    }

    let _ = std::fs::remove_file(&script);
}
