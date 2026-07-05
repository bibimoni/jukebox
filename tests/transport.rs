use jukebox::catalog::Catalog;
use jukebox::tui::context::{Context, ContextResolver};
use jukebox::tui::queue::{Transport, ShuffleMode, RepeatMode};

fn cat_with_artists() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    // 6 tracks: A A B B C C — to test artist-spacing under smart shuffle
    let tracks: Vec<_> = [("A","t1"),("A","t2"),("B","t3"),("B","t4"),("C","t5"),("C","t6")]
        .iter().map(|(a,id)| serde_json::json!({
            "id":id,"artists":[a],"primary_artist":a,"title":id,
            "bit_depth":16,"sample_rate_hz":44100,"source_path":"x","symlinked_into_artists":[a]
        })).collect();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":"/tmp","tracks":tracks}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

struct R;
impl ContextResolver for R {
    fn playlist_ids(&self, _: &str) -> Vec<String> { vec![] }
    fn queue_ids(&self) -> Vec<String> { vec![] }
}
fn artist_of(cat: &Catalog, id: &str) -> String {
    cat.tracks.iter().find(|t| t.id == id).unwrap().primary_artist.clone()
}

#[test]
fn next_walks_context_in_order() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into(),"t3".into(),"t4".into(),"t5".into(),"t6".into()] };
    let mut t = Transport::new(ctx);
    t.play_at(&R, &cat, "t1");
    assert_eq!(t.current(&R, &cat), Some("t1".into()));
    assert_eq!(t.next(&R, &cat), Some("t2".into()));
    assert_eq!(t.next(&R, &cat), Some("t3".into()));
}

#[test]
fn prev_walks_history_backward() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into(),"t3".into()] };
    let mut t = Transport::new(ctx);
    t.play_at(&R, &cat, "t1");
    t.next(&R, &cat); // -> t2
    t.next(&R, &cat); // -> t3
    assert_eq!(t.current(&R, &cat), Some("t3".into()));
    assert_eq!(t.prev(&R, &cat), Some("t2".into()));
    assert_eq!(t.prev(&R, &cat), Some("t1".into()));
}

#[test]
fn prev_replays_first_from_start_when_history_empty() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into()] };
    let mut t = Transport::new(ctx);
    t.play_at(&R, &cat, "t1");
    // no history yet → prev replays current (still t1)
    assert_eq!(t.prev(&R, &cat), Some("t1".into()));
}

#[test]
fn repeat_all_wraps_at_end() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into()] };
    let mut t = Transport::new(ctx);
    t.set_repeat(RepeatMode::All);
    t.play_at(&R, &cat, "t1");
    t.next(&R, &cat); // t2
    assert_eq!(t.next(&R, &cat), Some("t1".into())); // wraps
}

#[test]
fn repeat_one_replays_same_track() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into()] };
    let mut t = Transport::new(ctx);
    t.set_repeat(RepeatMode::One);
    t.play_at(&R, &cat, "t1");
    assert_eq!(t.next(&R, &cat), Some("t1".into()));
}

#[test]
fn repeat_off_stops_at_end() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into(),"t2".into()] };
    let mut t = Transport::new(ctx);
    t.set_repeat(RepeatMode::Off);
    t.play_at(&R, &cat, "t1");
    t.next(&R, &cat); // t2
    assert_eq!(t.next(&R, &cat), None); // stops
}

#[test]
fn smart_shuffle_avoids_back_to_back_same_artist() {
    let (_d, cat) = cat_with_artists();
    let ids = vec!["t1","t2","t3","t4","t5","t6"].into_iter().map(String::from).collect();
    let ctx = Context::Search { query: "x".into(), track_ids: ids };
    let mut t = Transport::new(ctx);
    t.set_shuffle(ShuffleMode::Smart, &R, &cat);
    let order: Vec<String> = t.order.iter().map(|&i| t.context.track_ids(&R)[i].clone()).collect();
    // No two adjacent share an artist.
    for w in order.windows(2) {
        assert_ne!(artist_of(&cat, &w[0]), artist_of(&cat, &w[1]),
            "smart shuffle placed same artist back-to-back: {:?}", order);
    }
    // All 6 present exactly once.
    let mut sorted = order.clone(); sorted.sort();
    assert_eq!(sorted, vec!["t1","t2","t3","t4","t5","t6"]);
}

#[test]
fn manual_queue_plays_after_context_ends() {
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search { query: "x".into(), track_ids: vec!["t1".into()] };
    struct RQ;
    impl ContextResolver for RQ {
        fn playlist_ids(&self, _: &str) -> Vec<String> { vec![] }
        fn queue_ids(&self) -> Vec<String> { vec![] }
    }
    let mut t = Transport::new(ctx);
    t.enqueue("t3".into());
    t.play_at(&RQ, &cat, "t1");
    assert_eq!(t.next(&RQ, &cat), Some("t3".into())); // context exhausted → manual queue
}

#[test]
fn prev_after_manual_queue_returns_last_context_track() {
    // Regression: the manual_queue branch of `next()` used to pop the last
    // context track's history entry, so `prev()` after a manual-queue
    // transition skipped that track. With a multi-track context [t1,t2] and
    // a manual queue [t3], play t1 → next→t2 → next→t3, then prev must
    // return t2 (the last context track that actually finished), not t1.
    let (_d, cat) = cat_with_artists();
    let ctx = Context::Search {
        query: "x".into(),
        track_ids: vec!["t1".into(), "t2".into()],
    };
    struct RQ;
    impl ContextResolver for RQ {
        fn playlist_ids(&self, _: &str) -> Vec<String> { vec![] }
        fn queue_ids(&self) -> Vec<String> { vec![] }
    }
    let mut t = Transport::new(ctx);
    t.enqueue("t3".into());
    t.play_at(&RQ, &cat, "t1");
    assert_eq!(t.current(&RQ, &cat), Some("t1".into()));
    assert_eq!(t.next(&RQ, &cat), Some("t2".into())); // advance within context
    assert_eq!(t.next(&RQ, &cat), Some("t3".into())); // context exhausted → manual queue
    // prev must walk back to t2 (the last context track that finished).
    assert_eq!(t.prev(&RQ, &cat), Some("t2".into()));
}
