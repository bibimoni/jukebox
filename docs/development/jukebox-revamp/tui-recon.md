# TUI Product & Interaction Reconnaissance Report

**Date:** 2026-07-12  
**Scope:** Read-only audit of TUI interaction patterns, keybinding design, overlay/modal handling, responsive layout, accessibility, state feedback, and UX gaps.  
**Repo:** `/Users/distiled/Dev/jukebox` @ v0.3.0

---

## 1. Keymap Design

### Keymap summary (source of truth: `overlay.rs:267-300` Help overlay)

| Category | Key | Action |
|----------|-----|--------|
| Navigation | `h j k l` / arrows | Move (← columns, ↑↓ within) |
| | `gg` / `G` | Top / bottom of column |
| | `1 2 3 4` | Switch view: Artists / Playlists / Queue / YouTube |
| | `Tab` / `Shift+Tab` | Cycle view forward / backward |
| Playback | `Enter` | Play selected in context |
| | `Space` | Play / pause |
| | `>` / `<` | Next / previous track |
| | `,` / `.` | Seek −5s / +5s |
| | `+` / `-` | Volume up / down |
| | `m` | Mute |
| | `z` / `Z` | Cycle shuffle / reshuffle |
| | `r` | Cycle repeat (off → all → one) |
| | `c` | Cycle continue (mode-dependent) |
| | `M` | Cycle source mode (Local / YouTube / Mixed) |
| Discover | `s` | Instant random track |
| | `S` | Discover overlay |
| | `f` | Filter focused column; Enter on filter jumps to match |
| Modes | `/` | Search (scoped to view) |
| | `?` | Help |
| | `:` | Command |
| | `a` | Add to playlist |
| | `:yt auth` / `:yt auth browser <name>` | YT cookie paste / browser auth |
| | `:yt logout` / `:yt setup` | Clear cookies / install deps |
| | `Esc` | Close overlay / cancel |
| | `q` | Quit |
| Mouse | click row | Focus + select |
| | dbl-click track | Play |
| | drag divider | Resize |
| | click progress/volume | Seek/set |
| | wheel | Scroll focused column |

### Reserved keys (never bound)
`Ctrl+C`, `Ctrl+Z`, `Ctrl+\`, `Ctrl+S`, `Ctrl+Q` (`input.rs:29-34`). Correct — preserves terminal signal handling under raw mode.

### Assessment
- **Vim-style navigation is well-implemented** with `hjkl` + arrow keys + `gg`/`G` leader key.
- **The keymap is dense** — almost every letter is bound. Potential for key collisions when adding new features (lyrics, command history).
- **No key conflicts found** in current implementation — overlay routing takes precedence, filter routing second, then base keymap.

---

## 2. Overlay / Modal Handling

### Overlay types (`app.rs:105-142`)
| Overlay | Purpose | Open | Close |
|---------|---------|------|------|
| `Search` | Search overlay (Local BM25 or YouTube) | `/` | `Esc` |
| `Help` | Full keymap reference | `?` | `Esc` |
| `PlaylistPicker` | Add-to-playlist picker | `a` | `Esc` |
| `Command` | `:` command input | `:` | `Esc` |
| `YtAuth` | YouTube cookie paste box | `:yt auth` | `Esc` / `Enter` (save) |
| `Discover` | Suggested albums/playlists | `S` | `Esc` |

### Routing (`input.rs:38-48`)
1. If an overlay is open → `handle_overlay_key` (full key capture)
2. If inline filter is active → `handle_filter_key` (typing narrows, Esc/Enter clear)
3. Otherwise → base keymap

### Issues
- **No nested overlays.** Opening a new overlay while one is open replaces it silently — the user could lose typed search input by accidentally pressing `?`. No confirmation.
- **`Overlay::Command` has no history** — the `input` field is a single `String`, no up/down traversal. The mission TODO (M4) calls for command history.
- **`Overlay::YtAuth` accepts a multi-line paste** but the input is a single `String` — no line rendering. A pasted cookies.txt with many lines would render as one long line. No visual feedback of paste success.
- **No overlay stacking** — `Esc` always closes the top overlay. If a command opens another overlay (e.g. `:yt auth` → `YtAuth`), the command overlay is replaced, not stacked.

---

## 3. Search Experience

### Local search (`SearchScope::Local`)
- **Instant / live-as-you-type** — `overlay.rs:208-211` shows "Tab → youtube · Enter plays selection" (no searching indicator).
- BM25 ranking via Tantivy with Lindera Japanese tokenization + kana↔romaji transliteration.
- **Cross-script matching works** — `burubado` → `ブルーバード`, typos tolerated (fuzzy edit-distance).

### YouTube search (`SearchScope::Youtube`)
- **Explicit-submit only** — typing never sends; `Enter` fires one async request (`app.rs:submit_yt_search`).
- **"searching…" indicator** shown while in flight (`overlay.rs:213-214`).
- **Scope toggle** with `Tab` — user can search local catalog from Y view or YouTube from a local view.
- **Concurrency handling is robust** — query tags in `Pending::Search(q)` prevent result misattribution (tested in `e2e_yt.rs:642-713`).

### Issues
- **No search history** — previous queries are not remembered. Reopening `/` starts empty.
- **No incremental YouTube search** — the explicit-submit design is intentional (avoids 3s/char stalls) but feels slow compared to local. Could offer optional debounced auto-search.
- **Empty results state** is handled (`overlay.rs:220-222`: "no results — edit the query or Tab → local").

---

## 4. Responsive Layout

### Breakpoints (`layout.rs:30-34`)
| Size | Behavior |
|------|----------|
| `< 60 cols` or `< 20 rows` | "terminal too small" message, no browse chrome |
| `60–80 cols` or `20–24 rows` | Narrow: single focused pane + compressed 1-row player bar + short footer |
| `≥ 80 cols` and `≥ 24 rows` | Full Miller columns + 2-row player bar + 1-row footer |

### Narrow mode (`layout.rs:91-109`)
- Single focused pane via `columns::render_narrow`.
- Compressed 1-row player bar via `player_bar::render_compact`.
- Rail (view letters) still visible.
- `h`/`l` drills in/out of the focused column.

### Assessment
- **Good responsive design** — the 3-tier breakpoint handles tmux splits (60×20) through full terminals (160×50).
- **Snapshot tests cover all 4 sizes** (`tests/snapshots/layout__*.snap`).
- **Column widths are persisted** (`state.rs:170-203` `LayoutWidths`) and restored on launch.
- **Divider drag** resizes columns (mouse).

---

## 5. Accessibility

### NO_COLOR support (`theme.rs:6-8`)
- `NO_COLOR` env var (no-color.org) collapses all colors to `Color::Reset`.
- **Colors are not the only signal** — selection uses both color (accent) and a `▶` glyph for now-playing.
- Quality tags use color (Magenta=Hi-Res, Green=CD) but also text labels ("24bit-48kHz").

### Issues
- **No screen reader support** — TUI is inherently visual; no aria-style announcements.
- **No high-contrast mode** beyond NO_COLOR.
- **No keyboard shortcut to reduce motion** — the cursor blink (`Modifier::SLOW_BLINK` at `overlay.rs:200`) is always on.
- **CJK display width** is approximated (`theme.rs:64-83`) — counts CJK as 2 columns. This is correct for most terminals but doesn't handle zero-width characters or combining marks.

---

## 6. State Feedback

### Player bar (`player_bar.rs`)
- Shows: play/pause glyph, now-playing title/artist, quality label, volume bar, time/duration, transport flags (SHUF/RPT/CONT/MODE).
- **Progress bar** is clickable (seek) and drag-able.
- **Volume bar** is clickable (set) and drag-able.

### Footer (`footer.rs`)
- Shows 6 most-used key hints by default.
- **Transient YT status/error** overrides the hint line when set (`yt_error` → yellow, `yt_status` → accent).
- **No timeout** — `yt_status`/`yt_error` persist until overwritten. A stale "connected via chrome" stays forever until another action clears it.

### Issues
- **No loading state for local playback** — when a track is loading (mpv spawn), there's no "loading…" indicator. The player bar just shows the track info once it starts.
- **No buffering indicator for YouTube** — the "searching…" indicator exists for search, but stream URL resolution has no visible feedback during the ~1.3s fast resolve.
- **yt_status never auto-clears** — a success message ("YT auth: connected") stays in the footer indefinitely until another YT action overwrites it.
- **No toast/notification system** — feedback is either the footer (transient, no timeout) or the YT error (persistent until cleared).

---

## 7. Empty / Loading / Error States

| State | Where | Handling |
|-------|-------|----------|
| Empty catalog | Artists view | Shows empty artist list; no "no music found — run jukebox sync" hint |
| Empty queue | Queue view | Shows empty list; no "press Enter to play" hint |
| No YT session | YouTube view | Shows "setup hint" (`columns.rs` test `youtube_view_shows_setup_hint_when_no_session`) |
| YT loading | YouTube view | `yt_lists_loading` flag → "loading…" text |
| YT error | Footer | `yt_error` shown in yellow |
| Search no results | Search overlay | "no results — edit the query or Tab → local" |
| Terminal too small | Full screen | "terminal too small — resize or press q to quit" |

### Missing empty states
- **No "first run" guidance in the TUI** — after `jukebox sync`, the user enters the TUI with a full catalog but no hint about what to do next (press `?` for help is in the footer, but no onboarding).
- **No "sync needed" hint** — if the search index is missing, `main.rs:25` silently treats it as "no search" (`search::Searcher::open(...).ok()`). The user might not realize search is unavailable.

---

## 8. YouTube View Interaction

### List navigation
- `4` or `Tab` switches to Y view.
- Col 0: account playlists `♫` + suggested/mood `✦`.
- Col 1: tracks of the focused list (lazy-loaded on focus).
- `Enter` plays the focused track.
- `Space` enqueues.

### Issues
- **No pagination** — `ytmusicapi.get_library_playlists()` returns a page; the sidecar fetches one page only. Large libraries show a truncated list with no "load more" affordance.
- **No search-within-list** — once a list's tracks are loaded, there's no way to filter them.
- **No track count display** — the `YtList.count` field exists but isn't shown in the UI (only the name is rendered).
- **Focused list auto-loads on tick** — `on_tick` fire-and-forget sends `get_playlist` for the focused list. If the user scrolls fast, this could queue multiple fetches (mitigated by `playlist_inflight` guard).

---

## 9. Filter (`f` key)

- Opens an inline filter on the focused column.
- Typing narrows the list (case-insensitive substring match).
- `Esc` or `Enter` clears the filter.
- `Enter` on a filter match jumps to that item.

### Issues
- **Filter only works on Artists/Albums/Tracks columns** — no filter in Playlists, Queue, or YouTube views.
- **No filter indicator** — when a filter is active, there's no visual cue (like "(filter: rock▏)" shown in `columns.rs:62` — actually this IS shown). OK, there IS an indicator.
- **No clear-filter key besides Esc/Enter** — `Backspace` on empty filter doesn't close it.

---

## 10. Command Mode (`:`)

### Supported commands (`input.rs:416-438`)
- `:yt auth` — open cookie paste overlay
- `:yt auth browser <name>` — browser auth
- `:yt logout` — clear cookies
- `:yt setup` — install deps

### Issues
- **No command feedback for unknown commands** — `input.rs:437` `_ => {}` silently ignores unknown commands. No "unknown command" error.
- **No tab completion** — typing `:yt ` doesn't suggest `auth`/`logout`/`setup`.
- **No command history** — up/down doesn't recall previous commands. (M4 in the TODO.)
- **No visible cursor in the command line** — the command input has no block cursor (unlike the search overlay which has `▏`).

---

## 11. Now-Playing & Context

- **Now-playing track** is marked with `▶` glyph in the track list (`columns.rs:427`).
- **Context** is shown in the column header (breadcrumb): "Tracks · {album} ← Albums · {artist}" (`columns.rs:138`).
- **Now-playing panel** in the player bar shows title, artist, quality.

### Issues
- **No album art** — TUI limitation, expected.
- **No lyrics display** — not implemented (M3 in the TODO).
- **No queue preview** — the Queue view shows the manual queue, not the upcoming context tracks. The user can't see "what's playing next" within the current context.

---

## 12. Defect List

### P1 — High

| ID | Location | Description |
|----|----------|-------------|
| TUI-P1-1 | `README.md:142-145` vs `overlay.rs:267-300` | README keybindings are almost entirely wrong (documented in quality-recon.md §12) |
| TUI-P1-2 | `input.rs:437` | Unknown `:` commands silently ignored — no "unknown command" feedback |
| TUI-P1-3 | `footer.rs:22-31` | `yt_status`/`yt_error` never auto-clear — stale messages persist indefinitely |

### P2 — Medium

| ID | Location | Description |
|----|----------|-------------|
| TUI-P2-1 | `app.rs:105-142` | No nested overlays — opening a new overlay replaces the current one silently (user could lose typed input) |
| TUI-P2-2 | `input.rs:416-438` | No command history, no tab completion, no visible cursor in command line |
| TUI-P2-3 | `columns.rs` | No track count or pagination in YouTube list view |
| TUI-P2-4 | `main.rs:25` | Missing search index silently degrades to "no search" with no user-visible hint |
| TUI-P2-5 | `player_bar.rs` | No loading/buffering indicator during track load or YouTube URL resolution |
| TUI-P2-6 | `app.rs` | No search history — reopening `/` starts empty |

### P3 — Low

| ID | Location | Description |
|----|----------|-------------|
| TUI-P3-1 | `overlay.rs:200` | Cursor blink (`SLOW_BLINK`) always on — no option to disable |
| TUI-P3-2 | `theme.rs:64-83` | CJK display width approximation doesn't handle zero-width or combining chars |
| TUI-P3-3 | `columns.rs` | No "first run" onboarding hint in the TUI |
| TUI-P3-4 | `overlay.rs:133-136` | `YtAuth` paste box renders multi-line paste as a single long line |

---

*End of report.*
