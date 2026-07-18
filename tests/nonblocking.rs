//! Slice 4 — non-blocking hot path tests.
//!
//! Verifies the three formerly-blocking sidecar roundtrips are now
//! fire-and-forget + `on_tick` drain (no UI freeze):
//!   1. `S` (discover) opens instantly — `home_suggestions` is async.
//!   2. Enter on a YT discover playlist — `get_playlist` is async.
//!   3. CONT=YouTube auto-advance — `get_watch_playlist` is async.
//!
//! Uses the same fake-sidecar pattern as `tests/e2e_yt.rs` (a per-test JSON
//! map of `{cmd -> canned response line}`, baked into the script so parallel
//! tests can't race on a shared env var).

use jukebox::source::TrackSource;
use jukebox::tui::app::{App, DiscoverItem, Overlay};
use jukebox::tui::queue::{ContinueMode, RepeatMode};
use jukebox::yt::session::Session;
use std::io::Write;

/// Write a per-test fake sidecar script + its map file. The map *path* is
/// baked into the script itself (no env var) so parallel `Session::spawn`
/// calls can't race on a shared JK_FAKE_MAP. Mirrors `e2e_yt::fake_sidecar`.
// `write_literal`: the fake sidecar body is a raw Python string containing
// `{`/`}` (JSON dicts); inlining it into the format string would re-interpret
// those braces as format args, so the literal stays as an arg.
#[allow(clippy::write_literal)]
fn fake_sidecar(map_json: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("nb-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("nb-fake-{}-{}.py", std::process::id(), n));
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
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        key = json.dumps({"ok": True, "data": {"resolve": {"url": "https://x/" + vid, "expires_at": None, "codec": "AAC", "abr": 256, "sample_rate": 48000, "container": "m4a", "premium": True}}})
    # Task 4: auto-respond to home/explore/charts with empty payloads so the
    # new on_tick fetch-on-first-visit (Home tab) doesn't block the pending
    # queue. See e2e_yt::fake_sidecar for the full rationale.
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

fn spawn_session(script: &std::path::Path) -> Session {
    Session::spawn(std::path::Path::new("python3"), script, None).unwrap()
}

/// Pump `on_tick` (with small sleeps for the sidecar reader thread to deliver
/// responses) until `cond(app)` is true, up to `max` iterations. Mirrors
/// `e2e_yt::tick_until`.
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

/// `S` in YouTube mode opens the discover overlay INSTANTLY — the old
/// `home_suggestions()` roundtrip blocked ~3s. Now `yt_discover_items`
/// fire-and-forgets `send_home_suggestions`, opens the overlay empty with
/// `discover_loading = true`, and `on_tick` populates the items when the
/// response lands.
#[test]
fn discover_opens_instantly_and_populates_on_tick() {
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD1\",\"name\":\"Focus\",\"count\":0},{\"id\":\"RD2\",\"name\":\"Chill\",\"count\":0}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    // Open discover — must return instantly (no blocking roundtrip). The
    // overlay opens with the generated Mix items (synchronous, from
    // reco::mixes — RC11-DEF-013) + discover_loading = true (the sidecar's
    // home_suggestions fetch is still in flight).
    app.open_discover();
    let (has_mix, has_playlist, loading) = match &app.overlay {
        Some(Overlay::Discover { items, .. }) => (
            items.iter().any(|d| matches!(d, DiscoverItem::Mix { .. })),
            items
                .iter()
                .any(|d| matches!(d, DiscoverItem::Playlist { .. })),
            app.discover_loading,
        ),
        other => panic!("expected Discover overlay, got {other:?}"),
    };
    assert!(
        loading,
        "discover should be loading (fire-and-forget in flight)"
    );
    assert!(
        has_mix,
        "RC11-DEF-013: overlay should open with generated Mix items (synchronous)"
    );
    assert!(
        !has_playlist,
        "Playlist items should NOT be present yet — they land on tick"
    );

    // Pump on_tick until the home_suggestions response lands + populates the
    // overlay. The discover overlay's Playlist items should now be present
    // (replacing the empty list) and discover_loading should be false.
    let populated = tick_until(&mut app, 100, |a| {
        matches!(
            &a.overlay,
            Some(Overlay::Discover { items, .. })
                if items.iter().any(|d| matches!(d, DiscoverItem::Playlist { .. }))
        ) && !a.discover_loading
    });
    assert!(
        populated,
        "on_tick should populate the discover overlay from pending_discover"
    );
    if let Some(Overlay::Discover { items, .. }) = &app.overlay {
        let pl: Vec<&DiscoverItem> = items
            .iter()
            .filter(|d| matches!(d, DiscoverItem::Playlist { .. }))
            .collect();
        assert!(
            pl.iter()
                .any(|d| matches!(d, DiscoverItem::Playlist { name, .. } if name == "Focus")),
            "Focus playlist should be in the discover items: {pl:?}"
        );
    }
    assert!(
        !app.discover_loading,
        "discover_loading should clear once items land"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// Enter on a YT discover playlist no longer blocks on `get_playlist` — it
/// fire-and-forgets `send_get_playlist` + records the intent in
/// `pending_discover_play`; `on_tick` starts playback when the tracks land.
#[test]
fn discover_playlist_selection_starts_playback_on_tick() {
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":2}]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"},{\"video_id\":\"v2\",\"title\":\"Other\",\"artist\":\"B\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    // Open discover + wait for the suggestions to land.
    app.open_discover();
    assert!(
        tick_until(&mut app, 100, |a| {
            matches!(
                &a.overlay,
                Some(Overlay::Discover { items, .. })
                    if items.iter().any(|d| matches!(d, DiscoverItem::Playlist { .. }))
            )
        }),
        "discover items should land"
    );

    // Enter on the first Playlist item. Must return instantly — the old
    // blocking get_playlist is gone. The intent is staged in
    // pending_discover_play; playback starts on a later tick.
    // RC11-DEF-013: navigate to the first Playlist item (Mix items come first
    // now, so cursor 0 is a Mix, not the sidecar's Playlist).
    if let Some(Overlay::Discover { items, cursor }) = &mut app.overlay {
        let pl_idx = items
            .iter()
            .position(|d| matches!(d, DiscoverItem::Playlist { .. }))
            .expect("test requires at least one Playlist item");
        *cursor = pl_idx;
    }
    app.play_discover_selection();
    assert_eq!(
        app.pending_discover_play,
        Some("PL1".into()),
        "Enter should stage the discover-play intent (non-blocking), not block"
    );
    assert!(
        app.now_playing.is_none(),
        "playback must NOT start synchronously — it starts when tracks land on tick"
    );

    // Pump on_tick until the playlist's tracks land + playback starts. The
    // first track (v1) resolves via the fake sidecar's resolve_url and the
    // cold-miss swap lands on a later tick.
    let played = tick_until(
        &mut app,
        200,
        |a| matches!(a.now_playing, Some(TrackSource::Remote { ref video_id }) if video_id == "v1"),
    );
    assert!(
        played,
        "discover selection should start playback of v1 on tick, got {:?}",
        app.now_playing
    );
    assert_eq!(
        app.pending_discover_play, None,
        "pending_discover_play should clear once playback starts"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// CONT=YouTube auto-advance no longer blocks on `get_watch_playlist` —
/// `next()` fire-and-forgets `send_watch_playlist` + records the seed in
/// `pending_radio_seed`; `on_tick` refills the RadioCursor + starts playback
/// when the response lands. The old track stays current until the next
/// track's id arrives (non-blocking).
#[test]
fn cont_youtube_auto_advance_non_blocking() {
    let wp = r#"{"get_watch_playlist":"{\"ok\":true,\"data\":{\"watch_playlist\":[{\"video_id\":\"yt1\",\"title\":\"A\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null},{\"video_id\":\"yt2\",\"title\":\"B\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null}]}}"}"#;
    let (script, map_file) = fake_sidecar(wp);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;

    // Play yt1 (cold miss → URL lands on tick).
    app.play_in_context_ids(vec!["yt1".into()], "yt1");
    assert!(
        tick_until(&mut app, 100, |a| a.now_playing.is_some()),
        "yt1 should resolve+play via the fake sidecar"
    );

    // Auto-advance: next() with the radio queue exhausted. Must NOT block —
    // the old `radio.advance(session, seed)` made a blocking roundtrip here.
    // Now it fire-and-forgets send_watch_playlist + stores the seed; the old
    // track stays current until on_tick drains pending_watch.
    app.next();
    assert_eq!(
        app.pending_radio_seed,
        Some("yt1".into()),
        "next() should stage the radio seed (non-blocking), not block on get_watch_playlist"
    );
    // The old track is still current immediately after next() — the context
    // switch + start_playback happen on tick when pending_watch lands.
    assert!(
        matches!(
            app.now_playing,
            Some(TrackSource::Remote { ref video_id }) if video_id == "yt1"
        ),
        "now_playing should still be yt1 immediately after next() (switch is on tick)"
    );

    // Pump on_tick until the radio advances to yt2 (the watch_playlist
    // response lands → RadioCursor refills, dropping the leading seed yt1 →
    // first track yt2 → cold-miss resolve → swap).
    let advanced = tick_until(&mut app, 100, |a| {
        matches!(
            a.now_playing,
            Some(TrackSource::Remote { ref video_id }) if video_id == "yt2"
        )
    });
    assert!(
        advanced,
        "CONT=YouTube should advance to yt2 on tick, got {:?}",
        app.now_playing
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// Regression: with RPT=all on a single-track context (the one the
/// CONT=YouTube radio continuation builds — `Context::Search { track_ids:
/// vec![vid] }`), `>` must still advance the radio, not replay the lone
/// track. Before the fix, `Transport::next()` wrapped the single-element
/// order and returned the same id, so `App::next()`'s radio-continuation
/// `None`-arm never fired and `load_track` reloaded the same track (replay).
/// The sibling test `cont_youtube_auto_advance_non_blocking` only passes
/// because it leaves repeat at the default Off.
#[test]
fn cont_youtube_next_advances_radio_with_repeat_all_single_track() {
    let wp = r#"{"get_watch_playlist":"{\"ok\":true,\"data\":{\"watch_playlist\":[{\"video_id\":\"yt1\",\"title\":\"A\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null},{\"video_id\":\"yt2\",\"title\":\"B\",\"artist\":\"X\",\"album\":null,\"dur\":null,\"isrc\":null}]}}"}"#;
    let (script, map_file) = fake_sidecar(wp);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.transport.continue_mode = ContinueMode::YouTube;
    // The user's setting from the report: RPT all. This is what diverges from
    // `cont_youtube_auto_advance_non_blocking` (repeat Off) and triggers the
    // wrap-to-same-track path in `Transport::next`.
    app.transport.set_repeat(RepeatMode::All);

    // Play yt1 (cold miss → URL lands on tick).
    app.play_in_context_ids(vec!["yt1".into()], "yt1");
    assert!(
        tick_until(&mut app, 100, |a| a.now_playing.is_some()),
        "yt1 should resolve+play via the fake sidecar"
    );

    // `>` on the single-track context with RPT=all: must stage the radio
    // seed (advance), NOT replay yt1. Before the fix, the wrap returned
    // Some("yt1") so the None-arm never ran and pending_radio_seed stayed
    // None.
    app.next();
    assert_eq!(
        app.pending_radio_seed,
        Some("yt1".into()),
        "next() with RPT=all on a single-track context should stage the \
         radio seed (advance), not replay the lone track"
    );

    // Pump on_tick until the radio advances to yt2.
    let advanced = tick_until(&mut app, 100, |a| {
        matches!(
            a.now_playing,
            Some(TrackSource::Remote { ref video_id }) if video_id == "yt2"
        )
    });
    assert!(
        advanced,
        "CONT=YouTube + RPT=all should advance to yt2 on tick, got {:?}",
        app.now_playing
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// Audio format switch doesn't block the input loop ≥100ms (AC-M9.2.4).
/// `set_output_format_async` spawns a background thread and returns
/// immediately, so a device-rate switch never freezes the TUI. The
/// synchronous `set_output_format` can take ~310ms (format-verify
/// polling + settle sleep); the async wrapper should return in <1ms
/// (thread spawn overhead only).
#[test]
fn audio_switch_does_not_block_input() {
    // The async variant returns immediately (the blocking work runs on
    // a spawned thread). Time the call — it must be well under 100ms.
    let start = std::time::Instant::now();
    let handle = jukebox::audio::set_output_format_async(96000, 24);
    let elapsed = start.elapsed();

    // Spawning a thread + returning should take <100ms (typically <1ms).
    // The actual format switch (up to 310ms on macOS) runs on the thread.
    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "set_output_format_async took {elapsed:?} — should return immediately (<100ms)"
    );

    // Wait for the background thread to finish (clean up — don't leave
    // orphaned threads that could interfere with other tests).
    let _ = handle.join();
}
