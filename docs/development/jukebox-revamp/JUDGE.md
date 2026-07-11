# JUDGE — Independent Judge Reports

(Reserved for fresh-thread judge reports. See ACCEPTANCE.md rubric.)

## Candidate 1 — pending

## Judge 1: Black-box product judge

**Date:** 2026-07-12
**Candidate commit:** 33ffecc (`revamp/product-polish`)
**Method:** Fresh, read-only evaluation. Generated own evidence (ran all gates, read source + tests + snapshots, inspected rendered states). Did not read implementer work logs or prior judge scores before forming this assessment.

---

### Mandatory release gates (all PASS)

| Gate | Command | Result |
|------|---------|--------|
| fmt | `cargo fmt --check` | ✅ PASS (exit 0) |
| clippy | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ PASS (exit 0) |
| test | `cargo test --all-features` | ✅ PASS — 360 tests, 0 failed (34 test binaries) |
| build | `cargo build --release` | ✅ PASS (exit 0) |
| bats | `bats scripts/test/*.bats` | ✅ PASS — 30 tests, 0 failed |

---

### Critical journey verification (A–H)

Each journey was verified with deterministic fixtures and fake sidecars (no real credentials, no network).

**Journey A — local-only first use** ✅
- Catalog load, BM25 search (Tantivy+Lindera, cross-script/fuzzy), Miller columns, mpv/afplay playback, transport (shuffle/repeat/continue/queue) — all existing + tested (`tests/transport.rs` 10 tests, `tests/search.rs` 4 tests, `tests/tui.rs` 5 tests).
- Lyrics now implemented: LRC parsing (synced/multi-timestamp/metadata/ms/minutes>59), plain parsing, sidecar `.lrc`/`.txt`, embedded FLAC tags, ytmusicapi `get_lyrics` (`tests/lyrics.rs` 21 tests, `src/lyrics/mod.rs` 504 lines).
- Queue UI wired: `e` enqueue, `x` remove, `:queue clear` — all tested (`tests/queue_playlist_ui.rs` 22 tests).
- Playlist add `a` + delete `d` — wired and tested.
- Command history: Up/Down recall, draft preservation, dedup-adjacent, bounded at 100, unicode, persistence round-trip (`tests/command_history.rs` 13 tests).
- State persistence (layout, playlists, command history) restored on launch (`src/main.rs:86-170`).

**Journey B — YouTube first login → playlists → play → restart** ✅
- Browser auth reads cookies once (single Keychain prompt), persists decrypted cookies to 0600 file (`yt.py:153-216`, `session.rs:608-625`).
- Session restore on restart: `load_cookies()` reads the persistent cache — no re-prompt (`main.rs:69-71,127-160`). No `yt_session = None` anywhere in `main.rs` (grep confirmed zero matches — the old repeated-login root cause is gone).
- Playlists load with **full pagination** (`yt.py:404` `get_library_playlists(limit=None)`, `yt.py:426` `get_playlist(..., limit=None)`). `tests/pagination_cache.rs::pagination_large_library` verifies 30 playlists aren't truncated to 25.
- Truthful state machine: `Ready` reached ONLY after a successful data fetch (`tests/provider_state.rs::refresh_success_promotes_to_ready`). Footer never says "connected" before fetch (`tests/provider_state.rs::footer_status_text_never_says_connected_before_fetch`).
- Fire-and-forget refresh at launch (non-blocking) — `main.rs:192-198`.

**Journey C — expired/revoked authorization** ✅
- `auth_status` now probes validity: `ytm.get_home(limit=1)` — distinguishes `ok` (cookie present) from `valid` (probe succeeds) from `expired` (auth-flavored error) (`yt.py:677-716`). `premium`/`account` always `False` (conservative — never a false claim).
- `AuthExpired` state distinct from `ProviderError` (`tests/provider_state.rs::auth_error_demotes_to_auth_expired`). Recovery hint: "run :yt auth browser <name>".
- Empty vs failed distinguished: `library_playlists` re-raises exceptions (no silent `[]`) (`yt.py:403-416`, `tests/pagination_cache.rs::empty_vs_failed_distinguished`).
- No false ready: `AuthenticatedNotSynced.is_ready()` == false (`state.rs:121-123`, `tests/provider_state.rs::no_false_connected_authenticated_not_synced_is_not_ready`).

**Journey D — offline YouTube use** ✅
- Cached playlists loaded at launch → `ReadyStale` (not `Failed`); label "offline — showing cached (press R to retry)" (`tests/pagination_cache.rs::offline_shows_cached_marked_stale`, snapshot `state_snapshots__offline_cache.snap`).
- `R` key retries (`input.rs:128` → `retry_yt_probe()`, `tests/pagination_cache.rs::r_key_can_retry_from_rate_limited`).
- Cache cleared on logout (`tests/pagination_cache.rs::cache_cleared_on_logout`).

**Journey E — hybrid playback** ✅
- ISRC exact match + title/artist fuzzy (gate 0.88, floor 0.80) — `tests/source_match.rs` 11 tests.
- Mixed mode plays local on match (`tests/e2e_yt.rs::mixed_mode_matches_local_on_isrc_and_plays_local`).
- Dead remote track skipped, not halt (`tests/e2e_yt.rs::dead_remote_track_is_skipped_not_halt`).
- CONT=YouTube with no session stops cleanly, no panic (`tests/e2e_yt.rs::cont_youtube_with_no_session_stops_cleanly_no_panic`).

**Journey F — command workflow** ✅
- Up/Down recall, draft preservation, dedup-adjacent, bounded at 100, unicode, persistence — all tested (`tests/command_history.rs`).
- Unknown command feedback: `execute_command` `_ =>` arm sets `yt_error = "unknown command: :{cmd}"` (`input.rs:758-765`). No silent `_ => {}` (grep confirmed).
- `:q`/`:quit`, `:yt auth`, `:yt auth browser <name>`, `:yt logout`, `:yt setup`, `:queue clear`, `:diag` all wired (`input.rs:718-767`).

**Journey G — lyrics** ✅
- LRC parsing: synced, multi-timestamp, metadata tags, milliseconds, minutes>59, blank lines, Unicode/CJK (`tests/lyrics.rs` 21 tests).
- Three sources: embedded FLAC (`metaflac --show-tag`), sidecar `.lrc`/`.txt`, ytmusicapi `get_lyrics` (two-step: browseId → lyrics, ms→s conversion) (`yt.py:573-626`).
- States: Loading, Available(synced/plain), NotFound, error — snapshot-tested (`state_snapshots.rs::state_lyrics_loading/available/unavailable`).
- Stale-discard via generation guard: `request_lyrics` bumps `lyrics_gen`; on_tick applies only if `track_id == vid && gen == self.lyrics_gen` (`app.rs:1658-1688`, `app.rs:2677-2688`). **Implementation is correct.**
- No fabricated lyrics: empty payload → NotFound, never invented text (`tests/lyrics.rs::no_fabricated_lyrics`).
- Non-blocking: fire-and-forget via `pending_lyrics`, drained on_tick.
- Scroll: `j`/`k`/PgUp/PgDn/`g`/`G` in lyrics overlay (`input.rs:625-639`).

**Journey H — degraded terminal** ✅
- Responsive: 120×24 (wide), 80×24 (standard), 70×24 (narrow), 50×18 (too-small) — snapshot-tested (`tests/layout.rs` 4 tests + `state_snapshots.rs::state_too_small`).
- Too-small shows "terminal too small — resize or press q to quit" (hard assertion `layout.rs:112-115`).
- NO_COLOR: theme collapses to `Color::Reset`; state icons are ASCII-safe (`[!]`, `[err]`, `[reauth]`, etc.) so states are distinguishable without color (`tests/theme.rs::no_color_reads_env`, `state.rs:224-237`).
- CJK width handling (`tests/theme.rs::disp_width_counts_cjk_as_two`, `display_width_zero_width`).

---

### Findings

#### F1 — P2: Tautological stale-lyrics test (test depth gap)
- **Severity:** P2
- **File:** `tests/lyrics.rs:318-335` (`stale_lyrics_dropped_on_track_change`)
- **Repro:** Read the test body — it asserts `assert_ne!("new_vid", "old_vid")` on two hardcoded string literals. This is a tautology that never exercises the actual generation guard in `app.rs:1670` (`track_id == vid && gen == self.lyrics_gen`). The companion test `lyrics_gen_increments_on_request` (line 338) similarly tests `wrapping_add` on a local variable, not the App's counter.
- **Impact:** The stale-discard **implementation is correct** (the guard exists and is wired in `on_tick`), but the test gives false confidence — a regression that broke the guard would not be caught.
- **Acceptance:** Replace with a test that drives `request_lyrics` for track A, then `request_lyrics` for track B (bumping `lyrics_gen`), delivers A's response via the fake sidecar, pumps `on_tick`, and asserts the overlay still shows B's Loading state (not A's lyrics).

#### F2 — P3: README keybindings incomplete
- **Severity:** P3
- **File:** `README.md:143-152` (Keybindings section)
- **Repro:** The README lists core keys but omits the new revamp keybindings: `L` (lyrics), `D` (diagnostics), `e` (enqueue), `x` (remove from queue), `d` (delete playlist), `R` (retry YouTube connection). The in-app Help overlay (`?`) does list all of these (`src/tui/view/overlay.rs:336-363`).
- **Impact:** A user reading the README learns an incomplete keymap. The in-app help is complete, so the keys are discoverable.
- **Acceptance:** Add the missing keybindings to the README table.

#### F3 — P3: mpv socket path predictable when XDG_RUNTIME_DIR unset
- **Severity:** P3
- **File:** `src/config.rs:38-40`
- **Repro:** When `XDG_RUNTIME_DIR` is unset (common on macOS, the primary target), the mpv socket falls back to the hard-coded `/tmp/jukebox-mpv.sock` — a predictable path with symlink/race risk on shared systems. The socket is a local control channel (no secrets transmitted); on single-user macOS the risk is minimal.
- **Impact:** No credential leak. A local attacker on a multi-user system could pre-create a symlink to redirect the socket, but mpv validates the peer. Minimal real-world risk on the primary target.
- **Acceptance:** Use `$TMPDIR` (per-user on macOS) or a random suffix instead of the hard-coded `/tmp` path.

#### F4 — P3: Premium detection conservative (known limitation)
- **Severity:** P3 (documented limitation, not a defect)
- **File:** `scripts/yt/yt.py:382,687,704`
- **Repro:** `auth_status` always reports `premium: False` and `account: False` even for a valid Premium account. The `resolve_url` path DOES detect premium (`abr >= 256 && quality == "premium" && authed`, line 563), so the progressive-upgrade still delivers 256k AAC when available (tested in `tests/e2e_yt.rs::progressive_upgrade_swaps_player_to_premium_and_resumes`).
- **Impact:** No false success claim. The conservative choice (comment line 378-379) is deliberate: "we report False so the Rust side never acts on a false 'premium' claim."
- **Acceptance:** Documented as a known limitation; no action required for release.

---

### Per-rubric scoring

| Category | Max | Score | % | Notes |
|----------|-----|-------|---|-------|
| Functional correctness and complete core journeys | 25 | 24 | 96% | All journeys A–H pass with deterministic fixtures. F1 is a test-depth gap, not a functional defect. |
| Provider, auth, persistence, and recovery reliability | 20 | 19 | 95% | Truthful state machine, no forced re-login, full pagination, empty≠failed, offline cache, logout clears state. F4 conservative premium (no false claim). |
| UX clarity, discoverability, interaction consistency | 20 | 19 | 95% | Footer derives from state machine, TTL auto-clear + dedup, unknown-command feedback, diagnostics overlay, all keys wired. F2 README incomplete (in-app help complete). |
| Playback correctness and responsiveness | 10 | 10 | 100% | All hot paths fire-and-forget (discover/playlist/auto-advance/audio switch), progressive upgrade with resume, gapless preload, two-tier resolve. |
| Automated test depth and determinism | 10 | 9 | 90% | 360 Rust + 30 bats tests, all deterministic. Fake-sidecar pattern race-free. F1 tautological test is the only depth gap. |
| Security and privacy | 5 | 4 | 80% | Cookies 0600, `/tmp/.config` refused for secrets, sidecar stderr logged, secret redaction, CLI sanitization, corrupt-DB recovery. F3 mpv socket predictable (no credential leak). |
| Terminal compatibility and accessibility | 5 | 5 | 100% | 4 size breakpoints, too-small message, NO_COLOR with ASCII icons, CJK width. |
| Maintainability and documentation | 5 | 4 | 80% | Excellent doc comments (every "why" explained), no TODO/FIXME/HACK in source, recon reports. F2 README keybindings incomplete. |
| **Total** | **100** | **94** | **94%** | |

---

### Verdict: **PASS**

All mandatory release gates are met:
- ✅ No P0 or P1 defect. All pre-revamp P0/P1 defects (false "connected", forced re-login, no pagination, lyrics missing, command history missing, blocking hot path, silent unknown commands, stale status messages, sidecar stderr null'd, temp cookie leaks, sidecar spawn panic) are fixed and regression-tested.
- ✅ No unresolved security or credential-leak issue. Cookie secrets protected (0600, `/tmp/.config` refused, redaction). F3 is a non-credential hardening item.
- ✅ All required local, YouTube, and hybrid journeys pass with deterministic fixtures (no external provider limitation blocks any core journey).
- ✅ YouTube session restoration and playlist display pass in deterministic tests (`pagination_large_library`, `offline_shows_cached_marked_stale`, `refresh_then_on_tick_populates_yt_lists`).
- ✅ No false connected/ready state (`YtState::is_ready()` only true post-fetch; footer derives from state machine; 7 tests verify).
- ✅ Command history works and tested (13 tests: recall, dedup, bound, unicode, persistence).
- ✅ Lyrics functional, truthful, non-blocking (21 tests; generation guard; no fabrication; fire-and-forget).
- ✅ Input responsive during slow provider and lyrics operations (4 nonblocking tests; all hot paths fire-and-forget).
- ✅ Full fmt/clippy/test/build gates green.
- ✅ Snapshots inspected (21 state + 4 layout insta snapshots, committed).
- ✅ No rubric category < 80% (lowest: Security 80%, Maintainability 80%).
- ✅ Score 94 ≥ 90.

**Findings to address (non-blocking for PASS, recommended before merge):**
1. F1 (P2): Replace the tautological `stale_lyrics_dropped_on_track_change` test with a real generation-guard exercise.
2. F2 (P3): Add missing keybindings (`L`, `D`, `e`, `x`, `d`, `R`) to the README.
3. F3 (P3): Use `$TMPDIR` or a random suffix for the mpv socket path fallback.

---

## Judge 2: Adversarial engineering judge

**Date:** 2026-07-12
**Candidate commit:** 33ffecc (`revamp/product-polish`)
**Method:** Fresh, independent, read-only evaluation in a new thread. Generated own evidence: ran every gate myself, read the risky-boundary source files (`yt/session.rs`, `yt/state.rs`, `yt/cache.rs`, `state.rs`, `tui/event.rs`, `audio.rs`, `lyrics/mod.rs`, `yt/sidecar.rs`, `player.rs`, `tui/app.rs`), inspected tests, and built two temporary external probes **outside** the candidate tree (per the isolation protocol) to falsify two claims that reading alone could not settle — (a) whether ratatui 0.30 passes raw control chars to its buffer (TUI escape-injection) and (b) whether a `player.load()` failure diverges `now_playing` from the backend. Did not read Judge 1's section before forming my assessment; findings below are independent and only reconciled against F1–F4 afterward.

---

### Mandatory release gates (all PASS — independently re-run)

| Gate | Command | Result |
|------|---------|--------|
| fmt | `cargo fmt --check` | ✅ PASS (exit 0) |
| clippy | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ PASS (exit 0) |
| test | `cargo test --all-features --no-fail-fast` | ✅ PASS — 360 tests, 0 failed (37 test binaries) |
| build | `cargo build --release` | ✅ PASS (exit 0) |
| bats | `bats scripts/test/*.bats` | ✅ PASS — 30 tests, 0 failed |

---

### Adversarial probes — what I specifically tried to break

| # | Area | What I did | Result |
|---|------|------------|--------|
| 1 | Stale async results | Read `apply_pair` generation guards (`session.rs:759-784`), `refresh_gen` bump on `send_refresh` + `clear_all_caches`, `lyrics_gen` guard in `on_tick` (`app.rs:1658-1688`). | ✅ Guarded. A stale refresh/lyrics response is dropped when `gen != current`. Verified by `tests/pagination_cache.rs` + `tests/provider_state.rs`. |
| 2 | Expired credentials | Read `retry_yt_probe` (`app.rs:2093-2147`) + `auth_status` sidecar probe (`yt.py:677-716`). | ✅ Truthful. Auth-expired detected via error-string keywords; degrades to `ProviderError` (retryable) if keywords miss — never false-ready. |
| 3 | Pagination | `grep limit scripts/yt/yt.py` — `get_library_playlists(limit=None)` (L404), `get_playlist(..., limit=None)` (L426). | ✅ Full pagination; `pagination_large_library` (30>25) guards truncation. |
| 4 | Offline recovery | `load_yt_lists_from_cache` + launch path (`main.rs:172-210`). | ✅ Cached lists → `ReadyStale`; `offline_shows_cached_marked_stale` passes. |
| 5 | Queue / now-playing divergence | **Built an external probe** (`/tmp/opencode/load-fail-probe`) with a `Player` whose `load()` returns `Err`, called `app.play_selected()`. | ❌ **DIVERGENCE CONFIRMED** — see A1 below. |
| 6 | Terminal escape injection (CLI) | Read `main.rs:279-292` + `lib.rs:25-36`. | ✅ CLI output routed through `sanitize_for_terminal`; `cli_output_sanitizes_control_chars` passes. |
| 7 | Terminal escape injection (TUI) | **Built an external probe** (`/tmp/opencode/escape-probe`) rendering `\x1b[2J…` via ratatui 0.30 `set_line`/`set_string` into a `Buffer`. | ✅ ratatui **drops** the ESC byte (zero-width) — no control char reaches the buffer/crossterm. TUI is safe by construction. |
| 8 | Secret leakage | Read `redact` (`event.rs:146-186`) + `log_to_file` rotation + `no_secret_in_logs`. | ⚠️ Partial — see A3 below. |
| 9 | Migration safety | `state.rs:50-98` `open_at` corrupt-DB auto-recovery + schema_version wipe. | ✅ `corrupt_db_recovers_to_defaults` passes; re-opens fresh after removing garbage. |
| 10 | Child-process cleanup | `Sidecar::Drop` (kill+wait), `MpvPlayer::Drop` (kill+wait+remove socket), `AfplayPlayer::Drop` (kill_current), panic hook + `TerminalGuard`. | ✅ All children reaped on exit/panic. |
| 11 | Large-input responsiveness | `track_by_id_fast` HashMap (`app.rs:570`), `track_rows` uses it (`columns.rs:632`). `TRACK_CACHE_CAP=256` LRU (`session.rs:366`). | ✅ O(1) lookups; `track_by_id_fast_is_o1_hashmap` + `track_cache_bounded_at_cap` pass. |
| 12 | Blocking calls in hot path | `grep` for sync `roundtrip`/`resolve_url` call sites in `src/tui`+`src/main`. | ✅ Only sync sidecar call on the hot path is `retry_yt_probe` (R key, explicit, ~1-3s). Launch + all per-tick fetches are fire-and-forget. `resolve_url` (sync, 15s) has **zero** call sites in `src/`. |
| 13 | `unwrap`/`expect` on external data | `grep` across `src/`. | ✅ The two `yt_session.as_mut().unwrap()` (`app.rs:1353,1450`) are inside `if let Some(session)=…` guards; `session.rs:526` `expect("just inserted")` is a local invariant; `proto.rs:53` serializes internally-built requests. No panic on external data. |
| 14 | `unsafe` blocks | `grep` across `src/`. | ✅ Only in `audio.rs` (CoreAudio FFI) and `event.rs` (signal registration) — both necessary and bounded. |

---

### Findings

#### A1 — P2: `player.load()` failure leaves `now_playing` set; UI/backend divergence with no auto-recovery
- **Severity:** P2 (medium — edge-case trigger, no data/security impact, but no auto-recovery)
- **File:** `src/tui/app.rs:813-814` (local path) and `src/tui/app.rs:1016-1017` (remote path)
- **Repro (independent, outside the candidate tree):**
  ```
  # /tmp/opencode/load-fail-probe — a Player impl whose load() returns Err
  app.play_selected();
  => now_playing_is_some=true
  => player_is_playing=false
  => DIVERGENCE_CONFIRMED: UI shows a now-playing track while the backend is NOT playing
  ```
  The code path: `let _ = self.player.load(&path);` discards the `Result<()>`, then `self.now_playing = Some(TrackSource::Local { track_id: id });` runs unconditionally. `MpvPlayer::load_at` resets `position`/`duration` to `None` *before* the failing `send()`, so the player bar shows the track title with no position/duration; `is_playing()` returns false (child not running); `track_ended()` returns false (no end-file event for a never-started track) → **auto-advance never fires** and the user is stuck on a silent "playing" track.
- **Trigger:** mpv IPC write fails (mpv crashed mid-session, broken socket, fd exhaustion) or afplay spawn fails. The launch path falls back to afplay if mpv fails to spawn (`player.rs:472-475`), but there is **no mid-session mpv respawn** (unlike the sidecar's respawn-backoff in `session.rs:459-475`). Once mpv dies mid-session, every later `load()` fails silently.
- **Impact:** UI/backend divergence; no user feedback; no auto-recovery; manual skip hits the same dead player. Not a false-ready state (the YT provider state machine is unaffected); not a crash; not a credential issue.
- **Why P2 not P1:** mpv does not crash under normal operation; the defect requires a backend failure. All core journeys pass when the player backend is healthy (the only condition the deterministic tests exercise, since `StubPlayer::load` always returns `Ok`).
- **Acceptance:** (1) Check `player.load()` result; on `Err`, do NOT set `now_playing`, surface `yt_error`/a dead-track marker, and advance (mirroring the `std::fs::metadata`-miss dead-skip path at `app.rs:789-800`). (2) Add a test with a `Player` whose `load()` returns `Err` asserting `now_playing` stays `None` and the queue advances. (3) Optionally add a player-liveness check + respawn analogous to the sidecar's.

#### A2 — P3: Stale-lyrics generation guard is correct but under-tested (corroborates Judge 1 F1)
- **Severity:** P3 (test-depth gap; implementation verified correct)
- **File:** `tests/lyrics.rs:318-346`
- **Repro:** `stale_lyrics_dropped_on_track_change` asserts `assert_ne!("new_vid", "old_vid")` on two string literals — a tautology that never reaches `app.rs:1670` (`track_id == vid && gen == self.lyrics_gen`). `lyrics_gen_increments_on_request` tests `wrapping_add` on a local, not `App::lyrics_gen`.
- **Impact:** A regression breaking the guard would not be caught. The implementation itself is sound (I traced `request_lyrics` → `lyrics_gen.wrapping_add(1)` → `on_tick` discard).
- **Acceptance:** Drive `request_lyrics(A)` then `request_lyrics(B)` via the fake sidecar, deliver A's response, pump `on_tick`, assert the overlay still shows B's Loading state.
- **Note:** Judge 1 independently reported this as F1. I confirm it from the adversarial side.

#### A3 — P3: Log redaction marker set is incomplete for YouTube cookie names
- **Severity:** P3 (low — defense-in-depth gap; no current leak path)
- **File:** `src/tui/event.rs:150-155` (`const MARKERS`)
- **Repro:** `MARKERS` covers `__Secure-3PAPISID=`, `SAPISID=`, `authorization=`, `cookie=`. YouTube auth also uses `__Secure-1PAPISID=`, `__Secure-3PSID=`, `SID=`, `HSID=`, `SSID=`, `APISID=`, `SIDCC=` — none are redacted. A log line containing `SID=<value>` would pass through unchanged. `no_secret_in_logs` only tests the four covered markers, so the gap is untested.
- **Impact:** Low in practice: the log only writes `yt_error: {e}` (`event.rs:341-344`), and the sidecar's error strings don't echo raw cookie values. The redact filter is defense-in-depth; the uncovered markers would only leak if a future sidecar change echoed them.
- **Acceptance:** Add the remaining YouTube cookie-name markers to `MARKERS` (or redact any `<Token>=<value>` where the token ends in `SID`/`APISID`/`PSID`); add a test feeding each marker through `redact`.

#### A4 — P3: Diagnostics overlay test doesn't drive the `:diag` command
- **Severity:** P3 (test-depth gap; wiring verified correct by reading)
- **File:** `tests/feedback.rs:173-193` (`diagnostics_view_openable`)
- **Repro:** The test sets `app.overlay = Some(Overlay::Diagnostics)` directly instead of driving `:diag` through `handle_key`. The wiring exists — I confirmed `Overlay::Diagnostics` variant (`app.rs:172`), `:diag` handler (`input.rs:737-739`), `D` key (`input.rs:183-185`), render call (`overlay.rs:64-66`), and Esc close (`input.rs:659-667`) — so this is a test-coverage gap, not a functional defect.
- **Impact:** A regression that broke the `:diag` command handler (e.g. a typo in the match arm) would not be caught.
- **Acceptance:** Replace the direct overlay assignment with `run_command(&mut app, "diag")` and assert the overlay opens.

#### A5 — P3: `retry_yt_probe` auth-expired classification is string-heuristic
- **Severity:** P3 (documented design tradeoff; degrades safely)
- **File:** `src/tui/app.rs:2132-2146`
- **Repro:** Auth-expired vs provider-error is decided by `lower.contains("login required" || "unauthorized" || "expired" || "logged out")`. If ytmusicapi changes its error wording, an expired cookie would be misclassified as `ProviderError` (retryable) instead of `AuthExpired` (re-auth).
- **Impact:** Safe degradation — `ProviderError` is still not-ready and still retryable; the user presses R again. No false-ready, no credential leak. The recovery hint is slightly less precise ("press R" vs "run :yt auth browser").
- **Acceptance:** Optionally have the sidecar return a structured `error_code` field (e.g. `"auth"`) instead of relying on error-message keywords. Non-blocking.

---

### Per-rubric scoring (independent of Judge 1)

| Category | Max | Score | % | Notes |
|----------|-----|-------|---|-------|
| Functional correctness and complete core journeys | 25 | 23 | 92% | All journeys pass under a healthy backend. A1 is a real divergence but only on player-load failure (edge case). Pagination full; stale-async guarded; empty≠failed. |
| Provider, auth, persistence, and recovery reliability | 20 | 18 | 90% | Truthful state machine; no forced re-login; corrupt-DB recovery; cache cleared on logout; sidecar respawn-backoff. A5 string-heuristic classification degrades safely. No mid-session mpv respawn (covered under playback). |
| UX clarity, discoverability, interaction consistency | 20 | 18 | 90% | State-derived footer + icons; TTL notifications; diagnostics overlay wired (`:diag`/`D`); unknown-command feedback. A1 leaves a silent "playing" track on backend failure (no user feedback). A4 test doesn't drive `:diag`. |
| Playback correctness and responsiveness | 10 | 8 | 80% | Async format switch (tested); gapless preload; progressive premium upgrade; CONT modes all non-blocking. **A1** — `player.load()` failure silently diverges `now_playing` with no auto-recovery and no mpv respawn. |
| Automated test depth and determinism | 10 | 9 | 90% | 360 Rust + 30 bats, all deterministic (fake-sidecar pattern). A2 + A4 are test-depth gaps on correct implementations. No test for player-load failure (A1). |
| Security and privacy | 5 | 4 | 80% | Cookies 0600; `/tmp/.config` refused for secrets; CLI sanitization; corrupt-DB recovery; TUI safe (ratatui strips ESC — independently verified). A3 redaction marker set incomplete (no current leak path). |
| Terminal compatibility and accessibility | 5 | 5 | 100% | NO_COLOR + ASCII icons; 4 size breakpoints; too-small guard; CJK width; SIGTSTP/SIGCONT. |
| Maintainability and documentation | 5 | 5 | 100% | Every module/function explains the "why"; feature-folder layering (`yt/`, `tui/`, `lyrics/`, `source/`); consistent best-effort error handling; bounded caches everywhere. |
| **Total** | **100** | **90** | **90%** | |

---

### Verdict: **PASS** (with non-blocking follow-ups)

All mandatory release gates are met, independently re-verified:
- ✅ No P0 or P1 defect. A1 is P2 (edge-case, no crash/data/credential impact); A2–A5 are P3.
- ✅ No unresolved security or credential-leak issue. A3 is a defense-in-depth gap with no current leak path (the log only records `yt_error` strings, which don't carry raw cookies). TUI escape injection is neutralized by ratatui's zero-width stripping (independently confirmed via external probe).
- ✅ All required local, YouTube, and hybrid journeys pass with deterministic fixtures; no external provider limitation blocks any core journey.
- ✅ YouTube session restoration + playlist display pass in deterministic tests (`pagination_large_library`, `offline_shows_cached_marked_stale`, `refresh_success_promotes_to_ready`).
- ✅ No false connected/ready state — `YtState::is_ready()` is true only for `Ready`/`ReadyStale`; 7 `provider_state` tests verify.
- ✅ Command history works and is tested (13 tests).
- ✅ Lyrics are functional, truthful, non-blocking (21 tests; generation guard implemented; no fabrication).
- ✅ Input responsive during slow provider + lyrics operations (4 nonblocking tests; all hot paths fire-and-forget; the only sync sidecar call is the explicit `R` retry probe).
- ✅ Full fmt/clippy/test/build/bats gates green.
- ✅ Snapshots/rendered-state inspected (21 state + layout snapshots; I additionally inspected ratatui's buffer behavior directly).
- ✅ No rubric category < 80% (lowest: Playback 80%, Security 80%).
- ✅ Score 90 ≥ 90.

**Independent adversarial findings to address before merge (non-blocking for PASS):**
1. A1 (P2): Handle `player.load()` failure — don't set `now_playing` on `Err`, surface feedback, advance the queue; add a failing-Player test. This is the one finding I raise that Judge 1 did not surface; I confirmed it with a reproducible external probe (`/tmp/opencode/load-fail-probe`), so it stands unless contradicted by concrete evidence.
2. A2 (P3): Replace the tautological stale-lyrics test with a real generation-guard exercise (corroborates Judge 1 F1).
3. A3 (P3): Complete the redaction marker set for YouTube cookie names; add a test per marker.
4. A4 (P3): Drive `:diag` through `handle_key` in `diagnostics_view_openable`.
5. A5 (P3): Optional — structured `error_code` from the sidecar instead of string-heuristic auth-expired classification.

**Reconciliation with Judge 1:** Both judges PASS. Judge 1 scored 94 (Playback 100%); I scored 90 (Playback 80%) because my adversarial probe caught the `player.load()` divergence (A1) that black-box journey testing with `StubPlayer` cannot surface. Average = 92. The A1 finding is reproducible and stands; addressing it before merge is recommended to lift the playback category and the average to the ≥93 target.
