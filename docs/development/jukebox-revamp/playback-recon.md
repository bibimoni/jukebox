# Playback, Concurrency & Performance Recon — Jukebox

**Specialist:** Playback, Concurrency & Performance (recon task S0.3.4, picked up by the TUI specialist to unblock M1)
**Date:** 2026-07-12
**Scope:** Playback state transitions, local/remote source resolution, queue/context semantics, prev/next, shuffle/repeat, seek/pause/resume/volume/mute, track-end events, provider fallback, buffering, cancellation, background tasks, UI-thread blocking, process lifecycle, cleanup, stale events, races, resource usage, performance baselines.
**Method:** Read every line of `src/player.rs` (478), `src/audio.rs` (449), `src/source/mod.rs` (87), `src/source/match_local.rs` (150), `src/source/device_rate.rs` (59), `src/yt/session.rs` (1038), `src/yt/sidecar.rs` (146), `src/yt/proto.rs` (163), `src/main.rs` (264), `src/config.rs` (191), `src/state.rs` (371), plus the TUI layer already read for `tui-recon.md` (`app.rs` 1819, `event.rs` 281, `queue.rs` 282, `input.rs` 780). Read all playback-relevant tests: `transport.rs`, `player.rs`, `source_match.rs`, `source_device_rate.rs`, `audio_restore.rs`, `yt_sidecar.rs`, `state_ext.rs`, `tui.rs`, `app.rs`.

---

## 1. Architecture Map

### Playback pipeline

```
User action (key/mouse/track-end)
  → input.rs::handle_key / event.rs::on_track_ended
    → app.rs::play_selected / next / prev / on_track_ended
      → queue.rs::Transport (cursor advance, history, shuffle/repeat)
        → app.rs::start_playback / load_track
          → app.rs::resolve_source (Local / Remote / Pending / None)
            → player.rs::Player::load / load_at  (mpv socket IPC or afplay spawn)
              → audio.rs::set_output_format (CoreAudio re-clock, macOS only)
```

### Three player backends (`src/player.rs`)

| Backend | Seek | Position | Duration | Volume | Mute | track_ended | How |
|---|---|---|---|---|---|---|---|
| `MpvPlayer` | ✓ relative+absolute | ✓ (observe_property) | ✓ | ✓ | ✓ | ✓ (end-file eof) | Unix socket IPC, non-blocking reads |
| `AfplayPlayer` | ✗ no-op | ✗ None | ✗ None | ✗ no-op | ✗ no-op | ✓ (child exited) | `afplay` subprocess, SIGSTOP/SIGCONT for pause |
| `StubPlayer` | ✓ | ✓ (fake) | ✓ (180s) | ✗ | ✗ | ✗ | In-memory, tests only |

`launch()` (`player.rs:470-477`): tries `MpvPlayer::spawn`; if spawn or socket-connect fails, falls back to `AfplayPlayer`. No user notification of the fallback — the user doesn't know they're on afplay until seek/progress/volume silently fail.

### YouTube sidecar pipeline (`src/yt/`)

```
App → Session → Sidecar (python subprocess) → ytmusicapi / yt-dlp
       ↑                                       
       ├── fire-and-forget: send_resolve, send_resolve_premium, send_search, send_get_playlist, send_refresh
       ├── sync roundtrip: resolve_url, search, get_playlist, home_suggestions, library_playlists, auth_status, ping
       └── on_tick drain_paired → pending_playlists/suggestions/tracks/search/premium_url/errors
```

**Non-blocking design:** The sidecar (`sidecar.rs`) spawns a reader thread that blocks on the child's stdout and pushes newline-delimited JSON lines into an `mpsc` channel. `try_recv()` drains the channel without blocking. `drain_paired()` (session.rs:714-725) is called from `on_tick` every 150ms and is non-blocking. Fire-and-forget sends (`send_*`) push to a FIFO `pending` deque and return immediately.

**Blocking design:** `roundtrip()` (session.rs:665-708) sends a request and **spin-waits** with `std::thread::sleep(10ms)` in a loop until the matching response arrives or the deadline expires (2-15s depending on the call). This blocks the calling thread — which is the TUI event loop thread for all sync calls.

### Source resolution (`src/source/`, `app.rs:638-684`)

`resolve_source(id)` policy:
- **Local mode:** catalog track → local file (if file exists); unknown id → None (dead).
- **YouTube mode:** catalog track present but NOT played locally — only streamed. Sidecar `resolve_url` → stream URL + fmt. Cold miss → `Resolved::Pending` (fire-and-forget both fast + premium tiers).
- **Mixed mode:** catalog track present → local; else remote stream.

`match_local` (`source/match_local.rs`): ISRC exact match (case-insensitive) → fuzzy title+artist (Levenshtein, TITLE_GATE=0.88, ARTIST_FLOOR=0.80, with kana/romaji translit variants). Conservative — borderline [0.80, 0.88) rejected.

### Device rate cadence (`src/source/device_rate.rs`)

`desired_switch(state, kind, switch_sample_rate)` → `Option<(sr, bd)>`:
- Local: switch to track's rate+depth; skip if same.
- Remote: switch once when YT session begins; **hold** across consecutive same-rate tracks (no mid-stream re-clock); re-clock if rate changes.
- Restored when local hi-res resumes.

`set_output_format` (`audio.rs:238-286`): `AudioObjectSetPropertyData` (sync) → `verify_format_landed` (poll up to 250ms) → 60ms settle sleep. **Blocks the calling thread for up to ~310ms** on a format change.

---

## 2. Playback State Transitions

### States a track can be in

| State | Field | Meaning |
|---|---|---|
| Playing (local) | `now_playing = Local{id}`, `player.is_playing()` | local file loaded in mpv/afplay |
| Playing (remote) | `now_playing = Remote{vid}`, `player.is_playing()` | YouTube stream loaded |
| Pending (cold miss) | `pending_play = Some(vid)`, old track still playing | URL not cached yet; on_tick will swap |
| Resolving | `is_resolving() = true`, spinner active | fast or premium resolve in flight |
| Dead | `dead.insert(id)` | file missing / unresolvable; skipped |
| Stopped | `now_playing = None`, `player.stop()` | context exhausted, CONT=Off |

### Transition diagram

```
play_selected ──→ start_playback ──→ resolve_source ──→ Local: player.load + now_playing=Local
                                     ├─→ Remote: load_remote + now_playing=Remote
                                     ├─→ Pending: pending_play=vid (old keeps playing)
                                     └─→ None: dead.insert + next

next ──→ transport.next ──→ load_track ──→ (same resolve chain)
      └─→ context exhausted ──→ CONT: Off(stop) / NextAlbum / Radio / YouTube(radio.advance)

prev ──→ transport.prev ──→ load_track

on_track_ended ──→ next

on_tick ──→ pending_play swap (URL landed)
         ──→ premium_swap (upgrade to 256k)
         ──→ preload_next_url (fire-and-forget premium resolve)
```

### Tests covering transitions
- `tests/transport.rs`: next walks context, prev walks history, repeat all/one/off, smart shuffle artist-spacing, manual queue after context, prev after manual queue.
- `tests/app.rs`: play_selected sets context, dead track skipped, cycle shuffle/repeat, volume clamps, prev across context switch, collaboration album, clamp_cursors, NextAlbum auto-advance, Radio keep playing, cycle_continue mode-aware, Mixed plays local, instant_random, discover.
- `tests/tui.rs`: play_in_context_ids + next/prev round-trip, on_track_ended auto-advance, all-dead termination.

---

## 3. Defect List

Format: **ID — severity — title** · file:line · reproduction · affected journey · acceptance criterion.

### P0

- **PB1 — P0 — `player.load()` errors discarded; now_playing diverges from actual playback.** `src/tui/app.rs:598-599` (`let _ = self.player.load(&path); self.now_playing = Some(...)`), `742-743`, `777-779`. Repro: make `player.load()` fail (mpv socket gone, file unreadable mid-session, afplay spawn fail) → `now_playing` is set to the track but nothing plays; the player bar shows a track that isn't playing. Affected: brief invariant "No status bar may show one track while the audio backend plays another" (`.opencode/prompt:660`); Journey A "normal playback and recovery from errors." AC: `now_playing` is only set when `player.load()` returns `Ok`; on `Err`, keep the prior `now_playing` or set a clear error state; surface the failure in the footer.

### P1

- **PB2 — P1 — Blocking `library_playlists()` at startup blocks TUI entry for up to 3s.** `src/main.rs:158-173` calls `s.library_playlists()` — a synchronous `roundtrip` with a 3s deadline (`session.rs:910-916`). Repro: launch with a saved YT browser profile and a slow network → the app takes up to 3s to appear (no TUI, no input). Affected: Journey B "first login" / "returning launches"; "Startup" responsiveness. AC: startup is non-blocking; the YT reachability probe is fire-and-forget or deferred to `on_tick`; the TUI renders immediately.

- **PB3 — P1 — Blocking `get_playlist()` in `play_discover_selection` blocks UI for up to 4s.** `src/tui/app.rs:1634-1640` calls `session.get_playlist(&id)` synchronously — `roundtrip` with a 4s deadline (`session.rs:918-927`). Repro: press `S` (discover), pick a YT playlist with `Enter` → UI freezes for up to 4s while fetching the playlist from YouTube. Affected: Journey G (discover → play). AC: discover selection triggers a fire-and-forget fetch; the UI shows a loading state; tracks play once the fetch lands in `on_tick`.

- **PB4 — P1 — Blocking `get_watch_playlist()` in `radio.advance` blocks UI for up to 4s on CONT=YouTube track end.** `src/tui/app.rs:870` calls `self.radio.advance(session, seed_id)` → `session.get_watch_playlist` — `roundtrip` with 4s deadline (`session.rs:943-954`). Repro: set `c` to CONT=YouTube, play a YT track, let it end naturally (or press `>`) → UI freezes for up to 4s while fetching the next radio track. Affected: Journey "end of context" / CONT=YouTube. AC: radio advance is fire-and-forget; the UI stays responsive; the next track starts when the response lands.

- **PB5 — P1 — `:yt setup` blocks the TUI for ~30s with no rendering or input.** `src/tui/app.rs:1020-1049` calls `crate::yt::session::run_setup(&reqs)` which runs `python3 -m venv` + `pip install` synchronously (`session.rs:101-158`). Repro: `:yt setup<Enter>` → TUI freezes for ~30s; the footer message "YT setup: installing deps…" is set but never rendered (the event loop is blocked). The user sees a frozen screen and may think the app crashed. Affected: first-run onboarding; "authenticated, unauthenticated, expired-session" states. AC: `:yt setup` runs in a background thread or subprocess; the TUI shows a spinner/progress; the event loop stays responsive; `yt_status` updates when complete.

- **PB6 — P1 — `set_output_format` blocks the UI thread for up to ~310ms on every format change.** `src/audio.rs:238-286` — `set_physical_format` does `AudioObjectSetPropertyData` (sync) + `verify_format_landed` (poll up to 250ms) + 60ms settle sleep. Called from `start_playback`/`load_track`/`load_remote` (app.rs:596-597, 735-736, 770-771). Repro: play a 96k track after a 44.1k track → ~310ms freeze on the track transition (no render, no input). Affected: "No blocking I/O on the UI thread" (brief line 458, 211); gapless playback. AC: the format switch is deferred to a background thread or the audio backend's own callback; the TUI never blocks for >16ms on the render path.

- **PB7 — P1 — `track_by_id` is an O(n) linear scan called per track per frame.** `src/tui/app.rs:383-385` — `self.catalog.tracks.iter().find(|t| t.id == id)`. Called from `track_rows` (columns.rs:425) **for every track in the visible list, every frame**. For a 1594-track library with 20 tracks visible, that's 20 × 1594 ≈ 32k comparisons per frame. Repro: open a 50-track album, press `j` repeatedly → each render frame does 50 full-catalog scans. Affected: "Fast libraries and large libraries" (brief line 55); "Excessive cloning or full-library scans on every frame" (brief line 682). AC: build a `HashMap<String, usize>` (id → track index) at catalog load; `track_by_id` is O(1).

- **PB8 — P1 — `clamp_cursors` → `current_context_ids` → `tracks_for_album` does a full-library O(n) scan every frame.** `src/tui/view/layout.rs:53` calls `app.clamp_cursors()` every render. `clamp_cursors` (app.rs:453-475) calls `current_context_ids()` (app.rs:477-510) which in the Artists view calls `tracks_for_album` (app.rs:435-445) — a full `catalog.tracks.iter().enumerate().filter()` scan. Repro: any rendering at 30fps with a 1594-track library → ~48k comparisons/sec just for cursor clamping. Affected: same as PB7. AC: cache the album→track_ids mapping (it's already in `albums_by_artist` as `track_indices` — use those instead of re-scanning).

- **PB9 — P1 — Unbounded `track_cache` and `transport.history`.** `src/yt/session.rs:215` — `track_cache: HashMap<String, RemoteTrack>` grows without bound as the user searches/browses YT. `src/tui/queue.rs:45` — `history: Vec<(String, Context)>` grows with every `next()`/context switch and is only popped by `prev()`. Repro: listen for hours, pressing `>` repeatedly → history grows to thousands of entries; search many queries → track_cache grows to thousands of entries. Affected: "Unbounded caches or history" (brief line 689); memory. AC: `track_cache` capped (e.g. LRU, 1000 entries); `history` capped (e.g. 500 entries, dropping oldest).

- **PB10 — P1 — AfplayPlayer: seek/position/duration are silent no-ops; no user feedback.** `src/player.rs:176-188` — `seek()` returns `Ok(())` without doing anything; `position()` and `duration()` return `None`. Repro: mpv unavailable → afplay fallback → press `,`/`.` (seek) → nothing happens, no error; the progress bar shows `--:-- / --:--`; press `+`/`-` (volume) → nothing happens. The user has no indication that these features are unavailable. Affected: "Seeking unsupported content" (brief line 653); "mpv socket unavailable → afplay fallback" (spec). AC: either afplay shows a "limited mode — no seek/volume/progress" status, or the fallback is communicated at launch; seek/volume on afplay surface a "not available in afplay mode" message.

### P2

- **PB11 — P2 — `MpvPlayer::spawn` blocks startup for up to 2s waiting for the IPC socket.** `src/player.rs:261-266` — 20 × 100ms sleep loop. Repro: launch on a slow machine → up to 2s before the TUI appears. If mpv fails to start, the fallback to afplay adds the 2s wait. Affected: startup responsiveness. AC: spawn mpv in a background thread; show the TUI immediately; connect to the socket lazily on first command.

- **PB12 — P2 — Premium warm-up of a hardcoded video id ties up the premium-resolve slot for ~10-15s on cold start.** `src/tui/app.rs:1674` — `session.send_resolve_premium("jNQXAC9IVRw".into())` ("Me at the zoo") is fired on every `refresh_yt_lists` (Y-view open). The premium inflight guard (`premium_resolve_busy()`) in `preload_next_url` (app.rs:712) skips the actual next track's premium preload while this warm-up is in flight. Repro: open the Y view → the first track's premium preload is delayed by up to 15s. Affected: gapless Premium handoff for the first track. AC: the warm-up resolves a video id that will never be the next track (or is skipped once a real preload is requested); or the warm-up is cancelled when a real preload is needed.

- **PB13 — P2 — `switch_to_radio` clones all track ids on every radio context rebuild.** `src/tui/app.rs:1724` — `self.catalog.tracks.iter().map(|t| t.id.clone()).collect()` — 1594 string clones. Called when CONT=Radio exhausts the context (which it rebuilds as the whole library every time). Repro: CONT=Radio, let tracks end repeatedly → each radio rebuild clones 1594 strings. Affected: performance at context boundaries. AC: hold a pre-built `Vec<String>` of all track ids on `App` and clone the `Vec` (which is just an Arc bump if using `Arc<[String]>`, or a cheap slice clone).

- **PB14 — P2 — No buffering state for mpv stream loading.** When mpv loads a YouTube stream URL, there's a brief period where `is_playing()` returns true (the child is running) but audio hasn't started (mpv is buffering). The player bar shows `⏸` (playing) during this gap. Repro: play a YT track with a cold cache → spinner spins (resolving), then `⏸` appears, then audio starts 0.5-1s later. The gap between `⏸` and audio start is indistinguishable from "playing but silent." Affected: "Buffering" (brief line 273); "What is currently playing?" (brief line 65). AC: mpv's `paused-for-cache` event is observed; a "buffering…" indicator is shown until the first audio frame.

- **PB15 — P2 — Progressive upgrade with afplay reloads from the beginning (position lost).** `src/tui/app.rs:1316-1328` — the premium swap calls `player.load_at(&p, pos)` which falls back to `load` + `seek_to` (player.rs:14-17). Afplay's `seek` is a no-op, so the new stream starts from 0. Repro: YouTube mode with afplay fallback → a premium URL lands mid-play → the track restarts from the beginning. Affected: afplay + YT edge case. AC: skip the progressive upgrade when the player can't seek (afplay), or show a status that the upgrade is deferred.

- **PB16 — P2 — Spinner advances at ~6.7fps (150ms/tick) — choppy.** `src/tui/app.rs:1376-1380` — `spinner_frame` advances once per `on_tick` (150ms poll). The TUI skill recommends 80ms for braille spinners. Repro: play a cold-miss YT track → the spinner visibly stutters. Affected: polish. AC: advance the spinner on a faster cadence (e.g. a separate timer or every other tick), or accept the trade-off with documentation.

### P3

- **PB17 — P3 — No explicit cancellation of in-flight sidecar requests.** The session uses inflight guards (`resolve_inflight`, `search_inflight`, `playlist_inflight`, `premium_resolve_inflight`) to prevent duplicate sends, but there's no way to cancel an in-flight request. If the user navigates away from the Y view, a `send_refresh` still completes and `pending_playlists` is folded into `yt_lists` on the next tick. Repro: switch to Y view (fires refresh), switch back to Artists immediately → the refresh still completes ~2s later, updating `yt_lists` (harmless but wasted work). Affected: "Cancellation" (brief line 271); wasted work. AC: a generation/cancel token per request; navigating away cancels pending requests or marks them stale.

- **PB18 — P3 — `track_rows` / `yt_track_rows` are rebuilt every frame.** `src/tui/view/columns.rs:418-438`, `444-482` — every render call reconstructs the full `Vec<Line>` for the track column. Repro: a 50-track album renders 50 lines every frame at 30fps = 1500 line constructions/sec. Affected: "Work repeated when state has not changed" (brief line 691). AC: cache the rendered lines and only rebuild when the track list, cursor, or now-playing changes; or accept the trade-off (lines are cheap).

- **PB19 — P3 — `drain_paired` + `pending_*` processing in `on_tick` does linear scans.** `src/tui/app.rs:1157-1161` — `self.yt_lists.iter_mut()` to find the matching list by id (linear). `url_cache.iter().find()` in `url_for` (session.rs:364) — O(n) with n≤2, so trivial. `track_cache.get()` is O(1) (HashMap). The `yt_lists` scan is O(lists) per pending_tracks response — typically <50 lists. Negligible.

- **PB20 — P3 — `is_playing()` / `track_ended()` race on afplay.** `src/player.rs:189-196` (`is_playing` probes `try_wait` through RefCell), `197-211` (`track_ended` probes + reaps). Between a `try_wait` returning `Some` (exited) in `is_playing` and the `track_ended` call that reaps, the player bar shows `▶` (not playing) while afplay has exited but `on_track_ended` hasn't fired yet. Repro: afplay playing, watch the player bar as the track ends → briefly shows `▶` before auto-advancing. Affected: minor visual glitch. AC: `is_playing` and `track_ended` share a single probe; or `is_playing` reaps and reports ended atomically.

---

## 4. Performance Baseline (no measurements taken — audit-only)

The brief says "Measure or establish reproducible baselines instead of guessing about performance." This is a read-only audit; I did not run benchmarks. The following are **estimated** hot paths based on code analysis:

| Path | Frequency | Cost | Est. impact |
|---|---|---|---|
| `clamp_cursors` → `current_context_ids` → `tracks_for_album` | every frame (30fps) | O(n=1594) full scan | **High** — PB8 |
| `track_rows` → `track_by_id` per visible track | every frame × 20 tracks | O(n=1594) each | **High** — PB7 |
| `render_artists` → `albums_by_artist.get` + `tracks_for_album` | every frame | O(1) + O(n) | **High** — same root cause as PB8 |
| `on_tick` → `drain_paired` + `pending_*` folding | every 150ms | O(responses) + O(lists) | Low — bounded by sidecar throughput |
| `on_tick` → `preload_next_url` → `send_resolve_premium` | every 150ms (if guard clear) | O(1) guarded | Low — inflight guard prevents re-send |
| `player_bar::build_info_line` → `now_playing_view` → `track_by_id` | every frame | O(n=1594) | **High** — same as PB7 |
| `MpvPlayer::track_ended` → non-blocking socket read | every 150ms | O(buffered events) | Low — 8KB reads, non-blocking |
| `set_output_format` | on format change | ~310ms blocking | **P1** — PB6 |
| `switch_to_radio` → clone all ids | on radio context build | O(n=1594) clones | Low — only on context end |

**Root cause cluster:** PB7 and PB8 share the same root cause — no `HashMap<id, &Track>` index. Building one at catalog load eliminates both O(n) scans. `albums_by_artist` already has `track_indices: Vec<usize>` per album — `tracks_for_album` re-scans the catalog when it could use those indices directly.

---

## 5. Process Lifecycle & Cleanup

| Component | Cleanup mechanism | Verdict |
|---|---|---|
| `MpvPlayer::Drop` | kill child + wait (reap) + remove socket | ✓ good |
| `AfplayPlayer::Drop` | kill_current (kill + wait) | ✓ good — prevents "weird songs" orphan |
| `Sidecar::Drop` | kill child + wait | ✓ good |
| `TerminalGuard::Drop` (event.rs) | disable mouse, leave alt screen, disable raw, show cursor | ✓ good |
| Panic hook (event.rs) | cleanup_terminal + cleanup_audio, then chain to prev | ✓ good — restores terminal + audio format before panic message |
| `SIGTSTP` (event.rs) | cleanup_terminal + cleanup_audio, raise SIGTSTP, re-enter on SIGCONT | ✓ good |
| State save on clean exit (main.rs:197-207) | save_layout + save_playlists (best-effort) | ✓ good |
| `on_tick` auto-respawn (app.rs:1057-1104) | backoff-gated (≤3 attempts, ≥5s apart) | ✓ good — prevents tight respawn loop |

**No leaked processes identified.** All child processes are killed+reaped on Drop. The panic hook covers crash paths. The SIGTSTP handler covers suspend/resume.

---

## 6. Stale Events & Races

| Race | Mechanism | Verdict |
|---|---|---|
| Stale search response applied to wrong query | `Pending::Search(q)` carries the query; `on_tick` only applies if `submitted == q` | ✓ handled |
| Stale premium URL for old track | `pending_premium_url` guarded by `same_track` check (app.rs:1308-1311) | ✓ handled |
| Stale pending_play for old track | `pending_play` single slot — new pick replaces old (app.rs:559, 732) | ✓ handled |
| Stale end-file from replaced track | `MpvPlayer::drain_socket` on load clears buffered events (player.rs:339) | ✓ handled |
| Stale playlist tracks for old list | `pending_tracks` carries the list id; `on_tick` matches by id (app.rs:1153-1163) | ✓ handled |
| Inflight guard not cleared on error | Error response clears the matching inflight guard (session.rs:599-621) | ✓ handled |
| Inflight guard not cleared on timeout | `roundtrip` timeout clears the inflight guard (session.rs:692-701) | ✓ handled |
| `is_playing` / `track_ended` race (afplay) | `try_wait` called from both; brief window where `is_playing` sees exited but `track_ended` hasn't reaped | ⚠ minor — PB20 |
| Progressive upgrade near track end | Guard: `dur - pos < 5.0` prevents swap in last 5s (app.rs:1315-1316) | ✓ handled — but see PB15 (afplay can't seek to pos) |

---

## 7. Summary (~25 lines)

**P0 (1):** PB1 — `player.load()` errors discarded via `let _ =`, `now_playing` set regardless — the player bar can show a track that isn't playing (violates the brief's hard invariant).

**P1 (9):** PB2 — blocking `library_playlists()` at startup (up to 3s). PB3 — blocking `get_playlist()` in Discover (up to 4s). PB4 — blocking `get_watch_playlist()` in CONT=YouTube radio advance (up to 4s). PB5 — `:yt setup` blocks TUI for ~30s with no rendering. PB6 — `set_output_format` blocks UI thread for ~310ms on every format change. PB7 — `track_by_id` O(n) linear scan per track per frame. PB8 — `clamp_cursors`→`tracks_for_album` O(n) full-library scan every frame. PB9 — unbounded `track_cache` and `transport.history`. PB10 — afplay seek/position/duration are silent no-ops with no user feedback.

**P2 (6):** PB11 — mpv spawn blocks startup up to 2s. PB12 — premium warm-up of hardcoded video ties up the inflight slot. PB13 — `switch_to_radio` clones all track ids. PB14 — no buffering state for mpv. PB15 — progressive upgrade with afplay reloads from beginning. PB16 — spinner at 6.7fps (choppy).

**P3 (4):** PB17 — no explicit cancellation of in-flight sidecar requests. PB18 — track rows rebuilt every frame. PB19 — linear scans in `on_tick` (negligible). PB20 — afplay `is_playing`/`track_ended` race (minor visual).

**Root cause cluster:** PB7 + PB8 share the same root cause — no `HashMap<id, &Track>` index. Building one at catalog load eliminates both O(n) per-frame scans. `albums_by_artist` already has `track_indices` — `tracks_for_album` re-scans when it could use those indices.

**Blocking-on-UI-thread cluster:** PB2 + PB3 + PB4 + PB5 + PB6 are all synchronous calls on the TUI event loop thread. The sidecar's `roundtrip()` spin-waits with `sleep(10ms)` for 2-15s deadlines. `set_output_format` blocks for ~310ms. `:yt setup` blocks for ~30s. The fix pattern is the same for all: move to fire-and-forget + `on_tick` folding (which is already the pattern for `send_refresh`/`send_search`/`send_get_playlist`/`send_resolve` — the sync `roundtrip` variants are the outliers).

**Cleanup:** all child processes (mpv, afplay, sidecar) are killed+reaped on Drop. Panic hook restores terminal + audio format. SIGTSTP/SIGCONT handled. No leaked processes.

**Stale events:** well-handled — query-carried `Pending` variants, single-slot `pending_play`, `drain_socket` on load, inflight-guard clearing on error/timeout. One minor race (PB20).

**Performance:** no measurements taken (read-only audit). Estimated hot paths: `track_by_id` (O(n) per track per frame) and `tracks_for_album` (O(n) per frame via `clamp_cursors`) are the worst offenders at ~48k-960k comparisons/sec for a 1594-track library. Both are fixable with a `HashMap<id, usize>` index built at catalog load.
