//! Integration tests for the user profile (`reco::profile`) and its SQLite
//! persistence layer (`state::save_profile_at` / `load_profile_at`).
//!
//! Verifies: profile building from events, affinity scoring, skip rate
//! computation, like/hide/block tracking, profile reset, JSON round-trip,
//! and SQLite persistence.

use jukebox::reco::events::{EventContext, EventSource, ListenEvent};
use jukebox::reco::profile::UserProfile;
use jukebox::state;
use std::path::PathBuf;

fn tmp_db() -> PathBuf {
    tempfile::tempdir().unwrap().keep().join("state.db")
}

fn ts(n: u64) -> u64 {
    n
}

// ---------------------------------------------------------------------------
// 1. Profile building from events
// ---------------------------------------------------------------------------

#[test]
fn profile_building_from_events() {
    let events = vec![
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(100),
            context: EventContext::Album,
        },
        ListenEvent::MeaningfulThreshold {
            track_id: "t1".into(),
            timestamp: ts(130),
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(200),
        },
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(250),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    let tp = p.tracks.get("t1").unwrap();
    assert_eq!(tp.play_count, 1);
    assert_eq!(tp.completion_count, 1);
    assert!(tp.liked);
    assert!(tp.score > 0.0, "profile should have positive score for t1");
    assert_eq!(p.event_count, 4);
}

// ---------------------------------------------------------------------------
// 2. Affinity scoring
// ---------------------------------------------------------------------------

#[test]
fn affinity_scoring() {
    // Completed alone: +2.0
    let p1 = UserProfile::build_from_events(&[ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    let score1 = p1.track_score("t1");

    // Liked alone: +5.0
    let p2 = UserProfile::build_from_events(&[ListenEvent::Liked {
        track_id: "t2".into(),
        timestamp: ts(100),
    }]);
    let score2 = p2.track_score("t2");

    // Liked > Completed
    assert!(
        score2 > score1,
        "Liked ({score2}) should score higher than Completed ({score1})"
    );

    // RapidlySkipped: -2.0 (strong negative)
    let p3 = UserProfile::build_from_events(&[ListenEvent::RapidlySkipped {
        track_id: "t3".into(),
        timestamp: ts(100),
    }]);
    assert!(
        p3.track_score("t3") < 0.0,
        "rapidly skipped should be negative"
    );
}

#[test]
fn track_started_not_positive_signal() {
    let p = UserProfile::build_from_events(&[ListenEvent::TrackStarted {
        track_id: "t1".into(),
        source: EventSource::Local,
        timestamp: ts(100),
        context: EventContext::Album,
    }]);
    assert_eq!(
        p.track_score("t1"),
        0.0,
        "TrackStarted alone must not produce a positive score"
    );
    assert_eq!(p.tracks.get("t1").unwrap().play_count, 1);
}

// ---------------------------------------------------------------------------
// 3. Skip rate computation
// ---------------------------------------------------------------------------

#[test]
fn skip_rate_computation() {
    let events = vec![
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(100),
            context: EventContext::Album,
        },
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(200),
            context: EventContext::Album,
        },
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(300),
            context: EventContext::Album,
        },
        ListenEvent::Skipped {
            track_id: "t1".into(),
            timestamp: ts(350),
            position_sec: 30.0,
        },
        ListenEvent::RapidlySkipped {
            track_id: "t1".into(),
            timestamp: ts(360),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    // 2 skips / 3 plays = ~0.667
    let rate = p.skip_rate("t1");
    assert!(
        (rate - 2.0 / 3.0).abs() < 1e-9,
        "skip rate should be 2/3, got {rate}"
    );
}

#[test]
fn skip_rate_zero_for_unplayed() {
    let p = UserProfile::new();
    assert_eq!(p.skip_rate("unknown"), 0.0);
}

// ---------------------------------------------------------------------------
// 4. Like / hide / block tracking
// ---------------------------------------------------------------------------

#[test]
fn like_tracking() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Liked {
            track_id: "t2".into(),
            timestamp: ts(100),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    assert!(p.is_liked("t1"));
    assert!(p.is_liked("t2"));
    assert!(!p.is_liked("t3"));
    assert_eq!(p.liked.len(), 2);

    // Unlike removes the like
    let events2 = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Unliked {
            track_id: "t1".into(),
            timestamp: ts(200),
        },
    ];
    let p2 = UserProfile::build_from_events(&events2);
    assert!(!p2.is_liked("t1"), "unlike should remove the liked flag");
    assert!(!p2.liked.contains("t1"));
}

#[test]
fn hide_tracking() {
    let p = UserProfile::build_from_events(&[ListenEvent::Hidden {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    assert!(p.is_hidden("t1"));
    assert!(!p.is_hidden("t2"));
    assert_eq!(p.hidden.len(), 1);
    assert!(p.tracks.get("t1").unwrap().hidden);
}

#[test]
fn block_tracking() {
    let p = UserProfile::build_from_events(&[ListenEvent::ArtistBlocked {
        artist: "Bad Artist".into(),
        timestamp: ts(100),
    }]);
    assert!(p.is_blocked("Bad Artist"));
    assert!(!p.is_blocked("Good Artist"));
    assert_eq!(p.blocked_artists.len(), 1);
    assert!(p.artists.get("Bad Artist").unwrap().blocked);
}

#[test]
fn play_less_tracking() {
    let p = UserProfile::build_from_events(&[ListenEvent::PlayLess {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    assert!(p.is_play_less("t1"));
    assert!(!p.is_play_less("t2"));
    assert_eq!(p.play_less.len(), 1);
}

#[test]
fn disliked_tracking() {
    let p = UserProfile::build_from_events(&[ListenEvent::Disliked {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    assert!(p.is_disliked("t1"));
    assert!(p.tracks.get("t1").unwrap().disliked);
    assert!(p.track_score("t1") < 0.0, "disliked should be negative");
}

// ---------------------------------------------------------------------------
// 5. Profile reset
// ---------------------------------------------------------------------------

#[test]
fn profile_reset() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Hidden {
            track_id: "t2".into(),
            timestamp: ts(100),
        },
        ListenEvent::ArtistBlocked {
            artist: "Bad".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "t3".into(),
            timestamp: ts(200),
        },
    ];
    let mut p = UserProfile::build_from_events(&events);
    assert!(!p.tracks.is_empty());
    assert!(!p.liked.is_empty());
    assert!(!p.hidden.is_empty());
    assert!(!p.blocked_artists.is_empty());
    assert!(p.event_count > 0);

    p.reset();

    assert!(p.tracks.is_empty());
    assert!(p.liked.is_empty());
    assert!(p.hidden.is_empty());
    assert!(p.blocked_artists.is_empty());
    assert!(p.disliked.is_empty());
    assert!(p.play_less.is_empty());
    assert_eq!(p.event_count, 0);
}

// ---------------------------------------------------------------------------
// 6. Top tracks and top liked
// ---------------------------------------------------------------------------

#[test]
fn top_tracks_sorted_by_score() {
    let events = vec![
        ListenEvent::Completed {
            track_id: "low".into(),
            timestamp: ts(100),
        },
        ListenEvent::Liked {
            track_id: "high".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "mid".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "mid".into(),
            timestamp: ts(200),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    let top = p.top_tracks(3);
    assert_eq!(top.len(), 3);
    // Liked (5.0) > 2xCompleted (4.0) > 1xCompleted (2.0)
    assert_eq!(top[0].0, "high");
    assert_eq!(top[1].0, "mid");
    assert_eq!(top[2].0, "low");
}

#[test]
fn top_liked_returns_only_liked() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "liked1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "not_liked".into(),
            timestamp: ts(100),
        },
        ListenEvent::Liked {
            track_id: "liked2".into(),
            timestamp: ts(100),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    let top = p.top_liked(10);
    assert_eq!(top.len(), 2, "only liked tracks should appear");
    assert!(top.contains(&"liked1".to_string()));
    assert!(top.contains(&"liked2".to_string()));
}

// ---------------------------------------------------------------------------
// 7. Persistence: save/load profile round-trip
// ---------------------------------------------------------------------------

#[test]
fn profile_persistence_round_trip() {
    let path = tmp_db();
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(200),
        },
        ListenEvent::Hidden {
            track_id: "t2".into(),
            timestamp: ts(300),
        },
        ListenEvent::ArtistBlocked {
            artist: "Bad".into(),
            timestamp: ts(400),
        },
    ];
    let profile = UserProfile::build_from_events(&events);

    state::save_profile_at(&path, &profile).unwrap();
    let loaded = state::load_profile_at(&path).unwrap();

    assert_eq!(loaded.tracks.len(), profile.tracks.len());
    assert!(loaded.is_liked("t1"));
    assert!(loaded.is_hidden("t2"));
    assert!(loaded.is_blocked("Bad"));
    assert!((loaded.track_score("t1") - profile.track_score("t1")).abs() < 1e-9);
    assert_eq!(loaded.event_count, profile.event_count);
}

#[test]
fn profile_persistence_empty_db_returns_default() {
    let path = tmp_db();
    let loaded = state::load_profile_at(&path).unwrap();
    assert!(loaded.tracks.is_empty());
    assert!(loaded.liked.is_empty());
    assert_eq!(loaded.event_count, 0);
}

#[test]
fn profile_persistence_overwrite() {
    let path = tmp_db();
    let p1 = UserProfile::build_from_events(&[ListenEvent::Liked {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    state::save_profile_at(&path, &p1).unwrap();

    let p2 = UserProfile::build_from_events(&[ListenEvent::Liked {
        track_id: "t2".into(),
        timestamp: ts(100),
    }]);
    state::save_profile_at(&path, &p2).unwrap();

    let loaded = state::load_profile_at(&path).unwrap();
    assert!(loaded.is_liked("t2"));
    assert!(!loaded.is_liked("t1"), "old profile should be overwritten");
}

#[test]
fn profile_persistence_clear() {
    let path = tmp_db();
    let p = UserProfile::build_from_events(&[ListenEvent::Liked {
        track_id: "t1".into(),
        timestamp: ts(100),
    }]);
    state::save_profile_at(&path, &p).unwrap();
    assert!(state::load_profile_at(&path).unwrap().is_liked("t1"));

    state::clear_profile_at(&path).unwrap();
    let loaded = state::load_profile_at(&path).unwrap();
    assert!(loaded.tracks.is_empty(), "cleared profile should be empty");
}

// ---------------------------------------------------------------------------
// 8. JSON round-trip
// ---------------------------------------------------------------------------

#[test]
fn json_roundtrip_preserves_all_data() {
    let events = vec![
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(100),
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(200),
        },
        ListenEvent::Hidden {
            track_id: "t2".into(),
            timestamp: ts(300),
        },
        ListenEvent::ArtistBlocked {
            artist: "Bad".into(),
            timestamp: ts(400),
        },
        ListenEvent::PlayLess {
            track_id: "t3".into(),
            timestamp: ts(500),
        },
        ListenEvent::Disliked {
            track_id: "t4".into(),
            timestamp: ts(600),
        },
    ];
    let p = UserProfile::build_from_events(&events);
    let json = p.to_json().unwrap();
    let back = UserProfile::from_json(&json).unwrap();

    assert_eq!(back.tracks.len(), p.tracks.len());
    assert!(back.is_liked("t1"));
    assert!(back.is_hidden("t2"));
    assert!(back.is_blocked("Bad"));
    assert!(back.is_play_less("t3"));
    assert!(back.is_disliked("t4"));
    assert_eq!(back.event_count, p.event_count);
    assert!((back.track_score("t1") - p.track_score("t1")).abs() < 1e-9);
    assert!((back.track_score("t4") - p.track_score("t4")).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// 9. Complex multi-event scenario
// ---------------------------------------------------------------------------

#[test]
fn complex_multi_event_scenario() {
    // A realistic scenario: user plays t1, completes it, likes it; plays t2
    // and rapidly skips; plays t3 and skips at 30s; hides t4; blocks "Bad Artist".
    let events = vec![
        ListenEvent::TrackStarted {
            track_id: "t1".into(),
            source: EventSource::Local,
            timestamp: ts(100),
            context: EventContext::Album,
        },
        ListenEvent::MeaningfulThreshold {
            track_id: "t1".into(),
            timestamp: ts(130),
        },
        ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(200),
        },
        ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: ts(250),
        },
        ListenEvent::TrackStarted {
            track_id: "t2".into(),
            source: EventSource::Youtube,
            timestamp: ts(300),
            context: EventContext::Radio,
        },
        ListenEvent::RapidlySkipped {
            track_id: "t2".into(),
            timestamp: ts(305),
        },
        ListenEvent::TrackStarted {
            track_id: "t3".into(),
            source: EventSource::Local,
            timestamp: ts(400),
            context: EventContext::Album,
        },
        ListenEvent::Skipped {
            track_id: "t3".into(),
            timestamp: ts(430),
            position_sec: 30.0,
        },
        ListenEvent::Hidden {
            track_id: "t4".into(),
            timestamp: ts(500),
        },
        ListenEvent::ArtistBlocked {
            artist: "Bad Artist".into(),
            timestamp: ts(600),
        },
    ];
    let p = UserProfile::build_from_events(&events);

    // t1: strong positive (MeaningfulThreshold + Completed + Liked)
    assert!(
        p.track_score("t1") > 5.0,
        "t1 should have strong positive score"
    );
    assert!(p.is_liked("t1"));
    assert_eq!(p.tracks.get("t1").unwrap().completion_count, 1);

    // t2: strong negative (RapidlySkipped)
    assert!(p.track_score("t2") < -1.0, "t2 should be strongly negative");
    assert_eq!(p.tracks.get("t2").unwrap().rapid_skip_count, 1);

    // t3: weak negative (Skipped at 30s, not rapid)
    assert!(p.track_score("t3") < 0.0, "t3 should be weakly negative");
    assert!(
        p.track_score("t3") > p.track_score("t2"),
        "t3 should be less negative than t2"
    );

    // t4: hidden
    assert!(p.is_hidden("t4"));

    // Blocked artist
    assert!(p.is_blocked("Bad Artist"));

    // Event count
    assert_eq!(p.event_count, 10);

    // Top tracks: t1 should be #1
    let top = p.top_tracks(3);
    assert_eq!(top[0].0, "t1", "highest-scoring track should be first");
}
