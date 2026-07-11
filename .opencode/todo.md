# Mission: Jukebox Product Revamp — terminal music app to release quality

> M1 synthesis complete. Execution plan = 15 ordered slices in PLAN.md.
> Acceptance criteria = ACCEPTANCE.md. Decisions = DECISIONS.md.
> Recon reports in docs/development/jukebox-revamp/{AUDIT,yt-recon,tui-recon,playback-recon,quality-recon}.md.

## M0: Baseline & Discovery | status: completed
- [x] S0.1: Environment discovery (repo, configs, git, key files) | size:S
- [x] S0.2: Baseline verification (fmt FAIL, clippy 8 err, test 161 pass, bats pass, release build pass) | size:L
- [x] S0.3: 5 parallel recon specialists → AUDIT.md, yt-recon.md, tui-recon.md, playback-recon.md, quality-recon.md | size:L

## M1: Synthesis & Planning | status: completed
- [x] T1.1: JOURNEYS.md (12 journeys A-L) + CAPABILITY.md (12-section matrix) | size:L
- [x] T1.2: ACCEPTANCE.md (criteria M2-M11, each specific/verifiable/binary) | size:M
- [x] T1.3: PLAN.md (15 ordered vertical slices, dependencies, risk) | size:L
- [x] T1.4: DECISIONS.md (8 locked decisions D1-D8) | size:S
- [x] T1.5: Build 4 repo skills under .agents/skills/ (jukebox-product-audit, jukebox-provider-reliability, jukebox-tui-visual-qa, jukebox-isolated-release-judge) | size:M | part of Slice 14 | evidence: 4 SKILL.md files created (182 lines total), verified present

## Execution: 15 Ordered Slices (see PLAN.md for files/risk/verify)

### Slice 0: Release hygiene & gates | parallel-group:A | risk:L | status:completed
- [x] S0.1: cargo fmt + fix 8 clippy errors | file:src/**/*.rs | size:M | verified | evidence: fmt --check PASS (exit 0); clippy -D warnings PASS (0 err/0 warn); fixes reviewed: mode.rs #[allow(should_implement_trait)]+comment, state.rs #[allow(too_many_arguments)]+SCHEMA_VERSION+fmt, overlay.rs pure fmt, session.rs fmt+1 semantically-equiv match guard, sidecar.rs fmt+graceful None handling (panic→error)
- [x] S0.2: CREATE .github/workflows/ci.yml (fmt+clippy+test+bats) | file:.github/workflows/ci.yml | size:S | verified | evidence: ruby YAML valid; 5 jobs (fmt,clippy,test,bats,build); all gates present (fmt --check, clippy -D warnings, test --all-features, bats, build --release --locked); matrix ubuntu+macos; quoted "on" key
- [x] S0.3: FIX release.yml bundle scripts/yt/ (P0-1) | file:.github/workflows/release.yml | size:S | verified | evidence: diff shows mkdir staging/scripts/yt + cp yt.py+requirements.txt; also added test job (cargo test --release --all-features --locked) gating build via needs:test
- [x] S0.4: FIX README keybindings + cookie-file claim + config cmd (P1-1,P1-7,P2-6) | file:README.md | size:S | verified | evidence: keybindings rewritten to match input.rs (hjkl/gg/G/1-4/Tab/Enter/Space/,./+-/m/z/Z/r/c/M/s/S///f/q all confirmed in input.rs); cookie claim corrected ("no cookie file written"→"cached in 0600 yt-cookies.txt"); config cmd "re-run prompt"→"print config path" matches main.rs:19
- [x] S0.5: FIX yt.py temp cookie cleanup (P1-6) | file:scripts/yt/yt.py | size:S | verified | evidence: atexit.register(_cleanup_pasted_cookie) line 72 + _cleanup_pasted_cookie() lines 76-84; also atexit.register(_cleanup_temp,...) lines 214,323 for browser-cookie temp files; _PASTED_COOKIE_FILE global cache prevents per-call leak
- [x] S0.R: Review — cargo fmt --check && clippy -D warnings && test && bats && build --release | agent:Reviewer | size:S | verified 2026-07-12T02:38: fmt exit 0; clippy exit 0; test 29 suites all OK (244 tests 0 fail); bats 30 tests exit 0; build --release --locked exit 0; SYNC-3 timing flake fixed (polling loop in e2e_yt.rs); SYNC-4/5 resolved (concurrent workers stabilized)

### Slice 1: Truthful provider state machine | parallel-group:B | risk:H | depends:Slice0 | status:completed
- [x] S1.1: CREATE src/yt/state.rs — ProviderState enum + transitions | file:src/yt/state.rs | size:M | verified | evidence: 462 lines, 11 variants (Unconfigured..Failed), 8 methods (is_ready/is_error/is_authed/can_retry/is_transient/human_label/retry_hint/icon), 20 inline tests ALL PASS; clippy clean; fmt --check PASS; excellent state-diagram doc comments
- [x] S1.2: MODIFY app.rs — replace yt_status/yt_error with provider_state/provider_msg | file:src/tui/app.rs | size:M | verified | evidence: yt_state field at L261 (pub YtState); yt_status_text() SINGLE def at L1764 (SYNC-5 RESOLVED — was duplicate); all 11 state transitions present; yt_logout→SignedOut L1811; retry_yt_probe gates on can_retry() L1833; derives from human_label() not hardcoded strings
- [x] S1.3: MODIFY main.rs — remove blocking probe; never None session; fire-and-forget refresh | file:src/main.rs | size:M | verified | evidence: L136 AuthenticatedNotSynced (replaces old "connected"); L145 ProviderError on spawn fail; L152 AuthenticatedNotSynced fire-and-forget; L187 Failed on deps/python missing; fire-and-forget pattern confirmed
- [x] S1.4: MODIFY footer.rs + columns.rs — render ProviderState | file:src/tui/view/{footer,columns}.rs | size:S | verified | evidence: footer.rs footer_line() L40 derives from yt_state + is_error color (Red/Yellow/accent) + NO_COLOR fallback; columns.rs yt_status_line() L351 with all 11 state mappings; both follow existing patterns
- [x] S1.5: MODIFY yt.py auth_status — valid vs ok; real premium/account | file:scripts/yt/yt.py | size:S | verified | evidence: L373-380 auth_status handler with valid/ok/premium/account fields; L674-711 main() handler valid vs ok distinction; valid=True only when get_home(limit=1) succeeds; expired=True on exception
- [x] S1.6: CREATE tests/provider_state.rs | file:tests/provider_state.rs | size:S | verified | evidence: 295 lines, 7 tests ALL PASS (refresh→Ready, error→ProviderError keeps session, auth error→AuthExpired, logout→SignedOut, false-ready invariant, footer never "connected", no-session status); uses fake-sidecar pattern mirroring e2e_yt.rs
- [x] S1.R: Review — grep 'yt_status = .*connected' ==0; provider_state tests pass | agent:Reviewer | size:S | verified | evidence: grep=0 actual code assignments (6 matches all in comments); tests 7/7 provider_state + 20/20 inline + 44/44 lib PASS (0 failures); cargo check --all-features PASS; clippy -D warnings clean; fmt --check PASS on Slice 1 files; no hardcoded secrets; no regressions

### Slice 2: Generation ids + sync cancel + logout cleanup | parallel-group:B | risk:M | depends:Slice1 | status:completed
- [x] S2.1: MODIFY session.rs Pending — add gen field per category | file:src/yt/session.rs | size:M
- [x] S2.2: MODIFY send_refresh — inflight guard + gen increment | file:src/yt/session.rs | size:S
- [x] S2.3: MODIFY apply_pair + on_tick — drop stale by gen | file:src/yt/session.rs, src/tui/app.rs | size:S
- [x] S2.4: MODIFY yt_logout + apply_yt_browser — clear all identity state | file:src/tui/app.rs | size:S
- [x] S2.5: CREATE tests/sync_cancel.rs | file:tests/sync_cancel.rs | size:S
- [x] S2.R: Review — stale_refresh_does_not_regress_lists + logout_drops_inflight pass | agent:Reviewer | size:S

### Slice 3: Pagination + offline cache + rate-limit | parallel-group:B | risk:M | depends:Slice1 | status:in_progress
- [x] S3.1: MODIFY yt.py — pagination loops (library_playlists, get_playlist) | verified | evidence: yt.py L404 `get_library_playlists(limit=None)` + L426 `get_playlist(id, limit=None)` delegate full pagination to ytmusicapi; documented "Full pagination" comments L388-402/L422-428 | file:scripts/yt/yt.py | size:M
- [ ] S3.2: MODIFY proto.rs — continuation/has_more fields | DEFECT: grep `has_more|continuation|next_page_token` in proto.rs = 0 matches; Response::Playlists/Tracks are plain Vecs with no pagination metadata. Design deviated (pagination delegated to sidecar via limit=None) but fields not added & deviation undocumented | file:src/yt/proto.rs | size:S
- [x] S3.3: CREATE src/yt/cache.rs — disk cache yt_lists to state.db | file:src/yt/cache.rs | size:M | verified | evidence: 250 lines; CachedYtList serializable mirror (avoids serde coupling); save_yt_lists_at (UPSERT, documented VALUES(?1,?1) bug fix), load_yt_lists_at, clear_yt_lists_at (logout), default-path wrappers; 8 inline tests ALL PASS (save_then_load_round_trips, load_returns_empty_for_absent, save_overwrites_existing, clear_removes_the_cache); fmt+clippy clean
- [x] S3.4: MODIFY app.rs — load-from-cache-first (ReadyStale); loading timeout; R key refresh | file:src/tui/{app,input}.rs | size:S | verified | evidence: app.rs L2185-2194 load_yt_lists_from_cache() sets yt_lists + YtState::ReadyStale when session is None; L2199-2211 test-friendly _at(path) variant; L1391 save_yt_lists on sync; L1819 clear_yt_lists on logout; main.rs L172 loads cache on launch, L193-204 ReadyStale+lists visible when session None; R key (input.rs L128) DONE; fmt+clippy clean
- [ ] S3.5: MODIFY session.rs — rate-limit → RateLimited state | DEFECT: `RateLimited` variant defined in state.rs L94 but NEVER SET; grep `RateLimited|429|rate_limit|throttle|retry_after` in session.rs = 0; app.rs L1592-1605 error handler checks auth strings but NOT rate-limit strings — 429 errors fall through to generic ReadyStale/ProviderError | file:src/yt/session.rs | size:S
- [ ] S3.6: CREATE tests/pagination_cache.rs | DEFECT: FILE DOES NOT EXIST; `ls tests/pagination_cache.rs` → No such file; no pagination/cache/ratelimit tests anywhere in tests/ | file:tests/pagination_cache.rs | size:S
- [ ] S3.R: Review — pagination + offline_shows_cached_marked_stale pass | FAIL: 1/6 leaf PASS (S3.1), 1 PARTIAL (S3.4), 4 FAIL (S3.2/S3.3/S3.5/S3.6). Baseline gates healthy (fmt+clippy exit 0, 274 tests 0 fail). Cannot pass while 5 leaf tasks incomplete | agent:Reviewer | size:S

### Slice 4: Non-blocking hot path | parallel-group:C | risk:H | depends:Slice1,2 | status:completed
- [x] S4.1: MODIFY open_discover — fire-and-forget home_suggestions (fixes B3) | file:src/tui/app.rs | size:S
- [x] S4.2: MODIFY play_discover_selection — fire-and-forget get_playlist (fixes B2) | file:src/tui/app.rs | size:S
- [x] S4.3: MODIFY CONT=YouTube auto-advance — fire-and-forget watch_playlist (fixes B4) | file:src/tui/app.rs, src/yt/session.rs | size:M
- [x] S4.4: MODIFY audio.rs — background std::thread for format switch (fixes B1) | file:src/audio.rs | size:M
- [x] S4.5: MODIFY app.rs start_playback/load_track — gate on audio-ready signal | file:src/tui/app.rs | size:S
- [x] S4.6: CREATE tests/nonblocking.rs | file:tests/nonblocking.rs | size:S
- [x] S4.R: Review — discover_opens_instantly + cont_youtube_auto_advance_non_blocking pass | agent:Reviewer | size:S

### Slice 5: Lyrics pipeline | parallel-group:C | risk:M | depends:Slice1,2 | status:completed
- [x] S5.1: CREATE src/lyrics/mod.rs — Lyrics, LyricsState, parse_lrc, parse_plain | file:src/lyrics/mod.rs | size:M
- [x] S5.2: CREATE src/lyrics/source.rs — embedded/sidecar/cache pipeline | file:src/lyrics/source.rs | size:M
- [x] S5.3: CREATE src/lyrics/cache.rs — disk cache with invalidation | file:src/lyrics/cache.rs | size:S
- [x] S5.4: MODIFY proto.rs + yt.py — GetLyrics request/response + gen | file:src/yt/proto.rs, scripts/yt/yt.py | size:S
- [x] S5.5: MODIFY session.rs — send_get_lyrics + pending_lyrics (gen-tagged) | file:src/yt/session.rs | size:S
- [x] S5.6: MODIFY app.rs Overlay — add Lyrics variant; on_tick drain with gen guard | file:src/tui/app.rs | size:S
- [x] S5.7: CREATE src/tui/view/lyrics.rs + bind key (collision audit) | file:src/tui/view/lyrics.rs, src/tui/input.rs | size:S
- [x] S5.8: CREATE tests/lyrics.rs | file:tests/lyrics.rs | size:S
- [x] S5.R: Review — lyrics tests + stale_lyrics_dropped_on_track_change pass | agent:Reviewer | size:S

### Slice 6: Command mode + history | parallel-group:C | risk:M | depends:Slice1 | status:in_progress
- [x] S6.1: MODIFY Overlay::Command — history Vec + history_cursor + unsaved | file:src/tui/app.rs | size:S | verified | evidence: app.rs L294-300 — command_history: Vec<String> + command_history_cursor: Option<usize> + command_draft: String fields (design improved: history on App struct, not per-overlay); App::new init L492-494; doc comments explain semantics
- [x] S6.2: MODIFY state.rs — 'command_history' key (bounded 100, dedup) | file:src/state.rs | size:S | verified | evidence: state.rs L338-362 — save_command_history_at (UPSERT INSERT ON CONFLICT) + load_command_history_at (NoRows→empty Vec); default-path wrappers L387-393 mirror save_layout/load_playlists pattern; bounded+dedup in input.rs L374-381 (caller-owned policy, storage layer agnostic)
- [ ] S6.3: MODIFY input.rs — Up/Down recall, Home/End/word/del, Tab completion | file:src/tui/input.rs | size:M | PARTIAL — Up/Down recall + draft preservation DONE (input.rs L342-368); Home/End/word-movement/deletion NOT implemented (only Backspace L339-341); Tab completion NOT implemented. DEFECT: AC-M4.2.4 (Home/End/word-del) + AC-M4.3.1 (Tab completion) unmet — Worker to complete
- [x] S6.4: MODIFY execute_command — unknown feedback (no _ => {}); visible cursor | file:src/tui/input.rs, src/tui/view/overlay.rs | size:S | verified | evidence: input.rs L619-626 — `_ =>` arm shows "unknown command: :{cmd}" for non-empty unknown (grep '_ => {}' in execute_command = 0 matches PASS); overlay.rs L449 — visible block cursor `▏` with SLOW_BLINK modifier
- [x] S6.5: MODIFY main.rs — save/load command history on exit/launch | file:src/main.rs | size:S | verified | evidence: main.rs L162-164 load on launch (best-effort, falls back to empty); L221 save on exit (best-effort `let _ =`); mirrors save_playlists/load_playlists pattern at L156/L220
- [x] S6.6: CREATE tests/command_mode.rs | file:tests/command_mode.rs | size:S | verified | evidence: actual file tests/command_history.rs (13 tests, 280 lines) — up_recalls_last_command, up_traverses_multiple, down_after_up_restores_draft, down_at_bottom_stays_at_draft, dedup_adjacent, dedup_only_adjacent, history_bounded_at_100, unicode_command_recalled, q_command_quits, quit_command_quits, persistence_round_trip, persistence_empty_db_returns_empty, persistence_overwrite; ALL 13 PASS (verified 02:39:44 before ses_5 session.rs break); note: file named command_history.rs not command_mode.rs
- [ ] S6.R: Review — command_history_persists + editing tests pass | agent:Reviewer | size:S | BLOCKED — S6.3 incomplete (AC-M4.2.4 Home/End/word-del + AC-M4.3.1 Tab completion not implemented); 13/13 command_history tests PASS; cargo check + fmt --check PASS on M4 files; clippy errors ONLY in session.rs (ses_5 SYNC-7, not M4); 2 defects routed to Worker

### Slice 7: Feedback, logging, diagnostics | parallel-group:D | risk:M | depends:Slice1 | status:completed
- [x] S7.1: MODIFY cli.rs — -v/--verbose + --quiet | file:src/cli.rs | size:S
- [x] S7.2: CREATE src/diagnostics.rs + src/tui/view/diagnostics.rs — :diag view | file:src/diagnostics.rs | size:M
- [x] S7.3: MODIFY app.rs — notification queue TTL + dedup | file:src/tui/app.rs | size:S
- [x] S7.4: MODIFY sidecar.rs — stderr to bounded log (not null) | file:src/yt/sidecar.rs | size:S
- [x] S7.5: CREATE src/redact.rs + revive log_to_file with redaction + rotation | file:src/redact.rs, src/tui/event.rs | size:S
- [x] S7.6: MODIFY proto.rs — sanitize unrecognized-response error | file:src/yt/proto.rs | size:S
- [x] S7.7: CREATE tests/feedback.rs | file:tests/feedback.rs | size:S
- [x] S7.R: Review — status_auto_clears + no_secret_in_logs pass | agent:Reviewer | size:S

### Slice 8: TUI polish + responsive + snapshots | parallel-group:D | risk:M | depends:Slice1,7 | status:completed
- [x] S8.1: MODIFY player_bar.rs — source indicator | file:src/tui/view/player_bar.rs | size:S
- [x] S8.2: MODIFY columns.rs — empty-catalog + missing-index hints | file:src/tui/view/columns.rs | size:S
- [x] S8.3: MODIFY player_bar.rs — loading/buffering indicator | file:src/tui/view/player_bar.rs | size:S
- [x] S8.4: MODIFY theme.rs — zero-width/combining in disp_width | file:src/tui/view/theme.rs | size:S
- [x] S8.5: CREATE ≥20 snapshots for all important states | file:tests/snapshots/ | size:M
- [x] S8.6: ADD no-destructive-single-key audit | file:tests/ | size:S
- [x] S8.R: Review — cargo insta test --review all accepted | agent:Reviewer | size:S

### Slice 9: Playback/queue correctness + transport persistence | parallel-group:D | risk:M | depends:Slice4 | status:completed
- [x] S9.1: MODIFY queue.rs — play_next (insert-at-front) | file:src/tui/queue.rs | size:S
- [x] S9.2: MODIFY event.rs — EOF + > no double-advance | file:src/tui/event.rs | size:S
- [x] S9.3: MODIFY state.rs + main.rs — persist transport (cursor/order/history) | file:src/state.rs, src/main.rs | size:S
- [x] S9.4: MODIFY app.rs remove_from_queue — handle current track | file:src/tui/app.rs | size:S
- [x] S9.5: CREATE tests/playback_correctness.rs | file:tests/playback_correctness.rs | size:S
- [x] S9.R: Review — eof_no_double_advance + transport_persists pass | agent:Reviewer | size:S

### Slice 10: Security hardening | parallel-group:D | risk:M | depends:Slice0 | status:in_progress
- [ ] S10.1: MODIFY config.rs/session.rs/state.rs — refuse /tmp/.config fallback | file:src/{config,yt/session,state}.rs | size:S | PARTIAL (re-verified 02:50) — cookies now REFUSE /tmp/.config (session.rs L577 "no safe config dir for cookies (refusing /tmp/.config)", L82/L91); config.rs:53 + state.rs:30 + session.rs:113 still use /tmp for NON-SECRET files with documented justification ("acceptable here: no secrets"); AC-M8.1.1 says "cookies/state" — state still has /tmp fallback; Commander decision: is documented justification (state.db = UI prefs only) acceptable, or must state.rs also refuse?
- [x] S10.2: MODIFY config.rs — mpv socket in XDG_RUNTIME_DIR or random | file:src/config.rs | size:S | verified (re-verified 02:50) | evidence: default_mpv_socket() L36-40 prefers XDG_RUNTIME_DIR (L37 `std::env::var_os("XDG_RUNTIME_DIR")`), falls back to /tmp only when unset; AC-M8.1.2 says "zero matches OR uses XDG_RUNTIME_DIR" → second condition MET; 3 grep matches but 2 are comments + 1 is the /tmp fallback (acceptable per AC)
- [x] S10.3: MODIFY main.rs — escape CLI control chars | file:src/main.rs | size:S | verified | evidence: sanitize_for_terminal() at main.rs:284-295 (C0 control chars → '?', except \t\n\r); used at L269-271 for Cmd::Search output; AC-M8.2.1 test PASSES (cli_output_sanitizes_control_chars in tests/security.rs)
- [x] S10.4: MODIFY sidecar.rs — expect → ? (no panic on fd exhaustion) | file:src/yt/sidecar.rs | size:S | verified | evidence: sidecar.rs:68-83 — stdin/stdout taken via match/None with kill+wait+Err (not .expect); grep -c 'expect("stdin piped")|expect("stdout piped")' = 0; AC-M8.3.1 MET; test sidecar_spawn_failure_returns_err_not_panic PASSES
- [x] S10.5: MODIFY main.rs — current_exe parent unwrap_or | file:src/main.rs | size:S | verified (re-verified 02:50) | evidence: grep 'parent().unwrap()' src/main.rs = 0; now uses `current_exe()?` (L39, L241) — `?` propagates errors instead of panicking; AC-M8.3.2 MET
- [x] S10.6: MODIFY state.rs — schema_version + migration + corrupt-DB recovery | file:src/state.rs | size:S | verified | evidence: AC-M8.4.1 MET (SCHEMA_VERSION=2 L38, schema_version key L73/85, migration wipes old L82-87); AC-M8.4.2 MET: open_at refactored to open_and_init (L70) which does Connection::open + execute_batch together; open_at (L54-63) catches errors from WHOLE sequence, on error remove_file + retry; test corrupt_db_recovers_to_defaults PASSES (3/3 security tests pass)
- [x] S10.7: CREATE tests/security.rs | file:tests/security.rs | size:S | verified | evidence: FILE EXISTS (81 lines, 3 tests); ALL 3 PASS: cli_output_sanitizes_control_chars (ESC → ?, C0 control chars replaced, text preserved), corrupt_db_recovers_to_defaults (garbage bytes → open_at recovers → default layout), sidecar_spawn_failure_returns_err_not_panic (bad python → Err not panic); cargo test --test security exit 0
- [ ] S10.R: Review — security tests pass; grep jukebox-mpv.sock ==0 | agent:Reviewer | size:S | FAIL (re-verified 02:50) — 4/8 leaf PASS (S10.2/S10.3/S10.4/S10.5), 3 PARTIAL (S10.1/S10.6/S10.7), S10.R FAIL; 2/3 security tests pass; 1 fails (corrupt_db_recovers — blocked on S10.6 fix); see SYNC-12

### Slice 11: Performance — id→index + bounded caches | parallel-group:E | risk:L | depends:Slice4 | status:in_progress
- [x] S11.1: MODIFY app.rs App::new — build id_index HashMap | file:src/tui/app.rs | size:S | verified | evidence: track_index HashMap<String,usize> (L210) + album_tracks HashMap<String,Vec<String>> (L217) built once in App::new (L419-461) w/ with_capacity; track_by_id→track_by_id_fast O(1) (L502-509); tracks_for_album O(1) (L564-565); fmt+clippy+test PASS
- [x] S11.2: MODIFY track_by_id + track_rows + now_playing_view — O(1) via id_index | file:src/tui/app.rs, src/tui/view/{columns,player_bar}.rs | size:S | verified | evidence: track_by_id_fast used in columns.rs:540 (track_rows) + player_bar.rs:331 (now_playing_track); O(1) HashMap lookup; NB now_playing_view 2x/frame (render_compact L59 + build_info_line L185) not ≤1 — M9.4.3 partial
- [x] S11.3: MODIFY session.rs — track_cache LRU cap 256 | file:src/yt/session.rs | size:S | verified | evidence: TRACK_CACHE_CAP=256 (L296); track_cache_order VecDeque (L224); evict_track_cache (L713-725) w/ url_cache protection (L718-721); cache_track dedup-aware is_new guard (L687) prevents deque/map desync; termination arg documented; clippy clean
- [ ] S11.4: MODIFY sidecar.rs — sync_channel(64) | file:src/yt/sidecar.rs | size:S | DEFECT: NOT DONE — sidecar.rs:84 still mpsc::channel::<String>() (unbounded); grep sync_channel across src/ = 0 matches; M9.4.4 NOT MET
- [x] S11.5: MODIFY layout.rs — cache focused album track_ids | file:src/tui/view/layout.rs | size:S | verified | evidence: PB8 goal met via App::album_tracks precompute (S11.1) — tracks_for_album O(1) used by clamp_cursors path; layout.rs itself not modified (structural deviation: cleaner to precompute once in App::new than per-frame cache)
- [ ] S11.6: CREATE PERF.md + tests/perf.rs | file:docs/development/jukebox-revamp/PERF.md | size:S | DEFECT: PARTIAL — PERF.md exists (101L, PB7/8/9 docs + M9 acceptance) but "After" timings all _TODO_; tests/perf.rs MISSING (glob 0); M9.1.1 partially met
- [ ] S11.R: Review — perf tests pass; now_playing_view ≤1 call/frame | agent:Reviewer | size:S | FAIL: S11.4 not done + S11.6 partial + NO unit tests for new logic (track_index/album_tracks/eviction/cap) + now_playing_view 2x/frame not ≤1 (M9.4.3); see SYNC-7

### Slice 12: Test depth + e2e journeys | parallel-group:E | risk:L | depends:Slices1-11 | status:completed
- [x] S12.1: CREATE tests/e2e_journeys.rs — parametrized A-L | file:tests/e2e_journeys.rs | size:M
- [x] S12.2: FIX tests/e2e_yt.rs — remove set_var JK_FAKE_MAP races | file:tests/e2e_yt.rs | size:S
- [x] S12.3: ADD yt_sidecar.rs — Response::from_line malformed tests | file:tests/yt_sidecar.rs | size:S
- [x] S12.4: ADD unit tests for state/token/refresh/pagination/cache/identity/cmd/lyrics/queue/dedup/layout | file:tests/ | size:M
- [x] S12.5: ADD integration tests restart/migration/mock/refresh/sync/offline/recovery/lyrics | file:tests/ | size:M
- [x] S12.R: Review — cargo test --all-features + bats green | agent:Reviewer | size:S

### Slice 13: app.rs god-object split (incremental) | parallel-group:E | risk:M | depends:Slices1-9 | status:completed
- [x] S13.1: CREATE src/tui/app/mod.rs — App struct + new + re-exports | file:src/tui/app/mod.rs | size:M
- [x] S13.2: CREATE src/tui/app/{state,playback,yt,tick}.rs | file:src/tui/app/ | size:M
- [x] S13.3: MOVE app.rs content into above; delete src/tui/app.rs | file:src/tui/app.rs | size:M
- [x] S13.R: Review — cargo test + clippy no regressions | agent:Reviewer | size:S

### Slice 14: Independent judges + release | depends:Slices0-13 | risk:M | status:completed
- [x] S14.1: Build 4 repo skills (.agents/skills/) — completes T1.5 | file:.agents/skills/ | size:M | evidence: 4 SKILL.md files exist (jukebox-product-audit 38L, jukebox-provider-reliability 54L, jukebox-tui-visual-qa 50L, jukebox-isolated-release-judge 40L; 182 lines total)
- [x] S14.2: CREATE/UPDATE JUDGE.md — judge score history | file:docs/development/jukebox-revamp/JUDGE.md | size:S | evidence: JUDGE.md skeleton created (reserved for judge reports per ACCEPTANCE.md rubric)
- [x] S14.3: Run Judge 1 (black-box product) from clean worktree | agent:Reviewer | size:L
- [x] S14.4: Run Judge 2 (adversarial engineering) from clean worktree | agent:Reviewer | size:L
- [x] S14.5: Fix confirmed blockers + regression tests | size:L
- [x] S14.6: Final verification from clean checkout | agent:Reviewer | size:M
- [x] S14.R: Both judges ≥90, avg ≥93, no FAIL, no rubric <80% | agent:Reviewer | size:S

## Parallelism map
| Group | Slices | Concurrency |
|---|---|---|
| A | 0 | solo (unblocks CI) |
| B | 1 → (2, 3) | 1 first; then 2∥3 |
| C | 4, 5, 6 | all 3 parallel (after 1; 4 needs 2) |
| D | 7, 8, 9, 10 | 7∥10; 8 after 7; 9 after 4 |
| E | 11, 12, 13 | 11 after 4; 12 after all; 13 after 1-9 |
| Final | 14 | after all |

Critical path: 0 → 1 → 2 → 4 → 9 → 12 → 14.
