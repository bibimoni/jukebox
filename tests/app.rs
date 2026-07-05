use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, View};
use jukebox::tui::queue::{ShuffleMode, RepeatMode};

fn cat_album() -> (tempfile::TempDir, Catalog, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3).map(|n| serde_json::json!({
        "id":format!("t{n}"),"artists":["40mP"],"primary_artist":"40mP","title":format!("Song{n}"),
        "album":"Cosmic","track_number":n,"bit_depth":24,"sample_rate_hz":96000,
        "source_path":format!("lossless/40mP/{n:02}.flac"),"symlinked_into_artists":["40mP"]
    })).collect();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":tracks}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap(), lossless)
}

#[test]
fn play_selected_sets_context_and_starts_playback() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    // Browse to the album's track column; cursor on track 2.
    app.view = View::Artists;
    app.cursors.artist = 0;          // 40mP
    app.cursors.album = 0;          // Cosmic
    app.cursors.track = 1;          // Song2
    app.play_selected();
    assert_eq!(app.now_playing.as_deref(), Some("t2"));
    // context is the album; next → Song3 (t3)
    app.next();
    assert_eq!(app.now_playing.as_deref(), Some("t3"));
    // prev → back to Song2 (consume off, history works)
    app.prev();
    assert_eq!(app.now_playing.as_deref(), Some("t2"));
}

#[test]
fn dead_track_skipped_and_marked() {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("X")).unwrap();
    std::fs::write(lossless.join("X").join("02.flac"), b"x").unwrap(); // only t2 exists
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
          {"id":"dead1","artists":["X"],"primary_artist":"X","title":"Gone","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/01.flac","symlinked_into_artists":["X"]},
          {"id":"alive2","artists":["X"],"primary_artist":"X","title":"Here","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/02.flac","symlinked_into_artists":["X"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    // Set context to both tracks, start at dead1.
    app.play_in_context_ids(vec!["dead1".into(),"alive2".into()], "dead1");
    assert!(app.dead.contains("dead1"));
    assert_eq!(app.now_playing.as_deref(), Some("alive2"));
}

#[test]
fn cycle_shuffle_advances_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.play_in_context_ids(vec!["t1".into(),"t2".into(),"t3".into()], "t1");
    app.cycle_shuffle();  // Off -> Smart
    assert_eq!(app.transport.shuffle, ShuffleMode::Smart);
    app.cycle_shuffle();  // Smart -> Random
    assert_eq!(app.transport.shuffle, ShuffleMode::Random);
    app.cycle_shuffle();  // Random -> Off
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);
}

#[test]
fn cycle_repeat_advances_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.cycle_repeat(); assert_eq!(app.transport.repeat, RepeatMode::All);
    app.cycle_repeat(); assert_eq!(app.transport.repeat, RepeatMode::One);
    app.cycle_repeat(); assert_eq!(app.transport.repeat, RepeatMode::Off);
}

#[test]
fn volume_clamps_and_mutes() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.volume = 5;
    app.volume_down(); assert_eq!(app.volume, 0);
    app.volume_down(); assert_eq!(app.volume, 0); // clamped
    app.volume = 98;
    app.volume_up(); assert_eq!(app.volume, 100);
    app.volume_up(); assert_eq!(app.volume, 100);
    let was = app.volume;
    app.toggle_mute(); assert!(app.muted);
    app.toggle_mute(); assert!(!app.muted); assert_eq!(app.volume, was);
}
