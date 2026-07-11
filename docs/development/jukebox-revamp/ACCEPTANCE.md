# Jukebox Acceptance Criteria

**Date:** 2026-07-12 · **Source:** Synthesized from recon reports + `.opencode/prompt` release gates (lines 939-970).

Each criterion is **specific** (file:line or test name), **verifiable** (command to run), and **binary** (pass/fail). A milestone passes only when ALL its criteria pass.

---

## M2: Reliability & Provider State (P0)

### M2.1 Truthful provider state machine (no false "connected")

- [ ] **AC-M2.1.1** `grep -n 'yt_status = .*connected' src/main.rs src/tui/app.rs` returns **zero matches** (all false-ready sites removed). *Evidence: yt-recon §10 table (10 sites).*
- [ ] **AC-M2.1.2** An explicit `ProviderState` enum exists with states {Unconfigured, SignedOut, Authenticating, AuthenticatedUnsynced, Synchronizing, Ready, ReadyStale, Offline, RateLimited, AuthExpired, Failed}. `grep -n 'enum ProviderState' src/yt/` returns ≥1 match. *Evidence: prompt lines 429-445.*
- [ ] **AC-M2.1.3** `yt_status`/`yt_error` freeform fields are removed or superseded by `ProviderState` + optional message. `grep -n 'pub yt_status' src/tui/app.rs` returns zero matches.
- [ ] **AC-M2.1.4** Test `provider_state_never_reports_ready_before_data` exists and passes: `cargo test provider_state_never_reports_ready` → PASS.
- [ ] **AC-M2.1.5** `auth_status` from sidecar distinguishes `ok` from `valid`: `yt.py` `auth_status` returns `ok=_has_auth(), valid=<network probe or expiry check>`. `grep -n 'valid' scripts/yt/yt.py` in the auth_status handler returns ≥1 match. *Evidence: yt-recon §4.*

### M2.2 Non-blocking startup; session restore without forcing re-login

- [ ] **AC-M2.2.1** `main.rs` launch path contains **no blocking `library_playlists()` roundtrip**. `grep -n 'library_playlists()' src/main.rs` returns zero matches (moved to fire-and-forget). *Evidence: playback-recon §8 B6, yt-recon §3.*
- [ ] **AC-M2.2.2** On launch probe error, `yt_session` is NOT set to `None`. `grep -n 'yt_session = None' src/main.rs` returns zero matches. *Evidence: yt-recon §3 (main.rs:151 root cause).*
- [ ] **AC-M2.2.3** Test `launch_probe_failure_keeps_session_and_degrades` exists and passes: `cargo test launch_probe_failure` → PASS.
- [ ] **AC-M2.2.4** A relaunch with valid cached cookies restores the session **without re-prompting the Keychain**: test `relaunch_restores_session_without_keychain` passes.

### M2.3 Token refresh / expiry / revocation lifecycle + tests

- [ ] **AC-M2.3.1** Expired-cookie detection exists: sidecar reads cookie `expires` and reports `AuthExpired` when expired. `grep -n 'expires' scripts/yt/yt.py` in auth path returns ≥1 match.
- [ ] **AC-M2.3.2** Revoked credentials do not cause silent empty: an empty `library_playlists` result for an authenticated session sets `AuthExpired`/`Failed`, not `Ready`. Test `expired_cookie_shows_auth_expired_not_empty` passes.
- [ ] **AC-M2.3.3** No endless silent retry loop on revoked creds: `grep -n 'loop' src/yt/session.rs` near refresh shows bounded attempts only. Test `revoked_credential_no_retry_loop` passes.
- [ ] **AC-M2.3.4** Missing scope distinguished from network failure: distinct `ProviderState::Failed` (scope) vs `Offline` (network). Tests `network_failure_shows_offline` and `scope_error_shows_failed` pass.

### M2.4 Playlist pagination complete or communicated

- [ ] **AC-M2.4.1** `get_library_playlists` pagination loop exists OR a "showing 25 of N" indicator renders. `grep -n 'continuation\|next_page\|limit=' scripts/yt/yt.py` returns ≥1 match in `library_playlists`. *Evidence: yt-recon §5.*
- [ ] **AC-M2.4.2** `get_playlist` track pagination loop exists OR truncation is communicated. `grep -n 'continuation\|limit=' scripts/yt/yt.py` in `get_playlist` returns ≥1 match.
- [ ] **AC-M2.4.3** Test `playlist_pagination_completes_or_signals` passes.

### M2.5 Offline / cache / rate-limit / retry actionable states

- [ ] **AC-M2.5.1** Playlists cached to disk across restart: `state.rs` has a `'yt_playlists'` key OR a cache file. `grep -n 'yt_playlists' src/state.rs` returns ≥1 match.
- [ ] **AC-M2.5.2** Cached playlists shown with "stale/offline" indicator. Test `offline_shows_cached_marked_stale` passes.
- [ ] **AC-M2.5.3** `yt_lists_loading` cannot hang forever: a timeout clears it. Test `loading_state_does_not_hang_on_dead_sidecar` passes.
- [ ] **AC-M2.5.4** Manual refresh (`R` or `:yt refresh`) exists with feedback. `grep -n 'fn refresh\| yt refresh' src/tui/` returns ≥1 match.

### M2.6 Logout / account-switch clears identity state

- [ ] **AC-M2.6.1** `yt_logout` clears `yt_lists`, `loaded_yt_lists`, `track_cache`, `url_cache`, pending slots. `grep -n 'yt_lists.clear\|track_cache.clear\|url_cache.clear' src/tui/app.rs` near `yt_logout` returns ≥4 matches. *Evidence: yt-recon §9.*
- [ ] **AC-M2.6.2** In-flight refresh after logout does NOT repopulate `yt_lists`: generation-id guard. Test `logout_drops_inflight_results` passes.
- [ ] **AC-M2.6.3** Account switch clears stale lists: Test `account_switch_clears_old_lists` passes.

---

## M3: Lyrics (first-class, non-blocking)

### M3.1 Lyrics provider pipeline

- [ ] **AC-M3.1.1** `Overlay::Lyrics` variant exists. `grep -n 'Lyrics' src/tui/app.rs` returns ≥1 match in the `Overlay` enum. *Evidence: AUDIT §13 Q5.*
- [ ] **AC-M3.1.2** Sidecar `get_lyrics` command exists. `grep -n 'get_lyrics\|GetLyrics' scripts/yt/yt.py src/yt/proto.rs` returns ≥1 match each.
- [ ] **AC-M3.1.3** Local `.lrc` parsing exists in Rust. `grep -rn 'fn parse_lrc\|LrcLine\|fn parse_lyrics' src/` returns ≥1 match.
- [ ] **AC-M3.1.4** Embedded-lyrics read (FLAC/ID3) considered OR documented as out-of-scope with rationale.

### M3.2 Loading / available / not-found / error / retry states

- [ ] **AC-M3.2.1** Lyrics overlay shows distinct states for: loading, available (synced), available (plain), not found, offline, provider error. Test `lyrics_state_matrix` (parametrized) passes for all 6 states.

### M3.3 Synced highlighting + scroll + Unicode

- [ ] **AC-M3.3.1** Synced-line highlighting: current line highlighted when timestamped lyrics available. Test `synced_lyrics_highlights_current_line` passes.
- [ ] **AC-M3.3.2** Scroll support (`j`/`k`/PgUp/PgDn/g/G) in lyrics overlay. Test `lyrics_scroll` passes.
- [ ] **AC-M3.3.3** Unicode + long lines render without panic at 80×24. Test `lyrics_unicode_long_lines_narrow` passes.

### M3.4 Cancellation: stale results can't overwrite newer track

- [ ] **AC-M3.4.1** Lyrics request carries a track-id / generation tag. `grep -n 'track_id\|gen' src/yt/session.rs` near lyrics returns ≥1 match.
- [ ] **AC-M3.4.2** Stale lyrics for old track dropped on track change. Test `stale_lyrics_dropped_on_track_change` passes.

### M3.5 Deterministic tests + no fabricated lyrics

- [ ] **AC-M3.5.1** No fabricated lyrics: when no source returns lyrics, state = "not found" (never invents text). Test `no_fabricated_lyrics` passes.
- [ ] **AC-M3.5.2** Lyrics cache with invalidation: test `lyrics_cache_invalidates_on_track_change` passes.
- [ ] **AC-M3.5.3** All lyrics tests run without network: `cargo test lyrics -- --no-run` shows no `#[ignore]` requiring creds.

---

## M4: Command Mode & Vim Interaction

### M4.1 In-session + persistent command history

- [ ] **AC-M4.1.1** `Overlay::Command` has a `history: Vec<String>` and `history_cursor`. `grep -n 'history' src/tui/app.rs` in `Overlay::Command` returns ≥1 match. *Evidence: AUDIT §13 Q6.*
- [ ] **AC-M4.1.2** `state.rs` has a `'command_history'` key. `grep -n 'command_history' src/state.rs` returns ≥1 match.
- [ ] **AC-M4.1.3** History persists across restart. Test `command_history_persists_across_restart` passes.

### M4.2 Up/Down traversal, cursor editing, dedup, bounded

- [ ] **AC-M4.2.1** `Up`/`Down` recall history; unfinished command preserved. Test `up_down_recall_preserves_unfinished` passes.
- [ ] **AC-M4.2.2** Adjacent identical entries deduped. Test `history_dedup_adjacent` passes.
- [ ] **AC-M4.2.3** History bounded (≤ configurable max, default 100). Test `history_bounded_at_max` passes.
- [ ] **AC-M4.2.4** Cursor movement: Home/End/word-movement/deletion. Test `command_line_editing` passes.

### M4.3 Completion / suggestions + unknown-command feedback

- [ ] **AC-M4.3.1** `Tab` completion for known commands/args. Test `command_tab_completion` passes.
- [ ] **AC-M4.3.2** Unknown command shows feedback (not silent `_ => {}`). `grep -n '_ => {}' src/tui/input.rs` in `execute_command` returns zero matches. Test `unknown_command_shows_feedback` passes. *Evidence: tui-recon §10 (TUI-P1-2).*
- [ ] **AC-M4.3.3** Visible block cursor in command line. Test `command_overlay_has_cursor` passes (snapshot or assertion).

### M4.4 Key-collision audit + tests

- [ ] **AC-M4.4.1** No key collisions: printable search/command chars not swallowed as navigation. Test `no_key_collision_search_cmd_nav` passes. *Evidence: tui-recon §1.*
- [ ] **AC-M4.4.2** Consistent overlay dismissal (`Esc`). Test `esc_closes_any_overlay` passes.

### M4.5 Tests for history/editing/unicode/persistence

- [ ] **AC-M4.5.1** Unicode input in command line doesn't corrupt. Test `command_unicode_input` passes.

---

## M5: Feedback, Logging, Diagnostics

### M5.1 Quiet / normal / verbose levels + diagnostics view

- [ ] **AC-M5.1.1** Verbosity flag exists (`-v`/`--verbose` or env). `grep -n 'verbose\|verbosity' src/cli.rs src/main.rs` returns ≥1 match. *Evidence: prompt lines 545-570.*
- [ ] **AC-M5.1.2** Diagnostics view discoverable (`:diag` or `D` key). Test `diagnostics_view_openable` passes.

### M5.2 High-signal notifications, dedup, no flood

- [ ] **AC-M5.2.1** `yt_status`/`yt_error` auto-clear after a timeout or on state change. `grep -n 'status_timeout\|auto_clear\|clear_after' src/tui/` returns ≥1 match. Test `status_auto_clears` passes. *Evidence: tui-recon §6 (TUI-P1-3).*
- [ ] **AC-M5.2.2** No repeating same status continuously. Test `notification_dedup` passes.

### M5.3 Secret redaction + bounded logging

- [ ] **AC-M5.3.1** Sidecar stderr captured to log file (not `Stdio::null()`). `grep -n 'Stdio::null' src/yt/sidecar.rs` returns zero matches OR stderr redirected to bounded log. *Evidence: quality-recon §3 (P3-2).*
- [ ] **AC-M5.3.2** Log redaction: `grep -rn 'cookie\|SAPISID\|token' src/` in log/error paths shows redaction. Test `no_secret_in_logs` passes.
- [ ] **AC-M5.3.3** Bounded log rotation. Test `log_rotation_bounded` passes.

### M5.4 Correlate user errors ↔ diagnostics

- [ ] **AC-M5.4.1** User-visible error includes a correlation id or "see diagnostics" hint. Test `error_correlates_to_diagnostics` passes.

---

## M6: TUI Polish & Responsive

### M6.1 Mode/provider status, now-playing, focus indicators

- [ ] **AC-M6.1.1** Provider status rendered from `ProviderState` (M2), not freeform string. Test `status_renders_provider_state` passes.
- [ ] **AC-M6.1.2** Source indicator (local/remote/mixed) visible per-track or in now-playing. Test `source_indicator_visible` passes. *Evidence: tui-recon §11.*

### M6.2 Empty/loading/error states everywhere

- [ ] **AC-M6.2.1** Empty catalog shows "run jukebox sync" hint. Test `empty_catalog_hint` passes. *Evidence: tui-recon §7.*
- [ ] **AC-M6.2.2** Missing search index shows hint. Test `missing_index_hint` passes.
- [ ] **AC-M6.2.3** Local-load/buffering indicator during track load. Test `loading_indicator_on_track_load` passes. *Evidence: tui-recon §6 (P2-5).*

### M6.3 Responsive 80×24..160×50 + too-small message

- [ ] **AC-M6.3.1** Snapshots exist for: local-populated, local-empty, YT-signed-out, YT-authenticating, YT-synchronizing, YT-ready, YT-no-playlists, offline-cache, provider-failure, search-empty, search-populated, queue-empty, queue-populated, lyrics-loading, lyrics-available, lyrics-unavailable, help, command+history, confirmation, too-small. ≥20 snapshot files in `tests/snapshots/`. *Evidence: prompt lines 614-632.*
- [ ] **AC-M6.3.2** Each snapshot inspected (not auto-updated): diff reviewed and committed.

### M6.4 Wide-char, truncation, no-color, no-flicker

- [ ] **AC-M6.4.1** Zero-width/combining chars handled in display width. Test `display_width_zero_width` passes. *Evidence: tui-recon §5 (P3-2).*
- [ ] **AC-M6.4.2** `NO_COLOR=1` test passes for all overlay states. Test `no_color_all_overlays` passes.
- [ ] **AC-M6.4.3** No accidental destructive single-key actions (no `q`-without-confirm that wipes data). Test `no_destructive_single_key` passes.

### M6.5 Snapshot assertions inspected

- [ ] **AC-M6.5.1** `cargo insta test --review` shows all snapshots accepted (no pending/unreviewed).

---

## M7: Playback & Queue Correctness

### M7.1 Queue/context/prev-next/repeat/shuffle semantics + tests

- [ ] **AC-M7.1.1** "Play next" (insert-at-front) exists. `grep -n 'play_next\|insert(0' src/tui/queue.rs` returns ≥1 match. Test `play_next_inserts_at_front` passes. *Evidence: playback-recon §3.*
- [ ] **AC-M7.1.2** EOF + `>` same-tick does NOT double-advance. Test `eof_and_next_no_double_advance` passes. *Evidence: playback-recon §10 D5.*
- [ ] **AC-M7.1.3** Removing currently-playing track doesn't interrupt playback but updates next. Test `remove_current_track` passes.
- [ ] **AC-M7.1.4** Transport persisted across restart (cursor/order/history). `grep -n 'transport\|cursor\|history' src/state.rs` returns persist logic. Test `transport_persists_across_restart` passes.

### M7.2 Hybrid source identity, fallback, dedup determinism

- [ ] **AC-M7.2.1** Source-failure fallback is automatic and documented. Test `hybrid_fallback_on_remote_fail` passes.
- [ ] **AC-M7.2.2** Queue/history retain stable identity across source switch. Test `stable_identity_across_source_switch` passes.

### M7.3 Source-failure recovery

- [ ] **AC-M7.3.1** Local file disappearance → dead + advance (existing). Test `local_missing_marks_dead` passes (already exists).
- [ ] **AC-M7.3.2** Remote stream failure → error + advance or retry. Test `remote_stream_failure_advances` passes.

### M7.4 Process cleanup, restore-state, mode-switch-while-playing

- [ ] **AC-M7.4.1** No leaked children on panic/exit. Test `no_leaked_children_on_exit` passes (or static assertion Drop guards).
- [ ] **AC-M7.4.2** Mode-switch-while-playing doesn't drop audio. Test `mode_switch_while_playing` passes.

---

## M8: Security & Robustness

### M8.1 Token/cookie perms + secret redaction

- [ ] **AC-M8.1.1** No `/tmp/.config` fallback for cookies/state when `dirs::config_dir()` is None: app refuses to start OR requires explicit `XDG_CONFIG_HOME`. Test `no_world_readable_config_fallback` passes. *Evidence: quality-recon §6 (P1-4).*
- [ ] **AC-M8.1.2** mpv socket path uses per-user runtime dir or random suffix. `grep -n 'jukebox-mpv.sock' src/config.rs` returns zero matches OR uses `XDG_RUNTIME_DIR`. *Evidence: quality-recon §6 (P1-5).*
- [ ] **AC-M8.1.3** Temp cookie files cleaned. `grep -n 'delete=False' scripts/yt/yt.py` returns zero matches. *Evidence: quality-recon §4 (P1-6).*

### M8.2 Terminal escape injection from metadata

- [ ] **AC-M8.2.1** CLI search output escapes control chars. Test `cli_search_escapes_control_chars` passes. *Evidence: quality-recon §5 (P1-2).*
- [ ] **AC-M8.2.2** `Response::from_line` error doesn't include raw cookie material. `grep -n 'unrecognized sidecar response' src/yt/proto.rs` shows sanitization.

### M8.3 Panic/unwrap audit on external data

- [ ] **AC-M8.3.1** `sidecar.rs:65-66` `expect("stdin/stdout piped")` returns `Err` instead of panicking. `grep -n 'expect("stdin piped")\|expect("stdout piped")' src/yt/sidecar.rs` returns zero matches. *Evidence: quality-recon §2 (P1-3).*
- [ ] **AC-M8.3.2** `main.rs:35,201` `current_exe().parent().unwrap()` uses `unwrap_or` or `?`. `grep -n 'parent().unwrap()' src/main.rs` returns zero matches.

### M8.4 Migration safety + corruption recovery

- [ ] **AC-M8.4.1** `state.db` has a `schema_version` key. `grep -n 'schema_version' src/state.rs` returns ≥1 match. *Evidence: quality-recon §11 (P2-3).*
- [ ] **AC-M8.4.2** Corrupt DB auto-recovers (delete + recreate). Test `corrupt_db_recovers` passes.

---

## M9: Performance

### M9.1 Baseline measurements

- [ ] **AC-M9.1.1** Baseline doc exists: `docs/development/jukebox-revamp/PERF.md` with startup/search/sync/lyrics/render timings before+after.

### M9.2 Remove blocking work from render/input path

- [ ] **AC-M9.2.1** No `roundtrip` calls on the play/discover/auto-advance path. `grep -n 'roundtrip\|home_suggestions()\|get_playlist()' src/tui/app.rs` on hot path returns zero blocking calls. *Evidence: playback-recon §8 B2/B3/B4.*
- [ ] **AC-M9.2.2** CONT=YouTube auto-advance is fire-and-forget (no 4s block). Test `cont_youtube_auto_advance_non_blocking` passes.
- [ ] **AC-M9.2.3** `S` Discover opens instantly (no 3s block). Test `discover_opens_instantly` passes.
- [ ] **AC-M9.2.4** Audio format switch doesn't block input loop ≥100ms. Test `audio_switch_does_not_block_input` passes (or measured <100ms).

### M9.3 Cancellation / generation-ids for stale bg

- [ ] **AC-M9.3.1** Generation id on refresh/resolve/lyrics. `grep -n 'gen\|generation\|epoch' src/yt/session.rs src/tui/app.rs` returns ≥1 match each. *Evidence: AUDIT §11 #3.*
- [ ] **AC-M9.3.2** Stale refresh cannot regress `yt_lists`. Test `stale_refresh_does_not_regress_lists` passes.
- [ ] **AC-M9.3.3** `send_refresh` has inflight guard. `grep -n 'refresh_inflight\|playlist_inflight' src/yt/session.rs` returns ≥1 match for refresh.

### M9.4 Bounded caches/history, no full-library scans per frame

- [ ] **AC-M9.4.1** `track_cache` bounded (LRU or cap). `grep -n 'track_cache.*cap\|LRU\|resize\|truncate' src/yt/session.rs` returns ≥1 match. *Evidence: playback-recon §11 D7.*
- [ ] **AC-M9.4.2** id→index HashMap exists. `grep -n 'HashMap<String, usize>\|id_index\|track_index' src/tui/app.rs src/catalog.rs` returns ≥1 match. *Evidence: playback-recon §11 D8.*
- [ ] **AC-M9.4.3** `now_playing_view` called once per frame. `grep -c 'now_playing_view' src/tui/view/player_bar.rs` ≤ 1 call site (cached).
- [ ] **AC-M9.4.4** Sidecar channel bounded. `grep -n 'sync_channel' src/yt/sidecar.rs` returns ≥1 match. *Evidence: playback-recon §11 D11.*

---

## M10: Test Depth & Determinism

### M10.1 Unit tests

- [ ] **AC-M10.1.1** Unit tests for: state transitions, token-expiry, refresh decisions, pagination, cache selection, hybrid identity, command parsing/history, lyrics parsing, queue/transport, error mapping, notification dedup, layout. `cargo test --lib` passes with ≥40 unit tests.

### M10.2 Integration tests

- [ ] **AC-M10.2.1** Integration tests: restart persistence, DB migration, mock YouTube flows, auth refresh, playlist sync, offline cached, provider recovery, playback backend contracts, local+remote fallback, lyrics provider+cache. `cargo test --test '*'` passes.
- [ ] **AC-M10.2.2** `Response::from_line` malformed-input tests exist. Test `from_line_malformed` passes. *Evidence: quality-recon §1 (P2-2).*
- [ ] **AC-M10.2.3** No `set_var("JK_FAKE_MAP")` parallel-test race. `grep -n 'set_var("JK_FAKE_MAP")' tests/e2e_yt.rs` returns zero matches. *Evidence: quality-recon §1 (P2-1).*

### M10.3 TUI tests + snapshots

- [ ] **AC-M10.3.1** TUI tests: key dispatch, focus, overlays, mode switching, empty states, progress/error states, size degradation, mouse hitboxes, snapshots. `cargo test --test columns --test layout --test input` passes.

### M10.4 E2E journeys with fixtures

- [ ] **AC-M10.4.1** E2E journeys A-L covered with deterministic fixtures (no real creds). `cargo test e2e_journey_` passes for all 12.

### M10.5 fmt/clippy/test/release gates green

- [ ] **AC-M10.5.1** `cargo fmt --check` exits 0. *Evidence: quality-recon §0 (42 files failing).*
- [ ] **AC-M10.5.2** `cargo clippy --all-targets --all-features -- -D warnings` exits 0. *Evidence: quality-recon §0 (8 errors).*
- [ ] **AC-M10.5.3** `cargo test --all-features` exits 0.
- [ ] **AC-M10.5.4** `bats scripts/test/*.bats` exits 0.
- [ ] **AC-M10.5.5** `cargo build --release` exits 0.

---

## M11: Independent Judges & Release

### M11.1 Judge 1 — black-box product judge (fresh, read-only)

- [ ] **AC-M11.1.1** Judge 1 run from clean worktree at candidate commit with only repo + requirements + rubric. Score recorded in `JUDGE.md`.

### M11.2 Judge 2 — adversarial engineering judge (fresh, read-only)

- [ ] **AC-M11.2.1** Judge 2 run from clean worktree. Score recorded in `JUDGE.md`.

### M11.3 Fix confirmed blockers + regression tests

- [ ] **AC-M11.3.1** Every judge blocker reproduced, fixed, and has a regression test. `JUDGE.md` lists each with test name.

### M11.4 Final verification from clean checkout

- [ ] **AC-M11.4.1** All AC-M10.5.* pass from a fresh `git worktree add` at the candidate commit.

### M11.5 Both judges ≥90, avg ≥93, no FAIL, no rubric <80%

- [ ] **AC-M11.5.1** `JUDGE.md` shows both scores ≥90, average ≥93, neither FAIL.
- [ ] **AC-M11.5.2** No rubric category <80% of available points (Functional 25, Provider 20, UX 20, Playback 10, Test 10, Security 5, Terminal 5, Maintainability 5).

### Release gates (cross-cutting)

- [ ] **AC-GATE-1** No known P0 or P1 defect open.
- [ ] **AC-GATE-2** No unresolved security/credential-leak issue.
- [ ] **AC-GATE-3** All required local/YouTube/hybrid journeys pass or have documented external limitation.
- [ ] **AC-GATE-4** No false connected/ready state (AC-M2.1.*).
- [ ] **AC-GATE-5** Command history works + tested (AC-M4.*).
- [ ] **AC-GATE-6** Lyrics functional/truthful/non-blocking (AC-M3.*).
- [ ] **AC-GATE-7** Input responsive during slow provider/lyrics (AC-M9.2.*).
- [ ] **AC-GATE-8** Full fmt/clippy/test/build gates green (AC-M10.5.*).
- [ ] **AC-GATE-9** Snapshots inspected (AC-M6.5.1).
- [ ] **AC-GATE-10** Both judges ≥90, avg ≥93, no FAIL (AC-M11.5.1).
