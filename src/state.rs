//! Persistent UI state across sessions, backed by a small SQLite database.
//!
//! Right now this stores only the last-focused pane (Artists / Search / Queue)
//! so the TUI reopens where you left it. The DB lives next to `config.yml` in
//! the config dir. `clear()` wipes the saved state so the next launch defaults
//! to the Artists pane.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The pane names stored in the DB. Keep these stable — changing them would
/// orphan previously-saved state. Match the `Pane` enum variants in `tui`.
pub const ARTISTS: &str = "artists";
pub const SEARCH: &str = "search";
pub const QUEUE: &str = "queue";

/// Resolve the state DB path: `<config_dir>/jukebox/state.db`. Honors
/// `$XDG_CONFIG_HOME`, else falls back to `dirs::config_dir()`, matching
/// `config::config_path()` so the two files live together.
///
/// The `/tmp/.config` fallback is acceptable here: `state.db` stores only UI
/// prefs (focus, column widths, volume, playlists) — no secrets. Cookie
/// secrets use `yt::session::cookies_file_opt()` which refuses the fallback.
pub fn db_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"));
    base.join("jukebox").join("state.db")
}

/// The current schema version. Stored in the DB as a 'schema_version' key.
/// Increment when the schema or stored JSON format changes incompatibly.
/// `open_at` auto-migrates older DBs by wiping + recreating (state is
/// ephemeral UI prefs, not user data — losing it is acceptable).
const SCHEMA_VERSION: u32 = 2;

/// Open (creating if missing) the state DB at `path` and ensure the schema
/// exists. Checks the stored schema version; if it's older than current,
/// wipes the DB and starts fresh (state is UI prefs, not user data).
/// Each launch opens + closes a connection — there's no long-lived
/// handle, so SQLite's file locking is fine for our single-process access.
///
/// **Corrupt-DB auto-recovery:** if opening or initializing the schema fails
/// (e.g. the file is garbage from a crash or short write), the file is
/// removed and re-opened fresh — state is ephemeral UI prefs, not user
/// data, so losing it is acceptable and beats a hard failure on launch.
fn open_at(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match open_and_init(path) {
        Ok(conn) => Ok(conn),
        Err(e) => {
            // Corrupt or unreadable DB — remove it and try once more. If the
            // remove fails (e.g. permissions) or the re-open fails, surface
            // the error so the caller isn't silently stuck.
            let _ = std::fs::remove_file(path);
            open_and_init(path).map_err(|_| e)
        }
    }
}

/// Open the DB at `path` and run the initial schema setup. Factored out of
/// `open_at` so the corrupt-DB recovery can retry the whole sequence (SQLite
/// doesn't validate the header on `Connection::open` — the error surfaces on
/// the first SQL operation, so we need to retry `execute_batch` too).
fn open_and_init(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    // Check schema version; wipe if older (migration = fresh start for UI prefs).
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM state WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .ok();
    let need_wipe = match stored {
        Some(s) => s.parse::<u32>().map(|v| v < SCHEMA_VERSION).unwrap_or(true),
        None => true, // no version key → first run or very old DB
    };
    if need_wipe {
        conn.execute("DELETE FROM state", [])?;
        conn.execute(
            "INSERT INTO state (key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
    }
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
        .query_row("SELECT value FROM state WHERE key = 'focus'", [], |row| {
            row.get(0)
        })
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

// --- Layout (focus, column widths, volume, shuffle/repeat) ---

/// Persisted browse layout + transport modes. `shuffle`/`repeat` are stored as
/// strings (`"off"`/`"smart"`/`"random"`, `"off"`/`"all"`/`"one"`) rather than
/// the enum types so we don't need serde derives on `ShuffleMode`/`RepeatMode`
/// (defined in `tui::queue`).
#[derive(Serialize, Deserialize, Clone)]
pub struct LayoutState {
    #[serde(default = "default_focus")]
    pub focus: String,
    #[serde(default)]
    pub widths: LayoutWidths,
    #[serde(default = "default_volume")]
    pub volume: u8,
    #[serde(default = "default_off")]
    pub shuffle: String,
    #[serde(default = "default_off")]
    pub repeat: String,
    #[serde(default = "default_off")]
    pub continue_mode: String,
    #[serde(default = "default_local")]
    pub source_mode: String,
    /// The browser profile to read YouTube cookies from at startup (e.g.
    /// "chrome"), set by `:yt auth browser <name>`. Empty/guest when unset —
    /// the sidecar then falls back to persisted pasted cookies, else guest.
    #[serde(default)]
    pub yt_browser: String,
}

fn default_focus() -> String {
    ARTISTS.to_string()
}

fn default_volume() -> u8 {
    70
}

fn default_off() -> String {
    "off".to_string()
}

fn default_local() -> String {
    "local".to_string()
}

impl Default for LayoutState {
    fn default() -> Self {
        LayoutState {
            focus: ARTISTS.to_string(),
            widths: LayoutWidths::default(),
            volume: 70,
            shuffle: "off".to_string(),
            repeat: "off".to_string(),
            continue_mode: "off".to_string(),
            source_mode: "local".to_string(),
            yt_browser: String::new(),
        }
    }
}

/// Persisted column widths. Mirrors `tui::app::ColumnWidths` but owned by the
/// state module so (de)serialization doesn't depend on that struct's layout.
#[derive(Serialize, Deserialize, Clone)]
pub struct LayoutWidths {
    #[serde(default = "default_rail")]
    pub rail: u16,
    #[serde(default = "default_col1")]
    pub col1: u16,
    #[serde(default = "default_col2")]
    pub col2: u16,
    #[serde(default = "default_col3")]
    pub col3: u16,
}

fn default_rail() -> u16 {
    4
}
fn default_col1() -> u16 {
    24
}
fn default_col2() -> u16 {
    28
}
fn default_col3() -> u16 {
    48
}

impl Default for LayoutWidths {
    fn default() -> Self {
        LayoutWidths {
            rail: 4,
            col1: 24,
            col2: 28,
            col3: 48,
        }
    }
}

/// The layout fields needed to persist a save, bundled into a struct so
/// `save_layout_at` / `save_layout` stay under clippy's argument-count limit
/// (the old 9-arg form tripped `too_many_arguments`). All fields are either
/// `Copy` (the enum modes + volume) or borrowed (`focus`, `widths`,
/// `yt_browser`), so callers build it inline at the call site with no extra
/// allocation.
pub struct LayoutSave<'a> {
    pub focus: &'a str,
    pub widths: &'a crate::tui::app::ColumnWidths,
    pub volume: u8,
    pub shuffle: crate::tui::queue::ShuffleMode,
    pub repeat: crate::tui::queue::RepeatMode,
    pub continue_mode: crate::tui::queue::ContinueMode,
    pub source_mode: crate::mode::SourceMode,
    pub yt_browser: &'a str,
}

/// Save the layout (focus + widths + volume + shuffle/repeat) to `path`.
/// `shuffle`/`repeat` are mapped from the enum modes to their string forms.
pub fn save_layout_at(path: &Path, input: &LayoutSave) -> Result<()> {
    let conn = open_at(path)?;
    let v = serde_json::to_string(&LayoutState {
        focus: input.focus.to_string(),
        widths: LayoutWidths {
            rail: input.widths.rail,
            col1: input.widths.col1,
            col2: input.widths.col2,
            col3: input.widths.col3,
        },
        volume: input.volume,
        shuffle: match input.shuffle {
            crate::tui::queue::ShuffleMode::Off => "off",
            crate::tui::queue::ShuffleMode::Smart => "smart",
            crate::tui::queue::ShuffleMode::Random => "random",
        }
        .to_string(),
        repeat: match input.repeat {
            crate::tui::queue::RepeatMode::Off => "off",
            crate::tui::queue::RepeatMode::All => "all",
            crate::tui::queue::RepeatMode::One => "one",
        }
        .to_string(),
        continue_mode: match input.continue_mode {
            crate::tui::queue::ContinueMode::Off => "off",
            crate::tui::queue::ContinueMode::NextAlbum => "next",
            crate::tui::queue::ContinueMode::Radio => "radio",
            crate::tui::queue::ContinueMode::YouTube => "youtube",
        }
        .to_string(),
        source_mode: input.source_mode.as_str().to_string(),
        yt_browser: input.yt_browser.to_string(),
    })?;
    conn.execute(
        "INSERT INTO state (key, value) VALUES ('layout', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [v],
    )?;
    Ok(())
}

/// Load the saved layout from `path`. Returns `LayoutState::default()` when no
/// 'layout' row exists yet (first launch).
pub fn load_layout_at(path: &Path) -> Result<LayoutState> {
    let conn = open_at(path)?;
    let v: Option<String> = conn
        .query_row("SELECT value FROM state WHERE key = 'layout'", [], |r| {
            r.get(0)
        })
        .ok();
    match v {
        Some(s) => Ok(serde_json::from_str(&s)?),
        None => Ok(LayoutState::default()),
    }
}

// --- Playlists ---

/// Save the user's playlists to `path` as a JSON array of `{name, track_ids}`.
pub fn save_playlists_at(path: &Path, playlists: &[crate::tui::app::Playlist]) -> Result<()> {
    let conn = open_at(path)?;
    let v = serde_json::to_string(playlists)?;
    conn.execute(
        "INSERT INTO state (key, value) VALUES ('playlists', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [v],
    )?;
    Ok(())
}

/// Load saved playlists from `path`. Returns an empty `Vec` if no 'playlists'
/// row exists yet (first launch).
pub fn load_playlists_at(path: &Path) -> Result<Vec<crate::tui::app::Playlist>> {
    let conn = open_at(path)?;
    match conn.query_row("SELECT value FROM state WHERE key = 'playlists'", [], |r| {
        r.get::<_, String>(0)
    }) {
        Ok(s) => Ok(serde_json::from_str(&s)?),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

// --- Command history ---

/// Save the `:` command history to `path` as a JSON array of strings under the
/// `'command_history'` key. UPSERT so the row is created on first save and
/// updated thereafter. Bounded + adjacent-deduped by the caller (the App); this
/// layer just persists whatever Vec it receives.
pub fn save_command_history_at(path: &Path, history: &[String]) -> Result<()> {
    let conn = open_at(path)?;
    let v = serde_json::to_string(history)?;
    conn.execute(
        "INSERT INTO state (key, value) VALUES ('command_history', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [v],
    )?;
    Ok(())
}

/// Load saved command history from `path`. Returns an empty `Vec` if no
/// `'command_history'` row exists yet (first launch).
pub fn load_command_history_at(path: &Path) -> Result<Vec<String>> {
    let conn = open_at(path)?;
    match conn.query_row(
        "SELECT value FROM state WHERE key = 'command_history'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        Ok(s) => Ok(serde_json::from_str(&s)?),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

// --- Default-path convenience wrappers for layout + playlists ---

/// Save the layout to the default DB path.
pub fn save_layout(input: &LayoutSave) -> Result<()> {
    save_layout_at(&db_path(), input)
}

/// Load the layout from the default DB path.
pub fn load_layout() -> Result<LayoutState> {
    load_layout_at(&db_path())
}

/// Save playlists to the default DB path.
pub fn save_playlists(playlists: &[crate::tui::app::Playlist]) -> Result<()> {
    save_playlists_at(&db_path(), playlists)
}

/// Load playlists from the default DB path.
pub fn load_playlists() -> Result<Vec<crate::tui::app::Playlist>> {
    load_playlists_at(&db_path())
}

/// Save command history to the default DB path.
pub fn save_command_history(history: &[String]) -> Result<()> {
    save_command_history_at(&db_path(), history)
}

/// Load command history from the default DB path.
pub fn load_command_history() -> Result<Vec<String>> {
    load_command_history_at(&db_path())
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
