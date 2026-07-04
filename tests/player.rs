use jukebox::player::{StubPlayer, Player};
use jukebox::tui::queue::Queue;

#[test]
fn stub_player_records_loads() {
    let mut p = StubPlayer::default();
    p.load(std::path::Path::new("/x.flac")).unwrap();
    assert_eq!(p.loaded(), Some(std::path::PathBuf::from("/x.flac")));
    assert!(p.is_playing(), "load starts playback");
    p.play_pause().unwrap();
    assert!(!p.is_playing(), "play_pause toggles to paused");
}

#[test]
fn queue_shuffle_is_deterministic_with_seed() {
    let mut q = Queue::new();
    for i in 0..5 { q.enqueue(format!("id{i}")); }

    fn playback_order(q: &mut Queue) -> Vec<Option<String>> {
        let mut seq = Vec::new();
        for _ in 0..5 {
            seq.push(q.current().cloned());
            q.next();
        }
        seq
    }

    q.shuffle(42);
    let order1 = playback_order(&mut q);
    q.shuffle(42);
    let order2 = playback_order(&mut q);
    assert_eq!(order1, order2, "same seed -> same playback order");

    // The shuffled order must differ from the input (identity) order, so the
    // shuffle is actually non-trivial rather than a no-op that the test above
    // would pass either way.
    let identity: Vec<Option<String>> = (0..5).map(|i| Some(format!("id{i}"))).collect();
    assert_ne!(order1, identity, "shuffle must change the playback order");
}

#[test]
fn queue_next_wraps_and_clear_resets() {
    let mut q = Queue::new();
    q.enqueue("a".into()); q.enqueue("b".into());
    assert_eq!(q.current(), Some(&"a".to_string()));
    q.next();
    assert_eq!(q.current(), Some(&"b".to_string()));
    q.next();
    assert_eq!(q.current(), Some(&"a".to_string())); // wrap
    q.clear();
    assert!(q.items().is_empty());
}
