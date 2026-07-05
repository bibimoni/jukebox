use jukebox::player::{Player, StubPlayer};

#[test]
fn stub_player_records_loads() {
    let mut p = StubPlayer::default();
    p.load(std::path::Path::new("/x.flac")).unwrap();
    assert_eq!(p.loaded(), Some(std::path::PathBuf::from("/x.flac")));
    assert!(p.is_playing(), "load starts playback");
    p.play_pause().unwrap();
    assert!(!p.is_playing(), "play_pause toggles to paused");
}
