# YouTube Auth & Playlist Recon — Jukebox

READ-ONLY reconnaissance of the YouTube integration in `/Users/distiled/Dev/jukebox`.
Traces the complete path from launch to visible playlists, focused on the reported
symptoms: "connected in logs but playlists empty", "repeatedly must log in", and
unclear empty/loading/disconnected/expired/sync/failure states.

All file:line references are to the current tree.

---

## 1. Credential discovery & login initiation

Two distinct auth paths, mutually exclusive:

### (a) Pasted Netscape `cookies.txt` — `:yt auth`
- `input.rs:418-419` opens the `Overlay::YtAuth` text input; `input.rs:306-309` on
  `Enter` calls `app.apply_yt_auth(input)` (`app.rs:968`).
- `apply_yt_auth` (`app.rs:968-989`):
  - Clears `yt_browser` (so the browser path is abandoned) (`app.rs:973`).
  - If no session: `Session::spawn(python, script, Some(cookies))` (`app.rs:975`,
    `session.rs:268`).
  - Else: `session.set_cookies(cookies, …)` (`app.rs:983`, `session.rs:446-458`)
    which writes the file and respawns the sidecar.
  - Sets `yt_status = "YT auth: connected via pasted cookies"` (`app.rs:988`).

### (b) Browser profile — `:yt auth browser <name>`
- `input.rs:427-436` parses the command and calls `app.apply_yt_browser(browser)`
  (`app.rs:995`).
- `apply_yt_browser` (`app.rs:995-1015`):
  - Stores `yt_browser = browser` (`app.rs:999`) — source of truth, persisted to
    `state.db` on clean exit (`main.rs:191`).
  - If no session: `Session::spawn_browser(…)` (`app.rs:1001`, `session.rs:295-322`).
  - Else: `session.set_browser(browser, …)` (`app.rs:1009`, `session.rs:464-470`).
  - Sets `yt_status = "YT auth: connected via {browser}"` (`app.rs:1014`).

### How the sidecar receives auth
`Sidecar::spawn` (`sidecar.rs:44-98`) passes three env vars to the python process:
- `JUKEBOX_YT_COOKIES` — the pasted cookies.txt content (string).
- `JUKEBOX_YT_BROWSER` — the browser profile name (`"chrome"`, …).
- `JUKEBOX_YT_COOKIES_FILE` — a persistent path the sidecar writes the
  **decrypted** browser cookies to, so the next launch is Keychain-prompt-free.

The python side (`yt.py`):
- `_browser_name()` (`yt.py:56-59`) reads `JUKEBOX_YT_BROWSER`.
- `_browser_cookie_jar()` (`yt.py:62-104`) reads the browser store ONCE via
  `browser_cookie3` (`bc3.chrome(domain_name="youtube.com")` etc.), cached for
  process lifetime in `_BC3_JAR`. On macOS this triggers a **Keychain password
  prompt** (Chromium encrypts cookies with a Keychain key).
- `_browser_cookies_file()` (`yt.py:122-181`) writes the decrypted jar to
  `JUKEBOX_YT_COOKIES_FILE` (0600) so subsequent launches load it directly via the
  pasted-cookies path — no browser, no Keychain.
- `_cookie_pair()` (`yt.py:34-53`) parses the pasted `JUKEBOX_YT_COOKIES` into a
  `Cookie:` header + a temp cookies.txt for yt-dlp.
- `_has_auth()` (`yt.py:205-216`) returns true iff a `SAPISID` or
  `__Secure-3PAPISID` cookie is present in whichever path is active. **It checks
  presence only, not validity** (see §4).

### Cookie file location & permissions
- `cookies_file()` (`session.rs:58-66`):
  `<XDG_CONFIG_HOME or dirs::config_dir() or "/tmp/.config">/jukebox/yt-cookies.txt`.
- Pasted path: `set_cookies` writes it with `0600` (`session.rs:451-453`).
- Browser path: `_browser_cookies_file` writes it with `0o600` (`yt.py:176-178`).
  **However** the `open(out_path, "w")` at `yt.py:150` uses the default umask
  first and only `chmod`s to 0600 after closing (`yt.py:174-179`); there is a
  brief window where the file exists with default perms. Not a major leak
  (seconds), but not strictly 0600 from creation.

---

## 2. Token / cookie persistence

- `cookies_file()` as above (`session.rs:58-66`).
- `load_cookies()` (`session.rs:69-77`): reads the file; `None` if absent/empty.
- `set_cookies` (`session.rs:446-458`): writes 0600 + respawns.
- Browser path persists via `_browser_cookies_file` (`yt.py:122-181`) to the same
  `cookies_file()` path, 0600.
- `yt_browser` (the chosen browser name) is persisted in `state.db` via
  `LayoutState.yt_browser` (`state.rs:129-133`), saved on clean exit
  (`main.rs:183-192`).

### Is there a refresh mechanism? — **NO.**
- Cookies are **static** from the moment they're read. There is no token refresh,
  no OAuth refresh token, no cookie renewal. ytmusicapi uses long-lived browser
  cookies (SAPISID etc.); yt-dlp uses the cookies.txt file. Neither library
  exposes or performs refresh.
- `ResolvedUrl.expires_at` exists in the protocol (`proto.rs:70`) but the sidecar
  **always returns `None`** for it (`yt.py:504`). So stream-URL expiry isn't
  tracked either; cached URLs in `url_cache` are used until evicted by the
  2-entry cap, not until expiry.
- The only "refresh" in the codebase is `send_refresh` (`session.rs:714-720`),
  which re-fetches **playlists**, not cookies.

### Stale handling
- None. When cookies expire/are revoked, nothing detects it. The next
  `library_playlists` call silently returns empty (YouTube downgrades to guest),
  or raises a parse error that the fallback swallows into `[]` (`yt.py:357-358`).

---

## 3. Session restore on restart — CRITICAL

`main.rs:42-56` launch sequence:
1. Resolve `yt_python` (`main.rs:42-48`): `$JUKEBOX_YT_PYTHON` → venv python
   (`venv_python()`) → `python3`.
2. `load_cookies()` (`main.rs:55`) → `Session::spawn(python, script, cookies)`
   (`main.rs:56`). Guest if no cookies. **Best-effort `.ok()`** — a spawn failure
   just means `yt_session = None`.

`main.rs:68-131` layout restore (if `state.db` loaded):
3. Restore `yt_browser = layout.yt_browser` (`main.rs:108`).
4. Re-load `cached_cookies = load_cookies()` (`main.rs:109`).
5. **Branch** (`main.rs:110-130`):
   - `yt_browser` set AND `cached_cookies.is_none()` → `spawn_browser` (this is
     the **one Keychain prompt** at launch, re-persisting the cache). On failure,
     clear `yt_browser` and set `yt_error` (`main.rs:118-126`).
   - `yt_browser` set AND `yt_session.is_some()` → set
     `yt_status = "connected via {browser}"` **with zero verification**
     (`main.rs:127-130`). This is a false-ready (§8, §10).

`main.rs:142-166` — **the BLOCKING `library_playlists()` probe**:
```rust
if app.yt_session.is_some() {
    let mut reachable = false;
    if let Some(s) = app.yt_session.as_mut() {
        match s.library_playlists() {      // BLOCKING, 3s deadline
            Ok(_) => reachable = true,     // Ok(EMPTY) still counts as reachable!
            Err(e) => {
                app.yt_session = None;     // <-- DISCARDS the session entirely
                app.yt_error = Some(...);
            }
        }
    }
    if !reachable && app.view == View::Youtube { app.view = View::Artists; }
}
```

### What happens when `library_playlists()` fails at launch?
- `app.yt_session = None` (`main.rs:151`). The session is **discarded for the
  entire app run**. The cookies file is NOT deleted, but there is NO code path
  that re-spawns from cached cookies after the probe fails. The `on_tick`
  respawn logic (`app.rs:1059-1104`) only fires when `yt_session.is_some()` and
  the child died — it never re-creates a `None` session.
- Result: any transient failure at launch (network blip, 3s timeout, an
  intermittent ytmusicapi parse error that escapes the fallback, IP rate-limit)
  strands the user in guest mode for the whole session. They must run
  `:yt auth browser <name>` again — which re-prompts the Keychain.

### What happens when it returns `Ok(empty)`?
- `reachable = true` (`main.rs:149`). The session is KEPT. `yt_status` already
  says "connected" (set at `main.rs:117/129`). The Y view will show an empty
  playlist list. This is the "connected but empty" state (§10).

### Why the probe is fragile
- `library_playlists` roundtrip deadline is **3 seconds** (`session.rs:883`).
- A cold ytmusicapi init (`yt.py:527-545`) does a network call to
  `music.youtube.com` to validate; on macOS the first sidecar also pays the
  Keychain read + cookie-file write. Under load / VPN / cold cache, 3s is tight.
- The `ytmusicapi init` failure path (`yt.py:528-545`) prints an error and sets
  `have=False`, then the sidecar stays alive serving ping/auth_status but errors
  every data request. So `library_playlists` returns an `Error` → session
  discarded.

---

## 4. Expiry / refresh / revocation

### Does ytmusicapi/yt-dlp expose token expiry? — **NO.**
- `auth_status` (`yt.py:328-330` and `yt.py:561-565`) returns
  `ok = _has_auth()`, which is `True` iff a `SAPISID`/`__Secure-3PAPISID` cookie
  **string exists** in the jar (`yt.py:205-216`). It does **not** check the
  cookie's `expires` attribute, does not probe the network, does not validate the
  SAPISIDHASH. An expired or revoked cookie still reports `ok=True`.
- `AuthStatus` (`proto.rs:80-86`) has `ok`, `premium`, `account` — but the sidecar
  sets **all three to the same value** `_has_auth()` (`yt.py:329-330`,
  `yt.py:562-564`). So `premium` and `account` are **not** real premium/account
  detection; they're identical to `ok`. A free account with a SAPISID cookie is
  reported as premium+account.

### How are expired cookies detected? — **They aren't.**
- There is no expiry check anywhere. The cookies.txt `expires` column is written
  (`yt.py:163`) but never read back for validation.
- Expired/revoked cookies manifest as **silent empty results**: YouTube
  downgrades the request to guest. `get_library_playlists()` for a guest returns
  `[]` (not an error). The sidecar's fallback further swallows any exception into
  `[]` (`yt.py:357-358`). So the Rust side sees `Ok([])` — indistinguishable from
  a genuinely-empty library.

### Is there any refresh? — **NO.** Just "fail → user re-auths."
- No refresh tokens, no re-login flow, no cookie renewal. The only recovery is
  the user running `:yt auth browser <name>` again (which re-reads the browser
  store, getting fresh cookies if the browser has re-authenticated with
  YouTube).

### Does revoked credential cause a silent retry loop? — **No loop, but silent.**
- `on_tick` respawn logic (`app.rs:1059-1104`) only triggers when the **child
  process** dies. A revoked credential does NOT kill the sidecar — it just makes
  data requests return empty/error. So there's no retry loop; there's just
  silent emptiness.
- The `_extract_with_retry` in `resolve_url` (`yt.py:304-322`) retries
  **transient network** errors (SSL EOF, timeout) up to 3x — but auth failures
  aren't transient and aren't retried there either.

---

## 5. Playlist fetching & pagination

### `library_playlists` (`yt.py:334-362`) — **NO PAGINATION**
```python
ps = ytm.get_library_playlists()   # no limit arg!
```
- `ytmusicapi.YTMusic.get_library_playlists(limit=25)` defaults to **25
  playlists**. There is no `limit` passed, no continuation token, no loop. A
  user with >25 library playlists sees only the first 25.
- Fallback `ytm.get_library()` (`yt.py:350`) also passes no limit.
- **Empty vs failed:** both produce `[]`. The primary path catches exceptions
  (`yt.py:346`) and the fallback catches exceptions and returns `[]`
  (`yt.py:357-358`). So a parse error, a network error that slips past, and a
  genuinely-empty library all yield `{"playlists": []}`. The Rust side
  (`session.rs:882-888`) returns `Ok(Vec)` for all three.

### `get_playlist` (`yt.py:363-365`) — **NO PAGINATION**
```python
p = ytm.get_playlist(arg.get("id", ""))   # no limit arg!
return {"tracks": [_track(t) for t in p.get("tracks", [])]}
```
- `ytmusicapi.YTMusic.get_playlist(playlistId, limit=100)` defaults to **100
  tracks**. No pagination loop. A playlist with >100 tracks is truncated to 100.

### `home_suggestions` (`yt.py:366-372`) — **NO PAGINATION**
- `ytm.get_home()` with no limit; iterates sections/contents. Whatever
  ytmusicapi returns is returned.

### What does an empty playlist look like vs a failed fetch?
- **Empty library / failed fetch:** `yt_lists` is empty (`app.rs:1144` sets it
  from the fetched `Vec<PlaylistSummary>`). col2 shows `"select a list to load
  its tracks"` (`columns.rs:256`) — **the same message** whether the library is
  genuinely empty or the fetch failed silently.
- **Explicit error:** `yt_error` set → col2 shows `"YT error: {e}"`
  (`columns.rs:249-250`). But the silent-empty path never sets `yt_error`.
- **Loading:** `yt_lists_loading = true` → col2 shows `"loading…"`
  (`columns.rs:251-252`). Set by `refresh_yt_lists` (`app.rs:1659`), cleared when
  playlists land (`app.rs:1145`) or on send error (`app.rs:1664`). If the
  sidecar dies mid-fetch and never responds, `yt_lists_loading` stays `true`
  forever (no timeout on the fire-and-forget path).

### Page size
- 25 for library playlists, 100 for playlist tracks — both ytmusicapi defaults,
  not explicit.

---

## 6. Cache reads / writes

### Playlist cache — **NONE.**
- `yt_lists` (`app.rs:188`) is in-memory only. Rebuilt from network every time
  the Y view is opened (`switch_view` → `refresh_yt_lists`, `input.rs:610-617`).
  No disk persistence. Lost on restart.
- `loaded_yt_lists` (`app.rs:192`) — in-memory set of expanded list ids.

### Video-id cache — in-memory only.
- `track_cache: HashMap<video_id, RemoteTrack>` (`session.rs:205`). Populated by
  search/get_playlist/watch_playlist results via `cache_track` (`session.rs:615-628`).
  Not persisted. Lost on restart.

### URL cache (`CachedResolve`) — in-memory only, cap 2.
- `url_cache: Vec<CachedResolve>` (`session.rs:209`), `URL_CACHE_CAP = 2`
  (`session.rs:262`). Holds current + next. Evicts oldest (`session.rs:388-390`).
  Each entry has `fast` (AAC 129k) and `premium` (AAC 256k) slots
  (`session.rs:43-48`). Not persisted. Lost on restart.

### Stale handling — **NONE.**
- `expires_at` is always `None` from the sidecar (`yt.py:504`), so no expiry-based
  eviction. Entries evict only by the cap (LRU-ish, actually FIFO by
  `remove(0)`). A stale URL (YouTube revokes stream URLs after ~6h) would be
  served until evicted; mpv would fail to play it, surfacing a player error —
  but the cache doesn't self-heal.

### `state.db` persistence
- Only `LayoutState` (view focus, widths, volume, modes, `yt_browser`) and
  `playlists` (local playlist definitions, NOT YouTube playlists) are persisted
  (`state.rs:113-165`, `main.rs:183-193`). No YT data persisted.

---

## 7. Background synchronization

### Sync model — fire-and-forget sends + non-blocking drain.
- `on_tick` (`app.rs:1056`) is called every poll cycle from the event loop.
  - It calls `session.drain_paired()` (`app.rs:1121`, `session.rs:698-709`) —
    non-blocking, drains all ready responses from the mpsc channel.
  - It folds `pending_playlists`/`pending_suggestions`/`pending_tracks`/
    `pending_search`/`pending_errors`/`pending_premium_url` into app state
    (`app.rs:1123-1290`).
- Fetches are initiated by fire-and-forget sends: `send_refresh`
  (`session.rs:714`), `send_get_playlist` (`session.rs:776`), `send_resolve`
  (`session.rs:726`), `send_resolve_premium` (`session.rs:746`), `send_search`
  (`session.rs:802`). All non-blocking.

### Can sync be cancelled/superseded? — **Partially, and not safely.**
- `refresh_yt_lists` (`app.rs:1653-1675`) clears `pending_playlists`/
  `pending_suggestions` (`app.rs:1661-1662`) before sending. But it does **NOT**
  check whether a refresh is already in flight, and `send_refresh`
  (`session.rs:714-720`) has **no inflight guard** (unlike `send_get_playlist` →
  `playlist_inflight`, `send_search` → `search_inflight`). So multiple
  refreshes can stack in the FIFO `pending` queue, and their responses are
  applied in arrival order — a stale refresh can overwrite a fresh one.
- There are **no generation ids**. No request tagging beyond the `Pending`
  variant kind. A refresh's response is paired by FIFO order, not by a token.

### Can it be cancelled? — **No.**
- There is no cancel/abort. The sidecar processes every request sent. A stale
  response (user switched away from Y view) is still applied to `yt_lists`
  (`app.rs:1126-1148` applies it unconditionally when `got_playlists.is_some()`).

### Is sync async or synchronous in `on_tick`?
- The **drain** is synchronous in `on_tick` (non-blocking). The **sends** happen
  on view-enter (`refresh_yt_lists`) and on focus changes (`send_get_playlist`
  via the lazy-load block, `app.rs:1351-1367`). Neither blocks the poll loop.
- The ONE blocking call in the launch path is `library_playlists()` at
  `main.rs:148` (§3).

---

## 8. Error mapping & UI state

### `AuthStatus` enum (`proto.rs:80-86`)
```rust
pub struct AuthStatus {
    pub ok: bool,
    pub premium: bool,
    pub account: bool,
}
```
- The sidecar sets all three to the same value `_has_auth()` (`yt.py:329-330`,
  `yt.py:562-564`). So `premium` and `account` are **always** equal to `ok`.
  There is no real premium detection anywhere.

### How errors become `yt_status` / `yt_error`
- `yt_error` (`app.rs:193`) — set on failures; shown in footer (`footer.rs:22-26`)
  and col2 (`columns.rs:249-250`).
- `yt_status` (`app.rs:197`) — set on successes/transient info; shown in footer
  (`footer.rs:27-31`).
- Both are **transient** — overwritten by the next status/error. There is no
  persistent "auth state" indicator; the footer flips between hints/status/error
  each tick. So an auth failure message can be overwritten by a later unrelated
  status (e.g. `"upgraded to AAC 256k"`, `app.rs:1330`), losing the failure
  signal.

### "Connected" claimed before data is usable — **YES, in 5 places.**
Every `yt_status = "connected…"` assignment happens **before** any
`library_playlists` fetch succeeds:

1. `main.rs:117` — after `spawn_browser` success, before any data fetch.
2. `main.rs:129` — after cached cookies loaded + `yt_session.is_some()`, with
   **no verification at all** (doesn't even call `auth_status`).
3. `app.rs:988` — after `set_cookies` (pasted), before any fetch.
4. `app.rs:1014` — after `set_browser`, before any fetch.
5. `app.rs:1093` — `"YT: sidecar restarted"` after an auto-respawn, no
   verification.

In all five, "connected" means "the sidecar process spawned and we have a
cookie string" — NOT "we can fetch your library." The actual library fetch
happens later (`refresh_yt_lists` on Y-view enter, or the launch probe), and
its result does **not** update `yt_status` — a successful fetch leaves
`yt_status` as whatever it was; a failed fetch sets `yt_error` (which can then
be overwritten).

### Is "connected" claimed before data is usable? — **YES.**
The launch probe (`main.rs:148`) is the only place that verifies reachability,
and even it only checks `Ok`/`Err`, not data presence. After it, `yt_status`
still says "connected" regardless of whether playlists arrived. The Y-view
col2 body (`columns.rs:249-259`) checks `yt_error` → `yt_lists_loading` →
`yt_session.is_none()` → `ids.is_empty()`, but never cross-references
`yt_status`. So the footer can say "connected" while col2 says "select a list"
(empty) — the two states are decoupled.

---

## 9. Logout & account switching

### `:yt logout` → `app.yt_logout()` (`app.rs:1384-1393`)
```rust
pub fn yt_logout(&mut self) {
    let p = crate::yt::session::cookies_file();
    let _ = std::fs::remove_file(&p);          // deletes cookies file
    self.yt_browser.clear();                    // clears browser choice
    if let Some(session) = self.yt_session.as_mut() {
        let _ = session.clear_cookies(&self.yt_python, &self.yt_script);
    }
    self.yt_status = Some("YT auth: logged out (guest mode)".into());
    self.yt_error = None;
}
```
- `cookies_file()` deleted (`app.rs:1386`). ✓
- `yt_browser` cleared (`app.rs:1387`). ✓ (persisted on clean exit,
  `main.rs:191`).
- `session.clear_cookies` (`session.rs:435-440`): clears `cookies` + `browser`
  fields and **respawns the sidecar guest** (`Sidecar::spawn(…, None, None,
  None)`). ✓
- `yt_status = "logged out (guest mode)"`. ✓

### What it does NOT clear — **stale data survives logout.**
- `yt_lists` is NOT cleared. The Y view continues to show the old playlists until
  a `refresh_yt_lists` is triggered (next Y-view enter). Since the session is
  now guest, that refresh returns empty/suggestions-only.
- `loaded_yt_lists` is NOT cleared.
- `track_cache` is NOT cleared (video_id → metadata survives; minor, keyed by
  id).
- `url_cache` is NOT cleared (surviving stream URLs remain playable; minor).
- `pending_playlists`/`pending_suggestions`/`pending_tracks`/`pending_search`/
  `pending_errors` are NOT cleared — an in-flight refresh can land AFTER logout
  and re-populate `yt_lists` with the now-logged-out account's data
  (`app.rs:1126-1148` applies unconditionally).

### Account switching (`:yt auth browser <other>`)
- `apply_yt_browser` (`app.rs:995-1015`) respawns the sidecar with the new
  browser. It does **NOT** clear `yt_lists`/`loaded_yt_lists` — the old account's
  lists stay until the next `refresh_yt_lists`. The launch probe / Y-view-enter
  refresh is the only thing that refreshes them.

### Crash-safety of logout
- `state.db` is only written on clean exit (`main.rs:183-192`). If the app
  crashes after `yt_logout`, `yt_browser` in `state.db` is still the old value
  → next launch tries to read the browser. But the cookies file was deleted, so
  launch hits the `cached_cookies.is_none()` branch (`main.rs:110`) →
  `spawn_browser` (Keychain prompt) → re-persists. So logout is eventually
  consistent but not crash-atomic.

---

## 10. The "connected but empty" mystery — EXACT root cause

There are **two interacting root causes**:

### Root cause #1 — Repeated login: the launch probe discards the session on any error
**`main.rs:148-158`** (the BLOCKING `library_playlists()` probe):
```rust
match s.library_playlists() {   // 3s deadline (session.rs:883)
    Ok(_) => reachable = true,
    Err(e) => {
        app.yt_session = None;   // <-- session discarded for the whole run
        app.yt_error = Some(...);
    }
}
```
- ANY error at launch — a 3s timeout, a transient network blip, the
  intermittent ytmusicapi `singleColumnBrowseResultsRenderer` parse error, an IP
  rate-limit — sets `app.yt_session = None`. The cookies file is not deleted, but
  **no code path re-spawns from cached cookies after the probe fails.** The
  `on_tick` respawn logic (`app.rs:1059-1104`) only respawns an existing
  `Some(session)` whose child died; it never re-creates a `None` session.
- The user is stranded in guest mode for the entire app run. To recover they
  must run `:yt auth browser <name>` again, which re-prompts the macOS Keychain
  (since the cache was loaded but the sidecar was discarded, the new spawn reads
  the browser fresh). **This is why the user repeatedly logs in.**
- The 3s deadline (`session.rs:883`) is too short for a cold ytmusicapi init
  (which does a network call to validate, `yt.py:527-545`) plus the macOS
  Keychain read + cookie-file write that happens on the first sidecar spawn.

### Root cause #2 — Connected but empty: `auth_status` lies + empty==failed conflation
**`yt.py:329-330`** (`auth_status`):
```python
ok = _has_auth()   # True iff SAPISID cookie STRING exists — not validity
return {"auth": {"ok": ok, "premium": ok, "account": ok}}
```
- `_has_auth()` (`yt.py:205-216`) checks **presence** of the SAPISID cookie,
  not whether it's expired/revoked. So `auth_status` reports `ok=True` even for
  an expired credential.
- `yt_status` is set to "connected" based on the sidecar **spawning**
  (`main.rs:117/129`, `app.rs:988/1014`) — before any data fetch verifies the
  credential actually works.
- When the cookie is expired/revoked, YouTube **silently downgrades the request
  to guest**. `get_library_playlists()` for a guest returns `[]` (not an error).
  The sidecar's `library_playlists` handler (`yt.py:343-362`) further catches
  exceptions and returns `[]` on the fallback (`yt.py:357-358`).
- So: expired cookie → sidecar spawns fine → `library_playlists` returns
  `Ok([])` (empty, no error) → launch probe passes (`main.rs:149`, `Ok(_)` →
  reachable, **does not check emptiness**) → `yt_status` says "connected" → Y
  view shows empty playlists → **user sees "connected" but nothing is there.**
- There is no expiry detection, no refresh, no revocation signal. The user
  eventually re-logs in (`:yt auth browser`), getting a fresh SAPISID from the
  browser — which works until the cookie expires again. Since browser cookies
  for YouTube can expire in hours/days (especially `__Secure-3PAPISID`), this
  recurs frequently.

### Contributing factor — no pagination (`yt.py:345`)
`get_library_playlists()` with no `limit` defaults to **25**. A user with >25
playlists sees a truncated list, which can read as "where are my playlists?" /
"empty-ish." `get_playlist` defaults to **100** tracks, truncating large
playlists. Not the primary bug, but compounds the "something is missing"
perception.

### Contributing factor — silent failure surfaces
- `yt_lists_loading` (`app.rs:189`) can hang `true` if the sidecar dies
  mid-fetch (no timeout on the fire-and-forget path). col2 then shows
  "loading…" forever.
- The footer `yt_status`/`yt_error` is transient and can be overwritten by
  unrelated later messages, losing the failure signal.

---

## Every "false ready/connected" location (file:line)

| # | file:line | Statement | Why it's a false-ready |
|---|-----------|-----------|----------------------|
| 1 | `main.rs:117` | `yt_status = "YT auth: connected via {browser}"` | Set after `spawn_browser` success, **before** any data fetch. Sidecar spawning ≠ credential valid. |
| 2 | `main.rs:129` | `yt_status = "YT auth: connected via {browser}"` | Set when `yt_browser` non-empty AND `yt_session.is_some()`, with **zero verification** — doesn't even call `auth_status`. Just "we have a session object." |
| 3 | `app.rs:988` | `yt_status = "YT auth: connected via pasted cookies"` | Set after `set_cookies` respawn, **before** any fetch. Cookies may be expired/revoked. |
| 4 | `app.rs:1014` | `yt_status = "YT auth: connected via {browser}"` | Set after `set_browser` respawn, **before** any fetch. Browser cookies may be expired. |
| 5 | `app.rs:1093` | `yt_status = "YT: sidecar restarted"` | Set after an auto-respawn, no verification of the new sidecar's auth. |
| 6 | `yt.py:329-330` | `auth_status: ok=premium=account=_has_auth()` | Reports `ok=True` on cookie **presence**, not validity. Expired/revoked SAPISID → `ok=True`. |
| 7 | `yt.py:562-564` | Same `auth_status` in the main loop | Same lie, on the `auth_status` command path. |
| 8 | `main.rs:149` | `Ok(_) => reachable = true` | Launch probe treats `Ok(empty)` as reachable — no data-presence check. Empty library (expired cookie → guest → empty) passes. |
| 9 | `proto.rs:80-86` + `yt.py:329` | `AuthStatus { ok, premium, account }` all identical | `premium`/`account` are not real detections; a free account with a SAPISID reports premium+account. |
| 10 | `app.rs:1126-1148` | `yt_lists = lists` (unconditional apply) | A stale refresh response (user switched away / logged out) still overwrites `yt_lists` — no generation/ownership check. |

---

## Summary

### Root causes of repeated login
- **`main.rs:151`** — the launch `library_playlists()` probe sets
  `app.yt_session = None` on ANY error, discarding the session for the whole
  run with no recovery path. Cookies file survives but is never re-loaded into
  a session. User must `:yt auth browser` again (re-prompts Keychain).
- **`session.rs:883`** — the probe's 3s deadline is too short for cold
  ytmusicapi init + macOS Keychain read, making the discard fire on slow-but-
  healthy machines.

### Root causes of "connected but empty"
- **`yt.py:329-330`** — `auth_status` reports `ok=True` on SAPISID cookie
  **presence**, not validity. Expired/revoked cookies report connected.
- **`main.rs:129`** — `yt_status="connected"` set with zero verification (just
  `yt_session.is_some()`), so the footer claims connected before any data
  fetch.
- **`main.rs:149`** — launch probe treats `Ok(empty)` as reachable; an expired
  cookie silently downgraded to guest returns an empty library, which passes.
- **`yt.py:357-358`** — the `library_playlists` fallback swallows exceptions
  into `[]`, erasing the distinction between "failed" and "genuinely empty."

### Top 5 reliability defects (ranked)
1. **Launch probe discards session on any error (`main.rs:151`)** — a single
   transient failure at startup strands the user in guest mode for the whole
   session, forcing re-login. Should retry / fall back to cached cookies /
   degrade gracefully instead of `None`-ing the session.
2. **`auth_status` lies (`yt.py:329-330`)** — reports connected on cookie
   presence, not validity. No expiry/revocation detection anywhere. Expired
   credentials silently produce empty results, and the UI claims "connected."
3. **No pagination (`yt.py:345`, `yt.py:364`)** — `get_library_playlists()`
   defaults to 25, `get_playlist` to 100, with no continuation loop. Libraries
   / playlists are silently truncated.
4. **False-ready status assignments (`main.rs:117/129`, `app.rs:988/1014`,
   `app.rs:1093`)** — "connected" set on spawn, before data fetch; never
   corrected by fetch outcome. Decoupled from actual data availability.
5. **`send_refresh` has no inflight guard / no generation ids
   (`session.rs:714-720`)** — multiple refreshes can stack and apply out of
   order; a stale refresh overwrites fresh data. `yt_lists_loading` can hang
   `true` if the sidecar dies mid-fetch (no fire-and-forget timeout).
