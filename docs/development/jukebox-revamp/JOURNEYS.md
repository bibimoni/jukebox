# User Journeys & Capability Matrix — Synthesis of 5 Recon Reports

**Date:** 2026-07-12
**Sources:** `AUDIT.md` (cartographer), `yt-recon.md` (YouTube/auth), `tui-recon.md` (TUI/interaction), `playback-recon.md` (playback/concurrency), `quality-recon.md` (quality/security/release).
**Method:** Read-only synthesis. Every claim carries a source report + file:line.

This document maps the end-to-end user journeys (what the user does → what the
code does → where it breaks) and a capability matrix (what the app can/can't do
today). It is the input to `PLAN.md`'s vertical slices and `ACCEPTANCE.md`'s
verifiable criteria.

---

## Part 1 — User Journeys

### J1: First-run / local-only use
**Steps:** install → `jukebox sync` → `jukebox` → browse Artists → Enter to play → `>/<` next/prev → `q` quit → restart.

**Code path:** `main.rs:8-178` (Cmd::Play) → `App::new` (`app.rs:324-381`) → `event::run` (`event.rs:191-274`) → `input::handle_key` → `play_selected` (`app.rs:788-814`) → `start_playback` (`app.rs:556-631`) → `resolve_source` (`app.rs:638-684`) → `Resolved::Local` → `player.load` → `on_track_ended` → `next` (`app.rs:835-900`).

**Where it works:** ✓ Catalog load, BM25 search (Tantivy+Lindera), Miller columns, mpv/afplay playback, transport (shuffle/repeat/continue), state persistence (layout + playlists), CoreAudio rate switch, panic-safe restore.

**Where it breaks:**
- **No onboarding hint** after first sync — the TUI opens with a full catalog but no "press `?` for help" nudge beyond the footer (tui-recon §7, TUI-P3-3).
- **Missing search index silently degrades** — `main.rs:25` `.ok()` swallows a missing index; the user doesn't know search is unavailable (tui-recon §7, TUI-P2-4).
- **README keybindings are wrong** (quality-recon §12, P1-1) — the user learns the wrong keys from docs.
- **`jukebox config` doesn't re-run the prompt** despite README claiming it does (quality-recon §12, P2-6).
- **CLI search output is unescaped** — a maliciously-tagged track title could inject terminal escapes (quality-recon §5, P1-2).
- **CoreAudio switch blocks 310ms** on every rate change (playback-recon §9.1) — perceptible stutter on hi-res → YT transitions.
- **`smart_shuffle` is O(n²)** on large libraries (playback-recon §9.7) — stutter on shuffle/context-switch.
- **`transport.history` unbounded** (playback-recon §9.3) — long session memory leak.

### J2: YouTube first login → playlists → play → restart
**Steps:** `:yt auth browser chrome` → Keychain prompt → "connected" → `4` (Y view) → playlists appear → Enter to play → `q` → restart → should reconnect without re-login.

**Code path:** `input.rs:427-436` → `apply_yt_browser` (`app.rs:995-1015`) → `Session::spawn_browser` (`session.rs:295-322`) → `Sidecar::spawn` (`sidecar.rs:44-98`) → `_browser_cookie_jar` (`yt.py:62-104`) → `_browser_cookies_file` (`yt.py:122-181`) writes 0600 cache → `switch_view` → `refresh_yt_lists` (`app.rs:1653-1675`) → `send_refresh` (`session.rs:714-720`) → `on_tick` merges (`app.rs:1126-1148`). On restart: `main.rs:108-130` restores `yt_browser` → `load_cookies` → `main.rs:127-129` sets "connected" → `main.rs:148` probe.

**Where it works:** ✓ Browser cookie read (single Keychain prompt), 0600 persistent cache, playlist fetch, fast/premium resolve, gapless preload, progressive upgrade.

**Where it breaks — THIS IS THE PRIMARY REPORTED ISSUE:**
- **Launch probe discards session on ANY error** (`main.rs:151`, yt-recon §3, §10): a 3s timeout / network blip / ytmusicapi parse error / IP rate-limit at launch sets `app.yt_session = None` for the whole run. No recovery path — `on_tick` respawn only fires on an existing `Some` session whose child died, never re-creates a `None` one. **The user must `:yt auth browser` again, re-prompting the Keychain. This is why the user repeatedly logs in.**
- **"Connected" set before data fetch** (`main.rs:117/129`, `app.rs:988/1014`, yt-recon §8): the footer says "connected" based on the sidecar spawning, not credential validity. Decoupled from actual data availability.
- **No pagination** (`yt.py:345`, `yt.py:364`, yt-recon §5): `get_library_playlists()` defaults to 25, `get_playlist` to 100, no continuation loop. Libraries/playlists silently truncated.
- **Empty == failed** (`yt.py:357-358`, yt-recon §5): the `library_playlists` fallback swallows exceptions into `[]`. An empty library, a failed fetch, and a guest-downgrade all yield `[]`. The Y view shows "select a list" identically in all three cases.
- **`send_refresh` no inflight guard / no generation ids** (`session.rs:714-720`, yt-recon §7, playback-recon §9.4): multiple refreshes stack; a stale refresh can overwrite fresh data via FIFO pairing (`app.rs:1126-1148` applies unconditionally).
- **`yt_lists_loading` can hang `true`** (yt-recon §5, playback-recon §9.4): if the sidecar dies mid-fetch, no timeout on the fire-and-forget path; col2 shows "loading…" forever.
- **`auth_status` lies** (`yt.py:329-330`, yt-recon §4, §8): reports `ok=True` on SAPISID cookie **presence**, not validity. `premium`/`account` are identical to `ok` — no real premium detection.
- **`yt_status`/`yt_error` never auto-clear** (tui-recon §6, TUI-P1-3): a stale "connected via chrome" persists indefinitely until overwritten.
- **`home_suggestions` blocks 3s** (`app.rs:1495`, AUDIT §3, playback-recon §9.1) when `S` is pressed.
- **`get_playlist` blocks 4s** (`app.rs:1636-1639`, AUDIT §3) on discover-Enter.

### J3: Expired / revoked credential
**Steps:** (working session) cookie expires mid-session → next action → what happens?

**Code path:** no expiry detection anywhere. `auth_status` (`yt.py:329-330`) checks presence only. Expired cookie → YouTube silently downgrades to guest → `get_library_playlists()` returns `[]` (not an error) → sidecar fallback swallows any exception into `[]` (`yt.py:357-358`).

**Where it breaks — THIS IS THE "CONNECTED BUT EMPTY" ISSUE:**
- **No expiry/revocation detection** (yt-recon §4): the cookies.txt `expires` column is written (`yt.py:163`) but never read for validation. `ResolvedUrl.expires_at` is always `None` (`yt.py:504`).
- **Expired → silent empty** (yt-recon §4, §10): expired cookie → guest downgrade → empty library → `Ok([])` → probe passes (`main.rs:149`, `Ok(_)` → reachable, no emptiness check) → `yt_status` says "connected" → Y view empty. **User sees "connected" but nothing is there.**
- **No refresh mechanism** (yt-recon §2): no OAuth refresh token, no cookie renewal. The only recovery is the user running `:yt auth browser` again (re-reading the browser store).
- **No revocation signal** (yt-recon §4): a revoked credential doesn't kill the sidecar — it just makes data requests return empty/error. No retry loop, just silent emptiness.

### J4: Offline / network failure
**Steps:** network drops mid-session → action → network returns → action.

**Code path:** `_is_transient` (`yt.py:291-301`) detects SSL EOF / timeout / connection reset. `_extract_with_retry` (`yt.py:304-322`) retries up to 3x with 0.4s/0.8s backoff for `resolve_url`. ytmusicapi init failure (`yt.py:527-545`) prints a network-flavored error and sets `have=False` but the sidecar stays alive serving ping/auth_status.

**Where it works:** ✓ Transient retry on resolve_url (3x). ✓ ytmusicapi init failure surfaces a network message. ✓ Sidecar stays alive (doesn't crash) so `on_tick` respawn isn't needed.

**Where it breaks:**
- **No retry for library_playlists/get_playlist/search** — only `resolve_url` retries (`yt.py:304-322`). A transient failure on a playlist fetch just returns `[]` or an error, no retry.
- **No offline indicator** (ACCEPTANCE A4): cached playlists aren't marked stale; the Y view shows empty with no "offline" hint.
- **No refresh-without-restart** (ACCEPTANCE A4): once the session is gone (`main.rs:151`), only a restart or `:yt auth` recovers it.
- **Launch probe kills session on network failure** (`main.rs:151`, J2): an offline launch strands the user in guest mode for the whole run.

### J5: Hybrid local+YouTube (Mixed mode)
**Steps:** `M` → Mixed → play a track that exists both locally and on YT.

**Code path:** `resolve_source` (`app.rs:638-684`) → catalog match first (Local/Mixed) → `match_local` (`match_local.rs:19-85`) ISRC exact or title+artist fuzzy (gate 0.88, floor 0.80) → `Resolved::Local`. Else `session.url_for` → `Resolved::Remote`/`Pending`.

**Where it works:** ✓ ISRC exact match (deterministic). ✓ Title+artist fuzzy with translit variants. ✓ Conservative thresholds (wrong substitution worse than streaming).

**Where it breaks:**
- **`match_local` is O(catalog) per track, no index** (playback-recon §9.8): loading a 100-track YT playlist in Mixed mode = 100 × catalog_size Levenshtein calls. Slow for large catalogs.
- **`track_cache` unbounded** (`session.rs:205`, playback-recon §9.3): grows with every unique video_id; long Mixed session = large HashMap.
- **`url_cache` FIFO not LRU, no expiry** (`session.rs:389`, playback-recon §9.9): stale URLs served until evicted by the 2-entry cap. A revoked URL would fail in mpv without self-heal.

### J6: Command mode
**Steps:** `:` → type `yt auth browser chrome` → Enter.

**Code path:** `input.rs:283-298` (Command overlay) → Enter → `execute_command` (`input.rs:416-439`) → match → `apply_yt_browser` / `yt_logout` / `yt_setup` / open `YtAuth` overlay. Unknown command: `_ => {}` (silent no-op).

**Where it works:** ✓ The 4 known commands work. ✓ Esc cancels.

**Where it breaks:**
- **No command history** (AUDIT §13, tui-recon §10): `Overlay::Command { input: String }` holds only current input. No Vec, no up/down, no persistence. (M4 in TODO.)
- **Unknown commands silently ignored** (`input.rs:437`, tui-recon TUI-P1-2): no "unknown command" feedback.
- **No tab completion** (tui-recon §10): typing `:yt ` doesn't suggest `auth`/`logout`/`setup`.
- **No visible cursor in command line** (tui-recon §10): unlike search overlay which has `▏`.
- **No nested overlays** (tui-recon §2): `:yt auth` opens `YtAuth` which replaces `Command`; if the user had typed a long command, it's lost.

### J7: Lyrics
**Steps:** (any track playing) → press `l` (or whatever key) → lyrics appear.

**Code path:** **NOWHERE.** `grep -i lyric` over `src/` → 0 matches (AUDIT §13, Q5). No `Overlay::Lyrics`, no `Request::GetLyrics`, no `yt.py get_lyrics`, no lyrics field on `Track`/`RemoteTrack`, no panel in `player_bar.rs`/`columns.rs`/`overlay.rs`.

**Where it breaks:** **Entirely unimplemented** despite `.opencode/prompt` listing lyrics as a first-class requirement (lines 489-514) and `ytmusicapi.get_lyrics` being available. (M3 in TODO.)

### J8: Degraded terminal / responsive
**Steps:** resize terminal to 60×20 → use the app.

**Code path:** `layout.rs:30-34` breakpoints: <60×20 → "terminal too small"; 60-80 → narrow single-pane; ≥80×24 → full Miller columns.

**Where it works:** ✓ 3-tier responsive (tui-recon §4). ✓ Snapshot tests cover 4 sizes. ✓ Column widths persisted. ✓ Divider drag. ✓ `NO_COLOR` collapses to `Color::Reset` (tui-recon §5).

**Where it breaks:**
- **No high-contrast mode** beyond NO_COLOR (tui-recon §5).
- **CJK display width approximation** doesn't handle zero-width/combining chars (tui-recon §5, TUI-P3-2).
- **Cursor blink always on** — no option to disable (tui-recon §5, TUI-P3-1).

### J9: Logout / account switch
**Steps:** `:yt logout` → verify cookies cleared → switch browser with `:yt auth browser firefox`.

**Code path:** `input.rs:421-422` → `yt_logout` (`app.rs:1384-1393`) → `remove_file(cookies_file)` → `yt_browser.clear()` → `session.clear_cookies` respawns guest → `yt_status = "logged out"`.

**Where it works:** ✓ Cookies file deleted. ✓ `yt_browser` cleared. ✓ Sidecar respawned guest. ✓ Status set.

**Where it breaks:**
- **`yt_lists` NOT cleared** (yt-recon §9): the Y view shows the old account's playlists until a `refresh_yt_lists` is triggered.
- **`loaded_yt_lists` NOT cleared** (yt-recon §9).
- **`track_cache`/`url_cache` NOT cleared** (yt-recon §9) — surviving data persists.
- **In-flight requests NOT cancelled** (yt-recon §9): a refresh landing AFTER logout can re-populate `yt_lists` with the now-logged-out account's data (`app.rs:1126-1148` applies unconditionally).
- **Account switch (`:yt auth browser <other>`) doesn't clear `yt_lists`/`loaded_yt_lists`** (yt-recon §9) — old lists stay until next refresh.
- **Not crash-atomic** (yt-recon §9): `state.db` only written on clean exit; a crash after logout leaves `yt_browser` in the DB, so next launch tries the browser.

### J10: Discover / radio auto-continue
**Steps:** `S` (discover) → Enter on a suggestion → play. OR: set continue=YouTube → let context exhaust → auto-advance.

**Code path:** `open_discover` (`app.rs:1453`) → `yt_discover_items` (`app.rs:~1490`) → `session.home_suggestions()` (**blocking 3s**, `session.rs:898-904`) → overlay. Enter → `play_discover_selection` (`app.rs:~1630`) → `session.get_playlist()` (**blocking 4s**, `session.rs:890-896`). Continue=YouTube: `next` (`app.rs:864-897`) → `radio.advance` (`session.rs:945-968`) → `get_watch_playlist(radio=True)`.

**Where it works:** ✓ RadioCursor advance + refill + seed-drop. ✓ Continue mode cycling.

**Where it breaks:**
- **`home_suggestions` blocks 3s** (AUDIT §3, playback-recon §9.1) — the event loop stalls when `S` is pressed.
- **`get_playlist` blocks 4s** (AUDIT §3, playback-recon §9.1) — discover-Enter stalls for 4s.
- **`get_watch_playlist` is a sync roundtrip** (`session.rs:912-918`, 4s deadline) — radio auto-advance blocks on a 4s network call.
- **No "next track" preview** (tui-recon §11) — the user can't see what's coming in the current context.

---

## Part 2 — Capability Matrix

Legend: ✅ works | ⚠️ partial/buggy | ❌ missing/broken | ➖ N/A

| Capability | Status | Evidence | Plan Slice |
|---|---|---|---|
| **Local playback (mpv)** | ✅ | `player.rs:172-388`, gapless, IPC, seek, volume | — |
| **Local playback (afplay)** | ✅ | `player.rs:68-170`, no seek/position | — |
| **mpv → afplay fallback signal** | ❌ | `player.rs:390-397` silent downgrade, no UI signal | S2 |
| **Local catalog load** | ✅ | `catalog.rs`, `catalog.json` | — |
| **BM25 search (Tantivy+Lindera)** | ✅ | `search.rs`, cross-script, fuzzy | — |
| **Missing index hint** | ❌ | `main.rs:25` `.ok()` swallows; no user signal | S7 |
| **YouTube auth (pasted cookies)** | ⚠️ | `session.rs:446-458` works; no expiry detection | S1 |
| **YouTube auth (browser)** | ⚠️ | `session.rs:295-322` works; Keychain prompt once; no expiry detection | S1 |
| **Session restore on restart** | ❌ | `main.rs:151` discards session on any probe error; forces re-login | S1 |
| **Token/cookie expiry detection** | ❌ | `yt.py:329-330` presence-only; `expires_at` always None | S1 |
| **Token refresh** | ❌ | no refresh mechanism anywhere | S1 |
| **Revocation handling** | ❌ | silent empty, no signal | S1 |
| **`auth_status` truthfulness** | ❌ | `yt.py:329` ok=premium=account=presence; no validity check | S1 |
| **YouTube search** | ✅ | `session.rs:802-814`, query-tagged, inflight guard | — |
| **YouTube search (live)** | ❌ | explicit-submit only (intentional); no debounced auto-search | — |
| **YouTube playlists fetch** | ⚠️ | `yt.py:345` works; no pagination (25 default) | S1 |
| **YouTube playlist tracks fetch** | ⚠️ | `yt.py:364` works; no pagination (100 default) | S1 |
| **Empty vs failed distinction** | ❌ | `yt.py:357-358` swallows exceptions into `[]` | S1 |
| **YouTube resolve (fast)** | ✅ | `session.rs:726-737`, tv_embedded, ~1.3s, AAC 129k | — |
| **YouTube resolve (premium)** | ✅ | `session.rs:746-758`, tv/web + EJS, ~10-15s, AAC 256k | — |
| **Progressive premium upgrade** | ✅ | `app.rs:1293-1333`, same-track guard, resume-at-pos | — |
| **Cold-miss non-blocking swap** | ✅ | `app.rs:608-616`, `on_tick` swap, give-up on fast fail | — |
| **URL cache** | ⚠️ | `session.rs:209`, cap 2, FIFO not LRU, no expiry | S1 |
| **Track cache** | ⚠️ | `session.rs:205`, unbounded | S10 |
| **Gapless preload** | ✅ | `app.rs:688-716`, premium preload for next track | — |
| **Autoplay radio (CONT=YouTube)** | ✅ | `session.rs:924-968`, RadioCursor | — |
| **Home suggestions** | ⚠️ | works but **blocks 3s** (`session.rs:898`) | S2 |
| **Discover overlay** | ⚠️ | works but `get_playlist` **blocks 4s** on Enter | S2 |
| **Lyrics** | ❌ | entirely missing (0 matches in `src/`) | S3 |
| **Command mode (basic)** | ⚠️ | 4 commands work; unknown silently ignored | S4 |
| **Command history** | ❌ | no Vec, no up/down, no persistence | S4 |
| **Command completion** | ❌ | no tab completion | S4 |
| **Queue (manual play-next)** | ✅ | `queue.rs:187-197`, enqueue/remove/clear | — |
| **Queue UI** | ❌ | no keybinding for enqueue/remove wired (PLAN S5) | S5 |
| **Playlist add (`a`)** | ❌ | overlay exists but not wired (PLAN S5) | S5 |
| **Shuffle (Off/Smart/Random)** | ✅ | `queue.rs:158-185`; smart is O(n²) | S10 |
| **Repeat (Off/All/One)** | ✅ | `queue.rs:154-156` | — |
| **Continue (Off/NextAlbum/Radio/YT)** | ✅ | `queue.rs:33-39`, `app.rs:846-898` | — |
| **`prev` across sessions** | ❌ | `transport.history` not persisted (AUDIT §4) | — |
| **State persistence (layout)** | ✅ | `state.rs:113-134`, `main.rs:183-192` | — |
| **State persistence (playlists)** | ✅ | `state.rs`, local playlists only | — |
| **State persistence (transport)** | ❌ | cursor/order/history/manual_queue not saved | — |
| **CoreAudio rate switch** | ⚠️ | `audio.rs` works; **blocks 310ms** per switch | S2 |
| **CoreAudio restore on exit** | ✅ | `audio.rs:73-78`, panic hook + guard | — |
| **Error feedback (YT)** | ⚠️ | `yt_error` set on errors; never auto-clears | S6 |
| **Status feedback (YT)** | ⚠️ | `yt_status` set on spawn; false "connected" (5 places) | S1 |
| **Loading states** | ⚠️ | `yt_lists_loading` for Y view; can hang true; no local-playback loading indicator | S2/S7 |
| **Empty states** | ⚠️ | Y view handles; Artists/Queue/Playlists lack "no music/empty" hints | S7 |
| **Offline states** | ❌ | no offline indicator; no cached-stale marking | S1 |
| **Rate-limit/retry states** | ❌ | only `resolve_url` retries (3x); no user-visible rate-limit signal | S1 |
| **Pagination** | ❌ | no pagination anywhere; no "load more" affordance | S1 |
| **Cancellation / generation ids** | ❌ | no generation ids; stale refresh can overwrite fresh | S1 |
| **`send_refresh` inflight guard** | ❌ | `session.rs:714` no guard; stacked refreshes | S1 |
| **Logout completeness** | ❌ | `yt_lists`/`loaded_yt_lists`/`track_cache`/inflight not cleared | S1 |
| **Account switch** | ❌ | doesn't clear `yt_lists`/`loaded_yt_lists` | S1 |
| **Responsive layout** | ✅ | 3-tier breakpoint, snapshots | — |
| **NO_COLOR** | ✅ | `theme.rs:6-8` | — |
| **High-contrast** | ❌ | none beyond NO_COLOR | S7 |
| **CJK width** | ⚠️ | `theme.rs:64-83` approximation; no zero-width/combining | S7 |
| **Unicode in search** | ✅ | Lindera + wana_kana + NFKC | — |
| **Terminal escape injection (TUI)** | ✅ | ratatui sanitizes all external data | — |
| **Terminal escape injection (CLI)** | ❌ | `main.rs:231-239` unescaped `println!` | S8 |
| **Cookie file perms** | ✅ | 0600 (`session.rs:452`, `yt.py:177`) | — |
| **Cookie temp file cleanup** | ❌ | `yt.py:50-53` `delete=False` leaks to `/tmp` | S8 |
| **`/tmp/.config` fallback** | ❌ | world-readable on multi-user systems | S8 |
| **mpv socket path** | ⚠️ | `/tmp/jukebox-mpv.sock` predictable; symlink risk | S8 |
| **Secret redaction in logs** | ⚠️ | no logging exists (`log_to_file` dead code); sidecar stderr null'd | S6 |
| **Sidecar stderr** | ❌ | `Stdio::null()` — debugging impossible | S6 |
| **`Response::from_line` malformed input** | ❌ | untested for non-JSON/traceback/huge payloads | S1 |
| **Process cleanup (mpv/afplay/sidecar)** | ✅ | all kill+reap on Drop; no zombie leak | — |
| **Panic safety** | ✅ | hook + guard restore terminal + audio | — |
| **SIGTSTP/SIGCONT** | ✅ | flag-based, restore + redraw | — |
| **`cargo fmt`** | ❌ | 42 files, 332 hunks (quality-recon P1-8) | S0 |
| **`cargo clippy`** | ❌ | 8 errors (quality-recon P1-9) | S0 |
| **`cargo test`** | ✅ | ~120 tests pass (quality-recon §1) | — |
| **`cargo build --release`** | ✅ | builds (quality-recon §1) | — |
| **CI test workflow** | ❌ | only `release.yml`; no test/clippy/fmt gate (quality-recon P0-2) | S9 |
| **Release archive (`scripts/yt/`)** | ❌ | **NOT bundled** — YT broken for all binstall users (quality-recon P0-1) | S9 |
| **Release tests before publish** | ❌ | release.yml builds but doesn't test (quality-recon P0-3) | S9 |
| **README keybindings** | ❌ | almost entirely wrong (quality-recon P1-1) | S11 |
| **README `jukebox config`** | ❌ | claims re-run prompt; doesn't (quality-recon P2-6) | S11 |
| **README browser auth** | ❌ | claims "no cookie file written"; does write one (quality-recon P1-7) | S11 |
| **README YT prereqs (binstall)** | ❌ | `pip install -r scripts/yt/requirements.txt` broken (file not in archive) | S9/S11 |
| **state.db schema versioning** | ❌ | no version key; no migration path (quality-recon P2-3) | — |
| **config.yml versioning** | ❌ | `version: 1` read but unused; no migration (quality-recon §11) | — |
| **Corrupt DB recovery** | ⚠️ | falls back to defaults; corrupt file persists, fails every launch | — |

---

## Part 3 — Cross-Cutting Defect Summary (by severity)

### P0 — Critical (release-blocking / primary reported issues)
1. **Launch probe discards session → forces re-login** (`main.rs:151`) — the "repeatedly log in" symptom.
2. **`auth_status` lies + empty==failed → "connected but empty"** (`yt.py:329`, `yt.py:357`) — the "connected but playlists empty" symptom.
3. **`scripts/yt/` not bundled in release** (`release.yml:70`) — YT broken for all binstall users.
4. **No CI test workflow** (`.github/workflows/`) — broken code can be released.
5. **Lyrics entirely missing** (0 matches in `src/`) — first-class requirement unimplemented.
6. **Command history missing** (AUDIT §13) — reported user issue unimplemented.

### P1 — High
7. **No pagination** (`yt.py:345`, `yt.py:364`) — silent truncation at 25/100.
8. **No cancellation / generation ids** (`session.rs:714`) — stale overwrites fresh.
9. **False "connected" status (5 places)** (`main.rs:117/129`, `app.rs:988/1014/1093`) — status decoupled from data.
10. **Logout/account-switch doesn't clear state** (`app.rs:1384`) — stale data survives.
11. **Blocking user actions (3s/4s)** (`app.rs:1495`, `app.rs:1636`, `main.rs:148`) — event loop stalls.
12. **CoreAudio switch blocks 310ms** (`audio.rs:265`) — stutter on rate transitions.
13. **README keybindings wrong** (quality-recon P1-1) — user learns wrong keys.
14. **CLI output unescaped** (`main.rs:231`) — terminal escape injection.
15. **`expect()` on sidecar spawn** (`sidecar.rs:65-66`) — panics on fd exhaustion.
16. **`/tmp/.config` fallback** (`config.rs:35`) — world-readable secrets.
17. **mpv socket path predictable** (`config.rs:52`) — symlink/race risk.
18. **Temp cookie files leak** (`yt.py:50-53`) — cookie material in `/tmp`.
19. **`fmt` fails (42 files)** / **clippy fails (8 errors)** — baseline gate broken.
20. **Unknown commands silently ignored** (`input.rs:437`) — no feedback.
21. **`yt_status`/`yt_error` never auto-clear** (`footer.rs:22-31`) — stale messages persist.

### P2 — Medium
22. **`smart_shuffle` O(n²)** (`queue.rs:243`) — stutter on large libraries.
23. **`match_local` O(catalog) per track** (`match_local.rs:52`) — slow Mixed mode.
24. **`transport.history` unbounded** (`queue.rs:45`) — memory leak.
25. **`track_cache` unbounded** (`session.rs:205`) — memory leak.
26. **`url_cache` FIFO not LRU, no expiry** (`session.rs:389`) — stale URLs.
27. **`send_refresh` no inflight guard** (`session.rs:714`) — stacked refreshes.
28. **`yt_lists_loading` can hang** — no fire-and-forget timeout.
29. **No schema versioning** (`state.rs`) — no migration path.
30. **`Response::from_line` untested for malformed** (`proto.rs:111`) — garbage input unhandled.
31. **`e2e_yt.rs` `set_var` race** (`tests/e2e_yt.rs`) — parallel test flakiness.
32. **False-confidence test** (`yt_sidecar.rs:120`) — doesn't test auth_status.
33. **`log_to_file` dead code** (`event.rs:96`) — no logging exists.
34. **Sidecar stderr null'd** (`sidecar.rs:55`) — debugging impossible.
35. **No search history** — reopening `/` starts empty.
36. **No onboarding hint** — first-run user has no guidance.

---

*End of synthesis. Input to `PLAN.md` slices and `ACCEPTANCE.md` criteria.*
