# Jukebox Architecture Audit

**Date:** 2026-07-12
**Scope:** `src/`, `scripts/yt/yt.py`, `tests/`, `specs/`, `README.md`
**Method:** READ-ONLY reconnaissance. All claims carry `file:line` evidence.

---

## 1. Architecture Map

Single binary, in-process, one Python sidecar subprocess for YouTube. No network
listeners, no threads beyond the sidecar reader thread.

**Modules** (`src/lib.rs:1-13`):

| Module | Role | Key types |
|---|---|---|
| `main` | CLI dispatch (`Play`/`Sync`/`Index`/`Search`/`Config`), launch wiring | — |
| `cli` | clap parse, first-run config prompt | `Cli`, `Cmd` (`cli.rs:6-32`) |
| `config` | YAML-ish config (hand-rolled, no serde_yaml) | `Config`, `PlayerKind` (`config.rs:13-25`) |
| `catalog` | `catalog.json` load | `Catalog`, `Track` (`catalog.rs:5-33`) |
| `search` | Tantivy + Lindera IPADIC index | `Searcher`, `Hit` (`search.rs:145-157`) |
| `translit` | kana↔romaji variant generation (wana_kana) | `variants` (`translit.rs:59`) |
| `state` | SQLite KV (`state.db`): layout, playlists, focus | `LayoutState` (`state.rs:113-134`) |
| `mode` | `SourceMode` {Local, Youtube, Mixed} | `mode.rs:13-19` |
| `player` | `Player` trait + `MpvPlayer`/`AfplayPlayer`/`StubPlayer` | `player.rs:8-44` |
| `audio` | macOS CoreAudio device-rate switch (no-op off-mac) | `set_output_format`, `CapturedFormat` (`audio.rs:425`) |
| `prompt` | stdin first-run dir prompt | `prompt_source_dir` (`prompt.rs:9`) |
| `source` | `TrackSource`, `RemoteTrack`, `StreamFormat`; `match_local`, `device_rate` | `source/mod.rs:20-74` |
| `tui/app` | **God object** (1819 lines): all state + all update methods | `App` (`app.rs:154-228`) |
| `tui/event` | Terminal event loop, panic/signal hygiene | `run` (`event.rs:191`) |
| `tui/input` | Key+mouse dispatch (780 lines) | `handle_key` (`input.rs:25`) |
| `tui/context` | `Context` enum + `ContextResolver` trait | `context.rs:36-62` |
| `tui/queue` | `Transport` engine: order/cursor/history/shuffle/repeat | `queue.rs:41-52` |
| `tui/view/*` | Pure renderers: `layout`, `columns`, `player_bar`, `overlay`, `footer`, `theme` | `view/mod.rs:1-6` |
| `yt/proto` | NDJSON wire protocol | `Request`, `Response` (`proto.rs:17-102`) |
| `yt/sidecar` | Python subprocess client + reader thread | `Sidecar` (`sidecar.rs:18-33`) |
| `yt/session` | **Fat module** (999 lines): auth, caches, inflight tracking, radio | `Session` (`session.rs:198-255`), `RadioCursor` (`session.rs:924-927`) |

**Ownership / data flow:**

```
main.rs ──► App (owns Catalog, Box<dyn Player>, Option<Searcher>,
                  Option<Session>, Transport, playlists, view state)
                  │
                  ├── tui::event::run ──► poll/draw loop
                  │       │
                  │       ├── input::handle_key ──► App methods (start_playback,
                  │       │                                  next/prev, etc.)
                  │       └── view::layout::draw ──► columns/player_bar/overlay
                  │
                  └── App ──► Session ──► Sidecar (stdin/stdout pipes + reader thread)
                              │             └── python yt.py ──► ytmusicapi + yt-dlp
                              ├── track_cache: HashMap<vid, RemoteTrack>
                              ├── url_cache: Vec<CachedResolve> (cap 2)
                              └── pending: VecDeque<Pending> (FIFO pairing)
```

`App` implements `ContextResolver` (`app.rs:274-285`) so `Transport` can resolve
Playlist/Queue/Youtube context ids. A `ClonedResolver` (`app.rs:297-321`) clone
of `manual_queue` + borrow of `playlists`/`yt_lists` works around the
`&self`/`&mut self.transport` split-borrow.

---

## 2. Startup & Shutdown Flow

**Startup** (`main.rs:8-178`, `Cmd::Play`):

1. `cli::ensure_config()` (`cli.rs:35`) — load `config.yml` or run first-run prompt.
2. `catalog::Catalog::load(catalog.json)` (`main.rs:21`).
3. `search::Searcher::open(search-index).ok()` (`main.rs:25`) — missing index is
   non-fatal (returns `None`, local search degrades to empty).
4. `player::launch(cfg.player, mpv_socket)` (`main.rs:26`) — mpv spawn; falls back
   to `AfplayPlayer` if mpv IPC socket unavailable (`player.rs:390-397`).
5. Resolve `yt.py` script + python (`main.rs:30-48`): `$JUKEBOX_YT_PYTHON` →
   venv python → `python3`.
6. `Session::spawn(python, script, cookies)` (`main.rs:56`) — `.ok()`, so spawn
   failure → `yt_session = None` (YT features degrade, local playback fine).
7. `App::new(...)` (`main.rs:58`) — builds artist index, albums_by_artist, transport.
8. Restore `LayoutState` from `state.db` (`main.rs:68-131`): column widths, volume,
   shuffle/repeat/continue/source_mode, view, `yt_browser`. If a saved browser
   profile has no cached cookies, re-reads the browser (Keychain prompt) at
   `main.rs:113`; on failure clears `yt_browser` (`main.rs:125`).
9. **Reachability probe** (`main.rs:142-166`): if `yt_session.is_some()`, calls
   `s.library_playlists()` — a **blocking ~3s roundtrip** (`session.rs:883`). On
   `Err`, sets `app.yt_session = None` (`main.rs:151`) + `yt_error`. Falls back
   `view = Artists` if it was on `Youtube`.
10. Apply volume/mute to player (`main.rs:170-171`).
11. `audio::capture_default_format()` (`main.rs:176`) — macOS only, for restore.
12. `tui::event::run(&mut app, captured)` (`main.rs:178`) — enters alt screen.

**Shutdown** (`main.rs:183-193`): on clean exit, best-effort `save_layout` +
`save_playlists` (errors dropped). `TerminalGuard::drop` (`event.rs:61-69`) +
panic hook restore terminal + audio. `MpvPlayer::drop`/`AfplayPlayer::drop`
kill+reap children (`player.rs:163-170`, `381-388`). `Sidecar::drop` kills the
python child (`sidecar.rs:135-139`).

**Blocking calls in startup:** step 9 (`library_playlists`, 3s deadline
`session.rs:883`) blocks the user before the TUI appears. The comment at
`main.rs:140` claims "the sidecar's ytmusicapi init prints a network-flavored
error and sets have=False, so this fetch returns an error fast (not a hang)" —
but the deadline is 3s, and a slow Keychain unlock + init can approach it.

---

## 3. Event Loop & Rendering

`event::run` (`event.rs:191-274`):

- Installs panic hook (`event.rs:114-124`) + SIGTSTP/SIGCONT handlers
  (`event.rs:129-142`).
- `enable_raw_mode`, `EnterAlternateScreen`, `EnableMouseCapture`, hide cursor.
- `TerminalGuard` scoped to the whole loop body (drop restores terminal).
- **Loop** (`event.rs:223-264`), no fixed cadence:
  1. Drain `NEED_REDRAW` (SIGCONT) → clear + draw.
  2. Drain `SIGTSTP_RECEIVED` → `handle_sigtstp` (restore terminal, raise
     SIGTSTP, on SIGCONT re-enter alt screen + redraw).
  3. `app.player.track_ended()` → `app.on_track_ended()` (mpv end-file eof or
     afplay child exit; `player.rs:335-379`, `149-161`).
  4. **`app.on_tick()`** — every iteration (~150ms cadence, see below).
  5. `terminal.draw(|f| view::layout::draw(f, app))` — **redraws every loop
     iteration**, not just on input. Wasteful but simple.
  6. `event::poll(POLL_TIMEOUT_MS = 150ms)` (`event.rs:42`); on ready,
     `handle_key` / `handle_mouse` / ignore Resize.

**Redraw strategy:** unconditional full redraw every iteration. No dirty
tracking. The poll timeout (150ms) caps idle CPU; `on_tick` + draw run even with
no input. `NEED_REDRAW` (SIGCONT) forces an extra clear+draw.

**Does `on_tick` block?** No — `drain_paired` uses `try_recv` (non-blocking,
`sidecar.rs:110-116`). But **user actions can block**:
- `refresh_yt_lists` (view switch to Youtube, `input.rs:616`) is fire-and-forget.
- `home_suggestions()` in `yt_discover_items` (`app.rs:1495`) — **blocking 3s**
  roundtrip when `S` is pressed.
- `get_playlist()` in `play_discover_selection` (`app.rs:1638`) — **blocking 4s**
  on Enter in the discover overlay.
- Launch `library_playlists()` (`main.rs:148`) — **blocking 3s**.

These block input + render for up to 4s. `on_tick` itself is non-blocking.

---

## 4. Application State

`App` struct (`app.rs:154-228`) — 31 fields:

- **Catalog-derived** (rebuilt in `new`, `app.rs:330-337`): `catalog`,
  `artists: Vec<String>`, `artist_index: BTreeMap<String, Vec<usize>>`,
  `albums_by_artist: BTreeMap<String, Vec<Album>>`.
- **Transport**: `transport: Transport`, `now_playing: Option<TrackSource>`,
  `dead: HashSet<String>`, `radio: RadioCursor`, `device_rate`.
- **Browse state**: `view: View`, `focus_col`, `cursors: ColumnCursors`,
  `column_widths: ColumnWidths`, `filter: Option<FilterState>`, `help_scroll`.
- **Player**: `player: Box<dyn Player>`, `volume: u8`, `muted`,
  `switch_sample_rate`, `playing_premium`, `pending_play: Option<String>`,
  `spinner_frame`.
- **YouTube**: `yt_session: Option<Session>`, `yt_lists: Vec<YtList>`,
  `yt_lists_loading`, `loaded_yt_lists: HashSet<String>`, `yt_error`,
  `yt_status`, `yt_python`, `yt_script`, `yt_browser`.
- **Source mode**: `source_mode: SourceMode`.
- **Overlays**: `overlay: Option<Overlay>`, `pending_g` (leader key).
- **Lifecycle**: `should_quit`.

**Persisted** (`state.db`, SQLite KV, `state.rs`):
- `'layout'` → JSON `LayoutState` (`state.rs:113-134`): `focus` (view),
  `widths`, `volume`, `shuffle`, `repeat`, `continue_mode`, `source_mode`,
  `yt_browser`. Saved on clean exit (`main.rs:183-192`).
- `'playlists'` → JSON array of `Playlist` (`state.rs:276-285`).
- `'focus'` → legacy single key (`state.rs:55-81`); superseded by `'layout'.focus`
  but still has read/write helpers. Doc rot: `state.rs:1-6` comment still says
  "stores only the last-focused pane".

**NOT persisted:**
- `now_playing`, `transport` cursor/order/history, `manual_queue`.
- `dead` track set (rebuilt each session; cleared on context switch
  `app.rs:803`).
- `yt_lists` (re-fetched on Y-view entry), `track_cache`, `url_cache`.
- `overlay` state, `filter`, `help_scroll`, `pending_g`, `spinner_frame`.
- `pending_play`, `playing_premium`.
- **Command history** (none exists — see §13).
- **Lyrics** (none exist — see §13).

`state.db` is opened + closed per call (`state.rs:33-45`); no long-lived
connection. Single-process, so SQLite locking is fine.

---

## 5. Playback Transport & Queue

`Transport` (`queue.rs:41-52`) owns the play order:

- `context: Context` — the active list (Album/Artist/Playlist/Search/Youtube/Queue).
- `order: Vec<usize>` — permutation over `context.track_ids(r)`.
- `cursor: usize` — index into `order`.
- `history: Vec<(String, Context)>` — for `prev()`; pushed on `next`/context
  switch (`queue.rs:112-113`, `app.rs:809`, `873`, `1702`, `1722`).
- `manual_queue: Vec<String>` — "play next" queue; drained after context
  exhaustion (`queue.rs:119-127`).
- `shuffle: ShuffleMode` (Off/Smart/Random), `repeat: RepeatMode`
  (Off/All/One), `continue_mode: ContinueMode` (Off/NextAlbum/Radio/YouTube).
- `rng_state: u64` — seeded xorshift64* for Fisher–Yates + smart shuffle.

**Player trait** (`player.rs:8-44`): `load`, `load_at` (resume at offset, used
by premium upgrade `app.rs:1328`), `play_pause`, `seek`, `seek_to`,
`stop`, `set_volume`, `set_muted`, `position`, `duration`, `is_playing`,
`track_ended`. Backends: `MpvPlayer` (Unix-socket IPC, non-blocking read,
`player.rs:173-388`), `AfplayPlayer` (SIGSTOP/SIGCONT pause, no seek/position,
`player.rs:68-170`), `StubPlayer` (tests).

**Source resolution** (`app.rs:638-684`, `resolve_source`):
- Local/Mixed: catalog match → `Resolved::Local { path, sr, bd }`.
- YouTube/Mixed-no-local: `session.url_for(id)` → `Resolved::Remote` (cached);
  else fire-and-forget both tiers (`send_resolve` fast + `send_resolve_premium`
  premium) → `Resolved::Pending` (`app.rs:681-683`).
- `pending_play` (`app.rs:227`) carries a cold-miss id; `on_tick` swaps the
  player in when the URL lands (`app.rs:1338-1346`).

**Auto-advance** (`app.rs:835-900`, `next`): repeat-one replays; else push
history, advance cursor; on exhaustion: manual_queue → next; else repeat-all →
loop; else `continue_mode` decides: Off stops, NextAlbum auto-continues
(`app.rs:851-858`), Radio rebuilds library shuffled (`app.rs:860-863`,
`switch_to_radio` `app.rs:1720-1731`), YouTube asks `RadioCursor::advance`
(`app.rs:864-896`).

---

## 6. Persistence

**Schema** (`state.rs:38-43`): single table `state(key TEXT PRIMARY KEY, value
TEXT NOT NULL)`. No migrations — schema is `CREATE TABLE IF NOT EXISTS`. New
keys are added by UPSERT; old keys orphan harmlessly. JSON values are
hand-versioned via `LayoutState` serde defaults (`state.rs:115-133`).

**What round-trips:**
- `LayoutState` (focus view, column widths, volume, shuffle/repeat/continue/
  source_mode strings, `yt_browser`).
- `Playlists` (name + track_ids).

**What does NOT round-trip** (see §4). Notably the transport state
(cursor/order/history/manual_queue) is rebuilt from scratch each launch — so
"prev" across sessions is impossible, and a saved `Youtube` view with
`continue_mode=YouTube` will have an empty radio cursor on relaunch.

**Cookies** (`session.rs:58-77`): `yt-cookies.txt` (Netscape, 0600) in config
dir. Browser-auth writes a decrypted cache here (`sidecar.rs` via
`JUKEBOX_YT_COOKIES_FILE`, `yt.py:144-181`) so subsequent launches don't
re-prompt the Keychain.

**Migration story:** none. Adding a field to `LayoutState` requires a serde
`#[serde(default)]` or old configs fail to parse. `yt_browser` uses
`#[serde(default)]` (`state.rs:132`) — safe. `continue_mode` and
`source_mode` are parsed-as-string with fallback to default
(`main.rs:86-98`) — safe.

---

## 7. Local Library Indexing / Search

**Indexing** (`main.rs:195-211`, `Cmd::Sync`): runs `scripts/standardize.sh`
(bash, out of scope) → `catalog.json` → `search::build_index`.

`build_index` (`search.rs:107-138`): Tantivy `MmapDirectory`; schema with
Lindera (embedded IPADIC) tokenizer on `artists`/`title`/`album`, lowercase
tokenizer on `title_variants`/`artist_variants` (`search.rs:44-76`). Variants
from `translit::variants` (`translit.rs:59`) — kana→romaji (with chōonpu
normalization, `translit.rs:29-43`) + kana↔kana. `delete_all_documents` +
re-commit on rebuild.

`Searcher::search` (`search.rs:187-219`): BM25 across title (×2 boost),
title_variants (×1.5, fuzzy edit-2), artists, artist_variants (fuzzy edit-2),
album. Returns `Hit { track_id, score }`. The TUI filters hits to catalog
tracks that still exist (`app.rs:1781-1784`) — index may lag a re-scan.

**Catalog** (`catalog.rs:5-33`): `Track` has `id`, `artists`, `primary_artist`,
`title`, `album`, `track_number`, `disc_number`, `bit_depth`, `sample_rate_hz`,
`isrc`, `source_path` (relative to parent of `source_root`, resolved at
`catalog.rs:48-53`), `symlinked_into_artists`. Albums grouped via
`build_albums_by_artist` (`context.rs:120-153`) — filed under every artist in
`symlinked_into_artists` so collaborators appear in their own Albums column.

**Translit** is also reused by `match_local` (`match_local.rs:40-49`) for
Mixed-mode ISRC-or-fuzzy local substitution (TITLE_GATE 0.88, ARTIST_FLOOR
0.80, `match_local.rs:14-15`).

---

## 8. Provider Abstractions

**`source/` module** (`source/mod.rs`): `TrackSource` (Local{track_id} |
Remote{video_id}), `RemoteTrack`, `StreamFormat` (codec/abr/sample_rate/
container/premium). `device_rate` (CoreAudio re-clock cadence,
`device_rate.rs:26-55`): switch once on YT session start, hold across same-rate
YT tracks, restore on local resume. `match_local` (ISRC exact or normalized
Levenshtein artist+title, `match_local.rs:19-85`).

**`yt/` module**:
- `Sidecar` (`sidecar.rs:18-33`): spawns `python script` with
  `JUKEBOX_YT_COOKIES` / `JUKEBOX_YT_BROWSER` / `JUKEBOX_YT_COOKIES_FILE` env.
  Reader thread (`sidecar.rs:68-86`) blocks on stdout, pushes complete lines to
  an `mpsc` channel; `try_recv` drains non-blocking.
- `Session` (`session.rs:198-255`): owns sidecar + `track_cache`
  (vid→RemoteTrack) + `url_cache` (Vec<CachedResolve>, cap 2 = current+next,
  `session.rs:262`) + FIFO `pending: VecDeque<Pending>` for response pairing.
  Two independent inflight guards: `resolve_inflight` (fast) and
  `premium_resolve_inflight` (`session.rs:214-219`).
- `RadioCursor` (`session.rs:924-968`): autoplay queue + cursor; refills via
  `get_watch_playlist(radio=True)` seeded by the just-finished track.

**Local vs YouTube vs Hybrid** (`mode.rs`, `app.rs:638-684`):
- **Local**: catalog id → local file.
- **YouTube**: catalog tracks are *streamed* not played locally
  (`app.rs:643-649` — only Local/Mixed take the local path).
- **Mixed**: catalog match → local; else `session.url_for` → remote; else
  Pending (fire-and-forget both tiers).

`Context::Youtube { key, name }` (`context.rs:56-60`) resolves via
`ContextResolver::yt_playlist_ids` (`context.rs:20-22`,
`app.rs:314-320` through `ClonedResolver`), distinct from
`Context::Playlist` (local-only) so same-named lists can't collide.

---

## 9. Configuration

Three layers:

1. **`config.yml`** (`config.rs:31-37`): `$XDG_CONFIG_HOME/jukebox/config.yml`
   else `dirs::config_dir`. Hand-rolled YAML reader/writer (`config.rs:118-161`)
   — no serde_yaml dependency (maintenance mode). Keys: `version`,
   `source_dir`, `filtered_dir`, `player` (mpv|afplay), `mpv_socket`,
   `switch_sample_rate` (default true). 0700 file perms on save
   (`config.rs:78`).
2. **`state.db`** (`state.rs:22-28`): SQLite KV, same config dir. Layout +
   playlists (see §6).
3. **Env vars**:
   - `XDG_CONFIG_HOME` — config + state + cookies location.
   - `JUKEBOX_YT_PYTHON` — override the sidecar interpreter (`main.rs:42`).
   - `JUKEBOX_YT_COOKIES` / `JUKEBOX_YT_BROWSER` / `JUKEBOX_YT_COOKIES_FILE`
     — passed to the sidecar (`sidecar.rs:56-61`); set per-spawn, not global.
   - `NO_COLOR` — monochrome theme (`theme.rs:6-8`).

No env-based config overrides beyond these. No `JUKEBOX_*` feature flags.

---

## 10. Error Propagation

`anyhow::Result` throughout. `?` propagation in `main`, `event::run`,
sidecar/session roundtrips. **Error swallowing / hazards:**

- **Best-effort `let _ =`** everywhere: player commands (`app.rs:598, 742, 778`,
  `1322, 1328`, `1736-1747`), audio switch (`app.rs:596, 740, 775, 1322`),
  state save (`main.rs:183, 193`), respawn (`app.rs:1085-1088`). Intentional —
  playback must not abort on a failed IPC write — but means failures are
  silent (no log, no UI signal) except where a `yt_error` is set.
- **`log_to_file`** exists (`event.rs:96-105`) but is `#[allow(dead_code)]` —
  **never called**. The sidecar's stderr is `Stdio::null()` (`sidecar.rs:55`),
  so Python tracebacks vanish. The only error surface to the UI is
  `Response::Error` → `pending_errors` → `yt_error` (`session.rs:583-605`,
  `app.rs:1217-1261`).
- **`.unwrap()` / `.expect()` hazards:**
  - `main.rs:35, 201` — `current_exe()?.parent().unwrap()` (safe in practice).
  - `sidecar.rs:65-66` — `child.stdin.take().expect("stdin piped")` (panics if
    spawn didn't pipe — would be a programming error, not runtime).
  - `session.rs:392` — `find(...).expect("just inserted")` (safe — just
    inserted above; but `expect` in a cache lookup is brittle if refactored).
  - `app.rs:1038, 1092` — `self.yt_session.as_mut().unwrap()` after a
    respawn-Ok branch (safe given control flow, but fragile under refactor).
  - `config.rs:126` — `line.split('#').next().unwrap()` (safe — iterator
    always yields at least one element).
  - `proto.rs:39` — `to_string(self).expect("request serializes")` (safe —
    serde_json on a fixed enum).
- **Roundtrip deadline errors** (`session.rs:669-687`): on timeout, frees the
  inflight guard but leaves the `pending` entry; later drained response pairs
  with it. `Err(anyhow!("sidecar roundtrip timeout"))` propagates to caller.

---

## 11. Likely Fault Boundaries

1. **Launch probe nukes the session** (`main.rs:151`): a single
   `library_playlists()` network failure → `yt_session = None`, destroying the
   sidecar + all caches. The user must re-trigger spawn (only via `:yt auth`
   re-run). A transient network blip at launch permanently disables YT for the
   session. See §Q1.
2. **`yt_status`/`yt_error` desync** (`main.rs:117/129` vs `:151`): "connected"
   status set before the probe; not cleared on probe failure. Both fields can
   be set simultaneously; footer shows error first but the `yt_status` field
   stays stale. See §Q2.
3. **No generation/cancellation id** for background fetches. `refresh_yt_lists`
   (`app.rs:1653-1675`) clears App-side staging slots but NOT the sidecar's
   in-flight requests. A stale `LibraryPlaylists`/`HomeSuggestions` response
   already sent can land after a fresh refresh, re-populate `pending_playlists`
   via `apply_pair` (`session.rs:572-573`), and `on_tick` merges the STALE list
   into `yt_lists` (`app.rs:1126-1149`). See §Q3.
4. **Single-slot staging fields** race: `pending_playlists`, `pending_suggestions`,
   `pending_tracks`, `pending_premium_url`, `pending_search` are all `Option`
   single slots. A second request's result overwrites the first's staging
   (mitigated for search by query-tagging, `session.rs:175`; NOT mitigated for
   playlists/suggestions).
5. **`url_cache` cap 2** (`session.rs:262`): only current+next. A premium
   preload for track N+1 evicts track N-1's entry; if the user hits `prev`,
   it's a cold miss again. Acceptable but a latency trap.
6. **Blocking user actions** (`home_suggestions` 3s, `get_playlist` 4s,
   `play_discover_selection`): the event loop stalls (no input, no render)
   for up to 4s. `on_tick` doesn't block, but `S`/discover-Enter do.
7. **afplay stacking** (`player.rs:77-84`): historically orphans; now
   `kill_current` on every load + `Drop` guard. Believed fixed but the
   `RefCell<Child>` + `&self` probe (`player.rs:147`) is unusual.
8. **mpv `drain_socket`** (`player.rs:254-265`) clears stale events on load;
   if a genuine end-file for the NEW track lands in the drain window it's
   lost (acceptable — auto-advance would be wrong anyway).
9. **Transport `prev` with no history** (`queue.rs:149-151`): replays current
   id from the start — but `load_track` is called, which re-resolves (and for
   YouTube, may re-fetch). A premium-upgrade mid-prev could race.
10. **`ClonedResolver` clone** (`app.rs:566, 691, 879`, etc.): `manual_queue`
    is cloned on every Transport call. Cheap for small queues, but a code smell
    indicating the `&self`/`&mut self.transport` split-borrow is fought
    everywhere rather than solved structurally.

---

## 12. Risky Coupling

- **`app.rs` is 1819 lines (83KB)** — a god object. The module doc
  (`app.rs:1-9`) claims "pure update methods, no I/O" but the methods call
  `self.player.load()`, `audio::set_output_format()`, `session.send_*()`,
  `std::fs::metadata()`, `session.home_suggestions()` (blocking). It holds all
  state AND all update logic AND source resolution AND YT lifecycle management.
  Splitting into `app/state.rs`, `app/playback.rs`, `app/yt.rs`, `app/discover.rs`
  would be a natural refactor boundary.
- **`session.rs` is 999 lines (45KB)** — owns sidecar, auth, two caches, FIFO
  pairing, six `pending_*` staging slots, two inflight guards, respawn
  backoff, AND `RadioCursor`. The `Pending` enum + `apply_pair` matcher
  (`session.rs:477-613`) is a large match that must stay in sync with
  `kind_for` (`session.rs:630-647`).
- **`input.rs` is 780 lines** — acceptable for a keymap, but mouse
  hit-testing (`input.rs:654-747`) hardcodes column geometry guesses that
  duplicate `columns.rs` layout logic (acknowledged at `input.rs:651-653`).
- **`view/columns.rs` 483 lines** — pure renderers, low risk.
- **Cross-module coupling**: `App` ↔ `Transport` ↔ `ContextResolver` form a
  triangle resolved only by `ClonedResolver`. `App` ↔ `Session` is tight
  (`on_tick` reaches into 8 Session fields). `main.rs` reaches into `App`
  fields directly for restore (`main.rs:69-131`).

---

## 13. Doc-vs-Implementation Conflicts

### README keybindings are largely WRONG (`README.md:142-145`)

README says vs actual (`input.rs`):

| README claim | Actual |
|---|---|
| `Tab` cycle panes | `Tab` cycles **views** (Artists→Playlists→Queue→Youtube), `input.rs:89` |
| `space` enqueue artist (Artists pane) | `Space` = **play/pause**, `input.rs:93` |
| `enter` enqueue result (Search) / play-now (Queue) | `Enter` = **play selected in context** (context-play, not enqueue), `input.rs:92` |
| `s`/`S` shuffle (S also jumps) | `s` = instant random track, `S` = discover overlay, `input.rs:112-113` |
| `r` remove | `r` = cycle repeat (off→all→one), `input.rs:105` |
| `c` clear queue | `c` = cycle continue mode, `input.rs:107` |
| `n`/`p` next/prev | **NOT BOUND**; next/prev is `>`/`<`, `input.rs:94-95` |
| `←/→` seek ±5s | `←/→` = move columns; seek is `,`/`.`, `input.rs:96-97, 64-65` |

The README describes the **old manual-enqueue model** that the 2026-07-06 TUI
revamp replaced. The in-app `?` help (`overlay.rs:259-300`) is correct. This is
the largest doc conflict.

### Lyrics: documented requirement, entirely unimplemented

- `specs/2026-07-06-tui-revamp-design.md:237` lists "Lyrics" (as a non-goal of
  *that* revamp — "Offline only").
- `docs/superpowers/specs/2026-07-08-youtube-integration-design.md:342`:
  "Lyrics (ytmusicapi `get_lyrics` already exists)."
- `.opencode/prompt` has extensive lyrics requirements (lines 489-514, 872-875,
  960-961, 1016): non-blocking provider pipeline, synced-line highlighting,
  stale-result guard, truthful unavailable state.
- `.opencode/todo.md:37` (M3) is **pending**.
- **Actual**: `grep -i lyric` in `src/` finds ZERO matches. No lyrics view,
  panel, overlay, provider, cache, or parser exists. There is no `Overlay::Lyrics`
  variant (`app.rs:105-142`). The player bar has no lyrics toggle. **Lyrics are
  entirely missing** despite the prompt listing them as a first-class
  requirement. See §Q5.

### Command history: documented requirement, unimplemented

- `.opencode/prompt` line 49 mentions "command history" as a reported issue.
- `.opencode/todo.md` has no explicit command-history task.
- **Actual**: `Overlay::Command { input: String }` (`app.rs:128-130`) holds
  only the current input. `execute_command` (`input.rs:416-439`) parses, runs,
  and sets `overlay = None`. There is **no history Vec, no up/down recall, no
  persistence to state.db**. `state.rs` has no `'command_history'` key. See §Q6.

### Other doc rot

- `state.rs:1-6` comment: "stores only the last-focused pane (Artists / Search /
  Queue)" — actually stores layout + playlists + yt_browser. "Search" isn't
  even a view (`View` has Artists/Playlists/Queue/Youtube, `app.rs:21-26`).
- `state.rs:14-15`: "Match the `Pane` enum variants in `tui`" — there is no
  `Pane` enum; it's `View`.
- `README.md:120`: "`jukebox config` re-runs the first-run prompt." — actually
  `jukebox config` with no args prints the config path (`main.rs:16`); only
  `ensure_config` runs the first-run prompt (`cli.rs:35-46`). The README
  overstates.
- `app.rs:1-9` module doc: "pure update methods, no I/O" — false (see §12).
- `README.md:60` "Up-Next pane for short lists" — exists
  (`columns.rs:274-289`, "Suggested / Up Next") but only shows suggested list
  *names*, not an actual up-next track queue. Mildly misleading.

---

## Specific Questions Answered

### Q1: Where does `app.yt_session` get set to None on launch, and what's the consequence?

**`main.rs:151`**, inside the reachability probe:

```rust
if app.yt_session.is_some() {
    let mut reachable = false;
    if let Some(s) = app.yt_session.as_mut() {
        match s.library_playlists() {   // blocking ~3s roundtrip
            Ok(_) => reachable = true,
            Err(e) => {
                app.yt_session = None;   // <-- HERE
                app.yt_error = Some(format!("YouTube unreachable: {e}..."));
            }
        }
    }
    if !reachable && app.view == View::Youtube {
        app.view = View::Artists;
    }
}
```

**Consequence:** a single transient network failure at launch (VPN blip,
rate-limit, captive portal) **destroys the entire `Session`** — the sidecar
process is killed+reaped (via `Sidecar::drop` when `Session` drops), and
`track_cache`/`url_cache`/auth are lost. The user is left with `yt_session =
None` for the whole session; the only recovery is to re-run `:yt auth browser
<name>` or `:yt auth` (which re-spawns). There is **no retry, no backoff, no
graceful degradation to a "YT offline, retry from the Y view" state** — the
session is gone. The comment (`main.rs:140`) justifies this as "don't strand
the user on the persisted Y view staring at an empty loading", but nuking the
session is far more aggressive than falling back the view. The probe itself
**blocks the launch for up to 3s** before the TUI appears.

### Q2: Is `yt_status = "connected"` set before any data is actually fetched?

**Yes, twice over:**

1. **`main.rs:117`**: after `Session::spawn_browser` succeeds at
   `main.rs:113-116`, `app.yt_status = Some("YT auth: connected via {browser}")`
   is set — but `spawn_browser` only spawns the process; ytmusicapi init
   happens async in the sidecar, and no YouTube data has been fetched.
2. **`main.rs:129`**: in the cached-cookies branch, same optimistic "connected"
   before the probe.
3. **`app.rs:988`** (`apply_yt_auth`) and **`app.rs:1014`**
   (`apply_yt_browser`): set `yt_status = "connected"` immediately on spawn
   success, before any fetch.

The probe at `main.rs:142-166` then runs and may set `yt_session = None` +
`yt_error` (line 151-156), **but it does NOT clear `yt_status`**. So after a
launch probe failure you can have:
- `yt_status = Some("YT auth: connected via chrome")` (stale, optimistic)
- `yt_error = Some("YouTube unreachable: ...")`
- `yt_session = None`

The footer (`footer.rs:22-33`) renders `yt_error` first (yellow) then
`yt_status` (accent) — so the error shows, but the `yt_status` field remains
"connected" indefinitely until something else overwrites it (e.g. `yt_logout`
at `app.rs:1391`, or a respawn at `app.rs:1093`). `on_tick`'s respawn path
(`app.rs:1059-1103`) can set `yt_status = "YT: sidecar restarted"` but only if
`should_respawn` — and `yt_session` is None, so the `if let Some(session) =
self.yt_session.as_mut()` guard at `app.rs:1059` skips entirely. **The session
is permanently unrecoverable from `on_tick`'s auto-respawn** because
`yt_session` is None, not a dead-but-present Session. So the optimistic
"connected" status can persist for the whole session alongside an error
saying it's unreachable. This is a real state inconsistency.

### Q3: Is there a generation/cancellation id for background sync? Can stale results overwrite newer state?

**No generation/cancellation id exists.** Mitigations are partial:

- **FIFO `pending` queue** (`session.rs:160-213`): responses pair with the
  oldest in-flight request kind. `Pending::matches` (`session.rs:188-196`)
  discriminates Resolve/ResolvePremium/Search by id/query. For
  Playlists/Suggestions/Tracks/Watch/Auth/Pong it's discriminant-only — so
  two `LibraryPlaylists` requests can't both be in flight usefully (the second
  just pushes a second `Pending::Playlists`).
- **`refresh_yt_lists`** (`app.rs:1653-1675`) clears App-side staging
  (`session.pending_playlists = None; session.pending_suggestions = None`)
  before re-sending, **but does NOT clear the sidecar's in-flight requests or
  the `pending` VecDeque entries**. A response to the FIRST refresh already
  traveling the pipe will be drained by `drain_paired` (`session.rs:698-709`),
  paired FIFO, and `apply_pair` (`session.rs:572-573`) re-sets
  `pending_playlists = Some(v)` — repopulating the slot the fresh refresh
  tried to clear. `on_tick` (`app.rs:1126-1149`) then merges that **stale**
  playlist list into `yt_lists`, replacing whatever was there.
- **Search** is better: `Pending::Search(q)` carries the query
  (`session.rs:175`), and `on_tick` only applies a search response if the
  overlay's `submitted` query matches (`app.rs:1186-1188`). A stale search for
  an abandoned query is dropped (tracks still cached). But two searches for the
  *same* query can't be distinguished.
- **Tracks** carries the playlist id (`session.rs:178`), matched in
  `apply_pair` (`session.rs:494-507`), so a stale tracks response lands on the
  right list id — but it can still overwrite a newer fetch of the same list.
- **Premium URL** (`session.rs:223, 565-570`) is a single slot; a stale
  premium resolve for an old track would be dropped by `on_tick`'s "same track"
  guard (`app.rs:1308-1311`).

**Verdict: stale results CAN overwrite newer state**, most dangerously for
`yt_lists` (playlists/suggestions have no id-tagged discard). The single-slot
staging + no-generation-id design means rapid re-refreshes (or a slow network
delivering an old response late) can regress `yt_lists` to stale data. The
`loaded_yt_lists.clear()` at `app.rs:1148` only runs *after* a merge, so it
doesn't prevent the stale merge itself.

### Q4: What does `on_tick` do? Does it block?

**`on_tick`** (`app.rs:1056-1381`), called every loop iteration
(`event.rs:249`):

1. **Auto-respawn dead sidecar** (`app.rs:1059-1104`): `session.is_alive()` →
   `mark_alive`; else `should_respawn()` (≤3 attempts, ≥5s apart,
   `session.rs:331-339`) → respawn preserving browser/pasted auth. Only runs
   if `yt_session.is_some()` — a None session is never auto-respawned (see Q2).
2. **`session.drain_paired()`** (`app.rs:1121`) — non-blocking `try_recv`
   loop, FIFO-pairs responses, applies caches. **Does not block.**
3. **Merge `pending_playlists`/`pending_suggestions`** into `yt_lists`
   (`app.rs:1123-1149`); clear `loaded_yt_lists`.
4. **Merge `pending_tracks`** into the matching `YtList.track_ids`
   (`app.rs:1153-1163`).
5. **Apply `pending_search`** to the open search overlay if the query matches
   (`app.rs:1169-1201`).
6. **Drain `pending_errors`** → clear `searching` for matching query, set
   `yt_error` footer (`app.rs:1217-1261`).
7. **Take `pending_premium_url`** for progressive upgrade (`app.rs:1266`).
8. **Cold-miss swap**: if `pending_play` has a URL now, stage the swap
   (`app.rs:1273-1290`).
9. **Progressive upgrade** (`app.rs:1307-1333`): same track, not near end, not
   already premium → `player.load_at(url, pos)`.
10. **Apply cold-miss swap / give up** (`app.rs:1338-1346`).
11. **Lazy-load focused YT list tracks** (`app.rs:1351-1367`): if empty + not
    loaded + not inflight → `send_get_playlist`.
12. **`preload_next_url`** (`app.rs:1371`) — fire-and-forget premium resolve for
    the next track.
13. **Spinner frame** advance while resolving (`app.rs:1376-1380`).

**Does it block?** `on_tick` itself: **no**. `drain_paired` is non-blocking,
`send_*` are non-blocking stdin writes. **However** user actions reachable
from the same keymap block:
- `S` → `open_discover` → `yt_discover_items` → `session.home_suggestions()`
  (**blocking 3s**, `app.rs:1495`, `session.rs:898-904`).
- Discover-Enter → `play_discover_selection` → `session.get_playlist()`
  (**blocking 4s**, `app.rs:1636-1639`, `session.rs:890-896`).
- Launch `library_playlists()` (**blocking 3s**, `main.rs:148`).

These stall the event loop (no input/render) for up to 4s. `on_tick`'s
non-blocking design is undermined by these synchronous roundtrips in the
discover path.

### Q5: Where are lyrics handled? Is there a lyrics view/panel?

**Nowhere. Lyrics are entirely unimplemented.**

- `grep -i lyric` over `src/` → 0 matches.
- No `Overlay::Lyrics` variant (`app.rs:105-142` lists Search/Help/
  PlaylistPicker/Command/YtAuth/Discover only).
- No lyrics field on `Track` (`catalog.rs:13-33`) or `RemoteTrack`
  (`source/mod.rs:45-60`).
- No `Request::GetLyrics` in the sidecar protocol (`proto.rs:17-35`); `yt.py`
  has no `get_lyrics` call (`yt.py:325-511`).
- No lyrics panel in `player_bar.rs` or `columns.rs` or `overlay.rs`.
- The spec `.opencode/prompt:489-514` requires: non-blocking provider pipeline,
  sidecar `.lrc`/plain-text, synced-line highlighting, stale-result guard,
  truthful unavailable state, no fabricated lyrics. **None of this exists.**
- `todo.md:37` (M3) is pending.

`docs/superpowers/specs/2026-07-08-youtube-integration-design.md:342` notes
"ytmusicapi `get_lyrics` already exists" — the API is available but unused.

### Q6: Where is command-mode history handled? Is it persistent?

**It isn't. There is no command history.**

- `Overlay::Command { input: String }` (`app.rs:128-130`) holds only the
  current input string.
- `handle_overlay_key` for `Overlay::Command` (`input.rs:283-298`): Char →
  push, Backspace → pop, Enter → `execute_command` + `overlay = None`. **No
  history Vec, no Up/Down recall, no persistence.**
- `execute_command` (`input.rs:416-439`) matches `yt auth` / `yt logout` /
  `yt setup` / `yt auth browser <name>`; unknown commands silently no-op
  (`_ => {}`).
- `state.rs` has keys `'layout'`, `'playlists'`, `'focus'` — **no
  `'command_history'`**.
- `.opencode/context.md:49` lists "command history" as a reported user issue.
  `.opencode/prompt` mentions it (line 49) but the todo has no task for it.

Command history is both **not implemented** and **not persisted**.

---

## Appendix: Test Surface (skimmed names)

`tests/`: `app.rs`, `audio_restore.rs`, `catalog.rs`, `cli.rs`, `columns.rs`,
`context.rs`, `config.rs`, `e2e_yt.rs` (41KB), `input.rs`, `layout.rs`,
`mode.rs`, `player.rs`, `player_bar.rs`, `search.rs`, `source_device_rate.rs`,
`source_match.rs`, `state_ext.rs`, `theme.rs`, `transport.rs`, `tui.rs`,
`yt_sidecar.rs`, `translit.rs`. Extensive unit + integration coverage; no
lyrics/command-history tests (features absent). `insta` snapshot tests for
TUI views.

---

# Part II — Consolidated Defect Synthesis

**Sources:** `playback-recon.md` (PB), `yt-recon.md` (YT), `tui-recon.md` (TUI),
`quality-recon.md` (Q), Part I architecture audit (A). Deduplicated by root
cause; cross-references show source report + ID. Severity per mission rubric.

## P0 — Release-blocking (must fix)

| ID | Location | Description | Sources | Slice |
|----|----------|-------------|---------|-------|
| A1 | `main.rs:151` | **Launch probe discards session on any error** — single transient failure strands user in guest mode for whole run; `on_tick` respawn only fires on `Some` (dead) session, never `None` | PB1, YT1 | S1 |
| A2 | `yt.py:329-330` | **`auth_status` lies** — reports `ok=True` on cookie presence not validity; no expiry/revocation detection anywhere; UI claims "connected" with empty results | YT2, YT7 | S1 |
| A3 | `main.rs:117/129`, `app.rs:988/1014/1093` | **False-ready status in 5 places** — "connected" set on spawn before any data fetch, never corrected by fetch outcome | YT4 | S1 |
| A4 | entire `src/` | **Lyrics entirely unimplemented** — no Overlay/View/key/provider/parser; zero `[Ll]yric` hits in source | TUI (D1), JOURNEYS G | S3 |
| A5 | `input.rs:283-298,416-439` | **Command mode minimal** — no history, no persistence, no up/down, no completion, unknown commands silently dropped (`_ => {}`) | TUI-P2-2, TUI-P1-2 | S4 |
| A6 | `input.rs` | **Queue view non-functional** — `enqueue`/`remove`/`clear` exist (`queue.rs:187-197`) but never called from UI; `x`/`:queue clear` unbound | TUI (D3) | S5 |
| A7 | `input.rs:350-352` | **Playlist manipulation non-functional** — `a` opens display-only picker; `d`/`:playlist new`/`:add` unbound | TUI (D4) | S5 |
| A8 | `release.yml:70` | **`scripts/yt/` not bundled in release archive** — YT integration broken for ALL binstall/binary-download users | Q P0-1 | S9 |
| A9 | `.github/workflows/` | **No CI test workflow** — tests exist but never run in CI; broken code can be released | Q P0-2 | S9 |
| A10 | `release.yml:64` | **Release builds but doesn't test** — no `cargo test` step before release build | Q P0-3 | S9 |
| A11 | 42 files | **`cargo fmt --check` fails on 42/42 files** (332 diff hunks) — codebase never formatted | Q P1-8 | S0 |
| A12 | 8 errors | **`cargo clippy -D warnings` fails with 8 errors** (too_many_arguments ×3, collapsible_match ×2, derivable_impls, should_implement_trait, doc_lazy_continuation) | Q P1-9 | S0 |

## P1 — High (functional bugs / significant risk)

| ID | Location | Description | Sources | Slice |
|----|----------|-------------|---------|-------|
| B1 | `session.rs:205` | **`track_cache` UNBOUNDED** — `HashMap` grows for entire session, no eviction; memory growth | PB2 | S10 |
| B2 | `app.rs:1495` | **`home_suggestions` blocks TUI up to 3s** (S/Discover) — synchronous `roundtrip` on hot path | PB3 | S2 |
| B3 | `app.rs:1638` | **`get_playlist` blocks TUI up to 4s** (Discover Enter) — synchronous `roundtrip` on hot path | PB4 | S2 |
| B4 | `app.rs:870` | **CONT=YouTube `get_watch_playlist` blocks TUI up to 4s** on end-of-track — synchronous `roundtrip` on hot path | PB5 | S2 |
| B5 | `audio.rs:265,304-317` | **`set_output_format` blocks ~310ms** (250ms poll + 60ms settle) per rate-changing track load — TUI freeze | PB6 | S2 |
| B6 | `yt.py:345,364` | **No pagination** — `get_library_playlists` defaults 25, `get_playlist` 100; silently truncated | YT3 | S1 |
| B7 | `session.rs:714-720` | **`send_refresh` no inflight guard / no generation ids** — stale refresh overwrites fresh `yt_lists`; `yt_lists_loading` can hang on sidecar death | YT5, PB7 | S1 |
| B8 | `main.rs:231-239` | **CLI search output unescaped** — `println!` with track titles → terminal escape injection (e.g. `\x1b[2J`) | Q P1-2 | S8 |
| B9 | `yt/sidecar.rs:65-66` | **`expect("stdin/stdout piped")`** on sidecar spawn — panics on fd exhaustion instead of degrading to guest | Q P1-3 | S8 |
| B10 | `config.rs:35`, `session.rs:62`, `state.rs:26` | **`/tmp/.config` fallback** world-readable on multi-user/headless systems if `dirs::config_dir()` is None | Q P1-4 | S8 |
| B11 | `player.rs:52` | **Predictable mpv socket path `/tmp/jukebox-mpv.sock`** — symlink/race attack on multi-user systems | Q P1-5 | S8 |
| B12 | `yt.py:50-53,153` | **Temp cookie files not cleaned up** — `NamedTemporaryFile(delete=False)` leaks cookie material to `/tmp` | Q P1-6 | S8 |
| B13 | `README.md:67-69,142-145` | **README inaccurate** — keybindings almost entirely wrong; claims "no cookie file written" but code writes one | Q P1-1/P1-7, TUI-P1-1 | S11 |
| B14 | `footer.rs:22-31` | **`yt_status`/`yt_error` never auto-clear** — stale messages persist indefinitely | TUI-P1-3 | S6 |
| B15 | `event.rs:96` | **`log_to_file` dead code** — no file logging; production debugging impossible | Q P3-3 | S6 |
| B16 | `app.rs:1384-1393` | **Logout doesn't clear stale data** — `yt_lists`/`loaded_yt_lists`/`track_cache`/staging/pending survive logout; in-flight refresh re-populates logged-out data | YT6 | S1 |

## P2 — Medium (correctness / robustness / maintainability)

| ID | Location | Description | Sources | Slice |
|----|----------|-------------|---------|-------|
| C1 | `session.rs:262,388-390` | **`url_cache` cap 2 → `prev` to YT track >2 back = cold miss + re-fetch gap** | PB8 | S10 |
| C2 | `event.rs:251` | **Full redraw every tick (150ms)** even at idle — main idle CPU cost | PB9 | S10 |
| C3 | `app.rs:384` | **Linear `track_by_id` O(n)** on every render/resolve (no track_id index) | PB10 | S10 |
| C4 | `queue.rs:243-281` | **Smart shuffle O(n²×catalog)** on Radio (whole library) — `artist_of` does O(catalog) scan per candidate | PB11 | S10 |
| C5 | `app.rs:105-142` | **No nested overlays** — opening new overlay replaces current silently (user loses typed input) | TUI-P2-1 | S7 |
| C6 | `main.rs:25` | **Missing search index silently degrades** — no user-visible hint | TUI-P2-4 | S7 |
| C7 | `player_bar.rs` | **No loading/buffering indicator** during track load or YT URL resolution (spinner exists but only for resolve) | TUI-P2-5 | S7 |
| C8 | `app.rs` | **No search history** — reopening `/` starts empty | TUI-P2-6 | S7 |
| C9 | `tests/e2e_yt.rs:181,214,257,305,565` | **`std::env::set_var("JK_FAKE_MAP")` in parallel tests** — races under `--test-threads=N` | Q P2-1 | S10 |
| C10 | `yt/proto.rs:111-153` | **`Response::from_line` untested for malformed input** — no test for non-UTF8/truncated/missing `data`/Python traceback to stdout | Q P2-2 | S10 |
| C11 | `state.rs` | **No schema versioning** — `state.db` has no version key; no migration path | Q P2-3 | S8 |
| C12 | `config.rs:118-142` | **Hand-rolled YAML parser** — no edge-case tests (quoted paths, multiline, missing fields) | Q P2-4 | S8 |
| C13 | `app.rs:297-301` | **`ClonedResolver` clones `manual_queue`** per Transport call — heap alloc per call | PB13 | S10 |
| C14 | `main.rs:11-18` | **`jukebox config` doesn't re-run prompt** — README says it does; code just prints path | Q P2-6 | S11 |

## P3 — Low (polish / hardening / minor inaccuracies)

| ID | Location | Description | Sources | Slice |
|----|----------|-------------|---------|-------|
| D1 | `sidecar.rs:24,135-139` | **Sidecar reader thread detached (not joined) on Drop** — brief window thread outlives struct; panic-isolated | PB12 | — |
| D2 | `overlay.rs:200` | **Cursor blink always on** — no option to disable | TUI-P3-1 | S7 |
| D3 | `theme.rs:64-83` | **CJK width approximation** doesn't handle zero-width/combining chars | TUI-P3-2 | S7 |
| D4 | `columns.rs` | **No first-run onboarding hint** in the TUI | TUI-P3-3 | S7 |
| D5 | `overlay.rs:133-136` | **YtAuth paste box** renders multi-line paste as single long line | TUI-P3-4 | S7 |
| D6 | `main.rs:35,201` | **`current_exe().parent().unwrap()`** panics if binary at filesystem root | Q P3-1 | S8 |
| D7 | `sidecar.rs:55` | **Sidecar stderr `Stdio::null()`** — debugging failures difficult | Q P3-2 | S6 |
| D8 | `standardize.sh:59` | **`rm -rf "$OUT"`** risky if `$OUT` is `/` or empty | Q P3-4 | S8 |
| D9 | `Cargo.toml:29` | **lindera embeds ~40MB IPADIC** — slow/RAM-heavy build | Q P3-5 | — |
| D10 | `release.yml:36-38` | **No Windows release target** — macOS + Linux only | Q P3-7 | S9 |
| D11 | `yt/proto.rs:152` | **Raw sidecar line in error** — could contain cookie material if sidecar buggy | Q P3-8 | S8 |
| D12 | `tests/state.rs:337-344` | **`tmp_db()` leaks TempDir** — acknowledged in comment | Q P3-9 | S10 |

## Defect counts by severity

| Severity | Count | Unique root causes |
|----------|-------|--------------------|
| P0 | 12 | A1–A12 |
| P1 | 16 | B1–B16 |
| P2 | 14 | C1–C14 |
| P3 | 12 | D1–D12 |
| **Total** | **54** | |

## Cross-report overlap map

| Root cause | Reports | IDs |
|------------|---------|-----|
| Launch probe discards session | playback, yt | PB1 = YT1 = A1 |
| auth_status lies / false-ready | yt, tui | YT2/YT4 = A2/A3 |
| No pagination | yt | YT3 = B6 |
| No generation ids / stale refresh | playback, yt | PB7 = YT5 = B7 |
| README keybindings wrong | quality, tui | Q-P1-1 = TUI-P1-1 = B13 |
| Logout doesn't clear stale data | yt | YT6 = B16 |
| track_cache unbounded | playback | PB2 = B1 |

## Slice mapping (→ PLAN.md)

| Slice | Defects addressed | Priority |
|-------|-------------------|----------|
| S0 | A11, A12 (fmt + clippy) | gate |
| S1 | A1, A2, A3, B6, B7, B16 (YT reliability) | P0 |
| S2 | B2–B5 (non-blocking discover/radio/audio) | P1 |
| S3 | A4 (lyrics) | P0 |
| S4 | A5 (command history) | P0 |
| S5 | A6, A7 (queue + playlist UI) | P0 |
| S6 | B14, B15, D7 (diagnostics) | P1 |
| S7 | C5–C8, D2–D5 (TUI polish) | P1 |
| S8 | B8–B12, C11, C12, D6, D8, D11 (security) | P1 |
| S9 | A8–A10, D10 (release/CI) | P0 |
| S10 | B1, C1–C4, C9, C10, C13, D12 (perf + tests) | P2 |
| S11 | B13, C14 (docs) | P1 |
| S12 | verification + judges | gate |
