//! Disk-backed lyrics cache with track-change invalidation.
//!
//! Caches ytmusicapi lyrics results to `state.db` so a re-open of the lyrics
//! overlay for a recently-played track is instant (no sidecar roundtrip).
//! Local lyrics (embedded/sidecar) are NOT cached here — they're always re-read
//! from disk (fast, and the file may have changed since the last read).
//!
//! ## Invalidation
//!
//! The cache is keyed by `video_id` (YouTube tracks). A local track's lyrics
//! are never cached (they're re-read from disk each time — a `metaflac` +
//! filesystem read is ~10ms). The cache is invalidated by:
//! - Track change: the overlay's `track_id` differs from the cached entry.
//! - Generation guard: `lyrics_gen` advances on every `request_lyrics`, so a
//!   stale cache hit for a prior track is never applied to the current overlay.
//! - `:yt logout`: clears the entire cache (credentials changed → old results
//!   may be for a different account).
//!
//! ## Schema
//!
//! `lyrics_cache` table in `state.db`:
//! ```sql
//! CREATE TABLE IF NOT EXISTS lyrics_cache (
//!   video_id TEXT PRIMARY KEY,
//!   lines    TEXT NOT NULL,   -- JSON array of LyricLineProto
//!   synced   INTEGER NOT NULL, -- 0 or 1
//!   cached_at INTEGER NOT NULL -- unix timestamp (seconds)
//! );
//! ```
//!
//! Entries expire after `CACHE_TTL` (24h) — ytmusicapi lyrics can change
//! (corrections, new sources), so we don't cache indefinitely.

use crate::yt::proto::LyricLineProto;
use anyhow::{Context, Result};
use std::time::{SystemTime, UNIX_EPOCH};

/// The lyrics cache key (YouTube video_id). Local tracks aren't cached.
pub type CacheKey = String;

/// Cache TTL: 24 hours. ytmusicapi lyrics can change, so we don't cache
/// indefinitely. A re-open after TTL re-fetches from the sidecar.
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

/// A cached lyrics entry: the parsed lines + whether they're synced.
#[derive(Clone, Debug)]
pub struct CachedLyrics {
    pub lines: Vec<LyricLineProto>,
    pub synced: bool,
}

/// Load a cached lyrics entry for `video_id` from `state.db`. Returns `None`
/// when the entry is absent, expired (past `CACHE_TTL`), or the DB is
/// unreadable (graceful degradation — the caller re-fetches from the sidecar).
pub fn load(db_path: &std::path::Path, video_id: &str) -> Option<CachedLyrics> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    ensure_schema(&conn).ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let row = conn
        .query_row(
            "SELECT lines, synced, cached_at FROM lyrics_cache WHERE video_id = ?1",
            rusqlite::params![video_id],
            |r| {
                let lines_json: String = r.get(0)?;
                let synced: i64 = r.get(1)?;
                let cached_at: i64 = r.get(2)?;
                Ok((lines_json, synced, cached_at))
            },
        )
        .ok()?;
    let (lines_json, synced, cached_at) = row;
    // Expire entries older than CACHE_TTL.
    if (now as i64 - cached_at) > CACHE_TTL_SECS as i64 {
        return None;
    }
    let lines: Vec<LyricLineProto> = serde_json::from_str(&lines_json)
        .context("parsing cached lyrics")
        .ok()?;
    Some(CachedLyrics {
        lines,
        synced: synced != 0,
    })
}

/// Save a lyrics entry for `video_id` to `state.db`. Best-effort: a failed
/// save (read-only dir, disk full) is silently dropped — the caller already
/// has the lyrics in memory; the cache is an optimization, not a requirement.
pub fn save(db_path: &std::path::Path, video_id: &str, lyrics: &CachedLyrics) {
    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        return;
    };
    if ensure_schema(&conn).is_err() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let lines_json = match serde_json::to_string(&lyrics.lines) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = conn.execute(
        "INSERT OR REPLACE INTO lyrics_cache (video_id, lines, synced, cached_at) \
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![video_id, lines_json, lyrics.synced as i64, now as i64],
    );
}

/// Clear the entire lyrics cache. Called on `:yt logout` (credentials changed
/// → cached results may be for a different account) and on schema migration.
pub fn clear(db_path: &std::path::Path) -> Result<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_schema(&conn)?;
    conn.execute("DELETE FROM lyrics_cache", [])?;
    Ok(())
}

/// Create the `lyrics_cache` table if it doesn't exist. Idempotent.
fn ensure_schema(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS lyrics_cache (
            video_id  TEXT PRIMARY KEY,
            lines     TEXT NOT NULL,
            synced    INTEGER NOT NULL,
            cached_at INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> std::path::PathBuf {
        let dir = tempfile::tempdir().unwrap();
        dir.keep().join("test-lyrics-cache.db")
    }

    #[test]
    fn save_then_load_roundtrip() {
        let db = tmp_db();
        let entry = CachedLyrics {
            lines: vec![
                LyricLineProto {
                    time: Some(1.5),
                    text: "hello".into(),
                },
                LyricLineProto {
                    time: None,
                    text: "world".into(),
                },
            ],
            synced: true,
        };
        save(&db, "vid1", &entry);
        let loaded = load(&db, "vid1").unwrap();
        assert!(loaded.synced);
        assert_eq!(loaded.lines.len(), 2);
        assert_eq!(loaded.lines[0].time, Some(1.5));
        assert_eq!(loaded.lines[0].text, "hello");
    }

    #[test]
    fn load_returns_none_for_absent() {
        let db = tmp_db();
        assert!(load(&db, "no-such-vid").is_none());
    }

    #[test]
    fn clear_removes_all_entries() {
        let db = tmp_db();
        let entry = CachedLyrics {
            lines: vec![LyricLineProto {
                time: None,
                text: "x".into(),
            }],
            synced: false,
        };
        save(&db, "v1", &entry);
        save(&db, "v2", &entry);
        assert!(load(&db, "v1").is_some());
        clear(&db).unwrap();
        assert!(load(&db, "v1").is_none());
        assert!(load(&db, "v2").is_none());
    }

    #[test]
    fn save_overwrites_existing() {
        let db = tmp_db();
        let e1 = CachedLyrics {
            lines: vec![LyricLineProto {
                time: Some(1.0),
                text: "old".into(),
            }],
            synced: true,
        };
        save(&db, "vid", &e1);
        let e2 = CachedLyrics {
            lines: vec![LyricLineProto {
                time: None,
                text: "new".into(),
            }],
            synced: false,
        };
        save(&db, "vid", &e2);
        let loaded = load(&db, "vid").unwrap();
        assert!(!loaded.synced);
        assert_eq!(loaded.lines[0].text, "new");
    }
}
