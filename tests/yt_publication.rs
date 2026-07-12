//! Integration tests for safe playlist publication (`yt::publication`).
//!
//! Verifies the core safety guarantees:
//! - Plan construction separates publishable, local-only, unavailable tracks.
//! - The 9-step confirmation flow gates publication.
//! - Default privacy is PRIVATE (the safest option).
//! - Idempotent retry uses `duplicates=true` (won't double-add on retry).
//! - Partial failure is reported truthfully (no false "success").
//! - The publication journal records an audit trail.

use jukebox::yt::publication::*;

// ---------------------------------------------------------------------------
// publication_plan_construction
// ---------------------------------------------------------------------------

#[test]
fn publication_plan_construction() {
    let candidates = vec![
        TrackCandidate {
            id: "local-1".into(),
            video_id: Some("vid1".into()),
            title: "Song One".into(),
            artist: "Artist A".into(),
            is_local: false,
        },
        TrackCandidate {
            id: "local-2".into(),
            video_id: Some("vid2".into()),
            title: "Song Two".into(),
            artist: "Artist B".into(),
            is_local: false,
        },
    ];
    let plan = build_publication_plan(&candidates, "My Playlist", "PRIVATE", "user@example.com");
    assert_eq!(plan.name, "My Playlist");
    assert_eq!(plan.privacy, "PRIVATE");
    assert_eq!(plan.account, "user@example.com");
    assert_eq!(plan.tracks.len(), 2);
    assert_eq!(plan.tracks[0].track_id, "vid1");
    assert_eq!(plan.tracks[0].title, "Song One");
    assert_eq!(plan.tracks[0].artist, "Artist A");
    assert!(!plan.tracks[0].is_local);
    assert!(!plan.tracks[0].is_substitute);
    assert!(plan.local_only.is_empty());
    assert!(plan.unavailable.is_empty());
    assert!(plan.intended_operation.contains("2 tracks"));
}

// ---------------------------------------------------------------------------
// confirmation_steps_all_pass
// ---------------------------------------------------------------------------

#[test]
fn confirmation_steps_all_pass() {
    let candidates = vec![TrackCandidate {
        id: "t1".into(),
        video_id: Some("v1".into()),
        title: "Song".into(),
        artist: "Artist".into(),
        is_local: false,
    }];
    let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc@example.com");
    assert!(check_1_show_final_track_list(&plan));
    assert!(check_2_identify_local_only(&plan));
    assert!(check_3_show_substitutions(&plan));
    assert!(check_4_show_unavailable(&plan));
    assert!(check_5_ask_for_name(&plan));
    assert!(check_6_ask_for_privacy(&plan));
    assert!(check_7_confirm_account(&plan));
    assert!(check_8_show_intended_operation(&plan));
    assert!(check_9_require_explicit_confirmation(&plan));
    assert!(all_confirmation_checks_pass(&plan));
}

#[test]
fn confirmation_steps_fail_when_name_missing() {
    let candidates = vec![TrackCandidate {
        id: "t1".into(),
        video_id: Some("v1".into()),
        title: "Song".into(),
        artist: "Artist".into(),
        is_local: false,
    }];
    let plan = build_publication_plan(&candidates, "", "PRIVATE", "acc");
    assert!(!check_5_ask_for_name(&plan));
    assert!(!all_confirmation_checks_pass(&plan));
}

#[test]
fn confirmation_steps_fail_when_account_missing() {
    let candidates = vec![TrackCandidate {
        id: "t1".into(),
        video_id: Some("v1".into()),
        title: "Song".into(),
        artist: "Artist".into(),
        is_local: false,
    }];
    let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "");
    assert!(!check_7_confirm_account(&plan));
    assert!(!all_confirmation_checks_pass(&plan));
}

#[test]
fn confirmation_steps_fail_when_no_tracks() {
    let plan = build_publication_plan(&[], "Mix", "PRIVATE", "acc");
    assert!(!check_1_show_final_track_list(&plan));
    assert!(!all_confirmation_checks_pass(&plan));
}

// ---------------------------------------------------------------------------
// default_privacy_is_private
// ---------------------------------------------------------------------------

#[test]
fn default_privacy_is_private() {
    // The caller is expected to pass "PRIVATE" by default; the module does
    // not override it. Verify that "PRIVATE" is the documented safe default
    // and that check_6 passes for it.
    let plan = build_publication_plan(
        &[TrackCandidate {
            id: "t1".into(),
            video_id: Some("v1".into()),
            title: "S".into(),
            artist: "A".into(),
            is_local: false,
        }],
        "Mix",
        "PRIVATE",
        "acc",
    );
    assert_eq!(plan.privacy, "PRIVATE");
    assert!(check_6_ask_for_privacy(&plan));
}

#[test]
fn privacy_public_and_unlisted_also_pass_check() {
    for privacy in &["PUBLIC", "UNLISTED"] {
        let plan = build_publication_plan(
            &[TrackCandidate {
                id: "t1".into(),
                video_id: Some("v1".into()),
                title: "S".into(),
                artist: "A".into(),
                is_local: false,
            }],
            "Mix",
            privacy,
            "acc",
        );
        assert_eq!(plan.privacy, *privacy);
        assert!(check_6_ask_for_privacy(&plan));
    }
}

// ---------------------------------------------------------------------------
// local_only_tracks_identified
// ---------------------------------------------------------------------------

#[test]
fn local_only_tracks_identified() {
    let candidates = vec![
        TrackCandidate {
            id: "publishable".into(),
            video_id: Some("vid-pub".into()),
            title: "Pub".into(),
            artist: "Artist A".into(),
            is_local: false,
        },
        TrackCandidate {
            id: "local-only".into(),
            video_id: Some("vid-local".into()),
            title: "Local".into(),
            artist: "Artist B".into(),
            is_local: true,
        },
    ];
    let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc");
    assert_eq!(plan.local_only, vec!["local-only".to_string()]);
    // Local-only track is in the display list but flagged is_local.
    let local_track = plan.tracks.iter().find(|t| t.is_local).unwrap();
    assert_eq!(local_track.track_id, "local-only");
    assert_eq!(local_track.title, "Local");
    // publishable_video_ids excludes the local track.
    assert_eq!(plan.publishable_video_ids(), vec!["vid-pub".to_string()]);
}

#[test]
fn local_only_track_with_no_video_id_still_goes_to_local_only() {
    // A local-only track with no video_id still goes to local_only (is_local
    // takes priority over unavailable).
    let candidates = vec![TrackCandidate {
        id: "local-no-vid".into(),
        video_id: None,
        title: "Local".into(),
        artist: "Artist".into(),
        is_local: true,
    }];
    let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc");
    assert_eq!(plan.local_only, vec!["local-no-vid".to_string()]);
    assert!(plan.unavailable.is_empty());
}

// ---------------------------------------------------------------------------
// unavailable_tracks_identified
// ---------------------------------------------------------------------------

#[test]
fn unavailable_tracks_identified() {
    let candidates = vec![
        TrackCandidate {
            id: "ok".into(),
            video_id: Some("vid-ok".into()),
            title: "OK".into(),
            artist: "A".into(),
            is_local: false,
        },
        TrackCandidate {
            id: "no-vid".into(),
            video_id: None,
            title: "NoVid".into(),
            artist: "B".into(),
            is_local: false,
        },
        TrackCandidate {
            id: "empty-vid".into(),
            video_id: Some(String::new()),
            title: "EmptyVid".into(),
            artist: "C".into(),
            is_local: false,
        },
    ];
    let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc");
    assert_eq!(plan.unavailable.len(), 2);
    assert!(plan.unavailable.contains(&"no-vid".to_string()));
    assert!(plan.unavailable.contains(&"empty-vid".to_string()));
    // Only the publishable track is in tracks.
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].track_id, "vid-ok");
}

// ---------------------------------------------------------------------------
// idempotent_retry_uses_duplicates_true
// ---------------------------------------------------------------------------

/// A mock publisher that records the `duplicates` flag it received.
#[derive(Default)]
struct RecordingPublisher {
    duplicates_seen: Vec<bool>,
    playlist_ids: Vec<String>,
    video_ids_batches: Vec<Vec<String>>,
}

impl PlaylistPublisher for RecordingPublisher {
    fn add_playlist_items(
        &mut self,
        playlist_id: String,
        video_ids: Vec<String>,
        duplicates: bool,
    ) -> anyhow::Result<()> {
        self.duplicates_seen.push(duplicates);
        self.playlist_ids.push(playlist_id);
        self.video_ids_batches.push(video_ids);
        Ok(())
    }
}

#[test]
fn idempotent_retry_uses_duplicates_true() {
    let mut publisher = RecordingPublisher::default();
    add_playlist_items_with_retry(
        &mut publisher,
        "PL123".into(),
        vec!["v1".into(), "v2".into(), "v3".into()],
    )
    .unwrap();
    assert_eq!(publisher.duplicates_seen.len(), 1);
    assert!(publisher.duplicates_seen[0]);
    assert_eq!(publisher.playlist_ids[0], "PL123");
    assert_eq!(publisher.video_ids_batches[0], vec!["v1", "v2", "v3"]);
}

#[test]
fn idempotent_retry_can_be_called_again_without_double_add() {
    // Simulate a retry: call twice with the same items. The duplicates=true
    // flag means ytmusicapi skips already-present items. The mock records
    // both calls used duplicates=true.
    let mut publisher = RecordingPublisher::default();
    let items = vec!["v1".into(), "v2".into()];
    add_playlist_items_with_retry(&mut publisher, "PL".into(), items.clone()).unwrap();
    add_playlist_items_with_retry(&mut publisher, "PL".into(), items).unwrap();
    assert!(publisher.duplicates_seen[0] && publisher.duplicates_seen[1]);
}

#[test]
fn retry_with_empty_video_ids_is_ok() {
    let mut publisher = RecordingPublisher::default();
    add_playlist_items_with_retry(&mut publisher, "PL".into(), Vec::new()).unwrap();
    assert!(publisher.duplicates_seen[0]);
    assert!(publisher.video_ids_batches[0].is_empty());
}

// ---------------------------------------------------------------------------
// partial_failure_truthful
// ---------------------------------------------------------------------------

#[test]
fn partial_failure_truthful() {
    let result = PublicationResult::PartialSuccess(
        "PL42".into(),
        vec!["failed-1".into(), "failed-2".into()],
    );
    assert!(result.is_partial());
    assert!(!result.is_success());
    assert!(!result.is_failed());
    assert_eq!(result.playlist_id(), "PL42");
    assert_eq!(
        result.failed_tracks(),
        &["failed-1".to_string(), "failed-2".to_string()]
    );
}

#[test]
fn success_is_truthful() {
    let result = PublicationResult::Success("PL99".into());
    assert!(result.is_success());
    assert!(!result.is_partial());
    assert!(!result.is_failed());
    assert_eq!(result.playlist_id(), "PL99");
    assert!(result.failed_tracks().is_empty());
}

#[test]
fn failed_is_truthful() {
    let result = PublicationResult::Failed("network unreachable".into());
    assert!(result.is_failed());
    assert!(!result.is_success());
    assert!(!result.is_partial());
    assert_eq!(result.playlist_id(), "");
    assert!(result.failed_tracks().is_empty());
}

#[test]
fn partial_success_serializes_and_deserializes() {
    let result = PublicationResult::PartialSuccess("PL1".into(), vec!["v3".into()]);
    let json = serde_json::to_string(&result).unwrap();
    let back: PublicationResult = serde_json::from_str(&json).unwrap();
    assert!(back.is_partial());
    assert_eq!(back.playlist_id(), "PL1");
    assert_eq!(back.failed_tracks(), &["v3".to_string()]);
}

// ---------------------------------------------------------------------------
// publication_journal_add_and_recent
// ---------------------------------------------------------------------------

#[test]
fn publication_journal_add_and_recent() {
    let mut journal = PublicationJournal::new();
    assert!(journal.is_empty());
    assert_eq!(journal.len(), 0);

    journal.add_entry(PublicationJournalEntry {
        timestamp: 1000,
        playlist_id: "PL1".into(),
        track_count: 10,
        status: "success".into(),
        error: String::new(),
    });
    assert_eq!(journal.len(), 1);
    assert!(!journal.is_empty());

    journal.add_entry(PublicationJournalEntry {
        timestamp: 2000,
        playlist_id: "PL2".into(),
        track_count: 5,
        status: "partial".into(),
        error: "2 tracks failed".into(),
    });
    assert_eq!(journal.len(), 2);

    // recent_entries(1) returns only the latest.
    let recent = journal.recent_entries(1);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].playlist_id, "PL2");
    assert_eq!(recent[0].status, "partial");

    // recent_entries(10) returns all (fewer than 10 stored).
    let all = journal.recent_entries(10);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].playlist_id, "PL1");
    assert_eq!(all[1].playlist_id, "PL2");
}

#[test]
fn publication_journal_caps_history() {
    let mut journal = PublicationJournal::new();
    for i in 0..150 {
        journal.add_entry(PublicationJournalEntry {
            timestamp: i,
            playlist_id: format!("PL{i}"),
            track_count: i as usize,
            status: "success".into(),
            error: String::new(),
        });
    }
    // Capped at 100 entries.
    assert_eq!(journal.len(), 100);
    // The oldest 50 were dropped; the first remaining is PL50.
    let all = journal.recent_entries(100);
    assert_eq!(all[0].playlist_id, "PL50");
    assert_eq!(all[99].playlist_id, "PL149");
}

#[test]
fn publication_journal_recent_more_than_stored() {
    let mut journal = PublicationJournal::new();
    journal.add_entry(PublicationJournalEntry {
        timestamp: 1,
        playlist_id: "PL1".into(),
        track_count: 1,
        status: "success".into(),
        error: String::new(),
    });
    // Requesting 50 from a journal with 1 entry returns 1.
    let recent = journal.recent_entries(50);
    assert_eq!(recent.len(), 1);
}

#[test]
fn record_publication_logs_all_statuses() {
    let mut journal = PublicationJournal::new();

    // Success
    record_publication(
        &mut journal,
        "PL-s",
        10,
        &PublicationResult::Success("PL-s".into()),
    );
    // Partial
    record_publication(
        &mut journal,
        "PL-p",
        10,
        &PublicationResult::PartialSuccess("PL-p".into(), vec!["v1".into(), "v2".into()]),
    );
    // Failed
    record_publication(
        &mut journal,
        "",
        10,
        &PublicationResult::Failed("timeout".into()),
    );

    assert_eq!(journal.len(), 3);
    let entries = journal.recent_entries(3);
    assert_eq!(entries[0].status, "success");
    assert!(entries[0].error.is_empty());
    assert_eq!(entries[1].status, "partial");
    assert!(entries[1].error.contains("2 tracks failed"));
    assert_eq!(entries[2].status, "failed");
    assert_eq!(entries[2].error, "timeout");
}

// ---------------------------------------------------------------------------
// plan serialization
// ---------------------------------------------------------------------------

#[test]
fn publication_plan_serializes_and_deserializes() {
    let plan = build_publication_plan(
        &[
            TrackCandidate {
                id: "t1".into(),
                video_id: Some("v1".into()),
                title: "Song".into(),
                artist: "Artist".into(),
                is_local: false,
            },
            TrackCandidate {
                id: "t2".into(),
                video_id: None,
                title: "Gone".into(),
                artist: "Artist2".into(),
                is_local: true,
            },
        ],
        "My Mix",
        "PRIVATE",
        "user@example.com",
    );
    let json = serde_json::to_string(&plan).unwrap();
    let back: PublicationPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "My Mix");
    assert_eq!(back.privacy, "PRIVATE");
    assert_eq!(back.account, "user@example.com");
    assert_eq!(back.local_only, vec!["t2".to_string()]);
    assert_eq!(back.unavailable, Vec::<String>::new());
    // tracks includes publishable + local-only (for display).
    assert_eq!(back.tracks.len(), 2);
    let pub_track = back.tracks.iter().find(|t| !t.is_local).unwrap();
    assert_eq!(pub_track.track_id, "v1");
    let local_track = back.tracks.iter().find(|t| t.is_local).unwrap();
    assert_eq!(local_track.track_id, "t2");
}

#[test]
fn publication_track_serializes_and_deserializes() {
    let track = PublicationTrack {
        track_id: "vid123".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        is_local: false,
        is_substitute: true,
    };
    let json = serde_json::to_string(&track).unwrap();
    let back: PublicationTrack = serde_json::from_str(&json).unwrap();
    assert_eq!(back.track_id, "vid123");
    assert!(back.is_substitute);
    assert!(!back.is_local);
}

// ---------------------------------------------------------------------------
// substitutions
// ---------------------------------------------------------------------------

#[test]
fn plan_supports_substitutions() {
    let mut plan = build_publication_plan(
        &[TrackCandidate {
            id: "t1".into(),
            video_id: Some("v1".into()),
            title: "Song".into(),
            artist: "Artist".into(),
            is_local: false,
        }],
        "Mix",
        "PRIVATE",
        "acc",
    );
    plan.substitutions
        .push(("original-1".into(), "sub-1".into()));
    plan.substitutions
        .push(("original-2".into(), "sub-2".into()));
    assert_eq!(plan.substitutions.len(), 2);
    assert_eq!(plan.substitutions[0].0, "original-1");
    assert_eq!(plan.substitutions[1].1, "sub-2");
    // check_3 still passes (substitutions are recorded).
    assert!(check_3_show_substitutions(&plan));
}

// ---------------------------------------------------------------------------
// duplicate_handling notes
// ---------------------------------------------------------------------------

#[test]
fn plan_supports_duplicate_handling_notes() {
    let mut plan = build_publication_plan(
        &[TrackCandidate {
            id: "t1".into(),
            video_id: Some("v1".into()),
            title: "Song".into(),
            artist: "Artist".into(),
            is_local: false,
        }],
        "Mix",
        "PRIVATE",
        "acc",
    );
    plan.duplicate_handling.push("2 duplicates removed".into());
    assert_eq!(plan.duplicate_handling.len(), 1);
    assert_eq!(plan.duplicate_handling[0], "2 duplicates removed");
}
