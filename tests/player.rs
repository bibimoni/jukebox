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
