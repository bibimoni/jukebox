//! Disk cache for YouTube playlist lists, backed by the state DB.
//!
//! Saves the `App::yt_lists` (account + suggested playlists) to `state.db`
//! under the `'yt_lists_cache'` key as JSON, so a launch while offline can
//! show cached playlists (marked stale via `YtState::ReadyStale`) instead of
//! an empty Y view. The cache is refreshed on every successful sync (on_tick
//! merge) and cleared on logout.
//!
//! ## Why a mirror struct (`CachedYtList`)?
//!
//! `YtList` (in `tui::app`) is a plain UI struct that doesn't derive
//! `Serialize`/`Deserialize` — adding those derives would couple the UI
//! struct to the storage format and pull `serde` into every module that
//! touches `YtList`. Instead we mirror just the four serializable fields
//! here, keeping the storage format local to this cache module. `kind` is
//! serialized as a lowercase string ("account" / "suggested") so the stored
//! JSON is stable across enum reordering.
//!
//! ## SQL key binding
//!
//! The row is keyed by the `KEY` constant (`'yt_lists_cache'`) via a bound
//! parameter: `VALUES (?1, ?2)` with `params![KEY, &json]`. Binding the same
//! value to both `key` and `value` columns (`VALUES (?1, ?1)`) — an earlier
//! bug — wrote the JSON into the `key` column, so `load_yt_lists`'s
//! `WHERE key = KEY` never matched and the cache was silently non-functional.
//! Two distinct bound params (key + value) fix the round-trip, and `KEY`
//! stays the single source of truth (no string duplication across
//! save/load/clear).

use crate::state;
use crate::tui::app::YtList;
use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// A serializable mirror of `YtList` (avoids adding serde derives to the
/// app struct, which carries `HashSet` and other non-serde fields nearby).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CachedYtList {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub track_ids: Vec<String>,
}

/// The `state`-table key under which the cached yt_lists are stored as JSON.
const KEY: &str = "yt_lists_cache";

/// Open the DB at `path` and ensure the `state` key/value table exists.
/// Idempotent. We don't run `state::open_at`'s schema-version migration here
/// — by the time the cache is touched at launch, `state::load_layout` has
/// already opened (and migrated) the DB, and the `state` table schema is
/// stable. This keeps the cache module self-contained (mirrors `lyrics/cache`).
fn open_at(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(conn)
}

/// Save the current `yt_lists` to `path` under the `'yt_lists_cache'` key.
/// UPSERT so the row is created on first save and updated thereafter.
pub fn save_yt_lists_at(path: &std::path::Path, lists: &[YtList]) -> Result<()> {
    let cached: Vec<CachedYtList> = lists
        .iter()
        .map(|l| CachedYtList {
            id: l.id.clone(),
            name: l.name.clone(),
            kind: match l.kind {
                crate::tui::app::YtListKind::Account => "account",
                crate::tui::app::YtListKind::Suggested => "suggested",
            }
            .to_string(),
            track_ids: l.track_ids.clone(),
        })
        .collect();
    let conn = open_at(path)?;
    let v = serde_json::to_string(&cached)?;
    // Bound key (?1 = KEY) + bound value (?2 = JSON). A prior `VALUES (?1, ?1)`
    // form bound the JSON to BOTH columns — the key column held the JSON, so
    // `load_yt_lists`'s `WHERE key = KEY` never matched and the cache was
    // silently non-functional. Two distinct params fix it.
    conn.execute(
        "INSERT INTO state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![KEY, &v],
    )?;
    Ok(())
}

/// Load cached `yt_lists` from `path`. Returns an empty `Vec` when no
/// `'yt_lists_cache'` row exists yet (first launch, or after `clear_yt_lists`).
pub fn load_yt_lists_at(path: &std::path::Path) -> Result<Vec<YtList>> {
    let conn = open_at(path)?;
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM state WHERE key = ?1",
            rusqlite::params![KEY],
            |r| r.get::<_, String>(0),
        )
        .ok();
    match v {
        Some(s) => {
            let cached: Vec<CachedYtList> = serde_json::from_str(&s)?;
            Ok(cached
                .into_iter()
                .map(|c| YtList {
                    id: c.id,
                    name: c.name,
                    kind: match c.kind.as_str() {
                        "suggested" => crate::tui::app::YtListKind::Suggested,
                        _ => crate::tui::app::YtListKind::Account,
                    },
                    track_ids: c.track_ids,
                })
                .collect())
        }
        None => Ok(Vec::new()),
    }
}

/// Clear the cached yt_lists at `path` (called on logout so stale data
/// doesn't survive a credential change).
pub fn clear_yt_lists_at(path: &std::path::Path) -> Result<()> {
    let conn = open_at(path)?;
    conn.execute("DELETE FROM state WHERE key = ?1", rusqlite::params![KEY])?;
    Ok(())
}

// --- Default-path convenience wrappers (best-effort) ---

/// Save the current `yt_lists` to the default state DB. Best-effort: a failed
/// save (read-only dir, disk full) is silently dropped — the caller already
/// has the lists in memory; the cache is an optimization, not a requirement.
pub fn save_yt_lists(lists: &[YtList]) {
    let _ = save_yt_lists_at(&state::db_path(), lists);
}

/// Load cached `yt_lists` from the default state DB. Empty on first launch
/// or after logout (the cache is cleared by `clear_yt_lists`).
pub fn load_yt_lists() -> Vec<YtList> {
    load_yt_lists_at(&state::db_path()).unwrap_or_default()
}

/// Clear the cached yt_lists in the default state DB (best-effort).
pub fn clear_yt_lists() {
    let _ = clear_yt_lists_at(&state::db_path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{YtList, YtListKind};

    fn tmp_db() -> std::path::PathBuf {
        let dir = tempfile::tempdir().unwrap();
        dir.keep().join("test-yt-lists-cache.db")
    }

    #[test]
    fn save_then_load_round_trips() {
        let db = tmp_db();
        let lists = vec![
            YtList {
                id: "PL1".into(),
                name: "My Playlist".into(),
                kind: YtListKind::Account,
                track_ids: vec!["v1".into(), "v2".into()],
            },
            YtList {
                id: "RD1".into(),
                name: "Suggested".into(),
                kind: YtListKind::Suggested,
                track_ids: vec![],
            },
        ];
        save_yt_lists_at(&db, &lists).unwrap();
        let loaded = load_yt_lists_at(&db).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "PL1");
        assert_eq!(loaded[0].name, "My Playlist");
        assert_eq!(loaded[0].kind, YtListKind::Account);
        assert_eq!(
            loaded[0].track_ids,
            vec!["v1".to_string(), "v2".to_string()]
        );
        assert_eq!(loaded[1].id, "RD1");
        assert_eq!(loaded[1].kind, YtListKind::Suggested);
    }

    #[test]
    fn load_returns_empty_for_absent() {
        let db = tmp_db();
        assert!(load_yt_lists_at(&db).unwrap().is_empty());
    }

    #[test]
    fn save_overwrites_existing() {
        let db = tmp_db();
        save_yt_lists_at(
            &db,
            &[YtList {
                id: "PL1".into(),
                name: "Old".into(),
                kind: YtListKind::Account,
                track_ids: vec![],
            }],
        )
        .unwrap();
        save_yt_lists_at(
            &db,
            &[YtList {
                id: "PL2".into(),
                name: "New".into(),
                kind: YtListKind::Account,
                track_ids: vec![],
            }],
        )
        .unwrap();
        let loaded = load_yt_lists_at(&db).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "PL2");
        assert_eq!(loaded[0].name, "New");
    }

    #[test]
    fn clear_removes_the_cache() {
        let db = tmp_db();
        save_yt_lists_at(
            &db,
            &[YtList {
                id: "PL1".into(),
                name: "X".into(),
                kind: YtListKind::Account,
                track_ids: vec![],
            }],
        )
        .unwrap();
        assert!(!load_yt_lists_at(&db).unwrap().is_empty());
        clear_yt_lists_at(&db).unwrap();
        assert!(load_yt_lists_at(&db).unwrap().is_empty());
    }
}
