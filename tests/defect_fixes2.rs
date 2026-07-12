//! Regression tests for Fixer 2 defects from the RC-01 black-box review.
//!
//! Each test maps to a DEF-NNN ID in `docs/development/jukebox-release-loop/DEFECTS.md`.
//! Tests are pure key→action dispatch (no terminal) unless noted.

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::source::TrackSource;
use jukebox::tui::app::{App, DiscoverItem, Overlay, View, YtList, YtListKind};
use jukebox::yt::session::Session;
use std::io::Write;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn cat_album() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3)
        .map(|n| {
            serde_json::json!({
                "id": format!("t{n}"),
                "artists": ["40mP"],
                "primary_artist": "40mP",
                "title": format!("Song{n}"),
                "album": "Cosmic",
                "track_number": n,
                "bit_depth": 24,
                "sample_rate_hz": 96000,
                "source_path": format!("lossless/40mP/{n:02}.flac"),
                "symlinked_into_artists": ["40mP"],
            })
        })
        .collect();
    let json = serde_json::json!({
        "version": 1,
        "built_at": "x",
        "source_root": lossless.to_str().unwrap(),
        "tracks": tracks,
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn cat_collab() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::create_dir_all(lossless.join("B")).unwrap();
    for n in 1..=2 {
        let artist = ["A", "B"][n - 1];
        std::fs::write(lossless.join(artist).join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A","B"],"primary_artist":"A","title":"One","album":"Collaborations","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac","symlinked_into_artists":["A","B"]},
          {"id":"t2","artists":["A","B"],"primary_artist":"A","title":"Two","album":"Collaborations","track_number":2,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/B/02.flac","symlinked_into_artists":["A","B"]},
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-defect2-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

fn focus_track_col(app: &mut App) {
    app.view = View::Artists;
    app.focus_col = 2;
    app.cursors.artist = 0;
    app.cursors.album = 0;
    app.cursors.track = 0;
}

// --- Fake sidecar helpers (for YouTube tests) ---

#[allow(clippy::write_literal)]
fn fake_sidecar(map_json: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("nb2-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("nb2-fake-{}-{}.py", std::process::id(), n));
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

// ---------------------------------------------------------------------------
// DEF-007: Discover overlay Enter doesn't play generated mixes
// ---------------------------------------------------------------------------

#[test]
fn def007_discover_enter_shows_loading_status() {
    let _xdg = isolate_xdg();
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD1\",\"name\":\"Daily Mix\",\"count\":2}]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    app.open_discover();
    assert!(tick_until(&mut app, 100, |a| {
        matches!(
            &a.overlay,
            Some(Overlay::Discover { items, .. })
                if items.iter().any(|d| matches!(d, DiscoverItem::Playlist { .. }))
        )
    }));

    app.play_discover_selection();
    assert!(
        app.yt_status.is_some(),
        "DEF-007: Enter on a discover mix should show a loading status"
    );
    assert_eq!(
        app.pending_discover_play,
        Some("RD1".into()),
        "DEF-007: pending_discover_play should be staged"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn def007_discover_enter_empty_response_shows_error() {
    let _xdg = isolate_xdg();
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD1\",\"name\":\"Daily Mix\",\"count\":0}]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    app.open_discover();
    assert!(tick_until(&mut app, 100, |a| {
        matches!(
            &a.overlay,
            Some(Overlay::Discover { items, .. })
                if items.iter().any(|d| matches!(d, DiscoverItem::Playlist { .. }))
        )
    }));

    app.play_discover_selection();
    assert!(app.pending_discover_play.is_some());

    let errored = tick_until(&mut app, 100, |a| {
        a.pending_discover_play.is_none() && a.yt_status.is_some()
    });
    assert!(
        errored,
        "DEF-007: empty mix response should clear pending + show error status"
    );
    assert!(
        app.yt_status.is_some(),
        "DEF-007: error status should be set for empty mix"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

#[test]
fn def007_discover_enter_no_session_shows_error() {
    let _xdg = isolate_xdg();
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Playlist {
            id: "RD1".into(),
            name: "Daily Mix".into(),
        }],
        cursor: 0,
    });
    app.play_discover_selection();
    assert!(
        app.yt_status.is_some(),
        "DEF-007: no-session Enter should show an error status"
    );
    assert!(app.pending_discover_play.is_none());
}

#[test]
fn def007_discover_enter_success_closes_overlay() {
    let _xdg = isolate_xdg();
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":2}]}}","get_playlist":"{\"ok\":true,\"data\":{\"tracks\":[{\"video_id\":\"v1\",\"title\":\"Song\",\"artist\":\"A\"},{\"video_id\":\"v2\",\"title\":\"Other\",\"artist\":\"B\"}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    app.open_discover();
    assert!(tick_until(&mut app, 100, |a| {
        matches!(
            &a.overlay,
            Some(Overlay::Discover { items, .. })
                if items.iter().any(|d| matches!(d, DiscoverItem::Playlist { .. }))
        )
    }));

    app.play_discover_selection();
    let played = tick_until(&mut app, 200, |a| {
        a.overlay.is_none()
            && matches!(a.now_playing, Some(TrackSource::Remote { ref video_id }) if video_id == "v1")
    });
    assert!(
        played,
        "DEF-007: successful discover Enter should close overlay + start playback"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// DEF-008: YouTube track enqueue produces no visible result
// ---------------------------------------------------------------------------

#[test]
fn def008_enqueue_sets_status_message() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    focus_track_col(&mut app);
    assert!(app.yt_status.is_none());
    app.enqueue_selected();
    assert_eq!(app.transport.manual_queue, vec!["t1".to_string()]);
    assert!(
        app.yt_status.is_some(),
        "DEF-008: enqueue should set a status-bar confirmation"
    );
}

#[test]
fn def008_enqueue_no_track_sets_error_status() {
    let _xdg = isolate_xdg();
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.enqueue_selected();
    assert!(
        app.yt_status.is_some(),
        "DEF-008: enqueue with no track should show 'no track selected' status"
    );
    assert!(app.transport.manual_queue.is_empty());
}

#[test]
fn def008_enqueue_youtube_track_adds_to_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_lists.push(YtList {
        id: "PL1".into(),
        name: "Mix".into(),
        kind: YtListKind::Account,
        track_ids: vec!["v001".into(), "v002".into()],
    });
    app.cursors.playlist = 0;
    app.focus_col = 1;
    app.cursors.track = 0;
    app.enqueue_selected();
    assert_eq!(
        app.transport.manual_queue,
        vec!["v001".to_string()],
        "DEF-008: YouTube track should be enqueued"
    );
    assert!(
        app.yt_status.is_some(),
        "DEF-008: YouTube enqueue should set status feedback"
    );
}

// ---------------------------------------------------------------------------
// DEF-009: Local playback failure is completely silent
// ---------------------------------------------------------------------------

#[test]
fn def009_missing_file_sets_visible_status() {
    let _xdg = isolate_xdg();
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Y")).unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"ghost1","artists":["Y"],"primary_artist":"Y","title":"Ghost",
        "album":"Missing","bit_depth":16,"sample_rate_hz":44100,
        "source_path":"lossless/Y/01.flac","symlinked_into_artists":["Y"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["ghost1".into()], "ghost1");
    assert!(
        app.yt_status.is_some(),
        "DEF-009: missing-file playback failure should set yt_status (visible in footer)"
    );
    assert!(
        app.yt_error.is_some(),
        "DEF-009: missing-file should set yt_error for diagnostics overlay"
    );
    assert!(app.dead.contains("ghost1"));
}

#[test]
fn def009_playback_error_appears_in_diagnostics() {
    let _xdg = isolate_xdg();
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Z")).unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"ghost2","artists":["Z"],"primary_artist":"Z","title":"Phantom",
        "album":"Missing","bit_depth":16,"sample_rate_hz":44100,
        "source_path":"lossless/Z/01.flac","symlinked_into_artists":["Z"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["ghost2".into()], "ghost2");
    app.on_tick();
    let msgs = app.diagnostics.messages();
    assert!(
        msgs.iter().any(|m| m.contains("yt_error") && m.contains("ghost2") || m.contains("file not found")),
        "DEF-009: playback error should be captured in diagnostics, got {msgs:?}"
    );
}

// ---------------------------------------------------------------------------
// DEF-016: `x` removes an unexpected item
// ---------------------------------------------------------------------------

#[test]
fn def016_x_removes_highlighted_item_not_stale_marker() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    app.transport.enqueue("t3".into());
    app.view = View::Queue;
    app.cursors.queue = 2;
    app.cursors.track = 0;
    app.clamp_cursors();
    assert_eq!(
        app.cursors.track, app.cursors.queue,
        "DEF-016: clamp_cursors should sync cursors.track with cursors.queue in Queue view"
    );
    app.remove_selected_from_queue();
    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t2".to_string()],
        "DEF-016: x should remove the highlighted item (t3), not the stale-marker item"
    );
}

#[test]
fn def016_cursor_sync_on_navigation() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    app.transport.enqueue("t3".into());
    app.view = View::Queue;
    app.cursors.queue = 1;
    app.cursors.track = 99;
    app.clamp_cursors();
    assert_eq!(
        app.cursors.track, 1,
        "DEF-016: track cursor should match queue cursor after clamp"
    );
    app.remove_selected_from_queue();
    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t3".to_string()],
        "DEF-016: should remove t2 (the one at cursor)"
    );
}

// ---------------------------------------------------------------------------
// DEF-017: Next/prev track order is non-sequential
// ---------------------------------------------------------------------------

#[test]
fn def017_next_skips_dead_tracks_automatically() {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("X")).unwrap();
    std::fs::write(lossless.join("X").join("02.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"dead1","artists":["X"],"primary_artist":"X","title":"Gone","album":"A","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/01.flac","symlinked_into_artists":["X"]},
          {"id":"alive2","artists":["X"],"primary_artist":"X","title":"Here","album":"A","track_number":2,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/02.flac","symlinked_into_artists":["X"]},
          {"id":"dead3","artists":["X"],"primary_artist":"X","title":"AlsoGone","album":"A","track_number":3,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/03.flac","symlinked_into_artists":["X"]},
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(
        vec!["dead1".into(), "alive2".into(), "dead3".into()],
        "dead1",
    );
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("alive2"));
    app.next();
    assert!(
        app.now_playing.is_none(),
        "DEF-017: next after alive2 should skip dead3 + stop (context exhausted)"
    );
}

#[test]
fn def017_next_advances_sequentially_in_album_order() {
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into(), "t2".into(), "t3".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    app.next();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t2"),
        "DEF-017: next should advance to t2 (sequential order)"
    );
    app.next();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t3"),
        "DEF-017: next should advance to t3 (sequential order)"
    );
}

#[test]
fn def017_next_skips_dead_and_continues_sequentially() {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("X")).unwrap();
    std::fs::write(lossless.join("X").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("X").join("03.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"a1","artists":["X"],"primary_artist":"X","title":"One","album":"A","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/01.flac","symlinked_into_artists":["X"]},
          {"id":"d2","artists":["X"],"primary_artist":"X","title":"Two","album":"A","track_number":2,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/02.flac","symlinked_into_artists":["X"]},
          {"id":"a3","artists":["X"],"primary_artist":"X","title":"Three","album":"A","track_number":3,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/03.flac","symlinked_into_artists":["X"]},
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["a1".into(), "d2".into(), "a3".into()], "a1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("a1"));
    app.next();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("a3"),
        "DEF-017: next should skip dead d2 and advance to a3"
    );
}

// ---------------------------------------------------------------------------
// DEF-020: Quick Picks missing from discover overlay
// ---------------------------------------------------------------------------

#[test]
fn def020_discover_overlay_shows_all_six_suggestions() {
    let _xdg = isolate_xdg();
    let map = r#"{"home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[{\"id\":\"RD001\",\"name\":\"Daily Mix 1\",\"count\":0},{\"id\":\"RD002\",\"name\":\"Daily Mix 2\",\"count\":0},{\"id\":\"RD003\",\"name\":\"Daily Mix 3\",\"count\":0},{\"id\":\"RD004\",\"name\":\"Discover Mix\",\"count\":0},{\"id\":\"RD005\",\"name\":\"Rediscover\",\"count\":0},{\"id\":\"RD006\",\"name\":\"Quick Picks\",\"count\":0}]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = spawn_session(&script);
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.source_mode = jukebox::mode::SourceMode::Youtube;

    app.open_discover();
    let populated = tick_until(&mut app, 100, |a| {
        if let Some(Overlay::Discover { items, .. }) = &a.overlay {
            items
                .iter()
                .filter(|d| matches!(d, DiscoverItem::Playlist { .. }))
                .count()
                >= 6
        } else {
            false
        }
    });
    assert!(
        populated,
        "DEF-020: discover overlay should show all 6 home suggestions (including Quick Picks)"
    );
    if let Some(Overlay::Discover { items, .. }) = &app.overlay {
        let names: Vec<&str> = items
            .iter()
            .filter_map(|d| match d {
                DiscoverItem::Playlist { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"Quick Picks"),
            "DEF-020: Quick Picks (RD006) should be in the discover overlay, got {names:?}"
        );
    }

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

// ---------------------------------------------------------------------------
// DEF-022: Discover overlay shows duplicate entry
// ---------------------------------------------------------------------------

#[test]
fn def022_local_smart_albums_no_duplicates_for_collab() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_collab();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.open_discover();
    let items = match &app.overlay {
        Some(Overlay::Discover { items, .. }) => items.clone(),
        _ => panic!("expected Discover overlay"),
    };
    let collab_items: Vec<&DiscoverItem> = items
        .iter()
        .filter(|d| matches!(d, DiscoverItem::Album { album, .. } if album == "Collaborations"))
        .collect();
    assert_eq!(
        collab_items.len(),
        1,
        "DEF-022: collaboration album should appear once in discover, not {} times",
        collab_items.len()
    );
}

// ---------------------------------------------------------------------------
// DEF-027: `x` clears ALL queue items instead of just the selected one
// ---------------------------------------------------------------------------
// The queue is value-addressed: `remove_from_queue(&id)` uses
// `retain(|x| x != id)`, which strips EVERY occurrence of the id. A queue
// with the same track enqueued N times (the RC-02 scenario: `e` pressed 3×
// on the same track) was wiped by a single `x`. Index-based removal deletes
// exactly the entry the cursor points at.

#[test]
fn def027_x_removes_only_one_duplicate_from_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Enqueue the SAME track three times (the RC-02 reproduction scenario).
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t1".into());
    app.view = View::Queue;
    app.cursors.queue = 0;
    app.clamp_cursors();

    app.remove_selected_from_queue();

    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t1".to_string()],
        "DEF-027: x should remove exactly ONE entry (the one at the cursor), \
         not all duplicates of the same track"
    );
    assert_eq!(
        app.cursors.queue, 0,
        "DEF-027: cursor should stay valid at index 0 after removing the first of 3"
    );
}

#[test]
fn def027_x_removes_one_from_middle_of_duplicate_queue() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Mixed queue: t1, t2, t1 — cursor on the second t1 (index 2).
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    app.transport.enqueue("t1".into());
    app.view = View::Queue;
    app.cursors.queue = 2;
    app.clamp_cursors();

    app.remove_selected_from_queue();

    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string(), "t2".to_string()],
        "DEF-027: x should remove only the entry at cursor 2, leaving t1 and t2"
    );
}

#[test]
fn def027_x_removes_last_item_and_adjusts_cursor() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t1".into());
    app.view = View::Queue;
    app.cursors.queue = 1;
    app.clamp_cursors();

    app.remove_selected_from_queue();

    assert_eq!(
        app.transport.manual_queue,
        vec!["t1".to_string()],
        "DEF-027: only the entry at cursor 1 should be removed"
    );
    assert_eq!(
        app.cursors.queue, 0,
        "DEF-027: cursor should step back to 0 after removing the last item"
    );
}

#[test]
fn def027_x_on_empty_queue_is_a_noop() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Queue;
    app.cursors.queue = 0;
    app.clamp_cursors();

    app.remove_selected_from_queue();

    assert!(
        app.transport.manual_queue.is_empty(),
        "DEF-027: x on an empty queue should be a no-op"
    );
}

// ---------------------------------------------------------------------------
// DEF-028: Discover overlay stays open after Enter error
// ---------------------------------------------------------------------------
// `play_discover_selection` never closed the overlay. On the error paths
// (empty album, no YouTube session, empty mix response) the overlay stayed
// open and the user had to manually press Escape. The fix closes the overlay
// on Enter in all cases; errors surface via yt_status in the footer.

#[test]
fn def028_enter_on_album_with_no_tracks_closes_overlay() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // An album that doesn't exist in the catalog → tracks_for_album is empty.
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Album {
            artist: "Ghost".into(),
            album: "No Such Album".into(),
        }],
        cursor: 0,
    });

    app.play_discover_selection();

    assert!(
        app.overlay.is_none(),
        "DEF-028: overlay should close after Enter on an album with no tracks"
    );
    assert!(
        app.yt_status.is_some(),
        "DEF-028: an error status should be surfaced for the empty album"
    );
}

#[test]
fn def028_enter_on_mix_with_no_session_closes_overlay() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Playlist {
            id: "RD1".into(),
            name: "Daily Mix".into(),
        }],
        cursor: 0,
    });

    app.play_discover_selection();

    assert!(
        app.overlay.is_none(),
        "DEF-028: overlay should close after Enter on a mix with no YouTube session"
    );
    assert!(
        app.yt_status.is_some(),
        "DEF-028: an error status should be surfaced for the no-session case"
    );
    assert!(
        app.pending_discover_play.is_none(),
        "DEF-028: no pending play should be staged without a session"
    );
}

#[test]
fn def028_enter_on_valid_album_closes_overlay() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // "Cosmic" is the album in cat_album (3 tracks).
    app.overlay = Some(Overlay::Discover {
        items: vec![DiscoverItem::Album {
            artist: "40mP".into(),
            album: "Cosmic".into(),
        }],
        cursor: 0,
    });

    app.play_discover_selection();

    assert!(
        app.overlay.is_none(),
        "DEF-028: overlay should close after Enter on a valid album"
    );
    assert!(
        app.now_playing.is_some(),
        "DEF-028: a valid album should start playback"
    );
}
