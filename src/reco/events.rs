//! Listening-event model — durable, versioned record of user playback behavior.
//!
//! Every meaningful playback action is recorded as a `ListenEvent`. The event
//! log is the *sole* input to the local recommendation engine (Level 2
//! personalization). Events are persisted to the state database (SQLite) so
//! they survive restarts, and the user can inspect, clear, reset, or disable
//! collection at any time (`PRIVACY.md`).
//!
//! ## Design principles
//!
//! - **Do not treat every start as positive.** A `TrackStarted` event alone is
//!   a weak signal — the user might immediately skip. Positive affinity is only
//!   recorded when the track reaches `MeaningfulThreshold` (≥50% or ≥30s).
//! - **Do not treat every skip as permanent dislike.** A single `Skipped` is a
//!   weak negative signal. Only `RapidlySkipped` (<10s) and multiple rapid
//!   skips of the same track accumulate into a strong negative.
//! - **Provenance over preference.** Events record *what happened*, not *what
//!   the recommender should do*. Interpretation lives in the profile builder.
//! - **Retention.** Events are retained for a configurable period (default 90
//!   days). `EventLog` is an in-memory ring buffer (10k cap) for fast access;
//!   `EventStore` is the durable SQLite backing.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Which playback source produced the event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum EventSource {
    #[default]
    Local,
    Youtube,
    Hybrid,
}

impl std::fmt::Display for EventSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventSource::Local => write!(f, "local"),
            EventSource::Youtube => write!(f, "youtube"),
            EventSource::Hybrid => write!(f, "hybrid"),
        }
    }
}

/// The context in which a track was played. Used by the candidate generator to
/// weight playlist/radio/mix neighbors differently from a bare album play.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum EventContext {
    #[default]
    Album,
    Playlist(String),
    Queue,
    Radio,
    Mix(String),
    Search,
    Discover,
    QuickPick,
}

/// A single recorded playback action. All variants carry a Unix-epoch-second
/// `timestamp` so the profile builder can compute recency and retention.
///
/// Events are serialized to JSON for SQLite storage (`EventStore`) and kept
/// in-memory in an `EventLog` ring buffer for fast access by the pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ListenEvent {
    /// A track started playing. **Weak signal** — the user might immediately
    /// skip. Only becomes positive when `MeaningfulThreshold` is reached.
    TrackStarted {
        track_id: String,
        source: EventSource,
        timestamp: u64,
        #[serde(default)]
        context: EventContext,
    },
    /// The track played past the meaningful threshold (≥50% of duration or
    /// ≥30s, whichever comes first). This is the first **positive** signal.
    MeaningfulThreshold { track_id: String, timestamp: u64 },
    /// The track played to completion (≥75% of duration). Strong positive.
    Completed { track_id: String, timestamp: u64 },
    /// The user skipped the track. **Weak negative** — may be mood, not
    /// dislike. Distinct from `RapidlySkipped`.
    Skipped {
        track_id: String,
        timestamp: u64,
        position_sec: f64,
    },
    /// The user skipped within the first 10 seconds. **Strong negative.**
    /// Multiple rapid skips of the same track accumulate into a strong
    /// dislike signal in the profile.
    RapidlySkipped { track_id: String, timestamp: u64 },
    /// The user replayed a track (seeked back to start). Positive signal.
    Replayed { track_id: String, timestamp: u64 },
    /// The user sought within the track. Neutral — informational only.
    Sought {
        track_id: String,
        timestamp: u64,
        from_sec: f64,
        to_sec: f64,
    },
    /// The user added a track to the queue. Positive intent.
    AddedToQueue { track_id: String, timestamp: u64 },
    /// The user removed a track from the queue. Mild negative.
    RemovedFromQueue { track_id: String, timestamp: u64 },
    /// The user liked a track. Strong positive — used as a direct seed.
    Liked { track_id: String, timestamp: u64 },
    /// The user unliked a previously-liked track. Neutralizes the like.
    Unliked { track_id: String, timestamp: u64 },
    /// The user explicitly disliked a track. Strong negative.
    Disliked { track_id: String, timestamp: u64 },
    /// The user hid a track from recommendations. The track must not appear in
    /// any future mix/radio/Home suggestion. Persistent negative.
    Hidden { track_id: String, timestamp: u64 },
    /// The user blocked an artist. All tracks by this artist are excluded from
    /// future recommendations. Persistent negative.
    ArtistBlocked { artist: String, timestamp: u64 },
    /// "Play less like this" — soft negative. Reduce frequency, don't
    /// eliminate. Distinct from `Hidden` (which fully excludes).
    PlayLess { track_id: String, timestamp: u64 },
    /// The user added a track to a playlist. Positive intent.
    AddedToPlaylist {
        track_id: String,
        playlist_name: String,
        timestamp: u64,
    },
    /// The user removed a track from a playlist. Mild negative.
    RemovedFromPlaylist {
        track_id: String,
        playlist_name: String,
        timestamp: u64,
    },
    /// A radio session was started from a seed. Used to weight radio context.
    RadioStarted { seed: String, timestamp: u64 },
    /// A generated mix was opened (viewed). Informational.
    MixOpened { mix_type: String, timestamp: u64 },
    /// A generated mix was played (at least one track started). Positive
    /// signal about the mix quality.
    MixPlayed { mix_type: String, timestamp: u64 },
    /// A recommendation was shown to the user. Paired with
    /// `RecommendationSelected`/`RecommendationDismissed` to measure
    /// recommendation quality.
    RecommendationShown {
        track_id: String,
        source: String,
        timestamp: u64,
    },
    /// The user selected (played) a shown recommendation. Strong positive for
    /// the recommendation source.
    RecommendationSelected {
        track_id: String,
        source: String,
        timestamp: u64,
    },
    /// The user dismissed a recommendation (skipped without playing). Mild
    /// negative for the recommendation source.
    RecommendationDismissed {
        track_id: String,
        source: String,
        timestamp: u64,
    },
    /// A search was performed. Informational — used for search-context
    /// candidate generation.
    SearchPerformed {
        query: String,
        scope: String,
        timestamp: u64,
    },
    /// The user selected a search result. Positive signal for that track.
    SearchResultSelected {
        track_id: String,
        query: String,
        timestamp: u64,
    },
    /// A source fallback occurred (local→YouTube or YouTube→local). Indicates
    /// availability state — used to avoid recommending unavailable tracks.
    SourceFallback {
        track_id: String,
        from_source: EventSource,
        to_source: EventSource,
        timestamp: u64,
    },
    /// Playback failed for a track. The track should be deprioritized in
    /// future recommendations (availability signal).
    PlaybackFailed {
        track_id: String,
        error: String,
        timestamp: u64,
    },
}

impl ListenEvent {
    /// The Unix-epoch-second timestamp of the event.
    pub fn timestamp(&self) -> u64 {
        match self {
            ListenEvent::TrackStarted { timestamp, .. }
            | ListenEvent::MeaningfulThreshold { timestamp, .. }
            | ListenEvent::Completed { timestamp, .. }
            | ListenEvent::Skipped { timestamp, .. }
            | ListenEvent::RapidlySkipped { timestamp, .. }
            | ListenEvent::Replayed { timestamp, .. }
            | ListenEvent::Sought { timestamp, .. }
            | ListenEvent::AddedToQueue { timestamp, .. }
            | ListenEvent::RemovedFromQueue { timestamp, .. }
            | ListenEvent::Liked { timestamp, .. }
            | ListenEvent::Unliked { timestamp, .. }
            | ListenEvent::Disliked { timestamp, .. }
            | ListenEvent::Hidden { timestamp, .. }
            | ListenEvent::ArtistBlocked { timestamp, .. }
            | ListenEvent::PlayLess { timestamp, .. }
            | ListenEvent::AddedToPlaylist { timestamp, .. }
            | ListenEvent::RemovedFromPlaylist { timestamp, .. }
            | ListenEvent::RadioStarted { timestamp, .. }
            | ListenEvent::MixOpened { timestamp, .. }
            | ListenEvent::MixPlayed { timestamp, .. }
            | ListenEvent::RecommendationShown { timestamp, .. }
            | ListenEvent::RecommendationSelected { timestamp, .. }
            | ListenEvent::RecommendationDismissed { timestamp, .. }
            | ListenEvent::SearchPerformed { timestamp, .. }
            | ListenEvent::SearchResultSelected { timestamp, .. }
            | ListenEvent::SourceFallback { timestamp, .. }
            | ListenEvent::PlaybackFailed { timestamp, .. } => *timestamp,
        }
    }

    /// The track_id associated with this event, if any. Events like
    /// `ArtistBlocked`, `RadioStarted`, `MixOpened`, `MixPlayed`, and
    /// `SearchPerformed` don't have a single track id.
    pub fn track_id(&self) -> Option<&str> {
        match self {
            ListenEvent::TrackStarted { track_id, .. }
            | ListenEvent::MeaningfulThreshold { track_id, .. }
            | ListenEvent::Completed { track_id, .. }
            | ListenEvent::Skipped { track_id, .. }
            | ListenEvent::RapidlySkipped { track_id, .. }
            | ListenEvent::Replayed { track_id, .. }
            | ListenEvent::Sought { track_id, .. }
            | ListenEvent::AddedToQueue { track_id, .. }
            | ListenEvent::RemovedFromQueue { track_id, .. }
            | ListenEvent::Liked { track_id, .. }
            | ListenEvent::Unliked { track_id, .. }
            | ListenEvent::Disliked { track_id, .. }
            | ListenEvent::Hidden { track_id, .. }
            | ListenEvent::PlayLess { track_id, .. }
            | ListenEvent::AddedToPlaylist { track_id, .. }
            | ListenEvent::RemovedFromPlaylist { track_id, .. }
            | ListenEvent::RecommendationShown { track_id, .. }
            | ListenEvent::RecommendationSelected { track_id, .. }
            | ListenEvent::RecommendationDismissed { track_id, .. }
            | ListenEvent::SearchResultSelected { track_id, .. }
            | ListenEvent::SourceFallback { track_id, .. }
            | ListenEvent::PlaybackFailed { track_id, .. } => Some(track_id),
            ListenEvent::ArtistBlocked { .. }
            | ListenEvent::RadioStarted { .. }
            | ListenEvent::MixOpened { .. }
            | ListenEvent::MixPlayed { .. }
            | ListenEvent::SearchPerformed { .. } => None,
        }
    }

    /// A short string identifying the event type for the `event_type` column.
    pub fn type_tag(&self) -> &'static str {
        match self {
            ListenEvent::TrackStarted { .. } => "track_started",
            ListenEvent::MeaningfulThreshold { .. } => "meaningful_threshold",
            ListenEvent::Completed { .. } => "completed",
            ListenEvent::Skipped { .. } => "skipped",
            ListenEvent::RapidlySkipped { .. } => "rapidly_skipped",
            ListenEvent::Replayed { .. } => "replayed",
            ListenEvent::Sought { .. } => "sought",
            ListenEvent::AddedToQueue { .. } => "added_to_queue",
            ListenEvent::RemovedFromQueue { .. } => "removed_from_queue",
            ListenEvent::Liked { .. } => "liked",
            ListenEvent::Unliked { .. } => "unliked",
            ListenEvent::Disliked { .. } => "disliked",
            ListenEvent::Hidden { .. } => "hidden",
            ListenEvent::ArtistBlocked { .. } => "artist_blocked",
            ListenEvent::PlayLess { .. } => "play_less",
            ListenEvent::AddedToPlaylist { .. } => "added_to_playlist",
            ListenEvent::RemovedFromPlaylist { .. } => "removed_from_playlist",
            ListenEvent::RadioStarted { .. } => "radio_started",
            ListenEvent::MixOpened { .. } => "mix_opened",
            ListenEvent::MixPlayed { .. } => "mix_played",
            ListenEvent::RecommendationShown { .. } => "recommendation_shown",
            ListenEvent::RecommendationSelected { .. } => "recommendation_selected",
            ListenEvent::RecommendationDismissed { .. } => "recommendation_dismissed",
            ListenEvent::SearchPerformed { .. } => "search_performed",
            ListenEvent::SearchResultSelected { .. } => "search_result_selected",
            ListenEvent::SourceFallback { .. } => "source_fallback",
            ListenEvent::PlaybackFailed { .. } => "playback_failed",
        }
    }

    /// Get the current Unix-epoch timestamp in seconds.
    pub fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// In-memory ring buffer for fast access by the pipeline. The durable backing
/// is `EventStore` (SQLite). `EventLog` holds the most recent 10,000 events so
/// the pipeline doesn't need to query the DB on every candidate generation.
#[derive(Debug, Default)]
pub struct EventLog {
    events: VecDeque<ListenEvent>,
    capacity: usize,
}

impl EventLog {
    const DEFAULT_CAPACITY: usize = 10_000;

    /// Create a new event log with the default capacity (10,000 events).
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Create a new event log with a custom capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Record an event. If the log is at capacity, the oldest event is evicted.
    pub fn record(&mut self, event: ListenEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Get the `n` most recent events (newest last).
    pub fn recent(&self, n: usize) -> Vec<&ListenEvent> {
        let start = self.events.len().saturating_sub(n);
        self.events.iter().skip(start).collect()
    }

    /// Get all events since the given timestamp.
    pub fn since(&self, timestamp: u64) -> Vec<&ListenEvent> {
        self.events
            .iter()
            .filter(|e| e.timestamp() >= timestamp)
            .collect()
    }

    /// Get all events for a specific track.
    pub fn for_track(&self, track_id: &str) -> Vec<&ListenEvent> {
        self.events
            .iter()
            .filter(|e| e.track_id() == Some(track_id))
            .collect()
    }

    /// Count of all recorded events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if no events are recorded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Load events from an iterator (e.g. from `EventStore::load_all`).
    pub fn extend_from(&mut self, events: impl IntoIterator<Item = ListenEvent>) {
        for e in events {
            self.record(e);
        }
    }
}

/// Durable SQLite backing for events. Each event is stored as a JSON blob with
/// a timestamp and type tag for efficient querying. The `events` table is
/// created by `state.rs` (schema version 3 migration).
pub struct EventStore;

impl EventStore {
    /// Save a single event to the database.
    pub fn save_at(event: &ListenEvent, conn: &rusqlite::Connection) -> anyhow::Result<()> {
        let json = serde_json::to_string(event)?;
        conn.execute(
            "INSERT INTO events (event_json, timestamp, event_type) VALUES (?1, ?2, ?3)",
            rusqlite::params![json, event.timestamp() as i64, event.type_tag()],
        )?;
        Ok(())
    }

    /// Load the `limit` most recent events (newest first).
    pub fn load_at(conn: &rusqlite::Connection, limit: usize) -> anyhow::Result<Vec<ListenEvent>> {
        let mut stmt = conn
            .prepare("SELECT event_json FROM events ORDER BY timestamp DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;
        let mut events = Vec::new();
        for row in rows {
            let json = row?;
            if let Ok(e) = serde_json::from_str::<ListenEvent>(&json) {
                events.push(e);
            }
        }
        // Reverse to chronological order (oldest first) for consistency with EventLog.
        events.reverse();
        Ok(events)
    }

    /// Load all events since the given timestamp (chronological order).
    pub fn load_since_at(
        conn: &rusqlite::Connection,
        since: u64,
    ) -> anyhow::Result<Vec<ListenEvent>> {
        let mut stmt = conn.prepare(
            "SELECT event_json FROM events WHERE timestamp >= ?1 ORDER BY timestamp ASC, id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![since as i64], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;
        let mut events = Vec::new();
        for row in rows {
            let json = row?;
            if let Ok(e) = serde_json::from_str::<ListenEvent>(&json) {
                events.push(e);
            }
        }
        Ok(events)
    }

    /// Clear all events from the database.
    pub fn clear_at(conn: &rusqlite::Connection) -> anyhow::Result<()> {
        conn.execute("DELETE FROM events", [])?;
        Ok(())
    }

    /// Count the total number of events.
    pub fn count_at(conn: &rusqlite::Connection) -> anyhow::Result<u64> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Delete events older than the given timestamp (retention enforcement).
    pub fn prune_before_at(conn: &rusqlite::Connection, before: u64) -> anyhow::Result<u64> {
        let count = conn.execute(
            "DELETE FROM events WHERE timestamp < ?1",
            rusqlite::params![before as i64],
        )?;
        Ok(count as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(base: u64, offset: u64) -> u64 {
        base + offset
    }

    #[test]
    fn event_source_display() {
        assert_eq!(EventSource::Local.to_string(), "local");
        assert_eq!(EventSource::Youtube.to_string(), "youtube");
        assert_eq!(EventSource::Hybrid.to_string(), "hybrid");
    }

    #[test]
    fn event_context_default_is_album() {
        assert!(matches!(EventContext::default(), EventContext::Album));
    }

    #[test]
    fn track_started_serializes_and_deserializes() {
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
    fn meaningful_threshold_event_has_correct_type_tag() {
        let e = ListenEvent::MeaningfulThreshold {
            track_id: "t1".into(),
            timestamp: 100,
        };
        assert_eq!(e.type_tag(), "meaningful_threshold");
    }

    #[test]
    fn completed_event_has_correct_type_tag() {
        let e = ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        };
        assert_eq!(e.type_tag(), "completed");
    }

    #[test]
    fn skipped_distinct_from_rapidly_skipped() {
        let skipped = ListenEvent::Skipped {
            track_id: "t1".into(),
            timestamp: 100,
            position_sec: 30.0,
        };
        let rapid = ListenEvent::RapidlySkipped {
            track_id: "t1".into(),
            timestamp: 100,
        };
        assert_eq!(skipped.type_tag(), "skipped");
        assert_eq!(rapid.type_tag(), "rapidly_skipped");
        assert_ne!(skipped.type_tag(), rapid.type_tag());
    }

    #[test]
    fn liked_event_serializes() {
        let e = ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"liked\""));
        let back: ListenEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.track_id(), Some("t1"));
    }

    #[test]
    fn artist_blocked_event_has_no_track_id() {
        let e = ListenEvent::ArtistBlocked {
            artist: "Bad Artist".into(),
            timestamp: 100,
        };
        assert!(e.track_id().is_none());
        assert_eq!(e.type_tag(), "artist_blocked");
    }

    #[test]
    fn hidden_event_serializes() {
        let e = ListenEvent::Hidden {
            track_id: "t1".into(),
            timestamp: 100,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"hidden\""));
    }

    #[test]
    fn play_less_event_distinct_from_hidden() {
        let play_less = ListenEvent::PlayLess {
            track_id: "t1".into(),
            timestamp: 100,
        };
        let hidden = ListenEvent::Hidden {
            track_id: "t1".into(),
            timestamp: 100,
        };
        assert_ne!(play_less.type_tag(), hidden.type_tag());
    }

    #[test]
    fn recommendation_shown_selected_dismissed_distinct() {
        let shown = ListenEvent::RecommendationShown {
            track_id: "t1".into(),
            source: "daily_mix".into(),
            timestamp: 100,
        };
        let selected = ListenEvent::RecommendationSelected {
            track_id: "t1".into(),
            source: "daily_mix".into(),
            timestamp: 100,
        };
        let dismissed = ListenEvent::RecommendationDismissed {
            track_id: "t1".into(),
            source: "daily_mix".into(),
            timestamp: 100,
        };
        assert_ne!(shown.type_tag(), selected.type_tag());
        assert_ne!(selected.type_tag(), dismissed.type_tag());
    }

    #[test]
    fn source_fallback_event_serializes() {
        let e = ListenEvent::SourceFallback {
            track_id: "t1".into(),
            from_source: EventSource::Local,
            to_source: EventSource::Youtube,
            timestamp: 100,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"source_fallback\""));
        let back: ListenEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timestamp(), 100);
    }

    #[test]
    fn playback_failed_event_serializes() {
        let e = ListenEvent::PlaybackFailed {
            track_id: "t1".into(),
            error: "network timeout".into(),
            timestamp: 100,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"playback_failed\""));
    }

    #[test]
    fn event_log_record_and_recent() {
        let mut log = EventLog::new();
        log.record(ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: ts(100, 0),
        });
        log.record(ListenEvent::Completed {
            track_id: "t2".into(),
            timestamp: ts(100, 1),
        });
        log.record(ListenEvent::Completed {
            track_id: "t3".into(),
            timestamp: ts(100, 2),
        });
        let recent = log.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].track_id(), Some("t2"));
        assert_eq!(recent[1].track_id(), Some("t3"));
    }

    #[test]
    fn event_log_capacity_eviction() {
        let mut log = EventLog::with_capacity(3);
        for i in 0..5 {
            log.record(ListenEvent::Completed {
                track_id: format!("t{i}"),
                timestamp: ts(100, i),
            });
        }
        assert_eq!(log.len(), 3);
        let recent = log.recent(3);
        // Oldest (t0, t1) should be evicted; t2, t3, t4 remain.
        assert_eq!(recent[0].track_id(), Some("t2"));
        assert_eq!(recent[2].track_id(), Some("t4"));
    }

    #[test]
    fn event_log_since() {
        let mut log = EventLog::new();
        log.record(ListenEvent::Completed {
            track_id: "t1".into(),
            timestamp: 100,
        });
        log.record(ListenEvent::Completed {
            track_id: "t2".into(),
            timestamp: 200,
        });
        log.record(ListenEvent::Completed {
            track_id: "t3".into(),
            timestamp: 300,
        });
        let since_200 = log.since(200);
        assert_eq!(since_200.len(), 2);
        assert_eq!(since_200[0].track_id(), Some("t2"));
    }

    #[test]
    fn event_log_for_track() {
        let mut log = EventLog::new();
        log.record(ListenEvent::Liked {
            track_id: "t1".into(),
            timestamp: 100,
        });
        log.record(ListenEvent::Skipped {
            track_id: "t1".into(),
            timestamp: 200,
            position_sec: 10.0,
        });
        log.record(ListenEvent::Liked {
            track_id: "t2".into(),
            timestamp: 300,
        });
        let for_t1 = log.for_track("t1");
        assert_eq!(for_t1.len(), 2);
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
    fn event_log_extend_from() {
        let mut log = EventLog::new();
        let events = vec![
            ListenEvent::Completed {
                track_id: "t1".into(),
                timestamp: 100,
            },
            ListenEvent::Completed {
                track_id: "t2".into(),
                timestamp: 200,
            },
        ];
        log.extend_from(events);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn track_started_not_treated_as_positive_in_type_tag() {
        // TrackStarted is a weak signal — the type tag distinguishes it from
        // MeaningfulThreshold (the first positive signal).
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
    fn all_event_types_have_unique_type_tags() {
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
        let mut tags: Vec<&str> = events.iter().map(|e| e.type_tag()).collect();
        let total = tags.len();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), total, "duplicate type tags found");
    }
}
