# Radio + Recommendation Issues

## Issue 1: Radio overlay shows raw video IDs instead of track names

**Current:** `Seed: track rzVKfAQp2No`
**Expected:** `Seed: あのバンド — Ado` (or whatever the track title is)

The radio overlay renders the seed as the raw track ID. It should resolve the ID to the track title using the catalog (local) or `track_cache` (YouTube).

**File:** `src/tui/view/radio.rs` — `render()` function reads `session.seed` which is a `RadioSeed` enum containing a raw String ID.

---

## Issue 2: Radio "n" (next) returns to YouTube panel, shows nothing playing, spinner spins forever

**Steps:**
1. `:radio` — overlay opens, shows seed + pool
2. Press `n` — overlay closes, returns to YouTube view
3. Player bar shows `[STOPPED]` or `[PLAYING]` with spinner but no track info
4. The radio track starts playing in the background but the UI doesn't show it

**Root cause:** `advance_radio()` in `input.rs` calls `app.play_radio_track()` which switches the transport context and starts playback, but:
- The overlay is closed (correct), but the now-playing view doesn't resolve because the YouTube track metadata may not be cached yet
- The spinner (`pending_resolve`) shows but the player bar shows "nothing playing" because `now_playing` is set to `Remote` but `now_playing_view()` can't find the track in `track_cache`

**Expected:** Pressing `n` should show the next track name in the player bar immediately (or "Loading..." with the track title).

---

## Issue 3: Radio overlay content bleeds through from the main YouTube view

**Current:** The radio overlay popup is semi-transparent — YouTube playlist/track text shows through on both sides of the overlay border.

**Expected:** The overlay should clear its full area before rendering (like the discover overlay does with `Clear`).

**File:** `src/tui/view/overlay.rs` — `render_radio_overlay` doesn't call `f.render_widget(Clear, area)` before the popup.

---

## Issue 4: Radio pool shows "50 tracks remaining" but no track list

**Current:** The overlay shows "Pool: 50 tracks remaining" but doesn't list the actual tracks in the pool.

**Expected:** Show the next few upcoming tracks (at least 5-10) so the user can see what's coming.

**File:** `src/tui/view/radio.rs` — `render()` only shows seed + pool count, not the actual candidate list.

---

## Issue 5: Recommendation engine has no listening data (profile always empty)

**Problem:** `record_listen_event()` exists on `App` but is never called during real playback. Without events, the profile is always empty, so:
- Mixes are random catalog tracks (not affinity-clustered)
- Radio picks random tracks (not taste-adjacent)
- Generator picks random tracks (not ranked by preferences)
- "70% discovery" vs "30% familiar" is meaningless

**Fix needed:** Wire `record_listen_event` into:
- `start_playback` / `load_track` → "track_started"
- `on_tick` (meaningful threshold: ≥50% or ≥30s) → "meaningful_threshold"
- `on_track_ended` → "completed"
- `next()` (user skip) → "skipped" or "rapidly_skipped" (if < 10s)
- `enqueue_selected` → "added_to_queue"
- `remove_selected_from_queue` → "removed_from_queue"
- Persist events to `state.db` on exit, load on launch
- Fix `EventSource` (Local/Youtube/Hybrid) based on `now_playing` source type
- Fix `EventContext` (Album/Playlist/Queue/Radio/Search) based on `transport.context`

---

## Issue 6: Discover Enter on a YouTube mix returns to YouTube panel with no feedback

**Steps:**
1. `M` to YouTube/Mixed mode
2. `S` for discover overlay — mixes appear (Daily Mix, Discover Mix, etc.)
3. `Enter` on a mix — overlay closes, returns to YouTube view
4. The YouTube view just shows the playlist list as before — no indication anything is loading
5. Eventually a track starts playing but the player bar shows "nothing playing" or a spinner with no track info
6. The user has no idea what's happening between pressing Enter and hearing audio

**Root cause:** `play_discover_selection()` (app.rs:2730) does:
1. Closes the overlay immediately (`self.overlay = None`) — user is back at YouTube view
2. Sends async `get_playlist(id)` to sidecar
3. Sets `yt_status = "Loading mix…"` — but this only shows in the footer briefly
4. When tracks land in `on_tick` (app.rs:2215), calls `play_in_context_ids(ids, &start)`
5. `play_in_context_ids` calls `start_playback` which resolves the track
6. For YouTube tracks, `resolve_source` returns `Pending` (stream URL not yet resolved)
7. `on_tick` pre-resolves the URL async — another wait
8. Eventually the URL lands and `on_tick` swaps the player in
9. But by now the user has been staring at an unchanged YouTube view for 5-10 seconds

**The user sees:** Enter → overlay closes → YouTube view (unchanged) → nothing happens for several seconds → audio starts but UI barely updates

**Expected:** After pressing Enter on a mix:
- Show a clear "Loading [mix name]..." overlay or status that persists until the first track starts playing
- When the first track starts, show its title in the player bar immediately
- If the track is still resolving the stream URL, show "Buffering [track name]..." not "nothing playing"

**Fix approach:**
- Don't close the overlay immediately — show a loading state inside it
- OR keep `yt_status` visible longer with the mix name
- Ensure `now_playing_view()` resolves YouTube track titles from `track_cache` even before the stream URL lands
- Show "Buffering..." instead of "nothing playing" when `now_playing` is set but `player.is_playing()` is false and a resolve is in flight

---

## Issue 7: Home overlay (`H`) shows empty sections

**Problem:** `open_home()` populates `HomeState` with sections but the render function may not show the section content properly. The sections are:
- Quick Picks (first 5 catalog tracks)
- Made for You (generated mixes)
- Start Radio (seed options)
- Your YouTube Library (playlists)

**Expected:** Each section should show its items with track/playlist names, navigable with j/k.

---

## Issue 8: Generator shows track names but no way to play the generated playlist

**Problem:** After `:gen` generates a playlist preview, pressing `Enter` saves it as a local playlist but doesn't play it. The user has to navigate to the Playlists view (press `2`) and find it.

**Expected:** Pressing `Enter` in the preview phase should offer to play the generated playlist immediately (or save + play).

---

## Issue 9: Dead code — `autoplay.rs` and `evaluation.rs` never called

**`autoplay.rs`** (110 lines): Continue-mode radio auto-advance. Never called from `app.rs` or `input.rs`. The continue mode uses the old YouTube `RadioCursor` instead.

**`evaluation.rs`** (419 lines): Profile quality metrics. Never called from anywhere. Was intended to show profile health/coverage.

**Action:** Either wire these in or remove them to reduce confusion.
