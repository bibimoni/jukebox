# jukebox TUI Revamp — Design

**Date:** 2026-07-06
**Status:** Approved (pending spec review)
**Scope:** A complete UI/UX revamp of the `jukebox` TUI from a three-pane manual-enqueue model to a Spotify-like hi-fi listening app: Miller-column browsing, context play, persistent player bar, repeat/shuffle/transport, saved playlists, overlay search, and mouse-resizable panels. Offline, in-process, single binary — unchanged from the original architecture.

## Goals

1. **Context play, not manual enqueue.** Selecting a track plays it and sets its source list (album, artist's tracks, playlist, or search results) as the play context — "up next" is the rest of that list in order or shuffled. The user never has to manually build a queue to get continuous playback.
2. **Feel like a normal music app** (Spotify / Apple Music): persistent now-playing bar, reliable previous/next, repeat modes, volume, mouse-resizable panels.
3. **Hi-fi first.** Sample rate + bit depth are first-class, prominently displayed and verified bit-perfect when sample-rate switching is on.
4. **Clean, discoverable keymap.** Vim conventions + the convergent standards from lazygit/yazi/helix; progressive disclosure (footer → `?` → `:`). No footguns (no single-key destructive clears, no `n`/`p` collision).
5. **Responsive.** A defined degradation path from wide (≥120 cols) down to 80×24, with a "terminal too small" floor below that.
6. **Robust terminal hygiene.** Alt screen, panic-safe restore, SIGWINCH debounced resize, SIGTSTP suspend, file-based logging (no `eprintln!` behind the alt screen).

## Non-Goals

- Changing the bash indexer (`scripts/standardize.sh`) or the catalog format. The indexer and `catalog.json` are immutable inputs.
- Network features, streaming, accounts, lyrics, recommendations. Offline only.
- Re-encoding or modifying audio. Tags are read-only.
- A server. Everything runs in-process in one binary.
- Album art rendering (image protocols are fragmented; out of scope for this revamp).

## Decisions (locked during brainstorming)

| Decision | Choice | Rationale |
|---|---|---|
| Screen layout | Miller columns (Artist → Album → Tracks) + persistent player bar | Best for browsing a large library (311 artists, 1594 tracks); shows structure. |
| Play model | Context play (Spotify model) | Core complaint: "auto discover queue instead of manual enqueuing." |
| Transport | Repeat modes (off/all/one) + shuffle toggle, **consume OFF** | Consume OFF makes "previous" reliable — played tracks stay in the context. |
| Smart shuffle | Artist-spaced (no back-to-back same artist), Spotify-style | Avoids the repetitiveness of pure random with 311 artists. |
| Playlists | In scope — sidebar entry, persisted in state DB | "Work like Spotify." |
| Search | Overlay popup (fzf-style) — `/`, plays in context, Esc closes | Doesn't disturb browse position; "summon, choose, return." |
| Sample-rate switching | Default ON (current behavior) | Audiophile-correct for a lossless library; bit-perfect output. |
| Mouse panel resizing | Drag dividers; widths persist | Lets the user weight screen real estate to taste. |
| Seek vs. navigate conflict | Arrows + `hjkl` navigate only; `,`/`.` seek, `<`/`>` skip track | Resolves the Left/Right double-duty. |

## Architecture

The existing bash indexer + `catalog.json` + Tantivy index are untouched. The Rust binary's TUI layer is restructured from one 470-line `tui/mod.rs` into focused modules. The player backends (`src/player.rs`: `MpvPlayer`, `AfplayPlayer`, `StubPlayer` via the `Player` trait) are reused as-is, with the trait gaining two small additions (volume + repeat-one on the player side is unnecessary; repeat-one is handled by reloading the same path on track-end).

### The "play context" abstraction (core idea)

Today there are three independent concepts — artist browse, search results, queue — that fight each other. The revamp unifies them: **anything you can pick a track from is a `Context`** — a `Vec<TrackId>` plus a cursor. Browsing an artist → that artist's albums; an album → album context; a playlist → playlist context; the search overlay → results context. Selecting a track plays it and sets that track's column-list as the active play context. The "queue" becomes the **playback engine** over the active context, not a separate manually-fed list.

This is what removes manual enqueuing: you pick a song, it plays, and the rest of the list is up-next automatically.

### Module layout

```
src/tui/
  app.rs        — App struct: all state (active context, transport, playlists, overlay, view state); pure update methods, no I/O
  event.rs      — terminal event loop (poll / draw / handle) + resize/suspend/error hygiene
  input.rs      — key + mouse dispatch → app methods; modal handling (overlay, command mode)
  context.rs    — Context abstraction (list + cursor) and context sources (artist/album/playlist/search)
  queue.rs      — queue + transport engine: next/prev, repeat modes, artist-spaced + random shuffle, history for "previous"
  view/
    layout.rs   — Miller-column layout + player bar + responsive breakpoints
    columns.rs   — Artist / Album / Track column rendering
    player_bar.rs — persistent bottom bar (title, transport, progress, volume, mode flags, quality)
    overlay.rs  — search popup + playlist picker/create + help (?)
    theme.rs    — semantic color tokens, NO_COLOR/monochrome handling, CJK width helpers
```

`src/state.rs` is extended (same SQLite DB, same `state` table key/value schema) to persist: focused view, column widths, volume, last-used shuffle/repeat mode, and playlists (see Data Model).

## Data Model

### Context

A `Context` is the source list for playback:

```rust
enum Context {
    Album { album: String, artist: String, track_ids: Vec<String> },
    Artist { artist: String, track_ids: Vec<String> }, // all of an artist's tracks, title-sorted
    Playlist { name: String },
    Search { query: String, track_ids: Vec<String> },
    Queue, // the manual queue (user-appended tracks), plays in queue order
}
```

The active context owns "up next." When the user picks a track, that track's column list becomes the active context and playback starts at the picked track's position in it.

### Transport / Queue engine

The `Queue` (renamed conceptually to "transport engine") holds:

- `context: Context` — the active source list.
- `order: Vec<usize>` — a permutation over the context's track ids (identity by default; shuffled when shuffle is on).
- `cursor: usize` — position in `order`.
- `history: Vec<(String, Context)>` — a session-wide stack of (track_id, context) pairs for reliable "previous." Pushed on every successful play. Consume is off, so the previous track is also still in its context — but the history stack is what makes `prev` work across a context switch (see below).
- `shuffle: ShuffleMode` — `Off | Smart | Random`.
- `repeat: RepeatMode` — `Off | All | One`.
- `queue: Vec<String>` — optional user-appended track ids ("add to queue"), played after the context's natural order ends.

**Next/Prev semantics:**
- `next()` advances `cursor` in `order`. At the end: repeat-all wraps to the start; repeat-one replays the current track (reload same path); repeat-off → if the manual `queue` is non-empty, play its next track; else stop. (Queued tracks are appended *after* the context, never interleaved.)
- `prev()` pops `history`: it replays the last-played track **and restores its context**, even if that track came from a different context than the current one (e.g. you switched albums mid-playback). If history is empty, prev replays the current track from its start (standard music-app behavior at the first track).
- Smart shuffle: a permutation that avoids two consecutive ids sharing a `primary_artist`, spacing the same artist across the list. Falls back to random if an artist holds more tracks than fit (so shuffle still completes). Computed once per context; reshuffle (`Z`) recomputes with a fresh seed.

### Playlists (persistence)

Stored in the existing `state.db` (SQLite), reusing the `state(key, value)` table with JSON values:

| key | value |
|---|---|
| `focus` | focused view (existing) |
| `column_widths` | `{ sidebar, col1, col2, col3 }` persisted divider ratios |
| `volume` | `0..=100` |
| `shuffle_mode`, `repeat_mode` | last-used modes |
| `playlists` | JSON array of `{ name: String, track_ids: Vec<String> }` |

Playlist mutation (create/rename/add/remove/delete) writes the `playlists` row. Track ids reference `catalog.tracks[].id` (stable, content-hashed — already the case).

## UI / Layout

### Miller columns

Left → right: **Artists → Albums → Tracks**. `h`/`l` (or Left/Right) move between columns; `j`/`k` (or Up/Down) move within. The Tracks column rows: `#  Title  Album  Quality  Duration`, currently-playing marked `▶` and highlighted.

Column widths default to roughly 25% / 25% / 50% of the main area (to the right of the view-switcher rail). Drag dividers to resize; persist via `column_widths`.

### View-switcher rail (far left)

A fixed-width (~3 col) icon rail on the far left that switches *what the columns show*. Not resizable, and collapses away below ~60 cols (replaced by `1`–`4` keys + `:`):

- `1` Artists — Miller columns (Artist → Album → Tracks). Default view.
- `2` Playlists — a single Tracks column showing the selected playlist's tracks. (A playlist picker opens first if none is selected.)
- `3` Queue — a single Tracks column showing the manual queue.
- `4` Search — opens the search overlay (same as `/`).

The rail is the *view switcher*, not a duplicate of the Artists column — so there's no redundancy with the Miller layout. Search is also reachable via `/` from anywhere; the rail entry just focuses the overlay.

### Player bar (full width, bottom)

```
▶ Kaguya→Luna — 40mP · Cosmic Princess Kaguya   ◀◀  ⏸  ▶▶   ━━━━●━━━━ 1:42/4:12   24-bit / 96 kHz · bit-perfect   vol ▰▰▰▱ 64%   🔀 smart  🔁 all
```

- Now-playing: title — artist · album.
- Transport glyphs: ◀◀ previous · ⏸/▶ play-pause · ▶▶ next. Clickable with the mouse.
- Progress: bar with elapsed/total, click-to-seek (mouse).
- Quality readout: `24-bit / 96 kHz`; when sample-rate switching is active, `· bit-perfect` confirms the device is at the track's native rate.
- Volume: `▰▰▰▱ 64%`; click/drag to set.
- Mode flags: shuffle mode (`off`/`smart`/`random`) and repeat mode (`off`/`all`/`one`).

### Search overlay

`/` opens a popup floating over the columns. Type → results narrow live (Tantivy, existing searcher). Enter on a result plays it in a Search context (the rest of the results are up-next). Esc closes, restoring the exact browse position.

### Responsive breakpoints

| Width | Behavior |
|---|---|
| ≥120 cols | Icon rail + full 3-column Miller + player bar. |
| ~80–120 | Rail stays; columns compress; quality column narrows to `24/96`. |
| ~60–80 | Rail collapses (use `1`–`4` keys); columns compress to 2 (Albums + Tracks) with `[Artist ▸ Album]` breadcrumbs to navigate up. |
| <80×24 | "terminal too small — resize or press q to quit" message; no render. |

## Keymap

Conventions: vim + lazygit/yazi/helix. Arrows alias `hjkl` and navigate only (no seek on arrows). Global unless noted.

### Global navigation
- `h j k l` / arrows — move (←→ between columns, ↑↓ within)
- `l`/Right — descend (Artist → Albums, Album → Tracks)
- `h`/Left — ascend back
- `gg` / `G` — top / bottom of current column
- `1` / `2` / `3` / `4` — switch view: Artists / Playlists / Queue / Search (matches the rail icons)
- `Tab` / `Shift+Tab` — cycle view forward / backward

### Playback
- `Enter` — play selected track in context (descend if on a non-track)
- `Space` — play/pause
- `>` / `<` — next / previous track (walks context; prev reliable, consume off)
- `,` / `.` — seek −5s / +5s
- `+` / `-` — volume up/down · `m` — mute
- `z` — cycle shuffle (off → smart → random) · `Z` — reshuffle now (fresh seed)
- `r` — cycle repeat (off → all → one)

### Modes / overlays
- `/` — open search overlay · `Esc` closes
- `n` / `N` — next / prev search match (overlay open)
- `a` — add selected track to a playlist (opens picker)
- `d` — remove from playlist (playlist view)
- `x` — remove from queue (queue view)
- `?` — help overlay (full keymap)
- `:` — command mode (`:playlist new <name>`, `:add`, `:queue clear`, `:goto <artist>`)
- `Esc` — close overlay / cancel / back
- `q` — quit

### Mouse
- Click row — focus + select
- Double-click track — play in context
- Drag column divider — resize (persist); double-click divider — reset to default ratio
- Click player-bar transport — prev / play-pause / next
- Click/drag progress bar — seek
- Click/drag volume — set
- Wheel — scroll focused column

### Keys reserved for the terminal (never bound)
`Ctrl+C` (SIGINT), `Ctrl+Z` (SIGTSTP), `Ctrl+\` (SIGQUIT), `Ctrl+S`/`Ctrl+Q` (XON/XOFF).

## Terminal hygiene (non-negotiables)

1. **Alternate screen** for the full-screen TUI; scrollback unpolluted.
2. **Panic-safe terminal restore.** Install a panic hook + `Drop` guard that disable raw mode, leave the alt screen, restore the cursor, and stop the player before printing any trace. (The existing `run()` cleanup is extended with a panic hook.)
3. **SIGWINCH resize**, debounced; re-layout on every resize; minimum 80×24 with a "too small" message.
4. **SIGTSTP suspend**: on `Ctrl+Z`, disable raw mode, leave alt screen, restore cursor, `kill(0, SIGTSTP)`; on `SIGCONT`, re-enter and force full redraw.
5. **No blocking I/O on the UI thread.** Player IPC reads are already non-blocking (mpv socket is `set_nonblocking(true)`); this is preserved.
6. **Logging to a file**, not `eprintln!`. Dead-track skips and sample-rate-switch failures log to `~/.cache/jukebox/jukebox.log` (or the platform cache dir). Today's `eprintln!` behind the alt screen is invisible or corrupts the UI — removed.
7. **Sample-rate switch crash safety.** Because switching is on by default and mutates the system audio device, the panic/exit path restores the default device format. The `App::Drop` / shutdown path runs `audio::restore_output_format()` (new helper) so a crash can't strand the device at a weird rate.
8. **CJK / wide-char width** via the existing `disp_width` helper (now in `view/theme.rs`), so Japanese titles align in tables. (Reaffirmed, not new.)

## Error Handling

- **Dead tracks** (missing source / broken symlink): filtered out of browse lists, not shown inline as `[dead]` clutter. If a dead track is encountered at play time (e.g. it died after the catalog was built), skip to next, log it, and mark it dead for the session so it's not retried. Same `play_current_queue` iteration cap (`queue.len()`) so an all-dead context can't recurse forever.
- **mpv socket unavailable** → afplay fallback (existing behavior, preserved).
- **Search index missing** → search overlay shows "index not found, run `jukebox sync`"; browse/playback still work.
- **Playlist persistence failure** → log + a non-blocking toast in the player bar ("playlist save failed").
- **Resize below floor** → render the too-small message; do not crash.

## Testing

Three layers, bottom-heavy (matches the TUI testing skill):

1. **Unit-test the update layer as pure functions.** The `app.rs` update methods (next/prev under each repeat×shuffle combination, smart-shuffle artist-spacing, context switching, playlist add/remove) are pure functions of state — feed a synthetic key/event, assert on state. This is the cheapest, highest-value layer and catches "previous stopped working" regressions directly.
2. **Golden/snapshot the rendered frame** at a pinned terminal size + monochrome color profile, using `ratatui::backend::TestBackend` + `insta`. Pin size and profile (the #1 cause of flaky snapshot tests). Cover: empty library, browse view, playing with repeat-one, search overlay open, too-small terminal.
3. **PTY end-to-end smoke** — one or two flows (launch, pick a track, verify play) — sparingly.

Existing tests (`tests/tui.rs`, `tests/player.rs`, etc.) are updated to the new module layout; the `Player` trait and backends retain their tests.

## Out of scope for this spec (future work)

- Album art / embedded image rendering.
- Lyrics.
- Mood/genre-based recommendation (no genre tags in source).
- Crossfade / gapless beyond mpv's existing `--gapless-audio=yes`.
- Remapping keybindings via config (the keymap is code-defined for this revamp).
