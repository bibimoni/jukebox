# YouTube Integration — Design Spec

**Date:** 2026-07-08
**App:** jukebox v0.2.0 — Rust TUI (ratatui 0.30) + mpv playback + CoreAudio rate switching
**Goal:** Make jukebox a seamless replacement for the YouTube Music desktop experience while preserving its filtered-lossless local library — local, YouTube, and mixed playback in one app, with app-parity browse/autoplay and hi-res local tracks in the same queue.

---

## 1. Goals & non-goals

**Goals**
- Three source modes — **Local**, **YouTube**, **Mixed** — cycled by `M`, persisted across sessions. Mode switches never stop playback.
- **Mixed mode** plays a track from the local library when a robust match exists, else streams from YouTube. YT fills gaps; local is preferred.
- **YouTube mode** provides account playlists, suggested/mood playlists, search, and uses YouTube's own autoplay/radio as the CONT (continue) engine.
- Streaming is **as smooth as YouTube Music web for a Premium account**: ad-free, max-bitrate (prefer AAC 256k), gapless handoff, no mid-stream stutter.
- Audio is always the **maximum-quality stream** YouTube offers; the app **knows the stream format in advance** so CoreAudio can re-clock the device from a hi-res local rate down to the stream's rate.
- **CoreAudio re-clock happens once** when a YT session begins, is **held** for consecutive YT tracks (mid-stream re-clock stutters), and is **restored** when a local track resumes.
- **Seamless, intuitive UI** with balanced element placement and consistent keybindings across all modes — every transport key behaves identically whether the playing track is local or YouTube.
- **In-app auth**: a shortcut opens a cookie-paste box; one paste derives auth for both `ytmusicapi` (metadata/radio) and `yt-dlp` (streaming). No leaving the app.
- **Completion gate**: a fresh, isolated judge agent scores the finished app against a rubric and must approve every point at **max** before the app is concluded; anything below max triggers a redo loop.

**Non-goals**
- No upload/management of the user's YouTube library beyond read (browse + play).
- No video. Audio-only playback (mpv `--no-video`, unchanged).
- No transcoding of local files; local stays bit-perfect.
- No Rust-native YouTube client in v1 (a Python sidecar is used; `rustypipe`/`rusty_ytdl` are noted as a future migration path but not in scope).
- No offline caching of streams.

---

## 2. Architecture & component model

**The guiding idea:** generalize the playback seam so a track is either a local file or a remote stream, **without touching `Transport`**. `Transport` keeps shuffling/repeating/historizing a list of ids; it never knows whether an id is local or YouTube. The mode + a resolver decide what an id *means* at load time.

### New components

```
jukebox (Rust TUI)
├── App                      (existing)  owns transport, views, modes, overlays
├── Transport                (existing)  UNCHANGED — permutes ids, shuffle/repeat/prev
├── Player trait / mpv       (existing)  load() now accepts a URL too (mpv loadfile = http)
├── source::SourceResolver   NEW         id → Local track (catalog) | Remote track (cache→sidecar)
├── source::RemoteTrack       NEW         { video_id, title, artist, album?, dur, fmt: {codec,abr,sr} }
├── source::TrackSource       NEW enum    Local { track_id } | Remote { video_id }
├── source::MatchLocal        NEW         ISRC → normalized artist+title fuzzy matcher (local-first)
├── yt::Sidecar               NEW         long-lived stdin/stdout JSON RPC → python process
├── yt::Session               NEW         auth (cookies), rate-limit state, remote track cache
├── yt::RadioCursor           NEW         drives get_watch_playlist continuation for CONT=YouTube
├── SourceMode                NEW enum    Local | Youtube | Mixed   (M cycles; persisted)
└── DeviceRateState           NEW         tracks CoreAudio "in YT rate" flag (switch-once-per-session)
```

### The Python sidecar

A single `scripts/yt/yt.py` (+ `scripts/yt/requirements.txt`: `ytmusicapi`, `yt-dlp`), invoked as a **long-lived child process** speaking line-delimited JSON over stdin/stdout. One process for the app's lifetime (no per-call spawn latency). Commands:

| Command | Returns | Purpose |
|---|---|---|
| `search(q, limit)` | `[{video_id, title, artist, album?, dur}]` | YT track search (scoped `/` in Y view) |
| `library_playlists()` | `[{id, name, count}]` | Account playlists for Y view col1 |
| `get_playlist(id)` | `[{video_id, title, artist, album?, dur}]` | Tracks of a playlist / mood list |
| `home_suggestions()` | `[{id, name, kind}]` | Suggested/mood playlists (✦ in Y view, `S` overlay) |
| `get_watch_playlist(video_id)` | `[{video_id, title, artist, ...}]` | Autoplay/radio queue (CONT=YouTube) |
| `resolve_url(video_id)` | `{url, expires_at, codec, abr, sample_rate, container}` | Stream URL + format (Premium-aware) |
| `ping` / `auth_status` | `{ok, premium, account}` | Liveness + auth state |

Each returns a small JSON object; the Rust side **never parses YouTube's internal format**. Auth cookies are passed to the sidecar at startup (and written to a `cookies.txt` the sidecar passes to yt-dlp via `--cookies-file`). The sidecar reuses one `ytmusicapi.YTMusic` instance and one yt-dlp extractor config across calls.

### Why this shape

The UX-critical path — shuffle, repeat, prev, history, the queue, browse cursors — is all driven by `Transport`, which is unchanged and already well-tested. YT content plugs in at two seams only: (a) *what's in the browse lists* (the `Y` view fetches from the sidecar instead of the catalog), and (b) *how an id resolves to audio* (`SourceResolver`). There is no second transport, no parallel queue logic, no forked keybinding path. So "keyboard shortcuts behave as expected" is mostly inherited: every transport key works identically whether the playing track is local or YouTube.

### Mixed mode

Is just the resolver's policy: given an id, prefer the local catalog match (ISRC-strong, then normalized artist+title fuzzy) → if a hit, play local; if no hit, treat the id as a `videoId` and stream. The `Y` view lists and the local Artists/Playlists lists both feed the same `Transport`.

### `now_playing` widening

`now_playing` widens from `Option<String>` (catalog id) to `Option<TrackSource>`, so the player bar can render either `24-bit / 96 kHz · bit-perfect` or `Opus 160k · YT` / `AAC 256k · YT Premium`. The `track_by_id` lookups in the view layer route through the same resolver so YT rows render titles/artists instead of raw videoIds.

---

## 3. Playback pipeline & smooth-stream guarantees

Five mechanisms. Invariant: a transport hiccup on the YouTube side must never freeze or crash local playback, and vice-versa.

### 3.1 Resolve-lazy, cache-then-preload

A `RemoteTrack` carries metadata but no URL. `resolve_url(video_id)` returns `{url, expires_at, codec, abr, sample_rate, container}`. URLs are resolved at most ~30s before load — never at enqueue time for a whole queue (they expire in hours). A small in-memory URL cache holds the **current + next** track's resolved URL; the next one is preloaded so handoff is instant; older entries evict. Resolving happens off the UI thread (the sidecar subprocess is inherently async w.r.t. the event loop; the TUI reads results on the next tick, never blocking).

### 3.2 Premium stream selection

With Premium cookies the sidecar asks yt-dlp for the highest-bitrate **audio-only** stream, preference order: AAC 256k (ad-free, Premium manifest) → Opus 160k → best available. Account cookies are passed so Premium ad-free manifests are selected and account rate limits (~2000 videos/hr) apply, not the ~300/hr guest cap. The sidecar runs a single-format extractor with `--no-playlist` so resolve is fast (~1–2s). The chosen format's `{codec, abr, sample_rate}` is reported back so the app knows the format **in advance** of loading.

### 3.3 CoreAudio re-clock: switch once per streaming session

`App` tracks the device's current rate via `DeviceRateState { current_sr, current_bd, in_yt_rate }`. Rules:
- Loading a **local** track → switch to that track's `sample_rate_hz`/`bit_depth` (today's behavior), set `in_yt_rate = false`.
- Loading a **remote** track while `in_yt_rate == false` → switch **once** to the stream's `sample_rate` (usually 44.1 or 48 kHz), set `in_yt_rate = true`.
- Loading subsequent **remote** tracks while `in_yt_rate == true` → **do not touch CoreAudio** (no mid-stream re-clock stutter; consecutive YT tracks at the same rate stay there).
- A local track resumes → switch to its rate, `in_yt_rate = false`.

So coming off a 192 kHz hi-res track into YouTube, the device drops to 48 kHz exactly once, holds for the whole YT listening session, and re-clocks back to hi-res the moment a local track plays. No flicker, no stutter. (If consecutive YT tracks differ in rate — rare, AAC 256k and Opus 160k are both 48 kHz/44.1 kHz — the first-different one is allowed to re-clock once, then hold again; the rule is "re-clock only on a rate *change* into YT, not per track.")

### 3.4 Gapless handoff

mpv already runs `--gapless-audio=yes`. For remote tracks we additionally pre-resolve the next track's URL (3.1) and, for the autoplay/CONT=YouTube case, fetch the next `get_watch_playlist` track ahead of time so when the current track's `end-file` fires, `App::next` hands mpv a URL it already has — no 1–2s resolve gap between songs.

### 3.5 Failure modes (all handled, never silent)

- **URL expired** (played a cached track hours later) → resolve on-demand at load; if still expired → mark remote track `dead` (reuse the existing local dead-track path), `App::next` skips it, never halts playback.
- **Network drop mid-stream** → mpv buffers; if it stalls > a few seconds, surface a dim `YT ⏪ buffering…` state in the player bar (not a freeze) and let the user skip.
- **Rate limit / PO-token 403** → sidecar returns a typed error; the app shows `YT rate-limited — pausing 30s` in the player bar, retries resolve, never crashes the loop.
- **Dead remote track** (removed/private) → `resolve_url` returns `unavailable` → treated exactly like a missing local file (added to `dead`, skipped, advance continues).
- **yt-dlp/ytmusicapi not installed** → the `Y` view shows a one-time setup hint (`:yt setup`) instead of an empty screen; local mode is fully functional regardless.
- **Sidecar process dies** → Rust respawns it once (best-effort) and re-auths; if it can't, degrade the `Y` view to an error state but keep local playback alive.

---

## 4. Mixed-mode matching & data flow

### 4.1 The matcher (local-first, robust to string differences)

Pure function `match_local(remote: &RemoteTrack, cat: &Catalog) -> Option<TrackId>`, tiered:

1. **ISRC (strong).** `Track.isrc` is already in the catalog. YouTube exposes ISRC for officially-sourced music (`VideoType::OFFICIAL_SOURCE_MUSIC`/`ATV`). Exact ISRC match → local copy wins. Deterministic, no false positives.
2. **Normalized artist+title fuzzy (fallback).** When ISRC is absent (UGC uploads, user music videos), normalize both sides through the existing `translit` module (romaji/kana/width/normalization already used for CJK titles) and lowercase/strip punctuation/drop feat. tokens, then compare `primary_artist + title` with a similarity threshold (Levenshtein ratio ≥ ~0.88). Borderline matches (0.80–0.88) are **not** auto-promoted — they stream from YT, because a wrong local substitution (playing the wrong song) is exactly the "weird behavior" the judge flags.
3. **No match → stream.** Conservative default.

Dedicated test module covers: exact ISRC, ISRC-case-insensitivity, normalized CJK title match, feat-token strip, borderline-rejection, no-match-fallback.

### 4.2 Mode semantics

| Mode | Browse surfaces | Mixed matcher used? | CONT=Radio uses |
|---|---|---|---|
| **Local** | A/P/Q (library only) | no — everything local | local library shuffle |
| **YouTube** | Y (account PLs, suggested, search) | no — everything streams | YouTube autoplay (`get_watch_playlist`) |
| **Mixed** | A/P/Q **and** Y, unified queue | **yes** — local-first | YouTube autoplay (when CONT=YouTube) |

In Mixed mode both local lists and YT lists feed one `Transport`; the resolver decides local-vs-stream per id at load. `M` cycles Local→YouTube→Mixed→Local; never stops playback.

### 4.3 Data flow — a YouTube play, end to end

```
user presses Enter on a Y-view track row
  → App::play_selected (unchanged signature)
  → context = Context::Search{ track_ids: [videoIds...] }  (YT rows are ids too)
  → transport.switch_context  (UNCHANGED — shuffles/permutates the ids)
  → start_playback
     → SourceResolver::resolve(id, mode)
        ├─ Mode=Local, or id in catalog & matcher hit (Mixed) → Local {path}
        └─ else → Sidecar::resolve_url(videoId) → {url, fmt}
     → if remote: ensure YT rate set once (§3.3); mpv loadfile(url)
     → now_playing = TrackSource::Remote{video_id}; fmt cached
  → player bar renders "Opus 160k · YT" (or "AAC 256k · YT Premium")

track ends (mpv end-file)
  → App::next  (UNCHANGED)
  → transport.next returns next id (or manual queue, or repeat)
  → if CONT=YouTube & context exhausted → RadioCursor::advance (pre-resolved §3.4)
  → resolve + load — next URL already pre-resolved → gapless
```

---

## 5. TUI redesign

### 5.1 Clutter audit of the current layout (specific cuts)

1. **Player-bar info line overloads 6 signals onto one row** (now-playing + transport glyphs + quality + volume + SHUF + RPT + CONT) and wraps below ~100 cols. → Split into two rows; drop decorative `◀◀ ⏸ ▶▶` transport glyphs (the `⏸/▶` play glyph already shows state).
2. **No footer hint bar** — every binding invisible until `?`. → Add a 1-line footer with the 5–6 most-used keys always visible.
3. **Rail's `/` is a dead glyph** implying a search view that doesn't exist. → Rail becomes `A/P/Q/Y`.
4. **Mode flags run together with no rhythm.** → `·`-separated, right-anchored, `MODE` last.
5. **No narrow fallback** — the app refuses to render below 80×24, unusable in a 60-col tmux split. → Add a 60–80 col single-pane drill-down; keep too-small message below ~60×20.

### 5.2 Default layout — Local mode, Artists view (≥120 cols)

```
┌─┐┌────────┬─────────┬──────────────────────────────┐
│A││ Artists│ Albums  │ Tracks                       │
│P││ Adele  │ 25      │  1 Hello              24bit-96│
│Q││ Aimer  │ Day by..│  2 Send My Love       24bit-96│
│Y││ Ado    │         │ ▶3 Water Under...     24bit-96│
└─┘└────────┴─────────┴──────────────────────────────┘
⏸ Hello — Adele · 25          24-bit / 96 kHz · bit-perfect   vol ▰▰▰▱ 70%
[━━━━━━━━━━━━━━━━━━━━━━━━] 1:23 / 4:12     SHUF off · RPT off · CONT off · MODE local
Enter play · Space pause · >/< next · M mode · / search · ? help · q quit
```

### 5.3 Y view — YouTube browse (Mixed mode)

```
┌─┐┌────────────────────┬───────────────────────────────┐
│A││ YouTube            │ Tracks                        │
│P││ ♫ Liked Songs      │  1 Hello              Opus 160k│
│Q││ ♫ My Road Trip     │ ▶2 Rolling Stone      AAC 256k │
│Y││ ♫ Chill Mix        │  3 ...               Opus 160k │
│ ││ ✦ Suggested        │                               │
│ │├────────────────────┴───────────────────────────────┤
│ ││ Suggested / Up Next                                │
│ ││ ✦ Focus Flow   ✦ Late Night   ▶ Like this →       │
│ │└────────────────────────────────────────────────────┘
└─┘
⏸ Rolling Stone — Adele       AAC 256k · YT Premium     vol ▰▰▰▱ 70%
[━━━━━━━━━━━━━━━━━━] 0:48 / 3:14    SHUF off · RPT off · CONT youtube · MODE mixed
Enter play · Space pause · >/< next · M mode · / search · ? help · q quit
```

- `Y` view reuses the **2-column Playlists shape** (list → tracks) for muscle-memory consistency with `P`.
- `♫` = account playlists, `✦` = suggested/mood playlists (`home_suggestions`).
- Because playlist track lists are typically short (<10), the space **below the track list** holds a **"Suggested / Up Next" pane**: related mood playlists and (in Mixed) the local smart-album suggestions. Dead space becomes a discovery surface. (In `P` view the same pane shows local smart-album suggestions; `S` jumps to it.)

### 5.4 Filter-on-focused-column — `f`

An **inline filter** bound to `f` that narrows whatever column has focus, via a 1-line filter prompt at the top of the column (not a modal), live-narrowing, `Esc`/`Enter` clears:

```
Artists (filter: ade▏)
┌─────────────────────────────────┐
│ Adele                           │
│ Aimer                           │
└─────────────────────────────────┘
```

- Works on the Artists column, the `P`/`Y` list column, etc. — "jump to this artist / find this playlist" without leaving the column.
- `f` is distinct from `/` track search; `/` is scoped to the active view's track pool.

### 5.5 Random start + suggested album — `s` and `S`

- **`s` — instant random track ("surprise me").** Picks a random track from the active source (local catalog in Local; YT in YouTube; both pools in Mixed) and plays it **in context** (its album/playlist becomes the context, so `>`/`<` and CONT behave coherently — not a one-shot orphan).
- **`S` — suggested album / discover overlay.** A small popup listing 3–5 suggestions to start from.
  - **Local smart-album heuristic** (good-enough, not YouTube-grade): score each album by `recency_decay(last_played)` (favors un-played-lately) `+ artist_diversity` (spreads across artists) `+ small_random`; weighted-random pick of 5; `S` picks one and starts from track 1 with CONT=NextAlbum.
  - In **YouTube/Mixed** mode the same `S` overlay shows YT mood/suggested playlists instead (`home_suggestions`/`get_mood_playlists`), so "pick a vibe and go" is one key in every mode.

### 5.6 Narrow layout — 60–80 cols (tmux split)

Miller collapses to one focused pane; `h`/`l` drills in/out. Chrome compresses to a 1-line player bar (info+flags share a row, gauge shrinks) + short footer.

```
┌─┐┌──────────────────────────────────┐
│A││ Albums · Adele         ← Artists │
│Y││ 25                               │
│ ││ Day by Day                       │
└─┘└──────────────────────────────────┘
⏸ Hello — Adele · 25    24bit/96 · vol 70%
[━━━━━━━━━━] 1:23/4:12  CONT off · MODE local
> next · M mode · / search · ? help
```

Below ~60×20: the existing "terminal too small" message.

### 5.7 Auth overlay — `:yt auth` (and `:yt logout`)

In-app cookie paste; no leaving the app. One paste derives `ytmusicapi` browser auth **and** `yt-dlp`'s `cookies.txt`.

```
┌─ YouTube auth ────────────────────────────────────┐
│ Paste your YouTube cookies (Premium recommended). │
│ Export from a logged-in youtube.com tab with a    │
│ "Get cookies.txt" browser extension, then paste:  │
│                                                   │
│ > ▏                                               │
│                                                   │
│ Enter: save & connect    ·    Esc: cancel         │
└───────────────────────────────────────────────────┘
```

### 5.8 Keybindings — consistency rules

- `1`/`2`/`3`/`4` jump to A/P/Q/Y (today `1`/`2`/`3` switch views; add `4` for Y). `Tab` cycles.
- Transport keys (`Enter`, `Space`, `>`/`<`, `,`/`.`, `+`/`-`, `m`, `z`/`Z`, `r`, `c`) work **identically** whether the playing track is local or YouTube (`Transport` is the same object).
- `M` cycles Local→YouTube→Mixed→Local; never stops playback.
- `/` track search scope follows the active view; the overlay form is identical everywhere.
- `f` filters the focused column (inline); `s` instant random; `S` discover overlay.
- `:yt auth` / `:yt logout` / `:yt setup` via the command overlay; `?` help updated to list YT keys.

### 5.9 Seamless-transition rendering invariants

- **Redraw on events only, never a fixed timer** (already true).
- **No full-screen clear between frames** — ratatui's diff render; the `Y`-view fetch and mode switch update fields *in place* (dim `loading…` row, never a cleared screen).
- **Overlay open/close clears only the popup region** (existing `overlay.rs` pattern; auth/filter/discover follow it).
- **Player-bar transitions** — when local yields to YT, now-playing + quality swap in one frame; no blank, no stale `24-bit`. The `in_yt_rate` CoreAudio switch (3.3) happens *before* the new title renders, so the rate never visibly disagrees with the quality label.

---

## 6. License & NOTICE

jukebox is MIT (Copyright (c) 2026 bibimoni). The YouTube integration adds runtime dependencies that are **not** linked into the binary (they're separate tools invoked as subprocesses), so the MIT license of jukebox is unaffected, but a NOTICE is required:

- Add a **NOTICE** file (or a `## Third-party runtime tools` section in README) documenting:
  - `ytmusicapi` (MIT) — used via the `scripts/yt/` sidecar.
  - `yt-dlp` (Unlicense) — used as a subprocess for stream URL extraction.
- Add a **YouTube ToS disclaimer** to README: automated access to YouTube may violate YouTube's Terms of Service; this integration is intended for personal use with content the user has the right to access (e.g. their Premium account). No warranty; user assumes responsibility.
- `scripts/yt/requirements.txt` pins `ytmusicapi` and `yt-dlp` versions; the README's "Runtime prerequisites" gains `python3`, `ytmusicapi`, `yt-dlp` (and notes `:yt setup` will install the Python deps if missing).

---

## 7. Testing

- **Unit:** `MatchLocal` (ISRC, normalized CJK, feat-strip, borderline-reject, no-match); `DeviceRateState` (switch-once-per-session transitions, restore-on-local); `SourceMode` cycling; sidecar JSON (de)serialization round-trips against fixture responses; `RadioCursor` advance.
- **Snapshot (pinned 80×24 and 120×30, NO_COLOR and color):** Local player bar, YT player bar, Y view with Up-Next pane, narrow fallback, auth overlay, discover overlay, filter prompt.
- **Integration (sidecar stubbed):** a fake sidecar returning canned JSON exercises search→play, mixed match→local, mixed no-match→stream, CONT=YouTube advance, dead-remote skip, sidecar-down degrade.
- **End-to-end (judge):** the real app + a real (or recorded) YouTube session, driven live — see §8.

---

## 8. Completion gate — the strict judge

The implementation is **not concluded** until a **fresh, isolated judge agent** (no shared context, given only the rubric + a running app, driven live via the `run`/`verify` skills — not just code reading) scores each dimension and **approves every point at max**. If any point is below max: diagnose → fix → re-judge, until all-max or the user intervenes.

Rubric (each 0–2: **max** = meets the bar, **mid** = works but rough, **zero** = broken/weird):

| # | Dimension | "Max" bar |
|---|---|---|
| 1 | Keybinding consistency | Every transport/mode key behaves identically across Local/YouTube/Mixed; `f`/`s`/`S`/`/` work in every applicable view; no dead keys; `q`/`Esc` consistent |
| 2 | Search coherence | `/` track search + `f` list filter return sensible results; YT and local search forms identical; empty/typo queries degrade gracefully (no "doesn't make sense" results) |
| 3 | Streaming smoothness | Gapless local↔YT↔local handoff; no >~0.5s gap between consecutive YT tracks; no mid-stream stutter; buffering/rate-limit surfaced, never a freeze |
| 4 | Layout balance | Player bar two rows balanced; footer hints present; `Y` view consistent with `P`; narrow (60–80) fallback renders; no crash ≤60×20 |
| 5 | Seamless transitions | No flicker/blank-flash on mode/view switch, overlay open/close, local→YT handoff; quality readout never disagrees with device rate |
| 6 | Edge cases | Dead/expired YT track skipped (no halt); sidecar-down keeps local alive; auth-not-set shows setup hint, not empty screen; no panics |
| 7 | Premium parity | Premium cookies → AAC 256k ad-free streams selected; quality readout reflects actual stream, not a hardcoded label |
| 8 | CoreAudio cadence | Re-clock happens exactly once entering a YT session, held through consecutive YT tracks, restored on return to local — never mid-stream |

---

## 9. Out-of-scope / future

- Rust-native YouTube (`rustypipe` + `rusty_ytdl`) to drop the Python runtime — noted, not v1.
- Lyrics (ytmusicapi `get_lyrics` already exists).
- YouTube library write operations (create/edit playlists).
- Stream caching / offline.
