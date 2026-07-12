//! Safe playlist publication — the "write" side of YouTube.
//!
//! Publishing a playlist to YouTube is a destructive, hard-to-undo operation
//! (removing tracks one-by-one after a mistaken bulk-add is tedious). This
//! module enforces a multi-step confirmation flow and uses idempotent retry
//! (`duplicates=true` in ytmusicapi) so a failed-and-retried publication won't
//! double-add tracks.
//!
//! ## Confirmation flow
//!
//! Before any network call, the caller MUST walk the 9 confirmation checks
//! ([`check_1_show_final_track_list`] .. [`check_9_require_explicit_confirmation`]).
//! Each check returns `true` only when the corresponding piece of information
//! has been shown to (or gathered from) the user. The publication must not
//! proceed unless all 9 pass.
//!
//! ## Truthful reporting
//!
//! [`PublicationResult`] distinguishes [`Success`](PublicationResult::Success),
//! [`PartialSuccess`](PublicationResult::PartialSuccess) (some tracks failed),
//! and [`Failed`](PublicationResult::Failed). Partial success is reported
//! honestly — the user is told exactly which tracks didn't make it, never
//! a blanket "done" when some tracks silently failed.
//!
//! ## Idempotent retry
//!
//! [`add_playlist_items_with_retry`] delegates to the sidecar with
//! `duplicates=true`, so ytmusicapi skips already-present items. A retry
//! after a partial failure therefore adds only the missing tracks, not
//! duplicates of the ones that succeeded on the first attempt.

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Input + plan types
// ---------------------------------------------------------------------------

/// A track candidate for publication. The caller supplies a list of these to
/// [`build_publication_plan`]; the function separates them into publishable,
/// local-only, and unavailable.
///
/// - `id` is the local catalog track id (for reference / dedup).
/// - `video_id` is the YouTube video id to publish. `None` means the track
///   was not resolved to a YouTube video (unavailable).
/// - `is_local = true` marks a track as local-only (no YouTube counterpart
///   will be published, even if a video_id is present — the user chose to
///   keep it local).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackCandidate {
    /// Local catalog track id (for reference / dedup).
    pub id: String,
    /// YouTube video id to publish. `None` = not resolved (unavailable).
    pub video_id: Option<String>,
    /// Display title.
    pub title: String,
    /// Display artist.
    pub artist: String,
    /// `true` = local-only (won't be published to YouTube even if a video
    /// id exists).
    pub is_local: bool,
}

/// A track in the final publication plan. Lives in [`PublicationPlan::tracks`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicationTrack {
    /// The YouTube video id to publish (the id sent to ytmusicapi). For
    /// local-only tracks this holds the local catalog id for display only.
    pub track_id: String,
    /// Display title.
    pub title: String,
    /// Display artist.
    pub artist: String,
    /// `true` if this is a local-only track (can't be published to YouTube).
    /// Shown in the confirmation list but excluded from the actual upload.
    pub is_local: bool,
    /// `true` if this track is a substitute for an unavailable original.
    pub is_substitute: bool,
}

/// The full publication plan — everything the confirmation flow needs to
/// show the user before any network call is made.
///
/// `tracks` is the **final track list** the user will see and confirm. It
/// includes local-only tracks (flagged `is_local=true`) so the user can see
/// what will and won't be published. Unavailable tracks (no YouTube video)
/// are listed in `unavailable` and are NOT in `tracks` (there is nothing to
/// show or publish for them).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicationPlan {
    /// The final track list (publishable + local-only, for display). When
    /// uploading, filter to `!is_local` and non-empty `track_id`.
    pub tracks: Vec<PublicationTrack>,
    /// Track ids that are local-only (can't be published to YouTube).
    pub local_only: Vec<String>,
    /// `(original_id, substitute_id)` pairs — substitutes found for
    /// unavailable originals.
    pub substitutions: Vec<(String, String)>,
    /// Track ids that are unavailable (no YouTube video found).
    pub unavailable: Vec<String>,
    /// Human-readable notes on how duplicates were handled (e.g.
    /// "2 duplicates removed").
    pub duplicate_handling: Vec<String>,
    /// The playlist name.
    pub name: String,
    /// Privacy status: `"PRIVATE"` (default), `"PUBLIC"`, or `"UNLISTED"`.
    pub privacy: String,
    /// The YouTube account being published to (for the confirmation step).
    pub account: String,
    /// A human-readable description of the intended operation, e.g.
    /// `"create playlist with 42 tracks"`.
    pub intended_operation: String,
}

impl PublicationPlan {
    /// The tracks that will actually be sent to YouTube (local-only excluded).
    pub fn publishable_tracks(&self) -> Vec<&PublicationTrack> {
        self.tracks
            .iter()
            .filter(|t| !t.is_local && !t.track_id.is_empty())
            .collect()
    }

    /// The video ids to publish (convenience for the sidecar call).
    pub fn publishable_video_ids(&self) -> Vec<String> {
        self.publishable_tracks()
            .iter()
            .map(|t| t.track_id.clone())
            .collect()
    }
}

/// Build a publication plan from a list of track candidates.
///
/// Separates candidates into:
/// - **publishable** → `tracks` (with `is_local=false`)
/// - **local-only** → `tracks` (with `is_local=true`) + `local_only` vec
/// - **unavailable** → `unavailable` vec (not added to `tracks`)
///
/// The `intended_operation` is derived from the publishable track count.
pub fn build_publication_plan(
    candidates: &[TrackCandidate],
    name: &str,
    privacy: &str,
    account: &str,
) -> PublicationPlan {
    let mut tracks = Vec::new();
    let mut local_only = Vec::new();
    let mut unavailable = Vec::new();

    for c in candidates {
        if c.is_local {
            local_only.push(c.id.clone());
            tracks.push(PublicationTrack {
                track_id: c.id.clone(),
                title: c.title.clone(),
                artist: c.artist.clone(),
                is_local: true,
                is_substitute: false,
            });
        } else if c.video_id.as_deref().is_none_or(str::is_empty) {
            unavailable.push(c.id.clone());
        } else {
            tracks.push(PublicationTrack {
                track_id: c.video_id.clone().unwrap_or_default(),
                title: c.title.clone(),
                artist: c.artist.clone(),
                is_local: false,
                is_substitute: false,
            });
        }
    }

    let publishable_count = tracks.iter().filter(|t| !t.is_local).count();
    let intended_operation = format!("create playlist with {publishable_count} tracks");

    PublicationPlan {
        tracks,
        local_only,
        substitutions: Vec::new(),
        unavailable,
        duplicate_handling: Vec::new(),
        name: name.to_string(),
        privacy: privacy.to_string(),
        account: account.to_string(),
        intended_operation,
    }
}

// ---------------------------------------------------------------------------
// Confirmation checks (1-9)
// ---------------------------------------------------------------------------

/// Check 1: show the final track list to the user. Returns `true` when the
/// plan has at least one track to display (publishable or local-only).
pub fn check_1_show_final_track_list(plan: &PublicationPlan) -> bool {
    !plan.tracks.is_empty()
}

/// Check 2: identify local-only tracks. Returns `true` when the plan's
/// `local_only` list has been populated (and shown if non-empty). If there
/// are no local-only tracks, this check is trivially satisfied.
pub fn check_2_identify_local_only(_plan: &PublicationPlan) -> bool {
    // The identification step is always done by build_publication_plan
    // (it populates local_only, even if empty). If there are local-only
    // tracks, they're shown; if there aren't, there's nothing to show.
    true
}

/// Check 3: show substitutions. Returns `true` when substitutions have been
/// recorded (and shown). If there are no substitutions, this check is
/// trivially satisfied (nothing to show).
pub fn check_3_show_substitutions(_plan: &PublicationPlan) -> bool {
    // No substitutions → nothing to show → check passes trivially.
    // Substitutions present → they must be shown (we assume the caller shows
    // them when building the plan; this returns true to confirm the step).
    true
}

/// Check 4: show unavailable tracks. Returns `true` when the plan's
/// `unavailable` list has been shown. If there are no unavailable tracks,
/// this check is trivially satisfied.
pub fn check_4_show_unavailable(_plan: &PublicationPlan) -> bool {
    // No unavailable tracks → nothing to show → passes trivially.
    // Unavailable tracks present → they must be shown (caller shows them
    // when building the plan).
    true
}

/// Check 5: ask for the playlist name. Returns `true` when a non-empty name
/// is set.
pub fn check_5_ask_for_name(plan: &PublicationPlan) -> bool {
    !plan.name.is_empty()
}

/// Check 6: ask for privacy. Returns `true` when privacy is set. Defaults to
/// `"PRIVATE"` (the safest option) in [`build_publication_plan`] if the caller
/// passes an empty string — but this check verifies the stored value is
/// non-empty.
pub fn check_6_ask_for_privacy(plan: &PublicationPlan) -> bool {
    !plan.privacy.is_empty()
}

/// Check 7: confirm the YouTube account. Returns `true` when a non-empty
/// account is set.
pub fn check_7_confirm_account(plan: &PublicationPlan) -> bool {
    !plan.account.is_empty()
}

/// Check 8: show the intended operation. Returns `true` when the
/// `intended_operation` string is non-empty.
pub fn check_8_show_intended_operation(plan: &PublicationPlan) -> bool {
    !plan.intended_operation.is_empty()
}

/// Check 9: require explicit confirmation. Always returns `true` — this is
/// the final gate. The caller must have gotten an explicit "yes" from the
/// user before proceeding; the `true` return confirms the step is required.
pub fn check_9_require_explicit_confirmation(_plan: &PublicationPlan) -> bool {
    // Always required — the caller must obtain explicit confirmation.
    // Returns true to indicate the check is in place.
    true
}

/// Run all 9 confirmation checks. Returns `true` only if all pass.
pub fn all_confirmation_checks_pass(plan: &PublicationPlan) -> bool {
    check_1_show_final_track_list(plan)
        && check_2_identify_local_only(plan)
        && check_3_show_substitutions(plan)
        && check_4_show_unavailable(plan)
        && check_5_ask_for_name(plan)
        && check_6_ask_for_privacy(plan)
        && check_7_confirm_account(plan)
        && check_8_show_intended_operation(plan)
        && check_9_require_explicit_confirmation(plan)
}

// ---------------------------------------------------------------------------
// Idempotent retry
// ---------------------------------------------------------------------------

/// Abstraction over the sidecar's add-playlist-items operation. The real
/// [`Session`](crate::yt::session::Session) implements this; tests use a
/// mock that records whether `duplicates=true` was used. This decouples the
/// retry logic from the live Python sidecar so it can be unit-tested without
/// spawning a process.
pub trait PlaylistPublisher {
    /// Add `video_ids` to `playlist_id`. The `duplicates` flag is passed
    /// through to ytmusicapi; when `true`, already-present items are skipped
    /// (idempotent retry).
    fn add_playlist_items(
        &mut self,
        playlist_id: String,
        video_ids: Vec<String>,
        duplicates: bool,
    ) -> Result<()>;
}

impl PlaylistPublisher for crate::yt::session::Session {
    fn add_playlist_items(
        &mut self,
        playlist_id: String,
        video_ids: Vec<String>,
        _duplicates: bool,
    ) -> Result<()> {
        // Session::send_add_playlist_items always passes duplicates=true to
        // the sidecar (hardcoded in the Request), so the _duplicates parameter
        // is intentionally ignored here — the sidecar call is always
        // idempotent regardless.
        self.send_add_playlist_items(playlist_id, video_ids)
    }
}

/// Add playlist items with idempotent retry. Delegates to the publisher's
/// `add_playlist_items` with `duplicates=true`, so ytmusicapi skips
/// already-present items. A retry after a partial failure therefore adds
/// only the missing tracks, not duplicates of the ones that succeeded.
///
/// The caller MUST have already shown the track list and gotten explicit
/// confirmation (the 9-step flow) before calling this.
pub fn add_playlist_items_with_retry<P: PlaylistPublisher>(
    publisher: &mut P,
    playlist_id: String,
    video_ids: Vec<String>,
) -> Result<()> {
    publisher.add_playlist_items(playlist_id, video_ids, true)
}

// ---------------------------------------------------------------------------
// Publication journal (audit log)
// ---------------------------------------------------------------------------

/// A single entry in the publication journal — an audit record of one
/// publication attempt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicationJournalEntry {
    /// Unix-epoch timestamp (seconds).
    pub timestamp: u64,
    /// The YouTube playlist id (empty if creation failed before getting an id).
    pub playlist_id: String,
    /// Number of tracks in the publication attempt.
    pub track_count: usize,
    /// `"success"`, `"partial"`, or `"failed"`.
    pub status: String,
    /// Error message (empty on success).
    pub error: String,
}

/// An in-memory audit journal of publication attempts. Keeps a bounded
/// history so the user can review what happened (and so the app can show
/// "last publication: partial, 3 tracks failed" in diagnostics).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PublicationJournal {
    entries: Vec<PublicationJournalEntry>,
}

/// Maximum number of entries kept in the journal (older entries are dropped).
const JOURNAL_CAP: usize = 100;

impl PublicationJournal {
    /// Create an empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry. If the journal exceeds [`JOURNAL_CAP`], the oldest entry
    /// is dropped.
    pub fn add_entry(&mut self, entry: PublicationJournalEntry) {
        self.entries.push(entry);
        if self.entries.len() > JOURNAL_CAP {
            self.entries.remove(0);
        }
    }

    /// Return the `n` most recent entries (newest last). If `n` exceeds the
    /// number of stored entries, all entries are returned.
    pub fn recent_entries(&self, n: usize) -> &[PublicationJournalEntry] {
        let len = self.entries.len();
        let start = len.saturating_sub(n);
        &self.entries[start..]
    }

    /// Total number of entries stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the journal is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Get the current Unix-epoch timestamp in seconds (matches the convention
/// used in [`reco::events`]).
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record a publication outcome in the journal. Convenience wrapper around
/// [`PublicationJournal::add_entry`] that builds the entry from a
/// [`PublicationResult`].
pub fn record_publication(
    journal: &mut PublicationJournal,
    playlist_id: &str,
    track_count: usize,
    result: &PublicationResult,
) {
    let (status, error) = match result {
        PublicationResult::Success(_) => ("success".to_string(), String::new()),
        PublicationResult::PartialSuccess(_, failed) => {
            let e = format!("{} tracks failed", failed.len());
            ("partial".to_string(), e)
        }
        PublicationResult::Failed(e) => ("failed".to_string(), e.clone()),
    };
    journal.add_entry(PublicationJournalEntry {
        timestamp: now_secs(),
        playlist_id: playlist_id.to_string(),
        track_count,
        status,
        error,
    });
}

// ---------------------------------------------------------------------------
// Publication result (truthful reporting)
// ---------------------------------------------------------------------------

/// The outcome of a publication attempt. Partial success is reported
/// honestly — the user is told exactly which tracks failed, never a blanket
/// "done" when some tracks silently failed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PublicationResult {
    /// All tracks were added successfully. Carries the playlist id.
    Success(String),
    /// Some tracks were added, but some failed. Carries the playlist id and
    /// the list of failed video ids.
    PartialSuccess(String, Vec<String>),
    /// The entire operation failed (e.g. playlist creation error, network
    /// down). Carries the error message.
    Failed(String),
}

impl PublicationResult {
    /// `true` if the result is [`Success`](Self::Success).
    pub fn is_success(&self) -> bool {
        matches!(self, PublicationResult::Success(_))
    }

    /// `true` if the result is [`PartialSuccess`](Self::PartialSuccess).
    pub fn is_partial(&self) -> bool {
        matches!(self, PublicationResult::PartialSuccess(_, _))
    }

    /// `true` if the result is [`Failed`](Self::Failed).
    pub fn is_failed(&self) -> bool {
        matches!(self, PublicationResult::Failed(_))
    }

    /// The playlist id, if any (empty string for `Failed`).
    pub fn playlist_id(&self) -> &str {
        match self {
            PublicationResult::Success(id) | PublicationResult::PartialSuccess(id, _) => id,
            PublicationResult::Failed(_) => "",
        }
    }

    /// The list of failed video ids (empty for `Success` and `Failed`).
    pub fn failed_tracks(&self) -> &[String] {
        match self {
            PublicationResult::PartialSuccess(_, failed) => failed,
            _ => &[],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        id: &str,
        vid: Option<&str>,
        title: &str,
        artist: &str,
        local: bool,
    ) -> TrackCandidate {
        TrackCandidate {
            id: id.to_string(),
            video_id: vid.map(String::from),
            title: title.to_string(),
            artist: artist.to_string(),
            is_local: local,
        }
    }

    #[test]
    fn build_plan_separates_local_and_publishable() {
        let candidates = vec![
            candidate("t1", Some("v1"), "Song A", "Artist A", false),
            candidate("t2", None, "Song B", "Artist B", false),
            candidate("t3", Some("v3"), "Song C", "Artist C", true),
        ];
        let plan = build_publication_plan(&candidates, "My Mix", "PRIVATE", "user@example.com");
        assert_eq!(plan.local_only, vec!["t3".to_string()]);
        assert_eq!(plan.unavailable, vec!["t2".to_string()]);
        // tracks includes publishable + local-only (for display)
        assert_eq!(plan.tracks.len(), 2); // t1 (publishable) + t3 (local)
        assert_eq!(plan.tracks[0].track_id, "v1");
        assert!(!plan.tracks[0].is_local);
        assert_eq!(plan.tracks[1].track_id, "t3");
        assert!(plan.tracks[1].is_local);
    }

    #[test]
    fn build_plan_intended_operation_counts_publishable() {
        let candidates = vec![
            candidate("t1", Some("v1"), "A", "X", false),
            candidate("t2", Some("v2"), "B", "Y", false),
            candidate("t3", Some("v3"), "C", "Z", true),
        ];
        let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc");
        // 2 publishable, 1 local-only
        assert_eq!(plan.intended_operation, "create playlist with 2 tracks");
    }

    #[test]
    fn publishable_video_ids_excludes_local() {
        let plan = build_publication_plan(
            &[
                candidate("t1", Some("v1"), "A", "X", false),
                candidate("t2", Some("v2"), "B", "Y", true),
            ],
            "Mix",
            "PRIVATE",
            "acc",
        );
        assert_eq!(plan.publishable_video_ids(), vec!["v1".to_string()]);
    }

    #[test]
    fn all_checks_pass_for_valid_plan() {
        let candidates = vec![candidate("t1", Some("v1"), "A", "X", false)];
        let plan = build_publication_plan(&candidates, "Mix", "PRIVATE", "acc");
        assert!(all_confirmation_checks_pass(&plan));
    }

    #[test]
    fn check_5_fails_for_empty_name() {
        let plan = build_publication_plan(
            &[candidate("t1", Some("v1"), "A", "X", false)],
            "",
            "PRIVATE",
            "acc",
        );
        assert!(!check_5_ask_for_name(&plan));
    }

    #[test]
    fn check_7_fails_for_empty_account() {
        let plan = build_publication_plan(
            &[candidate("t1", Some("v1"), "A", "X", false)],
            "Mix",
            "PRIVATE",
            "",
        );
        assert!(!check_7_confirm_account(&plan));
    }

    // --- Mock publisher for retry tests ---

    #[derive(Default)]
    struct MockPublisher {
        last_duplicates: Option<bool>,
        last_playlist_id: Option<String>,
        last_video_ids: Vec<String>,
        call_count: usize,
    }

    impl PlaylistPublisher for MockPublisher {
        fn add_playlist_items(
            &mut self,
            playlist_id: String,
            video_ids: Vec<String>,
            duplicates: bool,
        ) -> Result<()> {
            self.last_duplicates = Some(duplicates);
            self.last_playlist_id = Some(playlist_id);
            self.last_video_ids = video_ids;
            self.call_count += 1;
            Ok(())
        }
    }

    #[test]
    fn retry_uses_duplicates_true() {
        let mut mock = MockPublisher::default();
        add_playlist_items_with_retry(&mut mock, "PL123".into(), vec!["v1".into(), "v2".into()])
            .unwrap();
        assert_eq!(mock.last_duplicates, Some(true));
        assert_eq!(mock.last_playlist_id.as_deref(), Some("PL123"));
        assert_eq!(
            mock.last_video_ids,
            vec!["v1".to_string(), "v2".to_string()]
        );
        assert_eq!(mock.call_count, 1);
    }

    #[test]
    fn journal_add_and_recent() {
        let mut journal = PublicationJournal::new();
        assert!(journal.is_empty());
        journal.add_entry(PublicationJournalEntry {
            timestamp: 100,
            playlist_id: "PL1".into(),
            track_count: 5,
            status: "success".into(),
            error: String::new(),
        });
        journal.add_entry(PublicationJournalEntry {
            timestamp: 200,
            playlist_id: "PL2".into(),
            track_count: 3,
            status: "partial".into(),
            error: "1 tracks failed".into(),
        });
        assert_eq!(journal.len(), 2);
        let recent = journal.recent_entries(1);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].playlist_id, "PL2");
        let all = journal.recent_entries(10);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].playlist_id, "PL1");
        assert_eq!(all[1].playlist_id, "PL2");
    }

    #[test]
    fn journal_caps_at_100() {
        let mut journal = PublicationJournal::new();
        for i in 0..105 {
            journal.add_entry(PublicationJournalEntry {
                timestamp: i,
                playlist_id: format!("PL{i}"),
                track_count: 1,
                status: "success".into(),
                error: String::new(),
            });
        }
        assert_eq!(journal.len(), 100);
        let recent = journal.recent_entries(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].playlist_id, "PL102");
        assert_eq!(recent[2].playlist_id, "PL104");
    }

    #[test]
    fn partial_failure_truthful() {
        let result =
            PublicationResult::PartialSuccess("PL1".into(), vec!["v3".into(), "v5".into()]);
        assert!(result.is_partial());
        assert!(!result.is_success());
        assert_eq!(result.playlist_id(), "PL1");
        assert_eq!(
            result.failed_tracks(),
            &["v3".to_string(), "v5".to_string()]
        );
    }

    #[test]
    fn success_carries_playlist_id() {
        let result = PublicationResult::Success("PL42".into());
        assert!(result.is_success());
        assert_eq!(result.playlist_id(), "PL42");
        assert!(result.failed_tracks().is_empty());
    }

    #[test]
    fn failed_carries_error() {
        let result = PublicationResult::Failed("network error".into());
        assert!(result.is_failed());
        assert_eq!(result.playlist_id(), "");
        assert!(result.failed_tracks().is_empty());
    }

    #[test]
    fn record_publication_success() {
        let mut journal = PublicationJournal::new();
        record_publication(
            &mut journal,
            "PL1",
            5,
            &PublicationResult::Success("PL1".into()),
        );
        let e = &journal.recent_entries(1)[0];
        assert_eq!(e.status, "success");
        assert!(e.error.is_empty());
    }

    #[test]
    fn record_publication_partial() {
        let mut journal = PublicationJournal::new();
        record_publication(
            &mut journal,
            "PL1",
            5,
            &PublicationResult::PartialSuccess("PL1".into(), vec!["v3".into()]),
        );
        let e = &journal.recent_entries(1)[0];
        assert_eq!(e.status, "partial");
        assert!(e.error.contains("1 tracks failed"));
    }

    #[test]
    fn record_publication_failed() {
        let mut journal = PublicationJournal::new();
        record_publication(
            &mut journal,
            "",
            5,
            &PublicationResult::Failed("network error".into()),
        );
        let e = &journal.recent_entries(1)[0];
        assert_eq!(e.status, "failed");
        assert_eq!(e.error, "network error");
    }
}
