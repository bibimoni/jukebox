# Sync Issues (Unresolved Only)

## SYNC-16
- Severity: MEDIUM (Slice 10 S10.1 — state.rs /tmp/.config fallback — Commander decision needed)
- Files: src/state.rs:30, src/config.rs:53, src/yt/session.rs:113
- Problem: AC-M8.1.1 says "No /tmp/.config fallback for cookies/state when dirs::config_dir() is None." Cookies now REFUSE /tmp (session.rs L577 — fixed). But state.rs:30 and config.rs:53 still use `/tmp/.config` for non-secret files (state.db = UI prefs, config.yml = player config, venv = python deps). Each has a doc comment justifying it ("acceptable here: no secrets"). The AC literally says "cookies/state" — state still has the /tmp fallback.
- Fix: Commander decision — (a) accept the documented justification (state.db has no secrets, /tmp fallback is safe) and mark S10.1 [x] with a note, OR (b) require state.rs to also refuse /tmp and return Err/no-state when dirs::config_dir() is None (state.db is ephemeral UI prefs, losing it is acceptable). Option (a) is reasonable if the Commander accepts the risk.
- Status: pending (Commander decision)

## SYNC-15
- Severity: MEDIUM (Slice 11 S11.4 + S11.6 incomplete — performance tests + sync_channel)
- Files: src/yt/sidecar.rs:84, tests/perf.rs (MISSING), docs/development/jukebox-revamp/PERF.md
- Problem: S11.4: sidecar.rs:84 still `mpsc::channel::<String>()` (unbounded); `sync_channel(64)` NOT implemented; grep `sync_channel` across src/ = 0. S11.6: tests/perf.rs MISSING (glob 0); PERF.md exists (101L) but 4 `_TODO_` markers for "After" timings. M9.4.4 NOT MET; M9.1.1 partial.
- Fix: (1) sidecar.rs: change `mpsc::channel::<String>()` → `mpsc::sync_channel::<String>(64)` to bound the stderr reader's send buffer. (2) Create tests/perf.rs with benchmarks for track_by_id_fast, tracks_for_album, evict_track_cache (capacity + protection). (3) Fill in PERF.md timings. (4) Add now_playing_view ≤1 call/frame fix (M9.4.3 — render_compact L59 + build_info_line L185 both call it).
- Status: pending (Slice 11 worker to implement)

## SYNC-14
- Severity: MEDIUM (Slice 6 S6.3 incomplete — Command overlay line editing)
- Files: src/tui/input.rs:330-388 (Command overlay key handler)
- Problem: AC-M4.2.4 (Home/End/word-movement/word-deletion) + AC-M4.3.1 (Tab completion) NOT implemented. The Command overlay match (input.rs L330-388) only handles Char, Backspace, Up (history recall), Down (history forward), Enter (execute), and `_ => {}` (catch-all). Missing: Home (cursor to start), End (cursor to end), Ctrl-Left/Right (word movement), Ctrl-Backspace/Delete (word deletion), Tab (command completion from a known-command table). The command input is a simple String with push/pop — no cursor position tracking.
- Fix: (1) Add a `cursor: usize` field to Overlay::Command (or track cursor position). (2) Handle KeyCode::Home → cursor=0, KeyCode::End → cursor=input.len(). (3) Handle Ctrl+Left/Right → move cursor by word boundaries. (4) Handle Ctrl+Backspace/Delete → delete word before/after cursor. (5) Handle KeyCode::Tab → complete against a known-command table (e.g., ["queue", "clear", "q", "quit", "diag"]) — prefix-match, insert longest common prefix. (6) Add tests: command_line_editing (Home/End/word-del), command_tab_completion.
- Status: pending (Slice 6 worker to implement)

## SYNC-12
- Severity: HIGH (Slice 10 S10.6 — corrupt-DB recovery fails — test corrupt_db_recovers_to_defaults FAILS)
- Files: src/state.rs:50-63 (open_at), tests/security.rs:40-63 (corrupt_db_recovers_to_defaults)
- Problem: `open_at` (state.rs L54-63) catches `Connection::open` ERRORS (e.g., permission denied) and retries with `remove_file` + re-open. BUT "file is not a database" (SQLite error code 26 = SQLITE_NOTADB) occurs at `execute_batch` (L64), NOT at `Connection::open`. When a garbage file is opened, `Connection::open` SUCCEEDS (it just opens the file handle), but the first SQL operation (`execute_batch("CREATE TABLE IF NOT EXISTS...")`) fails with error 26. So the recovery path at L56-62 is never triggered for corrupt files. Test `corrupt_db_recovers_to_defaults` writes garbage bytes, calls `load_layout_at`, expects auto-recovery, but gets "Error code 26: File opened that is not a database."
- Fix: Wrap the `execute_batch` + schema version query (L64-87) in a try/catch. If they fail with an error containing "not a database" or error code 26, remove the file (`std::fs::remove_file(path)`), re-open fresh (`Connection::open(path)`), and re-run the schema setup. Alternatively, after `Connection::open` succeeds, try a trivial `SELECT 1` — if it fails with error 26, remove + re-open before proceeding to `execute_batch`.
- Status: pending (Slice 10 worker to implement — fixing this will also make S10.7 test pass)

## SYNC-11
- Severity: HIGH (Slice 3 S3.5 incomplete — rate-limit state unreachable)
- Files: src/yt/session.rs, src/tui/app.rs (L1592-1605 error handler)
- Problem: `YtState::RateLimited` is defined in src/yt/state.rs L94 but is NEVER set anywhere. grep `RateLimited|429|rate_limit|throttle|retry_after` in session.rs = 0 matches. The app.rs error handler (L1592-1605) detects auth errors via string matching (`contains("auth")`/`"401"`/`"unauthorized"`/`"expired"`/`"login"`) but has NO branch for rate-limit errors — a 429 response falls through to generic `ReadyStale`/`ProviderError`. So the `RateLimited` state and its hint ("rate limited — wait, then press R") are unreachable dead code.
- Fix: In app.rs error handler, add a rate-limit string check (`contains("429")` || `contains("rate")` || `contains("throttl")` || `contains("too many")`) BEFORE the auth check, and set `self.yt_state = YtState::RateLimited`. Alternatively, have session.rs detect 429 in the Response::Error path and surface a structured rate-limit signal. Add a test that simulates a 429 error and asserts the state becomes RateLimited.
- Status: pending (Slice 3 worker to implement)

## SYNC-10
- Severity: HIGH (Slice 3 S3.3 + S3.4 incomplete — offline disk cache not implemented)
- Files: src/yt/cache.rs (MISSING), src/tui/app.rs (no cache-load logic)
- Problem: S3.3 asked to CREATE src/yt/cache.rs to disk-cache yt_lists (playlists/suggestions) to state.db. The file DOES NOT EXIST (`ls src/yt/cache.rs` → No such file). Consequently S3.4's "load-from-cache-first" objective cannot work: grep `load_.*cache|cached_playlists|cached_suggestions|from_cache` in app.rs = 0 matches. The `ReadyStale` state exists and R-key retry works, but there is no SQLite-backed cache to populate the Y view from on startup or offline. So "offline shows cached marked stale" (the S3.R acceptance criterion) is not implemented.
- Fix: (1) Create src/yt/cache.rs with save_yt_lists/load_yt_lists functions backed by the existing state.db SQLite connection (mirror the save_playlists/load_playlists pattern in src/state.rs). (2) In app.rs App::new, call load_yt_lists to pre-populate playlists/suggestions from cache (mark state ReadyStale). (3) On successful refresh, overwrite the cache. (4) Add tests in tests/pagination_cache.rs (see SYNC-9).
- Status: pending (Slice 3 worker to implement)

## SYNC-9
- Severity: HIGH (Slice 3 S3.6 incomplete — no tests for pagination/cache/ratelimit)
- Files: tests/pagination_cache.rs (MISSING)
- Problem: S3.6 asked to CREATE tests/pagination_cache.rs. The file DOES NOT EXIST (`ls tests/pagination_cache.rs` → No such file). grep across tests/ for `pagination|cache.*stale|RateLimited|offline.*cached|has_more|continuation` returned 0 relevant matches (only an unrelated `source_device_rate.rs` and a ReadyStale.is_ready() assert in provider_state.rs from Slice 1). So NONE of Slice 3's deliverables have unit tests: no pagination test, no offline-cache test, no rate-limit-transition test.
- Fix: Create tests/pagination_cache.rs covering: (1) yt.py pagination — assert get_library_playlists/get_playlist return >25/>100 items (or mock limit=None path); (2) cache round-trip — save yt_lists to a temp state.db, reload, assert equal + stale flag; (3) rate-limit transition — simulate 429 error in session/app, assert YtState::RateLimited; (4) offline_shows_cached_marked_stale — load from cache when sidecar down, assert ReadyStale. Depends on SYNC-10 (cache.rs) and SYNC-11 (RateLimited) being fixed first.
- Status: pending (Slice 3 worker to implement, after SYNC-10 + SYNC-11)

## SYNC-8
- Severity: MEDIUM (Slice 3 S3.2 incomplete — proto.rs pagination metadata missing)
- Files: src/yt/proto.rs
- Problem: S3.2 asked to add `continuation`/`has_more` fields to proto.rs for paginated responses. grep `has_more|continuation|next_page_token` in proto.rs = 0 matches. Response::Playlists(Vec<PlaylistSummary>) and Response::Tracks(Vec<RemoteTrackSummary>) carry no pagination metadata. The implementation deviated: yt.py uses `limit=None` to fetch all results in one sidecar call, so the wire protocol doesn't need continuation tokens. This is a reasonable design choice BUT (a) the deviation is undocumented in proto.rs, and (b) the task was marked [x] without justification.
- Fix: Either (a) add `has_more: bool` / `continuation: Option<String>` fields to a Playlists/Tracks response wrapper and wire yt.py to send them (enables future incremental pagination), OR (b) document the design deviation in a proto.rs doc comment ("pagination is delegated to the sidecar via limit=None; the wire protocol returns complete result sets in one response, so no continuation token is needed") and formally descope S3.2. Option (b) is acceptable if the Commander approves.
- Status: pending (Commander decision: implement fields or descope with docs)

## RESOLVED (deleted from active list)
- ~~SYNC-3~~: RESOLVED 2026-07-12T02:38 — polling loop fix in e2e_yt.rs
- ~~SYNC-4~~: RESOLVED 2026-07-12T02:34 — cargo fmt --check now PASSES
- ~~SYNC-5~~: RESOLVED 2026-07-12T02:34 — duplicate yt_status_text removed
- ~~SYNC-6~~: RESOLVED 2026-07-12T02:49 — cargo clippy --tests --all-features exit 0; no E0502 borrow errors in app.rs:1730s (lyrics on_tick)
- ~~SYNC-7~~: RESOLVED 2026-07-12T02:49 — cargo clippy --lib --all-features exit 0; session.rs compiles clean (SYNC-7's 14 errors fixed by prior worker completing the Pending tuple-variant refactor)
- ~~SYNC-10a~~ (Slice 10 S10.2): RESOLVED 2026-07-12T02:50 — config.rs default_mpv_socket() now prefers XDG_RUNTIME_DIR (L37); AC-M8.1.2 MET
- ~~SYNC-10b~~ (Slice 10 S10.5): RESOLVED 2026-07-12T02:50 — main.rs now uses current_exe()? (L39, L241); grep parent().unwrap() = 0; AC-M8.3.2 MET
- ~~SYNC-10c~~ (Slice 10 S10.7): PARTIALLY RESOLVED 2026-07-12T02:50 — tests/security.rs now EXISTS (81 lines, 3 tests); 2 PASS, 1 FAIL (corrupt_db — see SYNC-12)
