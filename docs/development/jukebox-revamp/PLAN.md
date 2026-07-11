# Jukebox Revamp Implementation Plan — Ordered Vertical Slices

**Date:** 2026-07-12 · **Source:** Synthesized from recon reports + ACCEPTANCE.md.

Each slice delivers an observable user outcome, maps to milestone(s), lists files to modify, dependencies, risk, and a verification command. Slices are ordered by dependency; slices in the same parallel-group can run concurrently.

**Risk legend:** L = low (mechanical, gated), M = medium (localized logic), H = high (cross-cutting state/concurrency).

---

## Slice 0: Release hygiene & gates (M10.5, M8 partial) — parallel-group:A

**Outcome:** fmt/clippy/CI gates green; release archive bundles YT sidecar; README accurate. Unblocks all subsequent CI verification.

**Milestones:** M10.5.1-5, M8.1.3, release P0s (quality-recon §10 P0-1/P0-2/P0-3, §12 P1-1/P1-7/P2-6).

**Files:**
- MODIFY all `src/**/*.rs`, `tests/**/*.rs` — `cargo fmt` (AC-M10.5.1)
- FIX 8 clippy errors (quality-recon Appendix): `src/mode.rs:36`, `src/state.rs:207,305`, `src/tui/view/overlay.rs:160`, `src/yt/session.rs:680,929`, `src/yt/sidecar.rs:43,77`
- CREATE `.github/workflows/ci.yml` — runs fmt+clippy+test+bats on PR
- MODIFY `.github/workflows/release.yml:67-73` — add `scripts/yt/yt.py` + `scripts/yt/requirements.txt` to staging
- MODIFY `README.md:142-145` — keybindings match `overlay.rs:267-300` Help (AC-M6.4/P1-1)
- MODIFY `README.md:67-69` — correct "no cookie file written" claim (P1-7)
- MODIFY `README.md:120` — `jukebox config` behavior (P2-6)
- FIX `scripts/yt/yt.py:50-53,153` — `delete=True` or cleanup temp cookie files (P1-6)

**Dependencies:** none (first slice, unblocks CI).

**Risk:** L — mechanical; no behavior change. fmt touches many files but is safe.

**Verify:** `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features && bats scripts/test/*.bats && cargo build --release`

---

## Slice 1: Truthful provider state machine (M2.1, M2.2) — parallel-group:B — CRITICAL PATH

**Outcome:** No false "connected"; provider state is an explicit enum; launch never nukes the session; user answers "am I authenticated?" truthfully. Closes root causes #1 (repeated login) and #2 (connected-but-empty).

**Milestones:** M2.1.1-5, M2.2.1-4.

**Files:**
- CREATE `src/yt/state.rs` — `ProviderState` enum + transitions (Unconfigured/SignedOut/Authenticating/AuthenticatedUnsynced/Synchronizing/Ready/ReadyStale/Offline/RateLimited/AuthExpired/Failed)
- MODIFY `src/tui/app.rs:154-228` — replace `yt_status: Option<String>` + `yt_error: Option<String>` with `provider_state: ProviderState` + `provider_msg: Option<String>`
- MODIFY `src/main.rs:108-166` — remove blocking `library_playlists()` probe (AC-M2.2.1); on launch, set state `Authenticating`→fire-and-forget `send_refresh`; never set `yt_session = None` on probe error (AC-M2.2.2); degrade to `Offline`/`AuthExpired`
- MODIFY `src/main.rs:117,129` — remove premature "connected" assignments (AC-M2.1.1)
- MODIFY `src/tui/app.rs:968-1015` (`apply_yt_auth`, `apply_yt_browser`) — set `Authenticating` not "connected"
- MODIFY `src/tui/app.rs:1093` (respawn path) — set `Synchronizing` not "sidecar restarted"
- MODIFY `src/tui/view/footer.rs:22-33` — render `ProviderState` (accent/amber/red) + message
- MODIFY `src/tui/view/columns.rs:249-259` — Y-view col2 renders `ProviderState` (loading/empty/offline/authexpired/failed distinct)
- MODIFY `scripts/yt/yt.py:328-330,561-565` (`auth_status`) — return `ok=_has_auth(), valid=<expiry check on cookie expires>`, real `premium`/`account` detection (AC-M2.1.5)
- MODIFY `scripts/yt/yt.py:205-216` (`_has_auth`) — read cookie `expires`; report expired
- MODIFY `src/yt/session.rs:882-888` — distinguish `Ok(empty)` for authed vs guest (probe emptiness)
- CREATE `tests/provider_state.rs` — AC-M2.1.4, M2.2.3, M2.2.4, M2.3.2-4

**Dependencies:** Slice 0 (CI gate to verify).

**Risk:** H — touches the core state representation, launch path, and sidecar protocol. Highest-risk slice. Mitigation: pure state-machine module `src/yt/state.rs` unit-tested in isolation; App integration is mechanical field replacement.

**Verify:** `cargo test provider_state && grep -c 'yt_status = .*connected' src/main.rs src/tui/app.rs` → 0.

---

## Slice 2: Generation ids + sync cancellation + logout cleanup (M2.6, M9.3) — parallel-group:B — depends:Slice 1

**Outcome:** Stale background results can't overwrite newer state; logout/account-switch clears all identity state; in-flight results after logout are dropped.

**Milestones:** M2.6.1-3, M9.3.1-3.

**Files:**
- MODIFY `src/yt/session.rs:160-213` (`Pending`) — add `gen: u64` per category (Playlists/Suggestions/Tracks/Lyrics); `Pending::matches` compares gen
- MODIFY `src/yt/session.rs:714-720` (`send_refresh`) — add `refresh_inflight` guard (like `playlist_inflight`); increment `playlist_gen` on send
- MODIFY `src/yt/session.rs:572-613` (`apply_pair`) — tag response with the gen of the paired request; drop if `gen != current_gen`
- MODIFY `src/tui/app.rs:1653-1675` (`refresh_yt_lists`) — increment gen; clear `yt_lists`/`loaded_yt_lists`
- MODIFY `src/tui/app.rs:1126-1148` (`on_tick` playlist merge) — only apply if `resp_gen == current_gen`
- MODIFY `src/tui/app.rs:1384-1393` (`yt_logout`) — clear `yt_lists`, `loaded_yt_lists`, `track_cache`, `url_cache`, all `pending_*` (AC-M2.6.1)
- MODIFY `src/tui/app.rs:995-1015` (`apply_yt_browser`) — clear `yt_lists`/`loaded_yt_lists` on account switch
- CREATE `tests/sync_cancel.rs` — AC-M2.6.2, M2.6.3, M9.3.2

**Dependencies:** Slice 1 (state machine informs "ready" vs "stale").

**Risk:** M — localized to session pairing + app merge guards; protocol change is additive (gen field).

**Verify:** `cargo test sync_cancel && cargo test stale_refresh_does_not_regress_lists`.

---

## Slice 3: Pagination + offline cache + rate-limit (M2.4, M2.5) — parallel-group:B — depends:Slice 1

**Outcome:** All playlists/tracks load (no silent truncation); cached playlists shown offline; rate-limit/timeout states actionable.

**Milestones:** M2.4.1-3, M2.5.1-4.

**Files:**
- MODIFY `scripts/yt/yt.py:334-365` (`library_playlists`, `get_playlist`) — pagination loops with continuation tokens (AC-M2.4.1-2)
- MODIFY `src/yt/proto.rs` — add continuation/has_more fields to `Playlists`/`Tracks` responses
- MODIFY `src/yt/session.rs:882-896` — return `(items, has_more)`; app shows "showing N of M" or "load more"
- CREATE `src/yt/cache.rs` — disk cache for `yt_lists` to `state.db` key `'yt_playlists'` (AC-M2.5.1)
- MODIFY `src/tui/app.rs:1653` (`refresh_yt_lists`) — load from cache first (mark `ReadyStale`), then fetch
- MODIFY `src/tui/app.rs` (on_tick) — `yt_lists_loading` timeout guard (AC-M2.5.3)
- MODIFY `src/yt/session.rs:669-687` (roundtrip) — on 429/rate-limit → `RateLimited` state (AC-M2.5)
- ADD manual refresh key (`R`) + `:yt refresh` command in `src/tui/input.rs` (AC-M2.5.4)
- CREATE `tests/pagination_cache.rs` — AC-M2.4.3, M2.5.2, M2.5.3

**Dependencies:** Slice 1 (state for stale/offline/rate-limited).

**Risk:** M — sidecar pagination loops need care (continuation token handling); cache schema is additive.

**Verify:** `cargo test pagination_cache && cargo test offline_shows_cached_marked_stale`.

---

## Slice 4: Non-blocking hot path (M9.2) — parallel-group:C — depends:Slice 1,2

**Outcome:** `S`/Discover-Enter/CONT=YouTube auto-advance no longer freeze the UI (3-4s blocks removed); audio format switch doesn't block input ≥100ms.

**Milestones:** M9.2.1-4 (playback-recon §8 B1-B4).

**Files:**
- MODIFY `src/tui/app.rs:1453-1465` (`open_discover`) — open overlay instantly empty; `send_home_suggestions` fire-and-forget; `on_tick` merges `pending_suggestions` (AC-M9.2.3, fixes B3)
- MODIFY `src/tui/app.rs:1636-1639` (`play_discover_selection`) — fire-and-forget `send_get_playlist`; start playback on `on_tick` when tracks land (fixes B2)
- MODIFY `src/tui/app.rs:864-896` (CONT=YouTube auto-advance) — `RadioCursor::advance` → fire-and-forget `send_get_watch_playlist`; defer context switch + playback to `on_tick` (AC-M9.2.2, fixes B4)
- MODIFY `src/yt/session.rs:912` (`get_watch_playlist` roundtrip) — add `send_get_watch_playlist` (non-blocking) + `pending_watch` slot
- MODIFY `src/audio.rs:264-265` — move `set_output_format` + `verify_format_landed` + 60ms sleep to a `std::thread::spawn` that signals via a channel; `on_tick` checks readiness before `load` (AC-M9.2.4, fixes B1). Alternative: overlap settle with mpv buffer fill.
- MODIFY `src/tui/app.rs` (`start_playback`/`load_track`/`load_remote`) — gate on audio-ready signal
- CREATE `tests/nonblocking.rs` — AC-M9.2.2-4

**Dependencies:** Slice 1 (state during async), Slice 2 (gen ids for watch_playlist).

**Risk:** H — audio background thread adds concurrency to a previously-single-thread app; the discover/auto-advance conversion mirrors existing fire-and-forget patterns (lower risk). Mitigation: audio thread is bounded (one-shot, signals via mpsc, joined on next switch).

**Verify:** `cargo test nonblocking && cargo test discover_opens_instantly && cargo test cont_youtube_auto_advance_non_blocking`.

---

## Slice 5: Lyrics pipeline (M3) — parallel-group:C — depends:Slice 1,2

**Outcome:** Lyrics work as a first-class non-blocking feature with synced highlighting, truthful states, and stale-result guard.

**Milestones:** M3.1.1-4, M3.2.1, M3.3.1-3, M3.4.1-2, M3.5.1-3.

**Files:**
- CREATE `src/lyrics/mod.rs` — `Lyrics` type, `LyricsState` (Loading/Available{synced, plain}/NotFound/Offline/Error), `parse_lrc` (timestamped), `parse_plain`
- CREATE `src/lyrics/source.rs` — provider pipeline: embedded metadata (FLAC/ID3) → sidecar `.lrc` → sidecar `get_lyrics` → cache
- CREATE `src/lyrics/cache.rs` — disk cache (`state.db` `'lyrics:<track_id>'`) with invalidation on track change
- MODIFY `src/yt/proto.rs:17-35` — add `Request::GetLyrics { track_id, title, artist }` + `Response::Lyrics { track_id, synced, lines, gen }`
- MODIFY `scripts/yt/yt.py` — add `get_lyrics` command (ytmusicapi `get_lyrics` where available) + `.lrc` sidecar read
- MODIFY `src/yt/session.rs` — `send_get_lyrics` (fire-and-forget) + `pending_lyrics` slot + gen tag (AC-M3.4.1)
- MODIFY `src/tui/app.rs:105-142` (`Overlay`) — add `Lyrics { lines, state, scroll, gen }` variant
- MODIFY `src/tui/input.rs` — bind a lyrics key (e.g. `L` — verify no collision with current `M`/`m`/`a`/`f`/`S`/`s`); `j`/`k`/PgUp/PgDn/g/G scroll
- CREATE `src/tui/view/lyrics.rs` — render lyrics overlay (highlighted current line for synced; plain for unsynced; state text for loading/notfound/error)
- MODIFY `src/tui/view/mod.rs` — export lyrics renderer
- MODIFY `src/tui/app.rs` (on_track_start) — fire lyrics request with current track_id + gen
- MODIFY `src/tui/app.rs:1056` (`on_tick`) — drain `pending_lyrics`; apply only if `track_id == now_playing && gen == current_gen` (AC-M3.4.2)
- CREATE `tests/lyrics.rs` — AC-M3.2.1, M3.3.1-3, M3.4.2, M3.5.1-3

**Dependencies:** Slice 1 (ProviderState pattern reuse for LyricsState), Slice 2 (gen ids).

**Risk:** M — greenfield feature; reuses the proven fire-and-forget + on_tick pattern. Sidecar `get_lyrics` may have provider limits (document as external limitation if so). Key-binding collision check needed (tui-recon §1 keymap is dense).

**Verify:** `cargo test lyrics && cargo test stale_lyrics_dropped_on_track_change`.

---

## Slice 6: Command mode + history (M4) — parallel-group:C — depends:Slice 1

**Outcome:** Command mode has persistent history, Up/Down recall, editing, completion, unknown-command feedback, visible cursor.

**Milestones:** M4.1.1-3, M4.2.1-4, M4.3.1-3, M4.4.1-2, M4.5.1.

**Files:**
- MODIFY `src/tui/app.rs:128-130` (`Overlay::Command`) — add `history: Vec<String>`, `history_cursor: usize`, `unsaved: String` (AC-M4.1.1)
- MODIFY `src/state.rs` — add `'command_history'` key save/load (bounded 100, dedup adjacent) (AC-M4.1.2)
- MODIFY `src/main.rs:183-192` — save command history on clean exit; load on launch
- MODIFY `src/tui/input.rs:283-298` (`handle_overlay_key` for `Command`) — `Up`/`Down` recall (preserve `unsaved`); `Home`/`End`/word-movement/deletion; `Tab` completion (AC-M4.2.1, M4.2.4, M4.3.1)
- MODIFY `src/tui/input.rs:416-439` (`execute_command`) — remove `_ => {}`; unknown command → `Command` overlay shows "unknown: :foo" (AC-M4.3.2)
- MODIFY `src/tui/view/overlay.rs` (Command render) — visible block cursor `▏` (AC-M4.3.3)
- ADD known-command table (`yt auth`, `yt auth browser`, `yt logout`, `yt setup`, `yt refresh`, `diag`, `help`) for completion + `:help <cmd>` (AC-M4.3.3)
- CREATE `tests/command_mode.rs` — AC-M4.1.3, M4.2.1-4, M4.3.1, M4.4.1-2, M4.5.1

**Dependencies:** Slice 1 (ProviderState for `:yt refresh`/status commands).

**Risk:** M — localized to Command overlay + input handler + state persistence; no cross-module refactor.

**Verify:** `cargo test command_mode && cargo test command_history_persists_across_restart`.

---

## Slice 7: Feedback, logging, diagnostics (M5) — parallel-group:D — depends:Slice 1

**Outcome:** Quiet/normal/verbose levels; diagnostics view; auto-clearing notifications; secret redaction; bounded logs; sidecar stderr captured.

**Milestones:** M5.1.1-2, M5.2.1-2, M5.3.1-3, M5.4.1.

**Files:**
- MODIFY `src/cli.rs` — add `-v`/`--verbose` (counted) + `--quiet` (AC-M5.1.1)
- CREATE `src/diagnostics.rs` — in-memory ring buffer of recent events + `:diag`/`D` view (AC-M5.1.2)
- MODIFY `src/tui/view/diagnostics.rs` (new) — render diagnostics overlay
- MODIFY `src/tui/app.rs` — notification queue with TTL (auto-clear after N ticks) (AC-M5.2.1); dedup identical consecutive (AC-M5.2.2)
- MODIFY `src/yt/sidecar.rs:55` — redirect stderr to bounded log file (not `Stdio::null()`) (AC-M5.3.1, quality-recon P3-2)
- REVIVE `src/tui/event.rs:96-105` (`log_to_file`) — wire into error paths with redaction (AC-M5.3.2)
- CREATE `src/redact.rs` — scrub cookie/SAPISID/token patterns from strings before log/display (AC-M5.3.2)
- MODIFY `src/yt/proto.rs:152` — sanitize `unrecognized sidecar response` error (truncate, redact) (quality-recon P3-8)
- ADD bounded log rotation in `log_to_file` (AC-M5.3.3)
- MODIFY user-error strings — include "see :diag" or correlation id (AC-M5.4.1)
- CREATE `tests/feedback.rs` — AC-M5.2.1, M5.3.2, M5.3.3

**Dependencies:** Slice 1 (ProviderState drives notifications), Slice 2 (gen ids for correlation).

**Risk:** M — diagnostics + redaction are new modules; notification TTL is localized.

**Verify:** `cargo test feedback && grep -c 'Stdio::null' src/yt/sidecar.rs` → 0.

---

## Slice 8: TUI polish + responsive + snapshots (M6) — parallel-group:D — depends:Slice 1,7

**Outcome:** Source indicator visible; empty/loading/error states everywhere; ≥20 snapshots for all important states; wide-char/no-color correct.

**Milestones:** M6.1.1-2, M6.2.1-3, M6.3.1-2, M6.4.1-3, M6.5.1.

**Files:**
- MODIFY `src/tui/view/player_bar.rs` — source indicator (Local/YouTube/Mixed + quality) (AC-M6.1.2, tui-recon §11)
- MODIFY `src/tui/view/columns.rs` — empty-catalog "run jukebox sync" hint; missing-index hint (AC-M6.2.1-2)
- MODIFY `src/tui/view/player_bar.rs` — loading/buffering indicator during track load + YT resolve (AC-M6.2.3, tui-recon §6 P2-5)
- MODIFY `src/tui/view/theme.rs:64-83` — handle zero-width/combining in `disp_width` (AC-M6.4.1, tui-recon §5 P3-2)
- CREATE `tests/snapshots/*.snap` (≥20) — local-populated, local-empty, YT-signed-out, YT-authenticating, YT-synchronizing, YT-ready, YT-no-playlists, offline-cache, provider-failure, search-empty, search-populated, queue-empty, queue-populated, lyrics-loading, lyrics-available, lyrics-unavailable, help, command+history, confirmation, too-small (AC-M6.3.1)
- MODIFY `tests/layout.rs`, `tests/columns.rs`, `tests/player_bar.rs` — snapshot tests for new states
- ADD no-destructive-single-key audit (AC-M6.4.3)
- Run `cargo insta test --review` and inspect every diff (AC-M6.5.1)

**Dependencies:** Slice 1 (ProviderState for status rendering), Slice 5 (lyrics states), Slice 6 (command+history), Slice 7 (diagnostics view).

**Risk:** M — render-only changes; snapshot review is the quality gate.

**Verify:** `cargo insta test --review` (all accepted) && `cargo test --test columns --test layout --test player_bar`.

---

## Slice 9: Playback/queue correctness + transport persistence (M7) — parallel-group:D — depends:Slice 4

**Outcome:** "Play next" insert; EOF+`>` no double-advance; transport persisted across restart; source-failure recovery tested.

**Milestones:** M7.1.1-4, M7.2.1-2, M7.3.1-2, M7.4.1-2.

**Files:**
- MODIFY `src/tui/queue.rs:187` (`enqueue`) — add `play_next` (insert at front of `manual_queue`) (AC-M7.1.1, playback-recon §3)
- MODIFY `src/tui/input.rs` — bind `N` (or chosen key) to play-next; verify no collision
- MODIFY `src/tui/event.rs:241-255` — after `on_track_ended` advances, consume/discard pending `>` for that tick (AC-M7.1.2, playback-recon §10 D5)
- MODIFY `src/state.rs` — persist transport (cursor/order/history/manual_queue) under `'transport'` key (AC-M7.1.4)
- MODIFY `src/main.rs:68-131` — restore transport on launch
- MODIFY `src/tui/app.rs:191` (`remove_from_queue`) — handle removing currently-playing (don't interrupt; update next) (AC-M7.1.3)
- CREATE `tests/playback_correctness.rs` — AC-M7.1.1-4, M7.2.1-2, M7.3.2, M7.4.2

**Dependencies:** Slice 4 (non-blocking auto-advance for the EOF race context).

**Risk:** M — queue semantics changes are localized; transport persistence is additive (serde).

**Verify:** `cargo test playback_correctness && cargo test eof_and_next_no_double_advance && cargo test transport_persists_across_restart`.

---

## Slice 10: Security hardening (M8) — parallel-group:D — depends:Slice 0

**Outcome:** No world-readable config fallback; safe mpv socket; no CLI escape injection; no panic on fd exhaustion; migration versioning.

**Milestones:** M8.1.1-3, M8.2.1-2, M8.3.1-2, M8.4.1-2.

**Files:**
- MODIFY `src/config.rs:35`, `src/yt/session.rs:62`, `src/state.rs:26` — refuse `/tmp/.config` fallback; require explicit `XDG_CONFIG_HOME` or error (AC-M8.1.1, quality-recon P1-4)
- MODIFY `src/config.rs:52` (mpv socket) — use `XDG_RUNTIME_DIR` or random suffix (AC-M8.1.2, quality-recon P1-5)
- MODIFY `src/main.rs:231-239` — escape control chars in CLI `println!` (AC-M8.2.1, quality-recon P1-2)
- MODIFY `src/yt/sidecar.rs:65-66` — `expect` → `?` (return `Err`, degrade to guest) (AC-M8.3.1, quality-recon P1-3)
- MODIFY `src/main.rs:35,201` — `parent().unwrap()` → `unwrap_or(Path::new("."))` (AC-M8.3.2, quality-recon P3-1)
- MODIFY `src/state.rs` — add `schema_version` key + migration logic (AC-M8.4.1, quality-recon P2-3)
- MODIFY corrupt-DB path — delete + recreate (AC-M8.4.2)
- CREATE `tests/security.rs` — AC-M8.1.1, M8.2.1, M8.3.1, M8.4.2

**Dependencies:** Slice 0 (CI to verify), Slice 7 (redaction).

**Risk:** M — config-fallback refusal is a behavior change (could break headless users — provide clear error message); socket path change is localized.

**Verify:** `cargo test security && grep -c 'jukebox-mpv.sock' src/config.rs` → 0.

---

## Slice 11: Performance: id→index map + bounded caches (M9.1, M9.4) — parallel-group:E — depends:Slice 4

**Outcome:** Per-frame O(n×m) eliminated; `track_cache` bounded; `now_playing_view` once/frame; sidecar channel bounded; baseline measurements recorded.

**Milestones:** M9.1.1, M9.4.1-4.

**Files:**
- MODIFY `src/tui/app.rs:330-337` (`App::new`) — build `id_index: HashMap<String, usize>` from `catalog.tracks` (AC-M9.4.2, playback-recon §11 D8)
- MODIFY `src/tui/app.rs:384` (`track_by_id`) — use `id_index` O(1)
- MODIFY `src/tui/view/columns.rs:425` (`track_rows`) — use `id_index`
- MODIFY `src/tui/view/player_bar.rs:55,65,171,192` — call `now_playing_view()` once, cache in local (AC-M9.4.3, playback-recon §11 D9)
- MODIFY `src/yt/session.rs:205` (`track_cache`) — cap (LRU 256) (AC-M9.4.1, playback-recon §11 D7)
- MODIFY `src/yt/sidecar.rs:67` — `mpsc::sync_channel(64)` (AC-M9.4.4, playback-recon §11 D11)
- MODIFY `src/tui/view/layout.rs:53` (`clamp_cursors`) — cache focused album track_ids (playback-recon §11 D10)
- CREATE `docs/development/jukebox-revamp/PERF.md` — before/after timings (AC-M9.1.1)
- CREATE `tests/perf.rs` — assertions on no per-frame O(n) (or benchmark)

**Dependencies:** Slice 4 (non-blocking path for clean measurement).

**Risk:** L — HashMap is a well-understood optimization; cache caps are additive.

**Verify:** `cargo test perf && cargo bench` (or manual timing in PERF.md).

---

## Slice 12: Test depth + e2e journeys (M10.1-4) — parallel-group:E — depends:Slices 1-11

**Outcome:** Layered suite complete; 12 journeys A-L covered with fixtures; no parallel-test races; malformed-input tested.

**Milestones:** M10.1.1, M10.2.1-3, M10.3.1, M10.4.1.

**Files:**
- CREATE `tests/e2e_journeys.rs` — parametrized A-L (AC-M10.4.1)
- MODIFY `tests/e2e_yt.rs:181,214,257,305,565` — remove `set_var("JK_FAKE_MAP")` (AC-M10.2.3, quality-recon P2-1)
- ADD `tests/yt_sidecar.rs` — `Response::from_line` malformed-input tests (AC-M10.2.2, quality-recon P2-2)
- FIX `tests/yt_sidecar.rs:120-128` — fake returns `auth` not `pong` (quality-recon P2-7)
- ADD unit tests for: state transitions, token-expiry, refresh, pagination, cache, hybrid identity, command parsing/history, lyrics parsing, queue/transport, error mapping, notification dedup, layout (AC-M10.1.1)
- ADD integration: restart, migration, mock YT, auth refresh, playlist sync, offline, provider recovery, lyrics (AC-M10.2.1)

**Dependencies:** All feature slices (1-11) — tests pin their behavior.

**Risk:** L — test-only; uses existing fake-sidecar patterns.

**Verify:** `cargo test --all-features && bats scripts/test/*.bats`.

---

## Slice 13: app.rs god-object split (incremental) — parallel-group:E — depends:Slices 1-9

**Outcome:** `app.rs` (1819 lines) split into a directory `src/tui/app/` with focused modules; testability improved.

**Milestones:** Maintainability (rubric 5pts), supports M10.

**Files:**
- CREATE `src/tui/app/mod.rs` — `App` struct + `new` + re-exports
- CREATE `src/tui/app/state.rs` — browse state, cursors, column widths, filter
- CREATE `src/tui/app/playback.rs` — `start_playback`/`load_track`/`load_remote`/`next`/`prev`/`on_track_ended`
- CREATE `src/tui/app/yt.rs` — YT lifecycle, `refresh_yt_lists`, auth, discover, `on_tick` YT merge
- CREATE `src/tui/app/tick.rs` — `on_tick` orchestration
- MOVE `src/tui/app.rs` content into the above; keep public `App` type in `mod.rs`

**Dependencies:** Slices 1-9 (so the split moves already-refactored code, not mid-flight logic).

**Risk:** M — pure refactor; must preserve all tests. Mitigation: do after behavior slices stabilize; verify with full test suite after each move.

**Verify:** `cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings` (no regressions).

---

## Slice 14: Independent judges + release (M11) — depends:Slices 0-13

**Outcome:** Two fresh judges from clean worktrees; blockers fixed with regression tests; all release gates pass.

**Milestones:** M11.1.1, M11.2.1, M11.3.1, M11.4.1, M11.5.1-2.

**Files:**
- CREATE `docs/development/jukebox-revamp/JUDGE.md` — score history
- CREATE skill `.agents/skills/jukebox-isolated-release-judge/` (prompt lines 149-158)
- Run Judge 1 (black-box product) from clean worktree
- Run Judge 2 (adversarial engineering) from clean worktree
- Fix confirmed blockers + add regression tests
- Final verification from clean checkout (AC-M11.4.1)

**Dependencies:** All slices.

**Risk:** M — judge findings may require iteration; budget 2-3 judge rounds.

**Verify:** Both judges ≥90, avg ≥93, no FAIL, no rubric <80% (AC-M11.5.1-2).

---

## Parallelism map

| Group | Slices | Can run concurrently |
|---|---|---|
| A | 0 | Solo (unblocks CI) |
| B | 1 → (2, 3) | 1 first; then 2 and 3 parallel |
| C | 4, 5, 6 | All 3 parallel (after 1; 4 needs 2) |
| D | 7, 8, 9, 10 | 7+10 parallel; 8 after 7; 9 after 4 |
| E | 11, 12, 13 | 11 after 4; 12 after all; 13 after 1-9 |
| Final | 14 | After all |

**Critical path:** 0 → 1 → 2 → 4 → 9 → 12 → 14 (longest dependency chain).

**Fast wins (solo, low-risk):** Slice 0 (gates), Slice 10 (security), Slice 11 (perf map).
