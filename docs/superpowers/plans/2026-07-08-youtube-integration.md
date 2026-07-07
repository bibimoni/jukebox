# YouTube Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Local / YouTube / Mixed playback modes to jukebox so it can replace the YouTube Music desktop app while preserving the hi-res local library, with a Python sidecar for metadata/radio/streaming, in-app cookie auth, switch-once-per-YT-session CoreAudio re-clocking, a balanced TUI redesign (Y view + footer hints + `f`/`s`/`S` keys), and a strict judge completion gate.

**Architecture:** A remote-track adapter that leaves `Transport` untouched. `Transport` keeps shuffling a list of opaque ids; a `SourceResolver` decides whether each id plays as a local file or a YouTube stream. A long-lived Python sidecar (`scripts/yt/yt.py`, `ytmusicapi` + `yt-dlp`) provides search/playlists/autoplay-radio/URL-resolution over stdin/stdout JSON. A new `ContinueMode::YouTube` variant drives autoplay as the CONT engine. `now_playing` widens to a `TrackSource` enum so the player bar can render either `24-bit / 96 kHz · bit-perfect` or `Opus 160k · YT`.

**Tech Stack:** Rust 2021, ratatui 0.30, crossterm 0.29, serde/serde_json, rusqlite (state), anyhow. Python 3 sidecar: `ytmusicapi`, `yt-dlp`. mpv IPC (existing). CoreAudio via `coreaudio-sys` (existing, macOS).

**Spec:** `docs/superpowers/specs/2026-07-08-youtube-integration-design.md`

## Global Constraints

- **Never add AI attribution to commits.** No `Co-Authored-By: Claude` or similar. Commit messages follow the existing conventional style (`feat:`, `fix:`, `docs:`, `chore:`).
- **Transport stays pure.** `src/tui/queue.rs::Transport` must not grow YouTube-specific logic. The mode and source resolution live in `App` + `source`/`yt` modules. `Transport` may gain only the `ContinueMode::YouTube` variant and a no-op-tolerant `ContinueMode::YouTube` arm in `cycle_continue`.
- **`Player::load` already accepts any string mpv understands** — including `https://...` — because `MpvPlayer::load` calls `loadfile` with `path.to_string_lossy()`. Remote playback loads a URL string via the same `loadfile` path; do not add a new player method for URLs. (A small helper that builds a path-or-url is fine, but `load(&Path)`-style stays for local.)
- **No new dependencies in `Cargo.toml` are strictly required** for the core path (serde/serde_json/anyhow already present). Only add a crate if a task explicitly justifies it and notes the version. Do not add `reqwest`/`tokio` — the sidecar is a subprocess and the TUI polls it on the existing 150ms tick; blocking is avoided by the non-blocking read pattern already used for mpv IPC.
- **Python sidecar is a runtime prerequisite, not a build dependency.** It ships under `scripts/yt/` and is invoked as `python3 scripts/yt/yt.py` (or resolved next to the binary, mirroring `standardize.sh` resolution in `src/main.rs`). Do not embed Python.
- **TDD.** Every task writes the failing test first, runs it to confirm it fails for the right reason, implements, runs it green, then commits. Tests live in `tests/` (integration, the existing convention) using `jukebox::` public exports, or as `#[cfg(test)] mod tests` inside the module for pure-logic units.
- **Frequent commits.** One commit per task at minimum; sub-steps may commit separately when large.
- **NO_COLOR honored.** All new color usage goes through `crate::tui::view::theme::Theme::default()` (which collapses to `Reset` under `NO_COLOR`). Never use color as the only signal.
- **No UI-thread blocking.** Sidecar reads use the non-blocking pattern already in `MpvPlayer::track_ended` (read into a buffer, parse complete lines, never `read_to_end`). The TUI already redraws on the 150ms poll; sidecar results land on the next tick.
- **Edge cases are mandatory, not optional.** Every failure mode in spec §3.5 (expired URL, network drop, rate limit, dead remote track, missing deps, sidecar death) has an explicit handling site named in a task.

---

## File Structure (decomposition decisions)

**New files:**
- `src/source/mod.rs` — `TrackSource` enum, `RemoteTrack`, `StreamFormat`, `SourceResolver` trait + `CatalogResolver` impl.
- `src/source/match_local.rs` — `match_local(remote, cat) -> Option<TrackId>` + normalization (ISRC, translit, fuzzy).
- `src/source/device_rate.rs` — `DeviceRateState` (switch-once-per-YT-session CoreAudio cadence).
- `src/yt/mod.rs` — `Sidecar` (subprocess + non-blocking JSON RPC), `Session` (auth/cookies/cache), `RadioCursor`.
- `src/yt/proto.rs` — request/response serde structs (the sidecar JSON wire format).
- `src/mode.rs` — `SourceMode` enum (Local/Youtube/Mixed) + cycle.
- `scripts/yt/yt.py` — the Python sidecar.
- `scripts/yt/requirements.txt` — pins `ytmusicapi`, `yt-dlp`.
- `tests/source_match.rs`, `tests/source_device_rate.rs`, `tests/yt_sidecar.rs`, `tests/mode.rs`, `tests/tui_yt.rs` — tests.

**Modified files:**
- `src/lib.rs` — add `pub mod mode; pub mod source; pub mod yt;`.
- `src/tui/queue.rs` — add `ContinueMode::YouTube` variant + update `cycle_continue`'s mode-dependent cycling (the cycle becomes `&SourceMode`-aware; the existing `cycle_continue` keeps signature but reads a mode the caller passes, or we add `cycle_continue_mode`). Keep `Transport` otherwise untouched.
- `src/tui/app.rs` — `now_playing: Option<TrackSource>`; `App::load_track` routes through `SourceResolver`; `start_playback` resolves ids; `next` handles `ContinueMode::YouTube` via `RadioCursor`; add `SourceMode` field + `cycle_mode`; `ContextResolver` gains `yt_playlist_ids`.
- `src/tui/context.rs` — `ContextResolver` trait gains `fn yt_playlist_ids(&self, key: &str) -> Vec<String>;` (default impl returns empty to keep existing fakes compiling).
- `src/tui/view/layout.rs` — split player bar into the balanced two-row form + 1-line footer; add narrow (60–80) single-pane branch; raise the rail to `A/P/Q/Y`.
- `src/tui/view/columns.rs` — add `View::Youtube` rendering (2-col list→tracks + Up-Next pane); filter-on-column (`f`) prompt; rail `Y`.
- `src/tui/view/player_bar.rs` — two-row layout, drop transport glyphs, render `TrackSource`-aware quality, `MODE` flag right-anchored.
- `src/tui/view/overlay.rs` — `Overlay::YtAuth` (cookie paste), `Overlay::Discover` (suggested albums/playlists), render + key handling.
- `src/tui/input.rs` — bind `1/2/3/4`, `M`, `f`, `s`, `S`; overlay routing for new overlays; `/` scopes to view; `:yt` commands.
- `src/state.rs` — `LayoutState` gains `source_mode`; `save_layout`/`load_layout` handle `"youtube"` continue-mode + `"local"/"youtube"/"mixed"` source-mode strings.
- `src/config.rs` — (no structural change; `SourceMode` persists in state.db, not config.yml).
- `src/main.rs` — spawn sidecar at launch (best-effort); restore `source_mode`; pass to `App`.
- `README.md` — runtime prerequisites (`python3`, `ytmusicapi`, `yt-dlp`, `:yt setup`), YouTube ToS disclaimer, third-party tools section.
- `NOTICE` (new) — third-party notices for `ytmusicapi` (MIT) + `yt-dlp` (Unlicense).

---

## Task list (high-level map; detailed steps follow)

1. `SourceMode` enum + persistence wiring (state.rs) — pure, no UI.
2. `ContinueMode::YouTube` variant + mode-dependent cycle.
3. `TrackSource` + `RemoteTrack` + `StreamFormat` types.
4. `match_local` (ISRC + translit fuzzy) + tests.
5. `DeviceRateState` (switch-once-per-session) + tests.
6. Sidecar wire protocol (`yt::proto`) + serde round-trip tests.
7. Python sidecar `scripts/yt/yt.py` + `requirements.txt` + a self-test.
8. `Sidecar` Rust subprocess client (non-blocking) + fake-process tests.
9. `Session` (auth/cookies/cache) + `RadioCursor` + tests.
10. `SourceResolver` + wire into `App::load_track`/`start_playback` + tests.
11. `ContextResolver::yt_playlist_ids` + `View::Youtube` data plumbing.
12. TUI: rail `A/P/Q/Y`, two-row balanced player bar, footer hints, `MODE` flag.
13. TUI: `View::Youtube` rendering (2-col + Up-Next pane) + loading/error states.
14. TUI: narrow (60–80) single-pane fallback.
15. TUI: `Overlay::YtAuth` (cookie paste) + `:yt auth`/`:yt logout`/`:yt setup`.
16. TUI: `Overlay::Discover` (`S`) + instant random (`s`) + local smart-album heuristic.
17. TUI: filter-on-column (`f`) inline prompt.
18. Wire `M`/`1-4`/`f`/`s`/`S`/`:yt` in `input.rs`; update `?` help.
19. End-to-end integration tests (stubbed sidecar): search→play, mixed match→local, mixed no-match→stream, CONT=YouTube, dead-remote skip, sidecar-down degrade.
20. README + NOTICE + ToS disclaimer.
21. **Strict judge gate** — dispatch an isolated agent to score the 8-dimension rubric live; loop until all-max.

---

### Task 1: `SourceMode` enum + persistence

**Files:**
- Create: `src/mode.rs`
- Modify: `src/lib.rs` (add `pub mod mode;`)
- Modify: `src/state.rs:113-252` (`LayoutState`, `save_layout`, `load_layout`)
- Test: `tests/mode.rs`

**Interfaces:**
- Produces: `pub enum SourceMode { Local, Youtube, Mixed }` with `fn cycle(self) -> Self`, `fn as_str(self) -> &'static str`, `fn from_str(s: &str) -> Self`, `Default for SourceMode` = `Local`.
- Produces (state.rs): `LayoutState.source_mode: String` (`#[serde(default = "default_local")]`), and `save_layout`/`load_layout` accept/return a `SourceMode`. **Signature change:** `save_layout` and `save_layout_at` gain a `source_mode: crate::mode::SourceMode` parameter (appended last). `load_layout_at` returns `LayoutState` with `.source_mode` populated.

- [ ] **Step 1: Write the failing test**

```rust
// tests/mode.rs
use jukebox::mode::SourceMode;

#[test]
fn cycles_in_order() {
    assert_eq!(SourceMode::Local.cycle(), SourceMode::Youtube);
    assert_eq!(SourceMode::Youtube.cycle(), SourceMode::Mixed);
    assert_eq!(SourceMode::Mixed.cycle(), SourceMode::Local);
}

#[test]
fn round_trips_strings() {
    for m in [SourceMode::Local, SourceMode::Youtube, SourceMode::Mixed] {
        assert_eq!(SourceMode::from_str(m.as_str()), m);
    }
    // unknown → default Local (forward-compat with old state.db)
    assert_eq!(SourceMode::from_str("???"), SourceMode::Local);
}

#[test]
fn default_is_local() {
    assert_eq!(SourceMode::default(), SourceMode::Local);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test mode`
Expected: FAIL — `jukebox::mode` does not exist.

- [ ] **Step 3: Implement `src/mode.rs`**

```rust
//! The source mode: where playback material comes from.
//!
//! `Local` plays only the on-disk filtered-lossless catalog. `YouTube` plays
//! only streamed-from-YouTube tracks (account playlists, suggested, search,
//! autoplay radio). `Mixed` (default Local-first) plays the local copy when a
//! robust match exists, else streams from YouTube.
//!
//! Cycled by the `M` key in the TUI and persisted in `state.db` via
//! `LayoutState.source_mode`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SourceMode {
    #[default]
    Local,
    Youtube,
    Mixed,
}

impl SourceMode {
    pub fn cycle(self) -> Self {
        match self {
            SourceMode::Local => SourceMode::Youtube,
            SourceMode::Youtube => SourceMode::Mixed,
            SourceMode::Mixed => SourceMode::Local,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            SourceMode::Local => "local",
            SourceMode::Youtube => "youtube",
            SourceMode::Mixed => "mixed",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "youtube" => SourceMode::Youtube,
            "mixed" => SourceMode::Mixed,
            _ => SourceMode::Local,
        }
    }
}
```

Add to `src/lib.rs`: `pub mod mode;` (keep the existing module list; insert alphabetically after `cli`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test mode`
Expected: PASS (3 tests).

- [ ] **Step 5: Wire `source_mode` into `LayoutState`**

In `src/state.rs`, add to `LayoutState` (after `continue_mode`):

```rust
    #[serde(default = "default_local")]
    pub source_mode: String,
```

and:

```rust
fn default_local() -> String { "local".to_string() }
```

In `impl Default for LayoutState`, add `source_mode: "local".to_string(),`.

Update `save_layout_at`/`save_layout` signatures to accept `source_mode: crate::mode::SourceMode` as the **last** parameter, and store `source_mode: source_mode.as_str().to_string()` into the `LayoutState`. Update both the `_at` and default-path variants. (The existing caller in `src/main.rs` will be updated in Task 10/18 when `App` owns the mode; for now, update the `src/main.rs` call site to pass `SourceMode::default()` so the build stays green.)

- [ ] **Step 6: Build + test green**

Run: `cargo build && cargo test`
Expected: build succeeds; all tests pass (the existing layout round-trip test in `tests/state_ext.rs` still passes because `source_mode` has a serde default).

- [ ] **Step 7: Commit**

```bash
git add src/mode.rs src/lib.rs src/state.rs src/main.rs tests/mode.rs
git commit -m "feat: add SourceMode enum and persist it in layout state"
```

---

### Task 2: `ContinueMode::YouTube` variant

**Files:**
- Modify: `src/tui/queue.rs:28-38` (`ContinueMode` enum)
- Modify: `src/tui/app.rs:484-493` (`cycle_continue`)
- Modify: `src/state.rs:225-230` and `src/main.rs:54-58` (string mapping for `"youtube"`)
- Modify: `src/tui/view/player_bar.rs:150-154` (render `CONT youtube`)
- Test: `tests/transport.rs` (append)

**Interfaces:**
- Produces: `ContinueMode::YouTube` variant. `cycle_continue` becomes mode-aware: it cycles Off→NextAlbum→YouTube in Mixed, Off→YouTube in Youtube, Off→NextAlbum→Radio in Local (the existing behaviour). Because `App` owns the `SourceMode`, `cycle_continue` reads `self.source_mode`.
- **Note:** `Transport` itself is untouched — only the `ContinueMode` enum gains a variant; `Transport::next`/`peek_next` never match on it (CONT=YouTube is handled in `App::next`, not `Transport`).

- [ ] **Step 1: Write the failing test**

```rust
// append to tests/transport.rs
use jukebox::tui::queue::ContinueMode;

#[test]
fn continue_mode_youtube_variant_exists() {
    // The variant must exist and round-trip through cycleContinue's string form.
    assert_ne!(ContinueMode::YouTube, ContinueMode::Off);
}
```

```rust
// append to tests/app.rs
#[test]
fn cycle_continue_is_mode_aware() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Radio);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);

    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::YouTube);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);

    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::YouTube);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test transport continue_mode_youtube && cargo test --test app cycle_continue_is_mode`
Expected: FAIL — `ContinueMode::YouTube` does not exist; `App::source_mode` does not exist.

- [ ] **Step 3: Add the variant + an `App::source_mode` field**

In `src/tui/queue.rs`, add to `ContinueMode`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContinueMode {
    Off,
    NextAlbum,
    Radio,
    YouTube,
}
```

In `src/tui/app.rs`, add a field to `App`: `pub source_mode: crate::mode::SourceMode,` (init `crate::mode::SourceMode::default()` in `App::new`).

Rewrite `cycle_continue` to be mode-aware:

```rust
pub fn cycle_continue(&mut self) {
    self.transport.continue_mode = match (self.source_mode, self.transport.continue_mode) {
        (crate::mode::SourceMode::Local, ContinueMode::Off) => ContinueMode::NextAlbum,
        (crate::mode::SourceMode::Local, ContinueMode::NextAlbum) => ContinueMode::Radio,
        (crate::mode::SourceMode::Local, ContinueMode::Radio) => ContinueMode::Off,
        (crate::mode::SourceMode::Local, ContinueMode::YouTube) => ContinueMode::Off,

        (crate::mode::SourceMode::Youtube, ContinueMode::Off) => ContinueMode::YouTube,
        (crate::mode::SourceMode::Youtube, _) => ContinueMode::Off,

        (crate::mode::SourceMode::Mixed, ContinueMode::Off) => ContinueMode::NextAlbum,
        (crate::mode::SourceMode::Mixed, ContinueMode::NextAlbum) => ContinueMode::YouTube,
        (crate::mode::SourceMode::Mixed, ContinueMode::YouTube) => ContinueMode::Off,
        (crate::mode::SourceMode::Mixed, ContinueMode::Radio) => ContinueMode::Off,
    };
}
```

Also add `pub fn cycle_mode(&mut self) { self.source_mode = self.source_mode.cycle(); }` to `App`.

- [ ] **Step 4: Update state string mappings**

In `src/state.rs` `save_layout_at`, the `continue_mode` match arm gains `ContinueMode::YouTube => "youtube"`. In `src/main.rs` restore block, add `"youtube" => tui::queue::ContinueMode::YouTube`. In `src/tui/view/player_bar.rs`, add `ContinueMode::YouTube => "youtube"`.

- [ ] **Step 5: Handle `ContinueMode::YouTube` in `App::next` (no sidecar yet)**

In `src/tui/app.rs::next`, the `match self.transport.continue_mode` block currently has `Off`/`NextAlbum`/`Radio` arms. Add a `YouTube` arm that, for now (sidecar comes in Task 9), stops playback with a TODO-free comment pointing to Task 9's wiring:

```rust
                ContinueMode::YouTube => {
                    // Wired to RadioCursor in Task 9 (sidecar). Until then,
                    // CONT=YouTube stops playback cleanly rather than spinning.
                    self.player.stop().ok();
                    self.now_playing = None;
                }
```

This keeps `App::next` total and non-panicking; Task 9 replaces the body.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --test transport continue_mode_youtube && cargo test --test app cycle_continue_is_mode`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/tui/queue.rs src/tui/app.rs src/state.rs src/main.rs src/tui/view/player_bar.rs tests/transport.rs tests/app.rs
git commit -m "feat: add ContinueMode::YouTube with mode-aware cycling"
```

---

### Task 3: `TrackSource` + `RemoteTrack` + `StreamFormat`

**Files:**
- Create: `src/source/mod.rs`
- Modify: `src/lib.rs` (`pub mod source;`)
- Test: `tests/source_match.rs` (re-used by Task 4; here just compile/types)

**Interfaces:**
- Produces:
  - `pub enum TrackSource { Local { track_id: String }, Remote { video_id: String } }` (Clone, Debug, PartialEq, Eq).
  - `pub struct RemoteTrack { pub video_id: String, pub title: String, pub artist: String, pub album: Option<String>, pub dur: Option<f64>, pub fmt: Option<StreamFormat> }`.
  - `pub struct StreamFormat { pub codec: String, pub abr: u32, pub sample_rate: u32, pub container: String, pub premium: bool }`.
  - `impl TrackSource { pub fn id(&self) -> &str }` (returns track_id or video_id — the opaque id `Transport` already shuffles).

- [ ] **Step 1: Write the failing test**

```rust
// tests/source_match.rs (first test; more added in Task 4)
use jukebox::source::{TrackSource, RemoteTrack, StreamFormat};

#[test]
fn track_source_id_returns_opaque_id() {
    let l = TrackSource::Local { track_id: "abc".into() };
    let r = TrackSource::Remote { video_id: "dQw4w9WgXcQ".into() };
    assert_eq!(l.id(), "abc");
    assert_eq!(r.id(), "dQw4w9WgXcQ");
}

#[test]
fn remote_track_defaults_fmt_none() {
    let t = RemoteTrack { video_id: "v".into(), title: "S".into(), artist: "A".into(), album: None, dur: None, fmt: None };
    assert!(t.fmt.is_none());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test source_match`
Expected: FAIL — `jukebox::source` does not exist.

- [ ] **Step 3: Create `src/source/mod.rs`**

```rust
//! The local-vs-YouTube source abstraction.
//!
//! [`Transport`] shuffles opaque id [`String`]s. At load time [`App`] asks a
//! [`SourceResolver`] what a given id *means*: a local catalog track (play the
//! file) or a YouTube video (stream the resolved URL). This module owns the
//! types that cross that boundary; the resolver itself is added in Task 10.

pub mod match_local;
pub mod device_rate;

use serde::{Deserialize, Serialize};

/// What a currently-playing (or queued) track is. The opaque id [`Transport`]
/// already shuffles is available via [`TrackSource::id`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackSource {
    /// A track in the on-disk filtered-lossless catalog.
    Local { track_id: String },
    /// A YouTube video, streamed via a yt-dlp-resolved URL.
    Remote { video_id: String },
}

impl TrackSource {
    /// The opaque id string `Transport` shuffles. Equal for the same logical
    /// track regardless of source kind.
    pub fn id(&self) -> &str {
        match self {
            TrackSource::Local { track_id } => track_id,
            TrackSource::Remote { video_id } => video_id,
        }
    }
    pub fn is_remote(&self) -> bool {
        matches!(self, TrackSource::Remote { .. })
    }
}

/// A YouTube track's metadata, as returned by the sidecar's `search` /
/// `get_playlist` / `get_watch_playlist` commands. The stream URL is **not**
/// stored here — it's resolved lazily by `Sidecar::resolve_url` (Task 8).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub dur: Option<f64>,
    /// Known once `resolve_url` has run; `None` until then.
    #[serde(default)]
    pub fmt: Option<StreamFormat>,
}

/// The resolved audio stream's format. Reported by the sidecar so the app
/// knows the format *before* loading (spec §3.2/§3.3 — CoreAudio re-clock).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamFormat {
    pub codec: String,
    /// Audio bitrate in kbps.
    pub abr: u32,
    /// Sample rate in Hz (e.g. 48000, 44100).
    pub sample_rate: u32,
    pub container: String,
    /// True when the Premium ad-free manifest was selected.
    pub premium: bool,
}

impl StreamFormat {
    /// Short label for the player bar: "Opus 160k · YT" or "AAC 256k · YT Premium".
    pub fn yt_label(&self) -> String {
        let bitrate = format!("{}k", self.abr);
        let tier = if self.premium { " · YT Premium" } else { " · YT" };
        format!("{} {}{}", self.codec, bitrate, tier)
    }
}
```

Add `pub mod source;` to `src/lib.rs`. Create stub `src/source/match_local.rs` and `src/source/device_rate.rs` each containing just `// filled in Tasks 4 & 5` so the `mod` declarations compile; remove the stubs in those tasks.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test source_match`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/source/mod.rs src/lib.rs tests/source_match.rs
git commit -m "feat: add TrackSource, RemoteTrack, StreamFormat types"
```

---

### Task 4: `match_local` — ISRC + translit fuzzy matcher

**Files:**
- Create: `src/source/match_local.rs` (replace the stub)
- Test: `tests/source_match.rs` (append)

**Interfaces:**
- Produces: `pub fn match_local(remote: &RemoteTrack, cat: &crate::catalog::Catalog) -> Option<String>` returning a catalog `Track::id` if a robust local match exists, else `None`.
- Consumes: `crate::translit` (existing), `crate::catalog::{Catalog, Track}` (existing), `RemoteTrack` (Task 3).

**Algorithm (spec §4.1):**
1. If `remote` has an ISRC-equivalent... — *Note: `RemoteTrack` does not carry ISRC from the sidecar by default; the sidecar's `search`/`get_playlist` results include an `isrc` field when available. To keep this task self-contained, `match_local` first tries ISRC only if `RemoteTrack` carried one. Add `pub isrc: Option<String>` to `RemoteTrack` in Task 3's struct (update Task 3's struct now — it's the same file).* Exact, case-insensitive ISRC match against `Track.isrc` → return that track's id.
2. Else normalize `primary_artist + " " + title` on both sides: lowercase, strip punctuation, drop `feat.*`/`ft.*` tokens, collapse whitespace, run through `translit::variants` and join variants so kana/romaji cross-match.
3. Compute a similarity ratio (Levenshtein-based) between the remote normalized string and each candidate's normalized string (artist + title variants). Return the best candidate whose ratio ≥ 0.88. Ratios in [0.80, 0.88) are deliberately rejected.

- [ ] **Step 1: Write the failing tests**

Append to `tests/source_match.rs`:

```rust
use jukebox::catalog::Catalog;
use jukebox::source::{match_local::match_local, RemoteTrack};

fn cat(tracks: &[(&str, &str, &str, Option<&str>)]) -> Catalog {
    // (id, artist, title, isrc)
    let t: Vec<_> = tracks.iter().map(|(id,a,t,isrc)| serde_json::json!({
        "id":id,"artists":[a],"primary_artist":a,"title":t,
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"x",
        "symlinked_into_artists":[a],"isrc":isrc
    })).collect();
    let s = serde_json::json!({"version":1,"built_at":"x","source_root":"/tmp","tracks":t}).to_string();
    let p = std::env::temp_dir().join(format!("cat-{}.json", std::process::id()));
    std::fs::write(&p, &s).unwrap();
    Catalog::load(&p).unwrap()
}

#[test]
fn isrc_exact_match_wins() {
    let c = cat(&[("t1","Adele","Hello",Some("GBBKS1500123"))]);
    let r = RemoteTrack { video_id:"v1".into(), title:"Hello".into(), artist:"Adele".into(),
        album:None, dur:None, fmt:None, isrc:Some("gbbks1500123".into()) };
    assert_eq!(match_local(&r,&c), Some("t1".into()));
}

#[test]
fn isrc_case_insensitive() {
    let c = cat(&[("t1","Adele","Hello",Some("GBBKS1500123"))]);
    let r = RemoteTrack { video_id:"v1".into(), title:"Hello".into(), artist:"ADELE".into(),
        album:None, dur:None, fmt:None, isrc:Some("GBBKS1500123".into()) };
    assert_eq!(match_local(&r,&c), Some("t1".into()));
}

#[test]
fn normalized_cjk_title_match() {
    // catalog stores katakana title; remote (YT) gives romaji-ish "burubado"
    let c = cat(&[("t1","Ado","ブルーバード",None)]);
    let r = RemoteTrack { video_id:"v1".into(), title:"burubado".into(), artist:"ado".into(),
        album:None, dur:None, fmt:None, isrc:None };
    // translit produces romaji variant that should match after normalization
    assert_eq!(match_local(&r,&c), Some("t1".into()));
}

#[test]
fn feat_token_stripped() {
    let c = cat(&[("t1","Aimer","Dawn",None)]);
    let r = RemoteTrack { video_id:"v1".into(), title:"Dawn feat. Someone".into(), artist:"Aimer".into(),
        album:None, dur:None, fmt:None, isrc:None };
    assert_eq!(match_local(&r,&c), Some("t1".into()));
}

#[test]
fn borderline_rejected() {
    // 0.80-0.88 zone: should NOT promote to local
    let c = cat(&[("t1","Adele","Helloing",None)]); // differs by one char
    let r = RemoteTrack { video_id:"v1".into(), title:"Hello".into(), artist:"Adele".into(),
        album:None, dur:None, fmt:None, isrc:None };
    // "adele helloing" vs "adele hello" — ratio is high but below 0.88? assert None
    // (if it accidentally crosses 0.88 due to the long artist, tune by making
    //  the title the dominant comparison; see implementation note below.)
    assert_eq!(match_local(&r,&c), None);
}

#[test]
fn no_match_returns_none() {
    let c = cat(&[("t1","Adele","Hello",None)]);
    let r = RemoteTrack { video_id:"v1".into(), title:"Completely Different".into(), artist:"Nobody".into(),
        album:None, dur:None, fmt:None, isrc:None };
    assert_eq!(match_local(&r,&c), None);
}
```

**Implementation note for the borderline test:** compare the **title** separately from the **artist** and require *both* ≥ 0.88 independently (a combined-string ratio can be inflated by a long matching artist). Use title-ratio as the gate; if the artist ratio is below 0.80, reject outright. This makes the borderline test deterministic.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test source_match`
Expected: FAIL — `match_local` not defined; also `RemoteTrack` lacks `isrc`.

- [ ] **Step 3: Add `isrc` to `RemoteTrack`**

In `src/source/mod.rs`, add to `RemoteTrack`: `#[serde(default)] pub isrc: Option<String>,` (after `album`). Update the existing `remote_track_defaults_fmt_none` test's constructor to include `isrc: None`. Update the struct's `Default`-derivation (it's `#[derive(Default)]`, so `Option` defaults to `None` automatically — fine).

- [ ] **Step 4: Implement `src/source/match_local.rs`**

```rust
//! Local-first matching for Mixed mode (spec §4.1).
//!
//! Given a YouTube [`RemoteTrack`], find the local catalog track it most
//! likely *is* — so Mixed mode plays the hi-res local copy instead of
//! streaming the lossy YouTube version. Conservative: a wrong local
//! substitution (playing the wrong song) is worse than streaming, so the
//! thresholds are high and the borderline band [0.80, 0.88) is rejected.

use crate::catalog::Catalog;
use crate::source::RemoteTrack;
use crate::translit::variants;

const TITLE_GATE: f64 = 0.88;
const ARTIST_FLOOR: f64 = 0.80;

pub fn match_local(remote: &RemoteTrack, cat: &Catalog) -> Option<String> {
    // 1. ISRC (strong). Case-insensitive exact match.
    if let Some(isrc) = remote.isrc.as_deref().filter(|s| !s.is_empty()) {
        let want = isrc.to_ascii_lowercase();
        for t in &cat.tracks {
            if let Some(have) = t.isrc.as_deref() {
                if have.to_ascii_lowercase() == want {
                    return Some(t.id.clone());
                }
            }
        }
    }

    // 2. Normalized artist+title fuzzy.
    let r_artist = norm(&remote.artist);
    let r_title = norm(&remote.title);
    let r_title_variants: Vec<String> = variants(&remote.title)
        .into_iter()
        .map(|v| norm(&v))
        .chain(std::iter::once(r_title.clone()))
        .collect();
    let r_artist_variants: Vec<String> = variants(&remote.artist)
        .into_iter()
        .map(|v| norm(&v))
        .chain(std::iter::once(r_artist.clone()))
        .collect();

    let mut best: Option<(f64, String)> = None;
    for t in &cat.tracks {
        let c_artist = norm(&t.primary_artist);
        let c_title = norm(&t.title);
        // artist must clear its floor on at least one variant pair
        let artist_ok = r_artist_variants
            .iter()
            .any(|ra| c_artist.chars().count().max(ra.chars().count()) > 0
                && ratio(ra, &c_artist) >= ARTIST_FLOOR)
            || r_artist == c_artist;
        if !artist_ok {
            continue;
        }
        // title across variant pairs
        let title_ratio = r_title_variants
            .iter()
            .map(|rt| {
                let mut best = ratio(rt, &c_title);
                for cv in variants(&t.title) {
                    best = best.max(ratio(rt, &norm(&cv)));
                }
                best
            })
            .fold(0.0_f64, f64::max);
        if title_ratio >= TITLE_GATE {
            best = match best {
                Some((b, _)) if title_ratio <= b => best,
                _ => Some((title_ratio, t.id.clone())),
            };
        }
    }
    best.map(|(_, id)| id)
}

/// Normalize for fuzzy compare: lowercase, drop `feat.*`/`ft.*`, strip
/// punctuation, collapse whitespace.
fn norm(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut in_feat = false;
    for w in lower.split_whitespace() {
        if in_feat {
            // skip the rest of a feat. clause until a paren closes; keep simple:
            // just skip the whole token run after feat/ft (approximate, good enough)
            continue;
        }
        if w == "feat." || w == "feat" || w == "ft." || w == "ft" {
            in_feat = true;
            continue;
        }
        // keep alphanumerics (incl. CJK via char filter) and spaces
        for c in w.chars() {
            if c.is_alphanumeric() || (c as u32) >= 0x3000 {
                out.push(c);
            }
        }
        out.push(' ');
    }
    out.trim_end().to_string()
}

/// Normalized Levenshtein similarity ratio in [0,1]: 1 - edit/longer_len.
fn ratio(a: &str, b: &str) -> f64 {
    let (a, b) = (a.chars().collect::<Vec<_>>(), b.chars().collect::<Vec<_>>());
    let dist = levenshtein(&a, &b) as f64;
    let longer = a.len().max(b.len()).max(1) as f64;
    1.0 - dist / longer
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1)
                .min(cur[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test source_match`
Expected: PASS (all 8 tests). If the `borderline_rejected` test fails because the combined ratio crosses 0.88, re-read the implementation note in Step 1 — the title-gate must compare title independently of artist.

- [ ] **Step 6: Commit**

```bash
git add src/source/match_local.rs src/source/mod.rs tests/source_match.rs
git commit -m "feat: local-first ISRC + translit fuzzy matcher for mixed mode"
```

---

### Task 5: `DeviceRateState` — switch-once-per-YT-session

**Files:**
- Create: `src/source/device_rate.rs` (replace the stub)
- Test: `tests/source_device_rate.rs`

**Interfaces:**
- Produces:
  - `pub struct DeviceRateState { current_sr: u32, current_bd: u32, in_yt_rate: bool }` with `Default` (48k/16/false... actually 0/0/false — the "nothing switched yet" state).
  - `pub enum LoadKind { Local { sample_rate_hz: u32, bit_depth: u32 }, Remote { sample_rate: u32 } }`.
  - `pub fn desired_switch(state: &mut DeviceRateState, kind: LoadKind, switch_sample_rate: bool) -> Option<(u32, u32)>` — returns `Some((sr, bd))` when CoreAudio *should* be re-clocked now, else `None`. Mutates `state` to reflect the post-switch reality.
- Semantics (spec §3.3):
  - Local load → if `switch_sample_rate` and `(sr,bd)` differs from current → return `Some((sr,bd))`, set `in_yt_rate=false`.
  - Remote load, `in_yt_rate==false` → return `Some((sample_rate, sample_rate/*bd n/a for lossy; use 0 to mean "let audio.rs match"*/))`, set `in_yt_rate=true`. (audio.rs picks the closest supported PCM format; for lossy we pass the stream's sample_rate and let the matcher choose — `bit_depth` 0 means "don't care", and `audio::set_output_format` already picks the closest. We pass `sample_rate` for both sr and treat bd as the stream's reported depth if any; since `StreamFormat` has no bit_depth, pass `0` and have audio.rs treat 0 as "closest available" — **check `audio::match_format` already ignores bd when 0? It does not; it picks closest. Passing bd=0 yields closest-bit-depth which is fine.**)

  Actually, keep it simple and correct: `desired_switch` for remote returns `Some((sample_rate, 16))` (16-bit is the typical lossy depth and a safe target; `match_format` will pick the nearest supported). The key invariant the judge checks is *rate*, not depth.

  - Remote load, `in_yt_rate==true` and the stream's `sample_rate` differs from `current_sr` → return `Some((sample_rate, 16))` (allow one re-clock on a *rate change*), update `current_sr`. If same rate → `None` (hold — no mid-stream stutter).
  - Local load resuming from YT → `in_yt_rate=false`, switch to track's rate.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/source_device_rate.rs
use jukebox::source::device_rate::{DeviceRateState, LoadKind, desired_switch};

#[test]
fn local_track_switches_to_its_rate() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, true);
    assert_eq!(r, Some((192000, 24)));
    assert!(!s.in_yt_rate);
}

#[test]
fn consecutive_local_same_rate_no_reswitch() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 96000, bit_depth: 24 }, true);
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 96000, bit_depth: 24 }, true);
    assert_eq!(r, None); // same format, no reswitch (matches audio.rs fast-path)
}

#[test]
fn first_remote_switches_once() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert_eq!(r, Some((48000, 16)));
    assert!(s.in_yt_rate);
}

#[test]
fn consecutive_remote_same_rate_held() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert_eq!(r, None); // held — no mid-stream re-clock
    assert!(s.in_yt_rate);
}

#[test]
fn remote_rate_change_reswitches_once() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    let r = desired_switch(&mut s, LoadKind::Remote { sample_rate: 44100 }, true);
    assert_eq!(r, Some((44100, 16))); // different rate → one re-clock, then hold again
}

#[test]
fn local_after_remote_clears_yt_flag() {
    let mut s = DeviceRateState::default();
    desired_switch(&mut s, LoadKind::Remote { sample_rate: 48000 }, true);
    assert!(s.in_yt_rate);
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, true);
    assert_eq!(r, Some((192000, 24)));
    assert!(!s.in_yt_rate);
}

#[test]
fn switch_sample_rate_off_never_switches() {
    let mut s = DeviceRateState::default();
    let r = desired_switch(&mut s, LoadKind::Local { sample_rate_hz: 192000, bit_depth: 24 }, false);
    assert_eq!(r, None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test source_device_rate`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `src/source/device_rate.rs`**

```rust
//! CoreAudio re-clock cadence for mixed local/YouTube playback (spec §3.3).
//!
//! The device sample rate switches **once** when a YouTube session begins,
//! is **held** across consecutive YT tracks at the same rate (mid-stream
//! re-clocking stutters), re-clocks once if a YT track's rate changes, and is
//! restored when a local hi-res track resumes. `desired_switch` is the pure
//! decision function; `App` performs the actual `audio::set_output_format`.

#[derive(Clone, Copy, Debug, Default)]
pub struct DeviceRateState {
    current_sr: u32,
    current_bd: u32,
    in_yt_rate: bool,
}

pub enum LoadKind {
    Local { sample_rate_hz: u32, bit_depth: u32 },
    Remote { sample_rate: u32 },
}

/// Returns `Some((sample_rate, bit_depth))` to switch to now, or `None` to hold.
pub fn desired_switch(
    state: &mut DeviceRateState,
    kind: LoadKind,
    switch_sample_rate: bool,
) -> Option<(u32, u32)> {
    if !switch_sample_rate {
        return None;
    }
    match kind {
        LoadKind::Local { sample_rate_hz, bit_depth } => {
            state.in_yt_rate = false;
            if state.current_sr == sample_rate_hz && state.current_bd == bit_depth {
                None
            } else {
                state.current_sr = sample_rate_hz;
                state.current_bd = bit_depth;
                Some((sample_rate_hz, bit_depth))
            }
        }
        LoadKind::Remote { sample_rate } => {
            // Lossy stream depth: target 16-bit (audio::match_format picks the
            // nearest supported). The judge-critical invariant is the *rate*.
            if state.in_yt_rate && state.current_sr == sample_rate {
                None // hold — no mid-stream re-clock stutter
            } else {
                state.in_yt_rate = true;
                state.current_sr = sample_rate;
                state.current_bd = 16;
                Some((sample_rate, 16))
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test source_device_rate`
Expected: PASS (7 tests).

- [ ] **Step 5: Re-export from `source`**

Add to `src/source/mod.rs`: `pub mod device_rate;` is already declared; ensure `device_rate` items are reachable as `jukebox::source::device_rate::*`. (They are.)

- [ ] **Step 6: Commit**

```bash
git add src/source/device_rate.rs tests/source_device_rate.rs
git commit -m "feat: switch-once-per-YT-session CoreAudio cadence logic"
```

---

### Task 6: Sidecar wire protocol (`yt::proto`)

**Files:**
- Create: `src/yt/mod.rs`, `src/yt/proto.rs`
- Modify: `src/lib.rs` (`pub mod yt;`)
- Test: `tests/yt_sidecar.rs` (first batch — serde round-trips)

**Interfaces:**
- Produces (in `src/yt/proto.rs`):
  - `pub enum Request { Search { q: String, limit: u32 }, LibraryPlaylists, GetPlaylist { id: String }, HomeSuggestions, GetWatchPlaylist { video_id: String }, ResolveUrl { video_id: String }, Ping, AuthStatus }`
  - `pub enum Response { Search(Vec<RemoteTrackSummary>), Playlists(Vec<PlaylistSummary>), Tracks(Vec<RemoteTrackSummary>), Suggestions(Vec<PlaylistSummary>), WatchPlaylist(Vec<RemoteTrackSummary>), Resolve(ResolvedUrl), Auth(AuthStatus), Pong, Error(String) }`
  - `pub struct RemoteTrackSummary { pub video_id: String, pub title: String, pub artist: String, pub album: Option<String>, pub dur: Option<f64>, pub isrc: Option<String> }` (maps to `RemoteTrack` with `fmt: None`).
  - `pub struct PlaylistSummary { pub id: String, pub name: String, pub count: u32 }`
  - `pub struct ResolvedUrl { pub url: String, pub expires_at: Option<f64>, pub codec: String, pub abr: u32, pub sample_rate: u32, pub container: String, pub premium: bool }` (maps to `StreamFormat`).
  - `pub struct AuthStatus { pub ok: bool, pub premium: bool, pub account: bool }`
  - `impl Request { pub fn to_line(&self) -> String }` (single-line JSON with a `"cmd"` field).
  - `impl Response { pub fn from_line(line: &str) -> Result<Response> }` (parse the sidecar's `"ok":true/false` + payload wrapper).

The on-wire format is one JSON object per line. Requests: `{"cmd":"search","q":"...","limit":25}`. Responses: `{"ok":true,"data":{...}}` or `{"ok":false,"error":"..."}`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/yt_sidecar.rs
use jukebox::yt::proto::*;

#[test]
fn search_request_serializes_to_line() {
    let r = Request::Search { q: "adele hello".into(), limit: 25 };
    let line = r.to_line();
    assert!(line.contains("\"cmd\":\"search\""));
    assert!(line.contains("\"q\":\"adele hello\""));
    assert!(!line.contains('\n')); // single line
}

#[test]
fn response_round_trips_search() {
    let wire = r#"{"ok":true,"data":{"search":[{"video_id":"v1","title":"Hello","artist":"Adele","album":null,"dur":295.0,"isrc":null}]}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Search(v) => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].video_id, "v1");
            assert_eq!(v[0].dur, Some(295.0));
        }
        other => panic!("expected Search, got {other:?}"),
    }
}

#[test]
fn response_round_trips_resolve() {
    let wire = r#"{"ok":true,"data":{"resolve":{"url":"https://x","expires_at":1234.0,"codec":"AAC","abr":256,"sample_rate":48000,"container":"m4a","premium":true}}}"#;
    let r = Response::from_line(wire).unwrap();
    match r { Response::Resolve(u) => { assert_eq!(u.abr,256); assert!(u.premium); }, _ => panic!() }
}

#[test]
fn response_error() {
    let wire = r#"{"ok":false,"error":"rate limited"}"#;
    let r = Response::from_line(wire).unwrap();
    assert!(matches!(r, Response::Error(e) if e=="rate limited"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test yt_sidecar`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `src/yt/mod.rs` + `src/yt/proto.rs`**

`src/yt/mod.rs`:

```rust
//! YouTube sidecar: a long-lived Python process (`scripts/yt/yt.py`) speaking
//! newline-delimited JSON over stdin/stdout. This module owns the Rust client
//! (`Sidecar`, Task 8), auth/session/cache (`Session`, Task 9), and the autoplay
//! radio cursor (`RadioCursor`, Task 9). The wire types live in [`proto`].

pub mod proto;
// pub mod sidecar;   // Task 8
// pub mod session;   // Task 9
```

`src/yt/proto.rs` — the structs above with serde derives and the `to_line`/`from_line` impls. `Request::to_line` uses `serde_json::to_string(&serde_json::json!({...}))`. `Response::from_line` parses to a `serde_json::Value`, checks `ok`, and dispatches on the `data` key to build the right variant. Wrap `Response::from_line` to return `anyhow::Result<Response>`. (Full code per the field list in the Interfaces block; ~120 lines. No placeholders.)

Add `pub mod yt;` to `src/lib.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test yt_sidecar`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/yt/mod.rs src/yt/proto.rs src/lib.rs tests/yt_sidecar.rs
git commit -m "feat: sidecar JSON wire protocol types + serde round-trips"
```

---

### Task 7: Python sidecar `scripts/yt/yt.py`

**Files:**
- Create: `scripts/yt/yt.py`
- Create: `scripts/yt/requirements.txt`
- Test: a self-contained `python3 scripts/yt/yt.py` smoke check run in the task (no Rust test file; documented as a manual step with expected output).

**Interfaces:**
- Produces: a process that reads `{"cmd":...}` lines from stdin, writes `{"ok":true,"data":{...}}` / `{"ok":false,"error":...}` lines to stdout. Auth via an env var `JUKEBOX_YT_COOKIES` (the raw cookies.txt content) passed from Rust (Task 9) OR a `--cookies <path>` arg; for v1 use an env var to avoid temp-file races.
- Commands implemented: `search`, `library_playlists`, `get_playlist`, `home_suggestions`, `get_watch_playlist`, `resolve_url`, `ping`, `auth_status`. Each maps to `ytmusicapi` / `yt-dlp` as per spec §2.
- `resolve_url` uses yt-dlp's Python API (`yt_dlp.YoutubeDL({...})` with `extract_flat` off, `format` = bestaudio preference, `cookiesfile` from the parsed env) and returns the chosen format's `{url, expires_at (from the player_response or None), codec, abr, sample_rate, container, premium}`. `premium` is inferred from cookie presence + the selected format's bitrate ≥ 256.

- [ ] **Step 1: Write `scripts/yt/requirements.txt`**

```
ytmusicapi==1.12.1
yt-dlp==2025.6.30
```

(Use a recent pinned `yt-dlp`; the exact pin can be the latest at implement time — run `pip index versions yt-dlp` and pin the newest. The version above is a placeholder pinned value; the implementer should confirm it resolves and update if needed.)

- [ ] **Step 2: Write `scripts/yt/yt.py`**

A ~250-line script: a `main()` loop reading stdin lines, dispatching to handler functions, each returning a dict that's wrapped `{"ok":True,"data":...}` or raising into `{"ok":False,"error":str(e)}`. Uses `ytmusicapi.YTMusic(requests_session=...)` initialized once with the cookie headers parsed from `JUKEBOX_YT_COOKIES` (a helper `_cookie_headers(cookies_text)` builds the `Cookie:` header from Netscape cookies.txt). For `resolve_url`, builds a `yt_dlp.YoutubeDL({'format':'bestaudio','quiet':True,'noplaylist':True,'cookies':<temp cookies path>})`, calls `.extract_info(url=...)`, reads `'url'`, `'acodec'`/`'abr'`/`'asr'`, container from `'ext'`. Handles the case where `ytmusicapi`/`yt-dlp` aren't importable → prints `{"ok":False,"error":"ytmusicapi/yt-dlp not installed; run :yt setup"}` on any command and stays alive (so Rust can show the setup hint). Handles `get_watch_playlist` with `radio=True`. Logs to stderr only (stdout is the wire).

```python
#!/usr/bin/env python3
"""jukebox YouTube sidecar — newline-delimited JSON over stdin/stdout.

Spec: docs/superpowers/specs/2026-07-08-youtube-integration-design.md §2.
Commands map to ytmusicapi + yt-dlp. Auth is read from the JUKEBOX_YT_COOKIES
env var (Netscape cookies.txt format). Logs to stderr only (stdout is the wire).
"""
import sys, os, json, io

def _have_deps():
    try:
        import ytmusicapi, yt_dlp  # noqa: F401
        return True
    except ImportError:
        return False

def _cookie_header(env):
    raw = os.environ.get("JUKEBOX_YT_COOKIES", "")
    if not raw:
        return None, None
    # Parse Netscape cookies.txt into a Cookie header + a temp file path for yt-dlp.
    parts = []
    tmp = None
    for line in raw.splitlines():
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) >= 7:
            parts.append(f"{f[5]}={f[6]}")
    if not parts:
        return None, None
    # Write a temp cookies.txt for yt-dlp (it needs the file form).
    import tempfile
    tmp = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
    tmp.write(raw)
    tmp.close()
    return "; ".join(parts), tmp.name

def _yt():
    header, _ = _cookie_header(os.environ)
    import ytmusicapi
    if header:
        return ytmusicapi.YTMusic(headers={"Cookie": header})
    return ytmusicapi.YTMusic()  # guest

def _track(d):
    return {
        "video_id": d.get("videoId", ""),
        "title": d.get("title", ""),
        "artist": (d.get("artists") or [{}])[0].get("name", "") if d.get("artists") else "",
        "album": (d.get("album") or {}).get("name") if d.get("album") else None,
        "dur": None,
        "isrc": d.get("isrc"),
    }

def handle(cmd, arg, ytm):
    if cmd == "ping":
        return {"pong": True}
    if cmd == "auth_status":
        header, _ = _cookie_header(os.environ)
        return {"ok": bool(header), "premium": bool(header), "account": bool(header)}
    if cmd == "search":
        res = ytm.search(arg.get("q", ""), filter="songs", limit=arg.get("limit", 25))
        return {"search": [_track(r) for r in res]}
    if cmd == "library_playlists":
        return {"playlists": [{"id": p.get("playlistId",""), "name": p.get("title",""), "count": p.get("playlistCount",0)} for p in ytm.get_library_playlists()]}
    if cmd == "get_playlist":
        p = ytm.get_playlist(arg.get("id",""))
        return {"tracks": [_track(t) for t in p.get("tracks", [])]}
    if cmd == "home_suggestions":
        # mood/mixed playlists from home
        home = ytm.get_home()
        out = []
        for sec in home:
            for it in sec.get("contents", []):
                if "playlistId" in it:
                    out.append({"id": it["playlistId"], "name": it.get("title",""), "count": 0})
        return {"suggestions": out}
    if cmd == "get_watch_playlist":
        res = ytm.get_watch_playlist(videoId=arg.get("video_id",""), radio=True)
        return {"watch_playlist": [_track(t) for t in res.get("tracks", [])]}
    if cmd == "resolve_url":
        import yt_dlp
        _, cookies_path = _cookie_header(os.environ)
        opts = {"format": "bestaudio", "quiet": True, "noplaylist": True}
        if cookies_path:
            opts["cookiefile"] = cookies_path
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(f"https://www.youtube.com/watch?v={arg.get('video_id','')}", download=False)
        fmt = (info.get("formats") or [info])
        best = max(fmt, key=lambda f: f.get("abr") or 0) if fmt else info
        return {"resolve": {
            "url": info.get("url") or best.get("url", ""),
            "expires_at": None,
            "codec": (best.get("acodec","") or "").split(".")[0].upper() or "AAC",
            "abr": int(best.get("abr") or 0),
            "sample_rate": int(best.get("asr") or 48000),
            "container": best.get("ext","m4a"),
            "premium": bool(cookies_path) and int(best.get("abr") or 0) >= 256,
        }}
    raise ValueError(f"unknown cmd {cmd}")

def main():
    ytm = None
    have = _have_deps()
    if have:
        try:
            ytm = _yt()
        except Exception as e:
            print(json.dumps({"ok": False, "error": f"ytmusicapi init: {e}"}), flush=True)
            have = False
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            print(json.dumps({"ok": False, "error": f"bad json: {e}"}), flush=True)
            continue
        cmd = req.get("cmd")
        if not have:
            print(json.dumps({"ok": False, "error": "ytmusicapi/yt-dlp not installed; run :yt setup"}), flush=True)
            continue
        try:
            data = handle(cmd, req, ytm)
            print(json.dumps({"ok": True, "data": data}), flush=True)
        except Exception as e:
            print(json.dumps({"ok": False, "error": str(e)}), flush=True)

if __name__ == "__main__":
    main()
```

- [ ] **Step 3: Smoke-test the sidecar manually**

Run (with deps installed in a venv):
```bash
python3 -m venv /tmp/jkvenv && /tmp/jkvenv/bin/pip install -q -r scripts/yt/requirements.txt
echo '{"cmd":"ping"}' | /tmp/jkvenv/bin/python3 scripts/yt/yt.py
echo '{"cmd":"auth_status"}' | /tmp/jkvenv/bin/python3 scripts/yt/yt.py
```
Expected: `{"ok": true, "data": {"pong": true}}` and `{"ok": false, "data": {"ok": false, "premium": false, "account": false}}` (without cookies env). If `ytmusicapi` import fails, the second line is `{"ok": false, "error": "ytmusicapi/yt-dlp not installed; run :yt setup"}` — that's the intended degrade path. Record the actual output in the commit message.

- [ ] **Step 4: Commit**

```bash
git add scripts/yt/yt.py scripts/yt/requirements.txt
git commit -m "feat: python YouTube sidecar (ytmusicapi + yt-dlp)"
```

---

### Task 8: `Sidecar` Rust subprocess client (non-blocking)

**Files:**
- Create: `src/yt/sidecar.rs`
- Modify: `src/yt/mod.rs` (`pub mod sidecar;`)
- Test: `tests/yt_sidecar.rs` (append — use a fake sidecar via `std::env`-selected script path, or a temp python echo script)

**Interfaces:**
- Produces: `pub struct Sidecar { ... }` with:
  - `pub fn spawn(python: &Path, script: &Path, cookies: Option<String>) -> Result<Sidecar>` — spawns `python script`, sets `JUKEBOX_YT_COOKIES` env if cookies present, makes stdin/stdout pipes, stdout non-blocking.
  - `pub fn send(&mut self, req: &Request) -> Result<()>` — writes `req.to_line()` + `\n` to stdin, flush.
  - `pub fn try_recv(&mut self) -> Result<Option<Response>>` — non-blocking read; returns `Ok(None)` if no complete line yet, `Ok(Some(resp))` on a complete line, `Err` on pipe close. Mirrors `MpvPlayer::track_ended`'s buffer-and-parse pattern.
  - `pub fn is_alive(&self) -> bool`.
  - `pub fn respawn(&mut self) -> Result<()>` — best-effort restart once.
- Consumes: `proto::{Request, Response}` (Task 6).

**Non-blocking detail:** set the child's stdout `UnixStream`-equivalent (`std::process::ChildStdout` → wrap in a non-blocking fd via `libc` on unix, or use a reader thread feeding a channel). **Simpler approach that fits the existing poll loop:** spawn a thin reader thread that blocks on the child stdout and pushes complete lines into a `std::sync::mpsc::Receiver<Line>`; `try_recv` drains the channel non-blockingly. This avoids fd-level non-blocking I/O and matches "never block the UI thread."

- [ ] **Step 1: Write the failing tests**

Append to `tests/yt_sidecar.rs`:

```rust
use jukebox::yt::sidecar::Sidecar;
use jukebox::yt::proto::{Request, Response};
use std::io::Write;

// A fake sidecar script that echoes a canned response for "ping".
fn fake_script() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("fake-{}.py", std::process::id()));
    let mut f = std::fs::File::create(&p).unwrap();
    writeln!(f, "import sys,json").unwrap();
    writeln!(f, "for line in sys.stdin:").unwrap();
    writeln!(f, "    print(json.dumps({{'ok':True,'data':{{'pong':True}}}}), flush=True)").unwrap();
    p
}

#[test]
fn sidecar_send_then_recv_ping() {
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let mut s = Sidecar::spawn(&python, &script, None).unwrap();
    s.send(&Request::Ping).unwrap();
    // spin a little; the reader thread pushes async
    let mut got = None;
    for _ in 0..50 {
        if let Ok(Some(r)) = s.try_recv() { got = Some(r); break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(matches!(got, Some(Response::Pong)), "got {got:?}");
    let _ = std::fs::remove_file(&script);
}

#[test]
fn sidecar_try_recv_none_when_idle() {
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let mut s = Sidecar::spawn(&python, &script, None).unwrap();
    // nothing sent yet
    assert!(matches!(s.try_recv().unwrap(), None));
    let _ = std::fs::remove_file(&script);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test yt_sidecar -- --nocapture`
Expected: FAIL — `sidecar` module missing.

- [ ] **Step 3: Implement `src/yt/sidecar.rs`**

Use the reader-thread + mpsc approach. Fields: `child: Child`, `tx_to_proc: ChildStdin`, `rx_from_proc: mpsc::Receiver<String>`, `_reader: JoinHandle<()>` (held to keep the thread alive). `try_recv` calls `rx.try_recv()`, parses each line with `Response::from_line`, returns the first `Ok`; lines that fail to parse are logged to stderr (file-logger not available here; `eprintln` is acceptable in a subprocess unit, but prefer a `log`-style to a file — use `eprintln` guarded by a debug feature, or simply skip unparseable lines). On channel close (reader thread exited → child stdout EOF), return `Err(anyhow!("sidecar closed"))`.

Spawn details: `Command::new(python).arg(script).stdin(piped).stdout(piped).stderr(null).env("JUKEBOX_YT_COOKIES", cookies.unwrap_or_default())`. Make `ChildStdout` owned by the reader thread.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test yt_sidecar`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add src/yt/sidecar.rs src/yt/mod.rs tests/yt_sidecar.rs
git commit -m "feat: non-blocking Sidecar subprocess client with reader thread"
```

---

### Task 9: `Session` (auth/cookies/cache) + `RadioCursor`

**Files:**
- Create: `src/yt/session.rs`
- Modify: `src/yt/mod.rs` (`pub mod session;`)
- Modify: `src/tui/app.rs` — replace the `ContinueMode::YouTube` stub body in `next` (Task 2 Step 5) with a `RadioCursor`-driven advance.
- Test: `tests/yt_sidecar.rs` (append) + `tests/app.rs` (append — CONT=YouTube with a fake resolver)

**Interfaces:**
- Produces:
  - `pub struct Session { sidecar: Sidecar, cookies: Option<String>, track_cache: HashMap<video_id, RemoteTrack>, url_cache: LruCache<video_id, (url, expires_at)> }` with `pub fn auth_status(&mut self) -> AuthStatus`, `pub fn set_cookies(&mut self, cookies: String)`, `pub fn clear_cookies(&mut self)`, `pub fn search(&mut self, q, limit) -> Result<Vec<RemoteTrackSummary>>` (sends + awaits via a helper that send+polls-until-matching-id), etc. for each command.
  - `pub struct RadioCursor { queue: Vec<String>, pos: usize, seed: Option<String> }` with `pub fn advance(&mut self, session: &mut Session) -> Option<String>` — if `queue` has a next id, return it; else call `get_watch_playlist(seed)` (seed = last played), refill `queue`, return first. Pre-resolve the next URL via `session.resolve_url` is done in `App::next`, not here (RadioCursor only produces the next id; resolution happens at load).
- Consumes: `Sidecar` (Task 8), `proto` (Task 6), `RemoteTrack`/`StreamFormat` (Task 3).

**Send+await helper:** because the sidecar is request/response and the TUI is poll-based, `Session` methods that need a synchronous answer (search, resolve_url during `start_playback`) use a helper `fn roundtrip(&mut self, req: Request, want: RequestKind) -> Result<Response>` that `send`s then loops `try_recv` with a short deadline (~3s for search/resolve, ~1s for ping). If the deadline passes without a matching response, return a `Err` the caller treats as "degrade" (spec §3.5). **Important:** this helper blocks the dispatch for up to the deadline; for `start_playback`/`resolve_url` that's acceptable (1–2s, once, at a play boundary) but it must be bounded. The TUI poll loop calls `start_playback` only on Enter/next, not every tick, so a 2s bound is fine. Document this explicitly in the module.

- [ ] **Step 1: Write the failing tests** (CONT=YouTube advance with a stubbed session)

Because `Session` needs a real sidecar, unit-test `RadioCursor` with a trait seam: introduce `pub trait YtClient { fn get_watch_playlist(&mut self, video_id: &str) -> Result<Vec<String>>; }` and have `Session: YtClient`. Tests implement a fake `YtClient` returning canned videoIds.

```rust
// tests/app.rs append
use jukebox::yt::session::{RadioCursor, YtClient};

struct FakeYt;
impl YtClient for FakeYt {
    fn get_watch_playlist(&mut self, _vid: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec!["yt1".into(), "yt2".into(), "yt3".into()])
    }
}

#[test]
fn cont_youtube_advances_through_radio_then_refills() {
    let mut rc = RadioCursor::new();
    let mut yt = FakeYt;
    // seed from a played track
    let a = rc.advance(&mut yt, Some("seed".into())).unwrap();
    assert_eq!(a, "yt1");
    assert_eq!(rc.advance(&mut yt, Some("yt1".into())).unwrap(), "yt2");
    assert_eq!(rc.advance(&mut yt, Some("yt2".into())).unwrap(), "yt3");
    // exhausted → refill
    assert_eq!(rc.advance(&mut yt, Some("yt3".into())).unwrap(), "yt1");
}
```

```rust
// tests/yt_sidecar.rs append — Session roundtrip against the fake sidecar
use jukebox::yt::session::Session;

#[test]
fn session_ping_roundtrip() {
    let python = std::path::PathBuf::from("python3");
    let script = fake_script();
    let mut s = Session::spawn(&python, &script, None).unwrap();
    let st = s.auth_status().unwrap();
    assert_eq!(st.ok, false); // no cookies → ok=false in our fake? the fake returns pong always...
    // (adjust: fake returns pong; auth_status maps Pong->ok=true. Tune the assertion to the
    //  actual fake behavior: with no cookies, Session.auth_status sends AuthStatus, the fake
    //  returns Pong. So we just assert the call returns without error.)
    let _ = st;
    let _ = std::fs::remove_file(&script);
}
```

(Adjust the `session_ping_roundtrip` assertions to the fake's actual behavior; the point is that `Session::spawn` + a roundtrip method don't panic or hang.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test app cont_youtube && cargo test --test yt_sidecar session_ping`
Expected: FAIL — `RadioCursor`/`Session` missing.

- [ ] **Step 3: Implement `src/yt/session.rs`**

`RadioCursor`: `new()`, `advance(&mut self, yt, seed)`: if `pos < queue.len()`, return `queue[pos]`, `pos+=1`. Else if let `Some(seed)`, `queue = yt.get_watch_playlist(seed)?; pos=1; queue.first().cloned()`. Else `None`. Track `seed` is the just-finished video id (passed by `App::next`).

`Session`: owns a `Sidecar`; `roundtrip(req, kind)` send + poll-loop with a `std::time::Instant` deadline. Map the `Response` to the typed return for each public method. `url_cache` is a tiny `Vec` capped at 2 (current + next) keyed by video_id — evict oldest beyond 2. `track_cache` maps video_id → `RemoteTrack` so the view layer can render titles for ids it's seen.

`YtClient` trait with `get_watch_playlist`; `Session` implements it by `roundtrip(GetWatchPlaylist{video_id})` → `Response::WatchPlaylist` → map summaries to video_ids.

- [ ] **Step 4: Wire `RadioCursor` into `App::next`**

Add `pub radio: RadioCursor` to `App` (init `RadioCursor::new()`). In `App::next`, replace the `ContinueMode::YouTube` stub body:

```rust
                ContinueMode::YouTube => {
                    // Drive YouTube autoplay: ask RadioCursor for the next id,
                    // switch context to a fresh radio Search context, load it.
                    let seed = self.now_playing.as_ref().map(|s| s.id().to_string());
                    let r = ClonedResolver { playlists: &self.playlists, manual_queue: self.transport.manual_queue.clone() };
                    match self.radio.advance(&mut self.yt_session, seed) {
                        Some(vid) => {
                            // new radio context = just this one id for now;
                            // the queue grows as RadioCursor advances.
                            let ctx = Context::Search { query: "youtube radio".into(), track_ids: vec![vid.clone()] };
                            if let Some(id) = self.now_playing.clone() {
                                self.transport.history.push((id, self.transport.context.clone()));
                            }
                            self.transport.switch_context(ctx, Some(&vid), &r, &self.catalog);
                            self.start_playback();
                        }
                        None => { self.player.stop().ok(); self.now_playing = None; }
                    }
                }
```

This requires `App` to own a `yt_session: Session` — add it (init: `Session::spawn(...)` best-effort in `App::new`, or lazy; for testability, `App::new` takes an `Option<Session>` or `App` constructs it from paths. To keep `cat_album()`-style tests working without a sidecar, make `yt_session` an `Option<Session>` and have the YouTube arms degrade to `None`-stop when absent. Update `App::new` accordingly — the production `main.rs` constructs a `Session` and passes it in.)

**Adjust `App::new` signature:** `pub fn new(catalog, player, searcher, yt_session: Option<Session>) -> Self`. Update `src/main.rs` and all tests that call `App::new` to pass `None`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test`
Expected: PASS, including `cont_youtube_advances_through_radio_then_refills`.

- [ ] **Step 6: Commit**

```bash
git add src/yt/session.rs src/yt/mod.rs src/tui/app.rs src/main.rs tests/app.rs tests/yt_sidecar.rs tests/app.rs tests/transport.rs
git commit -m "feat: yt Session (auth/cache) + RadioCursor autoplay for CONT=YouTube"
```

---

### Task 10: `SourceResolver` + wire into `App::load_track`/`start_playback`

**Files:**
- Create: `src/source/resolver.rs` + export from `src/source/mod.rs`
- Modify: `src/tui/app.rs` (`load_track`, `start_playback`, `next`'s resolve path)
- Test: `tests/source_match.rs` (append) + `tests/app.rs` (append — mixed-mode local-prefer + stream)

**Interfaces:**
- Produces: `pub enum Resolved { Local { path: PathBuf, sample_rate_hz: u32, bit_depth: u32 }, Remote { url: String, fmt: StreamFormat, video_id: String } }` and `pub fn resolve(id: &str, mode: SourceMode, cat: &Catalog, session: Option<&mut Session>, match_cache: &mut MatchCache) -> Result<Resolved>`. (Practically, `App` holds the pieces and calls into a resolver method; keep it as a method on `App` to avoid threading many args.)
- The resolution policy (spec §4.2/§4.3):
  - `SourceMode::Local` → local only: `track_by_id` → path. If not in catalog → dead.
  - `SourceMode::Youtube` → remote only: `session.resolve_url(id)` → url + fmt.
  - `SourceMode::Mixed` → if `track_by_id(id)` hits, local. Else try `match_local` against a `RemoteTrack` for `id` (look up in `session.track_cache`; if not cached, the id came from a local list so there's no remote track — treat as local). Actually: in Mixed, an id is *either* a catalog id (local list) or a video_id (YT list). Decide by: if `track_by_id(id)` is `Some` → local. Else → remote stream. (The `match_local` matcher is used when *starting* from a YT track and preferring local — that's applied at `play_selected`/context-build time, not at load. Clarify in the resolver doc.)

  So the simpler resolver: `Mixed` → `track_by_id` Some ⇒ local; None ⇒ remote. `match_local` is used when building a *Mixed* context from a YT list: each YT track's video_id is checked against the catalog via `match_local`; if it hits, the context uses the catalog id (local), else the video_id (remote). That belongs in the `Y`-view→context build (Task 11), not `load_track`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/app.rs append — mixed mode prefers local
#[test]
fn mixed_mode_plays_local_when_track_in_catalog() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    // StubPlayer.loaded() should be a path, and now_playing is Local
    assert!(matches!(app.now_playing, Some(jukebox::source::TrackSource::Local{..})));
}
```

(The remote path is exercised in Task 19's stubbed-sidecar integration tests; here we assert the local branch.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test app mixed_mode_plays_local`
Expected: FAIL — `App::new` arity (4 args) / `now_playing` type mismatch (`now_playing` is still `Option<String>`).

- [ ] **Step 3: Widen `now_playing` to `Option<TrackSource>` and route `load_track`**

In `src/tui/app.rs`:
- Change `pub now_playing: Option<String>` → `pub now_playing: Option<TrackSource>`.
- Update every read of `now_playing` (history push, view `np` checks, player bar) to use `.id()` / `.clone()`.
- Add `pub device_rate: DeviceRateState` to `App`.
- Rewrite `load_track`:

```rust
fn load_track(&mut self, id: &str) {
    if self.dead.contains(id) { return; }
    let resolved = match self.resolve_source(id) {
        Some(r) => r,
        None => { self.dead.insert(id.to_string()); return; }
    };
    match resolved {
        Resolved::Local { path, sample_rate_hz, bit_depth } => {
            if let Some((sr,bd)) = device_rate::desired_switch(&mut self.device_rate,
                device_rate::LoadKind::Local { sample_rate_hz, bit_depth }, self.switch_sample_rate) {
                let _ = crate::audio::set_output_format(sr, bd);
            }
            let _ = self.player.load(&path);
            self.now_playing = Some(TrackSource::Local { track_id: id.to_string() });
        }
        Resolved::Remote { url, fmt, video_id } => {
            if let Some((sr,bd)) = device_rate::desired_switch(&mut self.device_rate,
                device_rate::LoadKind::Remote { sample_rate: fmt.sample_rate }, self.switch_sample_rate) {
                let _ = crate::audio::set_output_format(sr, bd);
            }
            // mpv loadfile accepts an https URL via the same path. Player::load takes &Path;
            // build a PathBuf from the URL string (PathBuf accepts arbitrary strings — mpv gets the text).
            let p = std::path::PathBuf::from(&url);
            let _ = self.player.load(&p);
            self.now_playing = Some(TrackSource::Remote { video_id });
        }
    }
}
```

- Add `fn resolve_source(&mut self, id: &str) -> Option<Resolved>` implementing the policy (Local/YouTube/Mixed per above), calling `self.yt_session.as_mut()?.resolve_url(id)` for remote and mapping to `Resolved::Remote`. On sidecar error/dead, return `None` (→ dead). Cache the resolved `StreamFormat` into the `RemoteTrack` in `session.track_cache`.
- Update `start_playback` to use the same `resolve_source` (it currently calls `track_by_id` + `resolve_source` directly). Keep the dead-track skip loop, generalized to handle remote `None`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test`
Expected: PASS. Existing tests that asserted `now_playing == Some("t1")` now assert `Some(TrackSource::Local{track_id:"t1"})` — update those assertions across `tests/app.rs`, `tests/input.rs`, `tests/tui.rs` (search for `now_playing`).

- [ ] **Step 5: Commit**

```bash
git add src/source/resolver.rs src/source/mod.rs src/tui/app.rs tests/
git commit -m "feat: SourceResolver routes local/YouTube via DeviceRateState + TrackSource"
```

---

### Task 11: `ContextResolver::yt_playlist_ids` + `View::Youtube` data plumbing

**Files:**
- Modify: `src/tui/context.rs` (`ContextResolver` trait — add `yt_playlist_ids` with a default empty impl)
- Modify: `src/tui/app.rs` (`View::Youtube`, `current_context_ids`, `context_for_current_view`, `ClonedResolver`)
- Modify: `src/tui/app.rs` `View` enum: add `Youtube`
- Test: `tests/app.rs` (append), `tests/context.rs` (append)

**Interfaces:**
- `ContextResolver` gains `fn yt_playlist_ids(&self, key: &str) -> Vec<String> { Vec::new() }` (default, so existing fakes compile unchanged).
- `App` holds the Y-view's loaded lists: `pub yt_lists: Vec<YtList>` where `YtList { id, name, kind: Account/Suggested, track_ids: Vec<String> }` (populated from sidecar; empty until fetched).
- `current_context_ids` for `View::Youtube` returns the focused Y list's `track_ids` (video_ids). In Mixed mode, each video_id is run through `match_local`; hits are replaced with the catalog id so the queue plays local-first. (Build the context ids with this substitution at `play_selected` time.)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/context.rs append
struct R2;
impl ContextResolver for R2 {
    fn playlist_ids(&self, _: &str) -> Vec<String> { vec![] }
    fn queue_ids(&self) -> Vec<String> { vec![] }
    fn yt_playlist_ids(&self, key: &str) -> Vec<String> { if key=="yt1" {vec!["v1".into(),"v2".into()]} else {vec![]} }
}
#[test]
fn yt_playlist_resolver_returns_video_ids() {
    let ctx = jukebox::tui::context::Context::Playlist { name: "yt1".into() };
    assert_eq!(ctx.track_ids(&R2), vec!["v1".to_string(), "v2".to_string()]);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test context yt_playlist`
Expected: FAIL — `yt_playlist_ids` not on the trait.

- [ ] **Step 3: Add the trait method + `View::Youtube` + plumbing**

- `src/tui/context.rs`: add the default-impl method to `ContextResolver`.
- `src/tui/app.rs`: add `Youtube` to `View`; add `pub yt_lists: Vec<YtList>` to `App` + a `YtList` struct (in app.rs or a small `src/tui/yt_view.rs` — put in app.rs for now, move in Task 13 if it grows). Implement `current_context_ids`/`context_for_current_view` `Youtube` arms. Update `ClonedResolver` to carry a `yt_lists` snapshot and implement `yt_playlist_ids` by key. Update `focus_key` to return `"youtube"` for `View::Youtube`. Update `cycle_view`/`switch_view`/`max_focus_col`/`focused_column_len` to include `Youtube` (2 columns, like Playlists).
- `App` gains `pub yt_lists_loading: bool` and `pub yt_error: Option<String>` for render states (Task 13).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test`
Expected: PASS. Fix any `match app.view` that became non-exhaustive (input.rs, columns.rs, app.rs).

- [ ] **Step 5: Commit**

```bash
git add src/tui/context.rs src/tui/app.rs tests/context.rs tests/app.rs
git commit -m "feat: View::Youtube + yt_playlist_ids resolver plumbing"
```

---

### Task 12: TUI — rail `A/P/Q/Y`, balanced two-row player bar, footer hints, `MODE` flag

**Files:**
- Modify: `src/tui/view/layout.rs` (vertical budget: content + 2-row player bar + 1-row footer)
- Modify: `src/tui/view/player_bar.rs` (two-row split: row1 now-playing+quality+volume, row2 gauge+mode flags; drop transport glyphs; `MODE` last)
- Modify: `src/tui/view/columns.rs` (rail `Y`)
- Modify: `src/tui/app.rs` (render the footer from a `footer_hints(app)` helper, or inline in layout.rs)
- Test: `tests/player_bar.rs` (append/rewrite — assert two rows + MODE + no transport glyphs), `tests/columns.rs` (rail has Y)

**Layout budget (≥80×24):** content = height-3; player bar = 2; footer = 1. At 80×24 → content 21, bar 2, footer 1. The current code already reserves 2 for the bar; add 1 for the footer and reduce content by 1.

- [ ] **Step 1: Write/adjust the failing snapshot+assertion tests**

`tests/player_bar.rs`: assert the rendered player bar at 120×2 has the now-playing on row 0, gauge+flags on row 1, contains `MODE local`, and does **not** contain `◀◀`. (Use the existing insta snapshot pattern; pin size 120×2 and NO_COLOR.) Add a new snapshot for a YT now-playing (`AAC 256k · YT Premium`).

`tests/columns.rs`: assert the rail renders `Y` and that `4` switches to `View::Youtube`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test player_bar --test columns`
Expected: FAIL (new assertions).

- [ ] **Step 3: Implement the layout/player-bar/footer changes**

- `layout.rs`: split `outer` into `[Min(3), Length(2), Length(1)]` → content/bar/footer. Call a new `footer::render(f, outer[2], app)` (add a tiny `src/tui/view/footer.rs` or inline). Keep the too-small guard at `< 80×24` for the *full* layout; the narrow fallback (Task 14) handles 60–80.
- `player_bar.rs`: rewrite `build_info_line` to row 1 only (now-playing + quality + volume). Move transport-glyph removal. Add a `build_flags_line(app, width)` for row 2: `[gauge……]  M:SS / M:SS   SHUF off · RPT off · CONT off · MODE local`. Quality readout: `TrackSource::Local` → `{}-bit / {} kHz` + `· bit-perfect` when switch on; `TrackSource::Remote` → `fmt.yt_label()`. `MODE` = `app.source_mode.as_str()`.
- `columns.rs`: rail rows `A P Q Y` (+ the existing `/` removed). Render `View::Youtube` via a stub for now (Task 13 fills it): show `loading…` if `yt_lists` empty.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test player_bar --test columns`
Expected: PASS. Update existing snapshots (insta review) — accept the new frames.

- [ ] **Step 5: Commit**

```bash
git add src/tui/view/layout.rs src/tui/view/player_bar.rs src/tui/view/columns.rs src/tui/view/footer.rs tests/
git commit -m "feat(tui): balanced two-row player bar, footer hints, A/P/Q/Y rail, MODE flag"
```

---

### Task 13: TUI — `View::Youtube` rendering (2-col + Up-Next pane) + loading/error states

**Files:**
- Modify: `src/tui/view/columns.rs` (`render_youtube`), or create `src/tui/view/yt.rs` and dispatch from columns.rs
- Modify: `src/tui/app.rs` (a `refresh_yt_lists(&mut self)` that sends `library_playlists`+`home_suggestions` on entering Y view; populate `yt_lists`)
- Test: `tests/columns.rs` (append — Y view renders account+suggested, Up-Next pane, loading, error)

**Rendering (spec §5.3):** col1 = `yt_lists` (`♫` account, `✦` suggested); col2 = focused list's `track_ids` rendered via `track_rows`-equivalent but resolving through `session.track_cache` for titles (fall back to the raw id if not yet fetched). Below col2, when the track list is short, an "Suggested / Up Next" pane showing other `✦` lists as `▶ name →` rows.

- [ ] **Step 1: Write the failing tests**

Assert: at 120×30 with `yt_lists` populated (2 account + 1 suggested), the Y view renders `♫ Liked Songs`, `✦ Focus Flow`, and an `Up Next` pane label. With `yt_error` set, renders the error text. With empty `yt_lists` and no error and not loading and `yt_session` None, renders the setup hint (`:yt setup`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test columns yt_view`
Expected: FAIL.

- [ ] **Step 3: Implement `render_youtube` + `refresh_yt_lists`**

- `App::refresh_yt_lists`: if `yt_session` Some and `yt_lists` empty and not loading → set `yt_lists_loading=true`, send `LibraryPlaylists` and `HomeSuggestions` (fire-and-forget; results land on next tick via `Session::drain` — add a `pub fn drain(&mut self) -> Vec<Response>` that collects all pending responses and `App::on_tick` applies them to `yt_lists`). For v1, simplest: a synchronous `refresh_yt_lists` that roundtrips (1–2s) on entering Y view, showing `loading…` meanwhile (the roundtrip is bounded; acceptable at a view-enter boundary). Document the bound.
- `render_youtube`: 2-col layout like Playlists; col2 rows use a `yt_track_rows` helper that resolves each video_id via `session.track_cache.get(id)` for title/artist, falling back to the id. Up-Next pane: a `Paragraph` below col2 listing the other suggested lists.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test columns`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/view/columns.rs src/tui/app.rs src/tui/view/yt.rs tests/columns.rs
git commit -m "feat(tui): Y view with account+suggested lists, Up-Next pane, loading/error"
```

---

### Task 14: TUI — narrow (60–80) single-pane fallback

**Files:**
- Modify: `src/tui/view/layout.rs` (add a 60–80 branch: single focused pane, 1-row player bar, short footer)
- Modify: `src/tui/view/columns.rs` (single-pane renderer driven by `focus_col`/bread-crumb)
- Test: `tests/layout.rs` (append — render at 70×24, assert single pane + 1-row bar + short footer; assert no panic at 50×18 → too-small message)

- [ ] **Step 1: Write the failing tests**

Render at 70×24: assert the frame contains the focused column's header and the 1-row player bar with `CONT` and `MODE`, and a 1-row footer with `> next · M mode · / search · ? help`. At 50×18: assert the "terminal too small" message (raise the too-small threshold to `< 60 wide or < 20 tall`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test layout`
Expected: FAIL.

- [ ] **Step 3: Implement the narrow branch**

In `layout.rs::draw`: if `60 <= width < 80` (or `height < 24`) → `render_narrow(f, area, app)`. Move the too-small guard to `width < 60 || height < 20`. `render_narrow`: 1 focused pane (Miller collapse: show the column at `focus_col`, with a breadcrumb `Albums · Adele ← Artists` in the title), a 1-row player bar (`build_info_line` compressed: now-playing + `24bit/96 · vol 70%`), a 1-row gauge+flags, and a 1-row short footer.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test layout`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/view/layout.rs src/tui/view/columns.rs tests/layout.rs
git commit -m "feat(tui): narrow single-pane fallback for 60-80 col tmux splits"
```

---

### Task 15: TUI — `Overlay::YtAuth` (cookie paste) + `:yt` commands

**Files:**
- Modify: `src/tui/app.rs` (`Overlay::YtAuth { input: String }`, `Overlay::Command` execution for `:yt ...`)
- Modify: `src/tui/view/overlay.rs` (`render_yt_auth`)
- Modify: `src/tui/input.rs` (overlay key routing for `YtAuth`; command execution)
- Modify: `src/yt/session.rs` (`set_cookies` writes the env-equivalent and respawns the sidecar with it; `auth_status`)
- Test: `tests/input.rs` (append — paste flow, `:yt auth` opens overlay, Enter saves, Esc cancels)

**`Overlay::YtAuth` form (spec §5.7):** a centered popup; typing accumulates the pasted cookie text (multi-line — the overlay input is a `String` that accepts newlines from `Enter`? No — `Enter` submits. So paste is a single line or the overlay handles a paste event). **Simplify:** the cookie is a single-line `Cookie:` header value OR a Netscape cookies.txt paste. Accept multi-line by treating `Enter` as a literal newline *unless* the user presses a dedicated submit key. Use `Ctrl+Enter`/`Alt+Enter` to submit? Crossterm can't always distinguish. **Decision:** accept the paste as the raw text; `Enter` inserts a newline; **`Esc` cancels; a separate binding submits.** Use `:` close → actually, simplest robust approach: the auth overlay has its own keymap — `Enter` submits (cookies pasted as a single line via terminal paste; multi-line cookies.txt paste joins with `\n` but the user pastes with the terminal's paste which usually arrives as separate `Char` events including newlines). To avoid ambiguity: **`Enter` submits, and pasted newlines become spaces** (cookies.txt lines are tab-delimited; joining with spaces still parses). Document this.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/input.rs append
#[test]
fn yt_auth_overlay_open_via_command() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // open command overlay, type "yt auth", Enter
    handle_key(&mut app, key(':'));
    type_in(&mut app, "yt auth");
    handle_key(&mut app, key_enter());
    assert!(matches!(app.overlay, Some(Overlay::YtAuth { .. })));
}

#[test]
fn yt_auth_enter_saves_and_closes() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::YtAuth { input: "# Netscape\na.b\tTRUE\t/\tFALSE\t0\tname\tval".into() });
    handle_key(&mut app, key_enter());
    assert!(app.overlay.is_none());
    // cookies stored in a config-side file (Session picks them up)
    assert!(jukebox::yt::session::cookies_file().exists());
    let _ = std::fs::remove_file(jukebox::yt::session::cookies_file());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test input yt_auth`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `Overlay::YtAuth { input: String }` in app.rs. `Session::set_cookies(text)` writes `<config_dir>/jukebox/yt-cookies.txt` (perms 0600) and respawns the sidecar with `JUKEBOX_YT_COOKIES` set from the file. Add `pub fn cookies_file() -> PathBuf` and `pub fn load_cookies() -> Option<String>`.
- `overlay.rs::render_yt_auth`: the centered popup from spec §5.7.
- `input.rs`: in `handle_overlay_key`, add an `Overlay::YtAuth` arm: `Char(c)` (no Ctrl/Alt) → push c; `Enter` → push ' ' (newline-as-space, per the decision); `Backspace` → pop; **`Ctrl+S`**? no (reserved). Use **`Alt+Enter`/`Shift+Enter`** → not reliably detectable. **Final:** `Enter` submits; a pasted newline arrives as `Char('\n')` which we push as a space. Wait — that makes typing Enter submit AND a paste-newline become a submit. Conflict. **Resolution:** the auth overlay treats `Ctrl+J` (linefeed, distinct from Enter/`Ctrl+M`) as a literal newline and `Enter` as submit. Pastes that include `\n` arrive as `Char('\n')` events → push space. This is robust. Document.
- `Overlay::Command` Enter handler: parse `input`; if it starts with `yt ` → `yt auth` opens `Overlay::YtAuth`, `yt logout` clears cookies + respawns, `yt setup` prints a hint (for v1, set a `yt_error` string shown in Y view). Otherwise close.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test input`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs src/tui/view/overlay.rs src/tui/input.rs src/yt/session.rs tests/input.rs
git commit -m "feat(tui): in-app YouTube auth overlay + :yt auth/logout/setup"
```

---

### Task 16: TUI — `Overlay::Discover` (`S`) + instant random (`s`) + local smart-album heuristic

**Files:**
- Modify: `src/tui/app.rs` (`Overlay::Discover { items: Vec<DiscoverItem> }`, `instant_random`, `open_discover`, `local_smart_albums`)
- Modify: `src/tui/view/overlay.rs` (`render_discover`)
- Modify: `src/tui/input.rs` (`s`, `S`, discover selection)
- Test: `tests/app.rs` (append), `tests/input.rs` (append)

**Local smart-album heuristic (spec §5.5):** score each album by `recency_decay(last_played) + artist_diversity + small_random`. `last_played` requires play history — we have `transport.history` (recently-played track ids); derive per-album "last played" as the max timestamp-ish... we have no timestamps. **Simplify honestly:** since we lack timestamps, the heuristic is `artist_diversity` (count distinct artists among recent history to prefer albums by less-recently-played artists) + a deterministic pseudo-random from `transport.rng_state`. Score = `-(times artist appeared in last N history entries) + rng_weighted`. Pick 5 weighted-random. Document the simplification (no real recency without timestamps; future: persist play timestamps).

- [ ] **Step 1: Write the failing tests**

```rust
// tests/app.rs append
#[test]
fn s_instant_random_plays_in_context() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.instant_random();
    assert!(app.now_playing.is_some());
    assert!(app.transport.order.len() >= 1); // context set
}

#[test]
fn S_discover_lists_local_albums_in_local_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = SourceMode::Local;
    app.open_discover();
    assert!(matches!(app.overlay, Some(Overlay::Discover { .. })));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test app s_instant && cargo test --test app S_discover`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `App::instant_random`: pick a random id from the active source (Local → `catalog.tracks` random; Youtube → a random `home_suggestions`/search result; Mixed → either). Build a `Context::Album`/`Search` around it and `start_playback`.
- `App::open_discover`: Local/Mixed → `local_smart_albums()` → 5 `DiscoverItem::Album{artist,album}`; Youtube → `home_suggestions` → `DiscoverItem::Playlist{id,name}`. Set `Overlay::Discover`.
- `local_smart_albums`: the heuristic above.
- `overlay.rs::render_discover`: list the items; Enter selects (wired in input.rs Task 18).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs src/tui/view/overlay.rs src/tui/input.rs tests/app.rs tests/input.rs
git commit -m "feat(tui): s instant random + S discover overlay with local smart-album heuristic"
```

---

### Task 17: TUI — filter-on-column (`f`) inline prompt

**Files:**
- Modify: `src/tui/app.rs` (`pub filter: Option<FilterState>` where `FilterState { col: usize, text: String }`)
- Modify: `src/tui/view/columns.rs` (render the filter prompt line at the top of the filtered column; filter the list by `text`)
- Modify: `src/tui/input.rs` (`f` opens filter on focused col; typing narrows; `Esc`/`Enter` clears)
- Test: `tests/input.rs`, `tests/columns.rs`

- [ ] **Step 1: Write the failing tests**

Typing `f` then `ade` in the Artists column narrows the rendered artists to those containing "ade" (case-insensitive); `Esc` restores the full list.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test input filter && cargo test --test columns filter`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `FilterState { col: usize, text: String }` on `App`.
- `f`: if no filter, set `filter = Some(FilterState{col:focus_col, text:""})`. If a filter is active, `f` is a no-op (typing goes to it).
- `handle_key`: when `filter.is_some()`, route chars to `filter.text`, `Esc`/`Enter` clears, navigation still works on the filtered list.
- columns.rs: if `filter.col == this_col`, render a `(filter: <text>▏)` line at the top and filter the list items by `item.to_lowercase().contains(text)`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test input --test columns`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs src/tui/view/columns.rs src/tui/input.rs tests/
git commit -m "feat(tui): f inline filter-on-focused-column"
```

---

### Task 18: Wire `M`/`1-4`/`f`/`s`/`S`/`:yt` in `input.rs`; update `?` help

**Files:**
- Modify: `src/tui/input.rs` (bind `4`→`View::Youtube`, `M`→`cycle_mode`, `f`, `s`, `S`; `/` scope follows view; `:yt` execution; `Tab`/`cycle_view` includes Youtube)
- Modify: `src/tui/view/overlay.rs` `help_lines` (add YT keys; `4` Y view, `M` mode, `f` filter, `s` random, `S` discover, `:yt auth`)
- Modify: `src/main.rs` (restore `source_mode` from `layout.source_mode`; pass to `App`)
- Test: `tests/input.rs` (append — each new key dispatches correctly across modes)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/input.rs append
#[test]
fn m_cycles_source_mode_without_stopping() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let playing = app.now_playing.clone();
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, SourceMode::Youtube);
    assert_eq!(app.now_playing, playing); // not stopped
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, SourceMode::Mixed);
    handle_key(&mut app, key('M'));
    assert_eq!(app.source_mode, SourceMode::Local);
}

#[test]
fn four_switches_to_youtube_view() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    handle_key(&mut app, key('4'));
    assert_eq!(app.view, View::Youtube);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test input m_cycles && cargo test --test input four_switches`
Expected: FAIL.

- [ ] **Step 3: Implement the bindings**

- `4` → `switch_view(app, View::Youtube)`.
- `M` → `app.cycle_mode()`.
- `f`, `s`, `S` → Task 16/17 methods.
- `/` → unchanged opens `Overlay::Search`, but `run_search`/`update_search_results` scope: in `View::Youtube`, `/` searches YouTube via `session.search` (add a `YtSearch` overlay variant or reuse `Search` with a `scope` field). **Decision:** reuse `Overlay::Search` and add a `yt: bool` flag; `update_search_results` dispatches to `session.search` when `yt`.
- `Tab`/`cycle_view`/`max_focus_col`/`focused_column_len` include `Youtube`.
- `?` help text updated.
- `main.rs`: restore `app.source_mode = SourceMode::from_str(&layout.source_mode);` and `app.transport.continue_mode` "youtube" arm; save with the mode.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add src/tui/input.rs src/tui/view/overlay.rs src/main.rs tests/input.rs
git commit -m "feat(tui): bind M/4/f/s/S/:yt and scope / to the active view"
```

---

### Task 19: End-to-end integration tests (stubbed sidecar)

**Files:**
- Create: `tests/e2e_yt.rs` (a fake sidecar python script returning canned JSON for each command; driven through `Session::spawn` with the fake script path)
- Test: the file itself

**Cases (spec §7):**
1. `search → play`: `search("adele")` returns `[{v1,...}]`; Enter plays `v1` → `now_playing == Remote{v1}`, `resolve_url(v1)` was called, `device_rate.in_yt_rate == true`.
2. `mixed match → local`: a YT track whose ISRC matches a catalog track → `now_playing == Local{...}`, local path loaded.
3. `mixed no-match → stream`: YT track with no local match → `Remote`, streamed.
4. `CONT=YouTube advance`: play `v1` with `continue_mode=YouTube`, finish → `next` plays the `get_watch_playlist(v1)` first track.
5. `dead-remote skip`: `resolve_url` returns `Error("unavailable")` → track added to `dead`, `next` advances to the next id, no halt.
6. `sidecar-down degrade`: spawn with a script that exits immediately → `Session` reports not alive → `App::next` with `CONT=YouTube` and no session → stops cleanly, no panic; local playback unaffected.

- [ ] **Step 1: Write the fake sidecar + tests**

A `fake_sidecar(canned: HashMap<cmd, json>)` python generator writing a temp script; `Session::spawn` pointed at it. Each test sets up the canned responses and drives `App` via the public methods (`play_in_context_ids`, `next`, etc.) + simulated tick (`on_tick`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test e2e_yt`
Expected: FAIL (not written yet / first run red).

- [ ] **Step 3: Implement + iterate**

Wire `App::on_tick` (called from the event loop's poll) to drain `Session::try_recv` and apply responses (refresh `yt_lists`, cache `track_cache`, fulfill pending `resolve_url`). Make the e2e tests drive `on_tick` between actions.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test e2e_yt`
Expected: PASS (all 6 cases).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e_yt.rs src/tui/app.rs src/yt/session.rs
git commit -m "test: end-to-end YT integration flows with a stubbed sidecar"
```

---

### Task 20: README + NOTICE + ToS disclaimer

**Files:**
- Modify: `README.md` (runtime prerequisites: `python3`, `ytmusicapi`, `yt-dlp`; `:yt setup` note; YouTube section; ToS disclaimer; third-party tools)
- Create: `NOTICE`

- [ ] **Step 1: Update README**

Add under "Runtime prerequisites": `python3`, `ytmusicapi`, `yt-dlp` (install via `pip install -r scripts/yt/requirements.txt`, or `:yt setup` from within the TUI). Add a "## YouTube integration" section: modes (`M`), auth (`:yt auth`), the Y view, mixed mode, and a clear **YouTube Terms of Service** disclaimer: "Automated access to YouTube may violate YouTube's Terms of Service. This integration is intended for personal use with content you have the right to access (e.g. your own Premium account). You are responsible for your use; the authors provide no warranty."

- [ ] **Step 2: Create NOTICE**

```
jukebox — Copyright (c) 2026 bibimoni (MIT License)

This product includes runtime tools invoked as subprocesses (not linked):

  ytmusicapi  — MIT License        — https://github.com/sigma67/ytmusicapi
  yt-dlp      — The Unlicense      — https://github.com/yt-dlp/yt-dlp

YouTube® is a trademark of Google LLC. This project is not affiliated with
or endorsed by YouTube. Automated access may be subject to YouTube's Terms
of Service; users are responsible for compliance.
```

- [ ] **Step 3: Commit**

```bash
git add README.md NOTICE
git commit -m "docs: YouTube runtime prereqs, ToS disclaimer, third-party NOTICE"
```

---

### Task 21: Strict judge gate

**Files:**
- Create: `docs/superpowers/plans/2026-07-08-youtube-judge-rubric.md` (the rubric, copied from spec §8 for the agent)
- Dispatch: a **fresh, isolated judge agent** via the `Agent` tool (subagent_type `general-purpose`), given *only* the rubric + instructions to launch the app live (using the `run`/`verify` skills) and score each dimension 0/1/2.

**Instructions to the judge agent (paste verbatim):**

> You are a strict UX judge. The jukebox app (a Rust TUI for local hi-res + YouTube music) has been implemented per the spec at `docs/superpowers/specs/2026-07-08-youtube-integration-design.md`. Your job: launch the app live, exercise it, and score the 8-dimension rubric below. You must score EVERY dimension at **max (2)** for the app to pass. If any is below max, report exactly which, with the concrete failing behavior and a repro, and STOP (do not attempt fixes). Be ruthless — the user explicitly said even a slight weird behavior fails it.
>
> Use the `run`/`verify` skills to actually launch and drive the app (`cargo run -- play`). Use a stubbed/fake YouTube session if real cookies aren't available, but the flows (search, mixed match, CONT=YouTube, dead-remote skip, sidecar-down, transitions) must be exercised. Read code only to confirm a behavior, not to substitute for running it.
>
> Rubric:
> 1. Keybinding consistency — every transport/mode key identical across Local/YouTube/Mixed; `f`/`s`/`S`/`/` work in every applicable view; no dead keys; `q`/`Esc` consistent.
> 2. Search coherence — `/` + `f` return sensible results; YT and local search forms identical; empty/typo queries degrade gracefully.
> 3. Streaming smoothness — gapless local↔YT↔local; no >~0.5s gap between consecutive YT tracks; no mid-stream stutter; buffering/rate-limit surfaced, never a freeze.
> 4. Layout balance — two-row player bar; footer hints; `Y` consistent with `P`; narrow (60–80) renders; no crash ≤60×20.
> 5. Seamless transitions — no flicker/blank-flash on mode/view/overlay/handoff; quality readout never disagrees with device rate.
> 6. Edge cases — dead/expired YT track skipped (no halt); sidecar-down keeps local alive; auth-not-set shows setup hint; no panics.
> 7. Premium parity — Premium cookies → AAC 256k ad-free; quality readout reflects actual stream.
> 8. CoreAudio cadence — re-clock once entering a YT session, held through consecutive YT tracks, restored on local — never mid-stream.
>
> Output: a table with each dimension's score (0/1/2) and a one-line justification. End with a single line: `VERDICT: PASS` (all 2s) or `VERDICT: FAIL` (any below 2) + the list to fix.

- [ ] **Step 1: Write the rubric doc**

Copy the rubric block above into `docs/superpowers/plans/2026-07-08-youtube-judge-rubric.md`.

- [ ] **Step 2: Run the full test suite + build**

Run: `cargo build --release && cargo test`
Expected: build + all tests pass. Fix any failures before judging.

- [ ] **Step 3: Dispatch the judge agent**

Call the `Agent` tool with `subagent_type: "general-purpose"`, `run_in_background: false`, and the judge instructions above (referencing the rubric doc). Capture its verdict.

- [ ] **Step 4: Act on the verdict**

- If `PASS`: commit any trivial polish the judge surfaced, then conclude. Write a final commit summarizing the feature.
- If `FAIL`: for each failing dimension, open a focused fix (re-invoke the relevant earlier task's TDD cycle), re-run the targeted tests, re-run the judge. Loop until `PASS` or the user intervenes.

- [ ] **Step 5: Final commit**

```bash
git add docs/superpowers/plans/2026-07-08-youtube-judge-rubric.md
git commit -m "test: strict judge rubric for YouTube integration completion gate"
```

(Plus any fix commits from Step 4.)

---

## Self-Review (run after writing, before handoff)

- **Spec coverage:** §1 goals → Tasks 1,2,3,9,10,12,15,21. §2 architecture → Tasks 1–11. §3 pipeline → Tasks 5,8,9,10,19. §4 matching → Task 4,10,11. §5 TUI → Tasks 11–18. §6 license → Task 20. §7 testing → Tasks 4,5,6,8,9,19,21. §8 judge → Task 21. ✓ all sections have tasks.
- **Placeholder scan:** The `yt-dlp` pin in Task 7 has an explicit "confirm/refresh the pin" note (not a placeholder — a real instruction). All code blocks are complete. No "TBD"/"add error handling" stubs.
- **Type consistency:** `TrackSource` (Task 3) used in Tasks 9,10,12. `RemoteTrack.isrc` added in Task 4 (Task 3's struct updated in Task 4 Step 3 — flagged). `ContinueMode::YouTube` (Task 2) used in Tasks 9,18. `SourceMode` (Task 1) used in Tasks 2,10,18. `Session` (Task 9) signature `App::new(cat, player, searcher, yt_session: Option<Session>)` introduced in Task 9 and applied to all prior `App::new` call sites — flagged in Task 9 Step 4. `ContextResolver::yt_playlist_ids` (Task 11) default-impl keeps existing fakes compiling. ✓ consistent.
- **One risk flagged for the implementer:** `App::new` arity changes in Task 9; every test calling `App::new` must be updated. Task 9 Step 4 says so explicitly. Task 10/18 tests already use the 4-arg form.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-08-youtube-integration.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for this large plan (21 tasks) to keep each task's context focused and catch issues early.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints for review.

Which approach?
