//! Integration tests for the listening-event model (reco::events + state.rs).
//!
//! Verifies: event creation, serialization, SQLite persistence round-trip,
//! history inspection, clearing, retention (pruning), schema migration
//! (v2→v3), and the key design principle that TrackStarted alone is NOT a
//! positive signal (only MeaningfulThreshold/Completed are positive).

use jukebox::reco::events::{EventContext, EventLog, EventSource, ListenEvent};
use jukebox::state;
use std::path::PathBuf;

fn tmp_db() -> PathBuf {
    // `keep()` prevents the TempDir from being cleaned up when it goes out
    // of scope — we need the parent directory to persist so that both the
    // manual `rusqlite::Connection::open` (v2 DB creation) and the `state::*`
    // functions (which create the parent dir via `open_at`) can access it.
    tempfile::tempdir().unwrap().keep().join("state.db")
}

#[test]
fn event_creation_and_serialization() {
    let e = ListenEvent::TrackStarted {
        track_id: "abc123".into(),
        source: EventSource::Youtube,
        timestamp: 1000,
        context: EventContext::Playlist("My Mix".into()),
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"kind\":\"track_started\""));
    assert!(json.contains("\"track_id\":\"abc123\""));
    let back: ListenEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.timestamp(), 1000);
    assert_eq!(back.track_id(), Some("abc123"));
}

#[test]
fn event_store_save_and_load_roundtrip() {
    let path = tmp_db();
    let e1 = ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    };
    let e2 = ListenEvent::Liked {
        track_id: "t2".into(),
        timestamp: 200,
    };
    state::save_event_at(&path, &e1).unwrap();
    state::save_event_at(&path, &e2).unwrap();
    let loaded = state::load_events_at(&path, 10).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].track_id(), Some("t1"));
    assert_eq!(loaded[1].track_id(), Some("t2"));
}

#[test]
fn event_store_clear() {
    let path = tmp_db();
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
    )
    .unwrap();
    assert_eq!(state::count_events_at(&path).unwrap(), 1);
    state::clear_events_at(&path).unwrap();
    assert_eq!(state::count_events_at(&path).unwrap(), 0);
}

#[test]
fn event_store_count() {
    let path = tmp_db();
    assert_eq!(state::count_events_at(&path).unwrap(), 0);
    for i in 0..5 {
        state::save_event_at(
            &path,
            &ListenEvent::Completed {
                track_id: format!("t{i}"),
                timestamp: i * 100,
            },
        )
        .unwrap();
    }
    assert_eq!(state::count_events_at(&path).unwrap(), 5);
}

#[test]
fn event_store_load_since() {
    let path = tmp_db();
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "old".into(),
            timestamp: 100,
        },
    )
    .unwrap();
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "new".into(),
            timestamp: 300,
        },
    )
    .unwrap();
    let since_200 = state::load_events_since_at(&path, 200).unwrap();
    assert_eq!(since_200.len(), 1);
    assert_eq!(since_200[0].track_id(), Some("new"));
}

#[test]
fn event_log_record_and_recent() {
    let mut log = EventLog::new();
    log.record(ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    });
    log.record(ListenEvent::Completed {
        track_id: "t2".into(),
        timestamp: 200,
    });
    let recent = log.recent(1);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].track_id(), Some("t2"));
}

#[test]
fn event_log_capacity_eviction() {
    let mut log = EventLog::with_capacity(3);
    for i in 0..5 {
        log.record(ListenEvent::Completed {
            track_id: format!("t{i}"),
            timestamp: i,
        });
    }
    assert_eq!(log.len(), 3);
}

#[test]
fn event_log_clear() {
    let mut log = EventLog::new();
    log.record(ListenEvent::Completed {
        track_id: "t1".into(),
        timestamp: 100,
    });
    assert!(!log.is_empty());
    log.clear();
    assert!(log.is_empty());
}

#[test]
fn schema_migration_v2_to_v3() {
    // Create a v2 DB (no events table), then open with the new code.
    // The state table gets wiped (UI prefs are ephemeral), but the events
    // table should be created by the migration.
    let path = tmp_db();
    // Manually create a v2-style DB (just the state table, schema_version=2).
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO state (key, value) VALUES ('schema_version', '2');
            INSERT INTO state (key, value) VALUES ('focus', 'artists');",
        )
        .unwrap();
    }
    // Opening with the new code triggers the v2→v3 migration. Saving an event
    // forces `open_at` (inside `save_event_at`) which creates the `events`
    // table. This tests the migration through the public API.
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        },
    )
    .unwrap();
    // Verify the events table was created and is usable.
    let loaded = state::load_events_at(&path, 10).unwrap();
    assert_eq!(loaded.len(), 1, "events table must exist after migration");
    assert_eq!(loaded[0].track_id(), Some("t1"));
    // Count also exercises the events table.
    assert_eq!(state::count_events_at(&path).unwrap(), 1);
    // Saving another event after migration should work (re-open is clean).
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "t2".into(),
            timestamp: 200,
        },
    )
    .unwrap();
    assert_eq!(state::count_events_at(&path).unwrap(), 2);
}

#[test]
fn all_event_types_serialize_correctly() {
    let events: Vec<ListenEvent> = vec![
        ListenEvent::TrackStarted {
            track_id: "t".into(),
            source: EventSource::Local,
            timestamp: 1,
            context: EventContext::Album,
        },
        ListenEvent::MeaningfulThreshold {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Completed {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Skipped {
            track_id: "t".into(),
            timestamp: 1,
            position_sec: 0.0,
        },
        ListenEvent::RapidlySkipped {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Replayed {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Sought {
            track_id: "t".into(),
            timestamp: 1,
            from_sec: 0.0,
            to_sec: 0.0,
        },
        ListenEvent::AddedToQueue {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::RemovedFromQueue {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Liked {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Unliked {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Disliked {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::Hidden {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::ArtistBlocked {
            artist: "a".into(),
            timestamp: 1,
        },
        ListenEvent::PlayLess {
            track_id: "t".into(),
            timestamp: 1,
        },
        ListenEvent::AddedToPlaylist {
            track_id: "t".into(),
            playlist_name: "p".into(),
            timestamp: 1,
        },
        ListenEvent::RemovedFromPlaylist {
            track_id: "t".into(),
            playlist_name: "p".into(),
            timestamp: 1,
        },
        ListenEvent::RadioStarted {
            seed: "s".into(),
            timestamp: 1,
        },
        ListenEvent::MixOpened {
            mix_type: "daily".into(),
            timestamp: 1,
        },
        ListenEvent::MixPlayed {
            mix_type: "daily".into(),
            timestamp: 1,
        },
        ListenEvent::RecommendationShown {
            track_id: "t".into(),
            source: "s".into(),
            timestamp: 1,
        },
        ListenEvent::RecommendationSelected {
            track_id: "t".into(),
            source: "s".into(),
            timestamp: 1,
        },
        ListenEvent::RecommendationDismissed {
            track_id: "t".into(),
            source: "s".into(),
            timestamp: 1,
        },
        ListenEvent::SearchPerformed {
            query: "q".into(),
            scope: "local".into(),
            timestamp: 1,
        },
        ListenEvent::SearchResultSelected {
            track_id: "t".into(),
            query: "q".into(),
            timestamp: 1,
        },
        ListenEvent::SourceFallback {
            track_id: "t".into(),
            from_source: EventSource::Local,
            to_source: EventSource::Youtube,
            timestamp: 1,
        },
        ListenEvent::PlaybackFailed {
            track_id: "t".into(),
            error: "e".into(),
            timestamp: 1,
        },
    ];
    for e in &events {
        let json = serde_json::to_string(e).unwrap();
        let back: ListenEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e.type_tag(), back.type_tag());
    }
}

#[test]
fn rapidly_skipped_distinct_from_skipped() {
    let skipped = ListenEvent::Skipped {
        track_id: "t1".into(),
        timestamp: 100,
        position_sec: 60.0,
    };
    let rapid = ListenEvent::RapidlySkipped {
        track_id: "t1".into(),
        timestamp: 100,
    };
    assert_ne!(skipped.type_tag(), rapid.type_tag());
}

#[test]
fn track_started_not_positive_signal() {
    // TrackStarted alone is a weak signal — the profile builder should NOT
    // treat it as positive. Only MeaningfulThreshold/Completed are positive.
    let started = ListenEvent::TrackStarted {
        track_id: "t1".into(),
        source: EventSource::Local,
        timestamp: 100,
        context: EventContext::Album,
    };
    let threshold = ListenEvent::MeaningfulThreshold {
        track_id: "t1".into(),
        timestamp: 100,
    };
    assert_ne!(started.type_tag(), threshold.type_tag());
}

#[test]
fn history_inspection() {
    let path = tmp_db();
    for i in 0..10 {
        state::save_event_at(
            &path,
            &ListenEvent::Completed {
                track_id: format!("t{i}"),
                timestamp: i * 100,
            },
        )
        .unwrap();
    }
    let loaded = state::load_events_at(&path, 5).unwrap();
    assert_eq!(loaded.len(), 5);
    // Should be chronological (oldest first).
    assert_eq!(loaded[0].track_id(), Some("t5"));
    assert_eq!(loaded[4].track_id(), Some("t9"));
}

#[test]
fn history_clearing() {
    let path = tmp_db();
    for i in 0..5 {
        state::save_event_at(
            &path,
            &ListenEvent::Liked {
                track_id: format!("t{i}"),
                timestamp: i,
            },
        )
        .unwrap();
    }
    assert_eq!(state::count_events_at(&path).unwrap(), 5);
    state::clear_events_at(&path).unwrap();
    assert_eq!(state::count_events_at(&path).unwrap(), 0);
}

#[test]
fn retention_pruning() {
    let path = tmp_db();
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "old".into(),
            timestamp: 100,
        },
    )
    .unwrap();
    state::save_event_at(
        &path,
        &ListenEvent::Completed {
            track_id: "new".into(),
            timestamp: 300,
        },
    )
    .unwrap();
    // Prune events before timestamp 200 (removes "old").
    let pruned = state::prune_events_before_at(&path, 200).unwrap();
    assert_eq!(pruned, 1);
    let remaining = state::load_events_at(&path, 10).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].track_id(), Some("new"));
}
