# UI Revamp Issues

## Design constraint: NO ICONS

The UI must NOT use icons (Nerd Font glyphs, Unicode symbols, emoji, or pictographic characters) for any UI element. All information must be conveyed through **text labels** and **layout** alone. This includes:

- No `♫`, `✦`, `◆`, `◎`, `▶`, `■`, `▰`, `▱`, `⠋`, `┏`, `┃`, `┗`, `━` or any other Unicode pictographs
- No Nerd Font PUA glyphs
- No emoji
- No box-drawing characters for borders — use plain ASCII (`+`, `-`, `|`) or CSS-style spacing
- No spinner animations with braille dots (`⠋⠙⠹`) — use text like `[loading]` or `[...]`
- No `▸` or `▶` markers — use `>` or `->` in plain ASCII
- No progress bar blocks (`▰▱`) — use text like `[|||||---]` or percentage text

**Rationale:** Icons are font-dependent, break in ASCII-only terminals, cause width-calculation bugs, and add visual noise without information. Text labels are universal, accessible, and testable. The existing `icons.rs` module and `FontMode` system should be removed entirely.

**Affected files:** `src/tui/view/icons.rs` (delete), `src/tui/view/theme.rs` (remove icon helpers), all view files that call `IconRenderer`, `icons.glyph()`, or use Unicode box-drawing characters.

---

## Issue 1: Too many features hidden behind command mode

**Problem:** New features (Home, Radio, Generator, Publication, Discover) are primarily accessed via command-mode commands (`:home`, `:gen`, `:radio`, `:publish`). The command bar is an alternative input method, not the primary one. Features should be discoverable through visible UI elements — buttons, panels, menu items — with commands as a keyboard shortcut for power users.

**Current state:**
- `:home` — no visible button or panel entry point
- `:gen` — no visible button or panel entry point
- `:radio` — no visible button or panel entry point
- `:publish` — no visible button or panel entry point
- `S` (discover) — keybinding only, not shown anywhere except help

**Expected:** Each feature should have a visible entry point in the UI (e.g., a sidebar, a menu, a panel tab). Commands should be the alternative, not the primary path.

---

## Issue 2: YouTube panel is a copy of the Artists panel — needs complete revamp

**Problem:** The YouTube view (press `4`) uses the same Miller-column layout as the Artists view: a narrow playlist list on the left, a wide track list on the right. This made sense for local browsing (Artist → Album → Track) but is wrong for YouTube, which has:
- Playlists (not albums) with flat track lists (no artist/album hierarchy)
- Mixes and suggestions (not a folder structure)
- Home/discovery sections (multi-section content, not a column drill-down)
- Radio seeds and generated content

**Current layout (YouTube view):**
```
┌Playlists──────────┐┌Tracks──────────────────────────────────────────────────┐
│> Liked Music       ││[Y]   1 あのバンド — Ado                          YT│
│> 好きな音楽        ││[Y]   2 Rock It — Sub Focus                       YT│
│> JPop              ││[Y]   3 more than words — Hitsujibungaku         YT│
│> Nhạc yt           ││[Y]   4 Plastic Love — Friday Night Plans         YT│
│> NightCore         ││[Y]   5 Umbrella — haruno                         YT│
│> ...               ││[Y]   6 Come My Way — Sơn Tùng M-TP               YT│
│                    ││[Y]   7 かぐや ...                                YT│
│                    ││[Y]   8 わたしに花束 — Ado                        YT│
│                    ││[Y]   9 春に舞う — Ado                            YT│
│                    ││[Y]  10 BEEP BEEP — Hoshimatic Project            YT│
│                    ││[Y]  11 アンノウン・マザーグース — Suisei         YT│
│                    ││  ... 40 more rows of wasted vertical space ...      │
│                    ││[Y]  50 向日葵 — Ado                              YT│
│                    ││[Y] ▸55 罪と罰 — Ado                              YT│
└────────────────────┘└──────────────────────────────────────────────────────┘
```

**Problems:**
- The playlist list is cramped (truncated names: "TapL's EDM Music Pla")
- The track list has 50+ rows but each row only uses ~40% of the horizontal space — the rest is empty
- No visual distinction between account playlists, suggested/mood playlists, generated mixes
- No place for Home/discovery sections, radio seeds, or generated content
- The `> ` marker and `[Y]` badge waste space without adding value at this density
- The track metadata (title — artist · album) is crammed into one line and truncated

**Expected:** A YouTube-specific layout that:
- Shows Home sections (Quick Picks, Made for You, Start Radio, Library) as horizontal sections or cards
- Distinguishes playlist types (account ♫, suggested ✦, generated) visually
- Uses the wide horizontal space for richer track metadata (title, artist, album, duration, quality on separate columns or a wider format)
- Has a sidebar or tab system for switching between Home, Library, Search, Discover within the YouTube view
- Doesn't force YouTube content into a Miller-column metaphor designed for local file browsing

---

## Issue 3: Player bar / status line needs a better spot and modes

**Problem:** The player bar is a thin 2-row strip at the bottom of the screen. It crams title, artist, album, quality, transport controls, volume, progress bar, and mode flags into a single line. At 100 columns this causes:
- Truncation of track titles (especially Japanese/long names)
- Mode flags abbreviated (`MODE yout` instead of `MODE youtube`)
- Transport controls overlapping with the "Next:" indicator
- No room for album art, lyrics preview, or richer metadata

**Current (2-row compact):**
```
[PLAYING] ▶ Song Title — Artist  0:00 / 3:00  ▸ Next: (end)  24bit/96  MODE local · CONT off
▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱ 0% --:-- / --:--          SHUF off · RPT off · CONT off · MODE youtube
```

**Expected:** A dedicated rectangle area for the player bar with two modes:

### Mini mode (current, for small terminals or when browsing)
Thin 2-row strip at the bottom — same as now but fixed.

### Big mode (for normal/wide terminals)
A larger rectangle (e.g., 8-10 rows) on the right side of the screen or bottom that shows:
```
┌Now Playing──────────────────────────────────────────────┐
│  ▶  罪と罰 - Crime And Punishment                       │
│     Ado · Ado's Utattemita Album                        │
│     24bit / 96kHz · AAC 256k · YT Premium               │
│                                                         │
│  ▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱▱▱  2:15 / 3:42                       │
│                                                         │
│  ◀◀  ⏮  ⏯  ⏭  ▶▶     🔊 ▰▰▰▰▱ 70%                      │
│                                                         │
│  SHUF off · RPT off · CONT yt · MODE youtube            │
│  Next: 向日葵 — Ado                                     │
└─────────────────────────────────────────────────────────┘
```

The big mode uses the wasted right-side space in the track list (currently empty). Toggle between mini/big with a key (e.g., `P` or a panel toggle).

---

## Issue 4: Track list has massive wasted horizontal space

**Problem:** The track list (right column) uses the full width of the screen but each track row only fills ~40% of it. The remaining 60% is empty space. This is especially bad in the YouTube view where track names are long Japanese titles.

**Current (100-wide terminal):**
```
┃[Y]   1 あのバンド — Ado                                                                   YT┃
┃[Y]   2 Rock It — Sub Focus                                                                YT┃
┃[Y]   3 more than words — Hitsujibungaku 12 hugs (like butterflies)                        YT┃
┃[Y]   7 かぐや (cv. 夏吉ゆうこ) & 月見ヤチヨ - ワールドイズマイン CPK! Remix / THE FIRST TAKE  YT┃
```

The `YT` badge sits far right, but everything between the track metadata and the badge is empty.

**Expected:** Use the horizontal space for:
- A multi-column layout: `# | Title | Artist | Album | Duration | Quality | Source`
- OR a card/tile layout (2-3 tracks per row) for wide terminals
- OR move the player bar / now-playing info into that right-side space (big mode, Issue 3)
- OR show additional metadata (duration, play count, last played) in the empty space

---

## Issue 5: Panel modularity — packable rectangle layout system

**Problem:** The current layout is hardcoded: Miller columns (3 fixed panels) or narrow (1 panel) with a fixed player bar at the bottom and a fixed footer. There's no way to rearrange, add, or remove panels.

**Expected:** A modular panel system where:
- The screen is divided into rectangular panel areas
- Each panel shows a specific feature: Track List, Now Playing, Queue, Lyrics, Mixes, Radio, Search Results, Home Sections, Diagnostics
- Panels can be arranged in a grid (rows/columns) or a split layout
- An **edit mode** lets the user select which panels to show and where to place them
- The layout is saved to `state.db` and restored on next launch

**Example layouts:**
```
┌Tracks────────────┐┌Now Playing──────────┐
│ track list        ││  big player bar      │
│                   ││  lyrics preview      │
│                   ││  next-up queue       │
└───────────────────┘└──────────────────────┘
┌Home Sections──────┐┌Tracks───────────────┐
│ Quick Picks       ││  track list          │
│ Made for You      ││                      │
│ Start Radio       ││                      │
│ Library           ││                      │
└───────────────────┘└──────────────────────┘
```

**Implementation notes:**
- Panels are rectangles with a type (TrackList, NowPlaying, Queue, Lyrics, Mixes, Radio, Home, Search, Diagnostics)
- Edit mode: press a key (e.g., `E`) to enter layout editing — arrow keys move between panel slots, `Tab` cycles panel types, `+`/`-` resizes
- Default layouts for small (80×24), medium (100×30), and large (120×40+) terminals
- The Miller-column layout becomes one of the default presets, not the only option

---

## Issue 6: No visual distinction between YouTube playlist types

**Problem:** In the YouTube view, all playlists look the same — account playlists, suggested/mood playlists, and generated mixes all appear as `> Name` in the left column. The `> ` marker and truncated names give no indication of what type each playlist is.

**Expected:**
- Account playlists: `♫ Liked Music`, `♫ JPop` (music note icon)
- Suggested/mood playlists: `✦ Japan Ballads`, `✦ moment` (star icon)
- Generated mixes: `◆ Daily Mix 1`, `◆ Discover Mix` (diamond icon)
- Radio seeds: `◎ Artist Radio` (bullseye icon)
- Each type should be visually grouped (sections in the list) or color-coded
- The count of tracks in each playlist should be shown

---

## Issue 7: No persistent visual feedback for async operations

**Problem:** When the app performs an async operation (loading mixes, resolving a stream URL, fetching playlist tracks), there's no persistent visual indicator. The status bar shows a brief message that disappears after 5 seconds (TTL), and the spinner is only in the player bar.

**Examples:**
- Press Enter on a YouTube mix → "Loading mix…" appears for 5s then vanishes, but the fetch may take longer
- Press `:radio` → radio overlay opens but the pool may still be filling
- Play a YouTube track → player bar shows a spinner but no indication of what's loading (track name, resolve tier)

**Expected:**
- A persistent loading state that stays visible until the operation completes or fails
- Show WHAT is loading (track name, mix name) not just that something is loading
- Show the operation type (resolving stream, fetching tracks, loading mix)
- Loading state should be in the relevant panel, not just the status bar

---

## Issue 8: Tab system for YouTube view instead of Miller columns

**Problem:** The YouTube view uses Miller columns (playlist list → track list) which is a local-browsing metaphor. YouTube content is better organized as tabs or sections.

**Expected:** A tab bar at the top of the YouTube view:
```
┌──────────────────────────────────────────────────────────────┐
│ [Home] [Library] [Search] [Discover] [Radio]                  │
├──────────────────────────────────────────────────────────────┤
│  (content of the active tab)                                 │
│                                                              │
│  Home: Quick Picks, Made for You, Start Radio, Explore       │
│  Library: account playlists + liked songs                    │
│  Search: YouTube search results                              │
│  Discover: mood/suggestion mixes                             │
│  Radio: active radio session                                  │
└──────────────────────────────────────────────────────────────┘
```

Tabs are navigable with `Tab`/`Shift+Tab` or number keys. Each tab shows its own content layout. This replaces the Miller-column layout for YouTube while keeping it for the local Artists view.

---

## Issue 9: Footer hint bar is cramped and not context-aware

**Problem:** The footer hint bar shows a fixed set of keybindings: `Enter play · q quit · ? help · 1-4 view · > < next prev · M mode · / search`. This is the same regardless of what the user is doing. It doesn't show context-relevant hints (e.g., when a track is playing, show `Space pause · L lyrics · e enqueue`).

**Expected:** Context-aware hints that change based on:
- Current view (Artists, Playlists, Queue, YouTube)
- Whether a track is playing
- Whether an overlay is open
- Whether YouTube is connected

---

## Issue 10: Wasted vertical space in narrow playlists column

**Problem:** The playlist list (left column in YouTube view) is ~20 characters wide but only shows ~10 playlists. The remaining rows are empty. The column is too narrow for playlist names and too tall for the number of items.

**Expected:**
- Wider column (or variable width) so names aren't truncated
- Show track count and type icon
- Group by type (account, suggested, generated)
- Use the vertical space for more metadata or collapse the column when not focused
