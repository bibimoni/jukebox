use jukebox::player::{Player, StubPlayer};
use std::path::Path;

#[test]
fn stub_player_records_loads() {
    let mut p = StubPlayer::default();
    p.load(std::path::Path::new("/x.flac")).unwrap();
    assert_eq!(p.loaded(), Some(std::path::PathBuf::from("/x.flac")));
    assert!(p.is_playing(), "load starts playback");
    p.play_pause().unwrap();
    assert!(!p.is_playing(), "play_pause toggles to paused");
}

/// A Player that records the last volume/mute it was told to apply. Proves
/// App's volume_up/down/mute actually reach the backend (mpv path).
#[derive(Default)]
struct RecordingPlayer {
    volume: Option<u8>,
    muted: Option<bool>,
}
impl Player for RecordingPlayer {
    fn load(&mut self, _: &Path) -> anyhow::Result<()> {
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        None
    }
    fn duration(&self) -> Option<f64> {
        None
    }
    fn is_playing(&self) -> bool {
        true
    }
    fn set_volume(&mut self, vol: u8) -> anyhow::Result<()> {
        self.volume = Some(vol);
        Ok(())
    }
    fn set_muted(&mut self, m: bool) -> anyhow::Result<()> {
        self.muted = Some(m);
        Ok(())
    }
}

#[test]
fn app_volume_up_reaches_player() {
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;
    let d = tempfile::tempdir().unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":"/tmp","tracks":[
      {"id":"t1","artists":["A"],"primary_artist":"A","title":"x","bit_depth":16,"sample_rate_hz":44100,"source_path":"x","symlinked_into_artists":["A"]}
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let rec = std::cell::RefCell::new(RecordingPlayer::default());
    // Box the recorder; we read its captured state after the calls.
    let rec_handle = std::rc::Rc::new(rec);
    let player: Box<dyn Player> = Box::new(RecProxy(rec_handle.clone()));
    let mut app = App::new(cat, player, None, None);
    app.volume = 50;
    app.volume_up();
    assert_eq!(app.volume, 55);
    assert_eq!(
        rec_handle.borrow().volume,
        Some(55),
        "volume_up must push to player"
    );
    assert_eq!(
        rec_handle.borrow().muted,
        Some(false),
        "volume_up unmutes via player"
    );
    app.toggle_mute();
    assert_eq!(
        rec_handle.borrow().muted,
        Some(true),
        "toggle_mute must push to player"
    );
}

#[test]
fn app_set_volume_reaches_player() {
    // The mouse volume path used to mutate App.volume directly without
    // pushing to the player — so the bar moved but audio stayed at the old
    // level until a keypress re-synced (the "mouse resets to 100%" bug).
    // set_volume must push to the player immediately.
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;
    let d = tempfile::tempdir().unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":"/tmp","tracks":[
      {"id":"t1","artists":["A"],"primary_artist":"A","title":"x","bit_depth":16,"sample_rate_hz":44100,"source_path":"x","symlinked_into_artists":["A"]}
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let rec_handle = std::rc::Rc::new(std::cell::RefCell::new(RecordingPlayer::default()));
    let player: Box<dyn Player> = Box::new(RecProxy(rec_handle.clone()));
    let mut app = App::new(cat, player, None, None);
    app.volume = 100;
    app.set_volume(33);
    assert_eq!(app.volume, 33, "set_volume sets the absolute value");
    assert_eq!(
        rec_handle.borrow().volume,
        Some(33),
        "set_volume must push to player"
    );
    assert_eq!(
        rec_handle.borrow().muted,
        Some(false),
        "set_volume unmutes via player"
    );
    // Values >100 clamp to 100.
    app.set_volume(150);
    assert_eq!(app.volume, 100);
    assert_eq!(rec_handle.borrow().volume, Some(100));
}

// Tiny proxy so we can share the recorder with App while App owns the Box.
struct RecProxy(std::rc::Rc<std::cell::RefCell<RecordingPlayer>>);
impl Player for RecProxy {
    fn load(&mut self, p: &Path) -> anyhow::Result<()> {
        self.0.borrow_mut().load(p)
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        self.0.borrow_mut().play_pause()
    }
    fn seek(&mut self, s: f64) -> anyhow::Result<()> {
        self.0.borrow_mut().seek(s)
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        self.0.borrow_mut().stop()
    }
    fn position(&self) -> Option<f64> {
        self.0.borrow().position()
    }
    fn duration(&self) -> Option<f64> {
        self.0.borrow().duration()
    }
    fn is_playing(&self) -> bool {
        self.0.borrow().is_playing()
    }
    fn set_volume(&mut self, v: u8) -> anyhow::Result<()> {
        self.0.borrow_mut().set_volume(v)
    }
    fn set_muted(&mut self, m: bool) -> anyhow::Result<()> {
        self.0.borrow_mut().set_muted(m)
    }
}

// ---------------------------------------------------------------------------
// RB-5: Playback can become unrecoverable after completion
// ---------------------------------------------------------------------------

/// A stub player whose `track_ended()` returns true after the first load
/// (simulating natural completion), and whose `load()` fails with a
/// broken-pipe-style error on the 2nd call (the first auto-advance after
/// eof — simulating a dead mpv socket). Subsequent loads succeed
/// (simulating respawn recovery or a live track). This lets us test that
/// the App's auto-advance loop skips the broken-pipe track and continues
/// to the next playable track instead of getting stuck.
#[derive(Default)]
struct BrokenPipeAfterEof {
    loads: u32,
    ended_fired: bool,
}
impl Player for BrokenPipeAfterEof {
    fn load(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.loads += 1;
        self.ended_fired = false;
        if self.loads == 2 {
            return Err(anyhow::anyhow!("Broken pipe (os error 32)"));
        }
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        None
    }
    fn duration(&self) -> Option<f64> {
        None
    }
    fn is_playing(&self) -> bool {
        true
    }
    fn track_ended(&mut self) -> bool {
        if !self.ended_fired && self.loads >= 1 {
            self.ended_fired = true;
            return true;
        }
        false
    }
}

/// RB-5: after natural completion, a broken-pipe error on the next track
/// must not leave playback stuck — the dead track is skipped and auto-
/// advance continues to the next playable track. Before the fix, every
/// subsequent track would fail with "Broken pipe (os error 32)" because the
/// dead mpv socket was never respawned.
#[test]
fn rb5_next_after_eof_skips_broken_pipe_track() {
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("a")).unwrap();
    std::fs::create_dir_all(lossless.join("b")).unwrap();
    std::fs::create_dir_all(lossless.join("c")).unwrap();
    std::fs::write(lossless.join("a/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("b/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("c/01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A"],"primary_artist":"A","title":"Track1",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/a/01.flac","symlinked_into_artists":["A"]},
          {"id":"t2","artists":["B"],"primary_artist":"B","title":"Track2",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["B"]},
          {"id":"t3","artists":["C"],"primary_artist":"C","title":"Track3",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/c/01.flac","symlinked_into_artists":["C"]},
        ]
    })
    .to_string();
    let p = root.join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let player: Box<dyn Player> = Box::new(BrokenPipeAfterEof::default());
    let mut app = App::new(cat, player, None, None);

    // Play t1 of a 3-track context.
    app.play_in_context_ids(vec!["t1".into(), "t2".into(), "t3".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));

    // Simulate natural completion → on_track_ended → next.
    assert!(
        app.player.track_ended(),
        "stub should report end after first load"
    );
    app.on_track_ended();

    // t2's load fails with broken pipe → marked dead → skip to t3.
    assert!(
        app.dead.contains("t2"),
        "RB-5: t2 must be marked dead after broken pipe"
    );
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t3"),
        "RB-5: auto-advance must skip the broken-pipe track and continue to t3"
    );
}

/// A player whose `track_ended()` returns true after the first load, and
/// whose `load()` always succeeds. Used to test the happy-path auto-advance
/// after natural completion (no broken pipe).
#[derive(Default)]
struct EndAfterFirst {
    loads: u32,
    ended_fired: bool,
}
impl Player for EndAfterFirst {
    fn load(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.loads += 1;
        self.ended_fired = false;
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        None
    }
    fn duration(&self) -> Option<f64> {
        None
    }
    fn is_playing(&self) -> bool {
        true
    }
    fn track_ended(&mut self) -> bool {
        if !self.ended_fired && self.loads >= 1 {
            self.ended_fired = true;
            return true;
        }
        false
    }
}

/// RB-5: the happy path — `on_track_ended` → `next()` → `load_track()`
/// after natural completion must advance to the next track without any
/// broken-pipe error. This is the positive control for the fix.
#[test]
fn rb5_on_track_ended_advances_happy_path() {
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("a")).unwrap();
    std::fs::create_dir_all(lossless.join("b")).unwrap();
    std::fs::write(lossless.join("a/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("b/01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A"],"primary_artist":"A","title":"Track1",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/a/01.flac","symlinked_into_artists":["A"]},
          {"id":"t2","artists":["B"],"primary_artist":"B","title":"Track2",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["B"]},
        ]
    })
    .to_string();
    let p = root.join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let player: Box<dyn Player> = Box::new(EndAfterFirst::default());
    let mut app = App::new(cat, player, None, None);

    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));

    // Natural completion → auto-advance.
    assert!(
        app.player.track_ended(),
        "should report end after first load"
    );
    app.on_track_ended();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t2"),
        "RB-5: on_track_ended must advance to t2 after natural completion"
    );
}

// ---------------------------------------------------------------------------
// F2: now_playing not cleared when a player.load() fails
// ---------------------------------------------------------------------------

/// A Player whose `load` succeeds on the first call and fails on the second
/// (simulating a dead mpv socket / spawn failure mid-session). Subsequent
/// loads succeed again (simulating respawn recovery). Used to verify that a
/// failed `play_selected` after a track was already playing clears
/// `now_playing` — the player backend killed the old child during the failed
/// load, so the UI must not show a stale track.
#[derive(Default)]
struct FailOnSecondLoad {
    loads: u32,
}
impl Player for FailOnSecondLoad {
    fn load(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.loads += 1;
        if self.loads == 2 {
            return Err(anyhow::anyhow!("load failed: broken pipe"));
        }
        Ok(())
    }
    fn play_pause(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn seek(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        None
    }
    fn duration(&self) -> Option<f64> {
        None
    }
    fn is_playing(&self) -> bool {
        true
    }
}

/// F2: when a track is playing (now_playing = A) and the user starts a new
/// track (B) whose `load` fails, the player backend (mpv/afplay) kills the
/// old child during the failed load — the old track is STOPPED. The UI must
/// clear `now_playing` so it doesn't show a stale track that's no longer
/// playing. The error must surface in `yt_error`, the failed track must be
/// marked dead, and a subsequent `next()` must dead-skip it and advance.
#[test]
fn f2_failed_play_clears_now_playing() {
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("a")).unwrap();
    std::fs::create_dir_all(lossless.join("b")).unwrap();
    std::fs::create_dir_all(lossless.join("c")).unwrap();
    std::fs::write(lossless.join("a/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("b/01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("c/01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A"],"primary_artist":"A","title":"Track1",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/a/01.flac","symlinked_into_artists":["A"]},
          {"id":"t2","artists":["B"],"primary_artist":"B","title":"Track2",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/b/01.flac","symlinked_into_artists":["B"]},
          {"id":"t3","artists":["C"],"primary_artist":"C","title":"Track3",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/c/01.flac","symlinked_into_artists":["C"]},
        ]
    })
    .to_string();
    let p = root.join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    let player: Box<dyn Player> = Box::new(FailOnSecondLoad::default());
    let mut app = App::new(cat, player, None, None);

    // Play t1 (1st load succeeds) → now_playing = t1.
    app.play_in_context_ids(vec!["t1".into(), "t2".into(), "t3".into()], "t1");
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t1"),
        "t1 should be playing after first load"
    );

    // Play t2 (2nd load fails) → old track was stopped by the backend;
    // now_playing must be cleared (not stale t1).
    app.play_in_context_ids(vec!["t1".into(), "t2".into(), "t3".into()], "t2");
    assert!(
        app.now_playing.is_none(),
        "F2: now_playing must be None when the new track's load fails (old track was stopped)"
    );
    assert!(
        app.yt_error.is_some(),
        "F2: the load error must surface in yt_error"
    );
    assert!(
        app.dead.contains("t2"),
        "F2: the failed track must be marked dead"
    );

    // A subsequent next() dead-skips t2 and advances to t3 (3rd load succeeds).
    app.next();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t3"),
        "F2: next() after a failed load must dead-skip and advance to a playable track"
    );
}

/// F2 (A1 guarantee): a failed `play_selected` on a fresh (not-playing) app
/// leaves `now_playing == None`. The old-track-staleness case is covered by
/// `f2_failed_play_clears_now_playing`; this test pins the fresh-start
/// invariant so a regression that sets now_playing before the load completes
/// is caught.
#[test]
fn f2_failed_play_on_fresh_app_leaves_now_playing_none() {
    use jukebox::catalog::Catalog;
    use jukebox::tui::app::App;

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let lossless = root.join("lossless");
    std::fs::create_dir_all(lossless.join("a")).unwrap();
    std::fs::write(lossless.join("a/01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root": lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A"],"primary_artist":"A","title":"Track1",
           "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/a/01.flac","symlinked_into_artists":["A"]},
        ]
    })
    .to_string();
    let p = root.join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();

    /// A Player whose `load` always fails.
    #[derive(Default)]
    struct AlwaysFail;
    impl Player for AlwaysFail {
        fn load(&mut self, _path: &Path) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("load failed: broken pipe"))
        }
        fn play_pause(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn seek(&mut self, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn position(&self) -> Option<f64> {
            None
        }
        fn duration(&self) -> Option<f64> {
            None
        }
        fn is_playing(&self) -> bool {
            false
        }
    }

    let player: Box<dyn Player> = Box::new(AlwaysFail);
    let mut app = App::new(cat, player, None, None);
    assert!(app.now_playing.is_none(), "fresh app: nothing playing");

    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert!(
        app.now_playing.is_none(),
        "F2: a failed play on a fresh app must leave now_playing == None"
    );
    assert!(app.yt_error.is_some(), "F2: error must surface");
    assert!(app.dead.contains("t1"), "F2: failed track must be dead");
}
