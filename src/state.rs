//! Persistent UI state across sessions, backed by a small SQLite database.
//!
//! Right now this stores only the last-focused pane (Artists / Search / Queue)
//! so the TUI reopens where you left it. The DB lives next to `config.yml` in
//! the config dir. `clear()` wipes the saved state so the next launch defaults
//! to the Artists pane.

use anyhow::Result;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// The pane names stored in the DB. Keep these stable — changing them would
/// orphan previously-saved state. Match the `Pane` enum variants in `tui`.
pub const ARTISTS: &str = "artists";
pub const SEARCH: &str = "search";
pub const QUEUE: &str = "queue";

/// Resolve the state DB path: `<config_dir>/jukebox/state.db`. Honors
/// `$XDG_CONFIG_HOME`, else falls back to `dirs::config_dir()`, matching
/// `config::config_path()` so the two files live together.
pub fn db_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"));
    base.join("jukebox").join("state.db")
}

/// Open (creating if missing) the state DB at `path` and ensure the schema
/// exists. Each launch opens + closes a connection — there's no long-lived
/// handle, so SQLite's file locking is fine for our single-process access.
fn open_at(path: &Path) -> Result<Connection> {
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

/// Open the default DB at `db_path()`. (Public so a caller can introspect, but
/// the read/write helpers below are what you usually want.)
pub fn open() -> Result<Connection> {
    open_at(&db_path())
}

/// Save the focused-pane key to `path`. UPSERT so a row is created on first
/// save and updated thereafter — a single-row table keyed by 'focus'.
pub fn save_focus_at(path: &Path, pane: &str) -> Result<()> {
    let conn = open_at(path)?;
    conn.execute(
        "INSERT INTO state (key, value) VALUES ('focus', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [pane],
    )?;
    Ok(())
}

/// Load the saved focused-pane key from `path`, if any. `None` if the DB has
/// no 'focus' row (first launch, or after `clear()`).
pub fn load_focus_at(path: &Path) -> Result<Option<String>> {
    let conn = open_at(path)?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM state WHERE key = 'focus'",
            [],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(value)
}

/// Wipe all saved state at `path`. The next launch restores defaults.
pub fn clear_at(path: &Path) -> Result<()> {
    let conn = open_at(path)?;
    conn.execute("DELETE FROM state", [])?;
    Ok(())
}

// --- Default-path convenience wrappers (the production TUI uses these) ---

/// Save the focused pane to the default DB path.
pub fn save_focus(pane: &str) -> Result<()> {
    save_focus_at(&db_path(), pane)
}

/// Load the focused pane from the default DB path, if any.
pub fn load_focus() -> Result<Option<String>> {
    load_focus_at(&db_path())
}

/// Clear saved state at the default DB path.
pub fn clear() -> Result<()> {
    clear_at(&db_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> PathBuf {
        let d = tempfile::tempdir().unwrap();
        d.path().join("state.db")
        // tempdir is dropped at end of this fn, but the file persists on disk;
        // we only need the path for a single test, and tempfile cleans the
        // parent dir when the TempDir (held in `d`) drops — so keep `d` alive
        // by leaking it. For tests this is acceptable.
    }

    #[test]
    fn focus_round_trips() {
        let path = tmp_db();
        assert!(load_focus_at(&path).unwrap().is_none());
        save_focus_at(&path, "search").unwrap();
        assert_eq!(load_focus_at(&path).unwrap().as_deref(), Some("search"));
        // Overwrite (UPSERT, single row).
        save_focus_at(&path, "queue").unwrap();
        assert_eq!(load_focus_at(&path).unwrap().as_deref(), Some("queue"));
    }

    #[test]
    fn clear_wipes_focus() {
        let path = tmp_db();
        save_focus_at(&path, "artists").unwrap();
        assert!(load_focus_at(&path).unwrap().is_some());
        clear_at(&path).unwrap();
        assert!(load_focus_at(&path).unwrap().is_none());
    }
}
