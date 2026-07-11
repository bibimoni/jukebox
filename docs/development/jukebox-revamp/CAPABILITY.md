# Jukebox Capability Matrix

**Date:** 2026-07-12 · **Source:** Synthesized from AUDIT.md, yt-recon.md, tui-recon.md, playback-recon.md, quality-recon.md.

Classification legend: **Core** = required; **Defective** = exists but broken; **Missing** = not implemented; **Provider-limited** = external constraint; **Out-of-scope** = intentionally excluded.

State: ✅ works · ⚠️ defective · ❌ missing

---

## 1. Provider state & auth lifecycle

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Explicit provider state machine | ❌ | Core | yt-recon §8; AUDIT §4 | `yt_status`/`yt_error` freeform strings; no enum; 10 false-ready sites |
| No false "connected" before data usable | ❌ | Core | yt-recon §10 (table) | `main.rs:117,129`; `app.rs:988,1014,1093` |
| `auth_status` reflects validity not presence | ❌ | Core | yt-recon §4 (`yt.py:329-330`) | `_has_auth()` checks SAPISID string exists |
| Session restore without forcing re-login | ⚠️ | Core | yt-recon §3 (`main.rs:151`) | Probe discards session on any error → repeated logins |
| Non-blocking startup (no 3s probe block) | ❌ | Core | playback-recon §8 B6 (`main.rs:148`) | Blocking `library_playlists()` 3s deadline |
| Token refresh / expiry / revocation | ❌ | Core | yt-recon §2,§4 | No refresh mechanism; no expiry detection; silent empty |
| Logout clears all identity state | ⚠️ | Core | yt-recon §9 (`app.rs:1384-1393`) | Cookie deleted; `yt_lists`/`track_cache`/`url_cache` NOT cleared |
| Account switch clears stale data | ❌ | Core | yt-recon §9 | `apply_yt_browser` doesn't clear lists |
| In-flight results don't resurrect logged-out data | ❌ | Core | yt-recon §9 (`app.rs:1126-1148`) | Unconditional apply |
| Real premium/account detection | ❌ | Important | yt-recon §4 | `premium`/`account` identical to `ok` |

## 2. Playlist sync, pagination, cache, offline

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Playlist pagination complete or communicated | ❌ | Core | yt-recon §5 (`yt.py:345,364`) | Defaults 25 playlists / 100 tracks, no continuation |
| Empty vs failed distinction | ❌ | Core | yt-recon §5 | Both → `[]`; `Ok(empty)` passes probe |
| Cached playlists shown when offline, marked stale | ❌ | Core | yt-recon §6 | In-memory only; lost on restart |
| Manual refresh with feedback | ⚠️ | Core | `app.rs:1653-1675` | `refresh_yt_lists` exists; `yt_lists_loading` can hang |
| Sync cancellable / superseded (generation ids) | ❌ | Core | AUDIT §11 #3; yt-recon §7 | No generation id; stale can regress `yt_lists` |
| `send_refresh` inflight guard | ❌ | Core | yt-recon §7 (`session.rs:714`) | Multiple refreshes stack FIFO |
| Retry with sensible backoff, no TUI freeze | ❌ | Core | — | No retry; probe nukes on first failure |
| Rate-limit detection + state | ❌ | Important | — | Not implemented |

## 3. Lyrics pipeline

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Discoverable lyrics view/overlay | ❌ | Core | AUDIT §13 Q5 | No `Overlay::Lyrics` |
| Current-track changes update request | ❌ | Core | — | No lyrics code at all |
| Slow retrieval never blocks playback/input | ❌ | Core | — | — |
| Stale results can't overwrite newer track | ❌ | Core | — | No generation id |
| States: loading/available/unsynced/not-found/offline/error/retry | ❌ | Core | — | — |
| Local caching + invalidation | ❌ | Core | — | — |
| Manual refresh | ❌ | Core | — | — |
| Synced-line highlighting (timestamped) | ❌ | Core | — | — |
| Plain lyrics useful when no timestamps | ❌ | Core | — | — |
| Unicode + long lines + scroll | ❌ | Core | — | — |
| No fabricated lyrics | ❌ | Core | — | — |
| Provider pipeline (embedded/sidecar `.lrc`/cache) | ❌ | Core | `.opencode/prompt:489-514` | ytmusicapi `get_lyrics` exists, unused |
| Deterministic tests | ❌ | Core | — | — |

## 4. Command mode & Vim interaction

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| In-session command history | ❌ | Core | AUDIT §13 Q6 | No Vec in `Overlay::Command` |
| Persistent history across restart | ❌ | Core | `state.rs` no 'command_history' key | — |
| Up/Down traversal | ❌ | Core | `input.rs:283-298` | Only Char/Backspace/Enter |
| History cursor preserves unfinished command | ❌ | Core | — | — |
| Dedup adjacent identical entries | ❌ | Core | — | — |
| Bounded configurable history size | ❌ | Core | — | — |
| Reverse search | ❌ | Important | — | — |
| Cursor movement / editing (Home/End/word/del) | ❌ | Core | — | — |
| Completion / suggestions | ❌ | Important | tui-recon §10 | No tab completion |
| Unknown-command feedback | ❌ | Core | `input.rs:437` (`_ => {}`) | Silently dropped |
| Command-specific help | ❌ | Important | — | — |
| Safe quoting/parsing | ⚠️ | Core | `input.rs:427-436` | Basic split; no quoting |
| No key collisions (search/cmd/nav) | ✅ | Core | tui-recon §1 | Overlay routing precedence works |
| Visible cursor in command line | ❌ | Core | tui-recon §10 (P2-2) | No block cursor |
| Tests for history/edit/unicode/persistence | ❌ | Core | — | — |

## 5. Feedback, logging, diagnostics

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Quiet / normal / verbose levels | ❌ | Core | quality-recon §3 (P3-3) | `log_to_file` dead code |
| Diagnostics view / discoverable log | ❌ | Core | — | — |
| High-signal notifications, dedup, no flood | ⚠️ | Core | tui-recon §6 | `yt_status`/`yt_error` transient, overwritten |
| `yt_status`/`yt_error` auto-clear | ❌ | Core | tui-recon §6 (TUI-P1-3) | Never auto-clears |
| Progress for perceptible operations | ⚠️ | Core | tui-recon §6 | Spinner exists; no local-load/buffer indicator |
| Success shown only when it matters | ⚠️ | Core | — | "connected" stays forever |
| Error with direct recovery action | ⚠️ | Core | yt-recon §8 | `yt_error` shown but no action hint |
| Secret redaction in logs | ⚠️ | Core | quality-recon §3 | No explicit logging; residual risk in `Response::Error` |
| Bounded logging / rotation | ❌ | Core | — | No logging active |
| Correlate user errors ↔ diagnostics | ❌ | Core | — | — |
| Sidecar stderr captured (not null'd) | ❌ | Important | quality-recon §3 (P3-2) | `Stdio::null()`; tracebacks vanish |

## 6. TUI polish: status, empty/loading/error, responsive, a11y

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Mode indicator (Local/YouTube/Mixed) | ✅ | Core | `player_bar.rs` transport flags | Shows MODE |
| Auth/provider status visible | ❌ | Core | yt-recon §8 | Decoupled from data; freeform string |
| Now-playing region persistent + understandable | ✅ | Core | `player_bar.rs` | Title/artist/quality |
| "Which source will this play from" answerable | ❌ | Core | tui-recon §11; prompt | No source indicator |
| "What will play next" answerable | ⚠️ | Core | tui-recon §11 | Queue view shows manual queue only, not context upcoming |
| "Is sync running" answerable | ⚠️ | Core | `yt_lists_loading` | Can hang true |
| "Why is this list empty" answerable | ❌ | Core | yt-recon §5 | Same message empty vs failed |
| Focus indication consistent | ✅ | Core | tui-recon §1 | Accent + `▶` |
| Empty states with guidance | ⚠️ | Core | tui-recon §7 | Missing "run jukebox sync" hint |
| Loading states | ⚠️ | Core | tui-recon §6 | YT loading; no local-load indicator |
| Error states with recovery | ⚠️ | Core | yt-recon §8 | Footer; no action hint |
| Responsive 80×24..160×50 + too-small msg | ✅ | Core | tui-recon §4 | 3-tier breakpoints + snapshots |
| Wide-char layout correct | ⚠️ | Core | tui-recon §5 (P3-2) | CJK=2; zero-width/combining not handled |
| Graceful truncation of long names | ✅ | Core | `theme.rs` pad_between | — |
| No-color operation | ✅ | Core | `theme.rs:6-8` | NO_COLOR collapses to Reset |
| No essential meaning by color alone | ✅ | Core | tui-recon §5 | `▶` glyph + text labels |
| No flicker / avoidable churn | ⚠️ | Core | AUDIT §3 | Full redraw every iteration |
| Mouse hitboxes | ⚠️ | Core | AUDIT §12 | Hardcoded geometry guesses (`input.rs:654-747`) |
| Snapshot/frame assertions inspected | ✅ | Core | `tests/snapshots/` | 4 sizes; need more states |

## 7. Playback: queue/context/prev-next/repeat/shuffle

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Play selected in context | ✅ | Core | `app.rs:788` | — |
| Queue precedence (context → manual) | ✅ | Core | `queue.rs:119-127` | Manual after context exhaust |
| "Play next" (insert-at-front) | ❌ | Important | playback-recon §3 | Only append |
| Prev after manual selection | ✅ | Core | `queue.rs:138-151` | History pop |
| Prev after context switch | ✅ | Core | `app.rs:823-828` | Push history on switch |
| Repeat one/all/off | ✅ | Core | `queue.rs:107` | — |
| Shuffle stability (keep current) | ✅ | Core | `queue.rs:169-177` | — |
| Reshuffle | ✅ | Core | `queue.rs:180` | — |
| Remove current/next track | ⚠️ | Core | `queue.rs:191` | Not specially handled |
| Natural track completion | ✅ | Core | `player.rs:335,149` | eof/child-exit |
| EOF + `>` same-tick double-advance | ⚠️ | Core | playback-recon §10 D5 | Race |
| Switching modes while playing | ✅ | Core | `mode.rs` | — |
| Now-playing never diverges from backend | ✅ | Core | playback-recon §5 | Converges after cold-miss |
| Transport persisted across restart | ❌ | Important | AUDIT §6 | Cursor/order/history not saved |
| Resume state | ❌ | Important | — | — |

## 8. Playback: hybrid, source-failure, process cleanup

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Hybrid local/remote matching (ISRC + fuzzy) | ✅ | Core | `match_local.rs` | Conservative thresholds |
| Source preference (local hi-res) | ✅ | Core | `app.rs:638-684` | Mixed prefers local |
| Fallback when preferred source fails | ✅ | Core | playback-recon §5 | Dead-mark + advance |
| Local file disappearance | ✅ | Core | `app.rs:582-590` | Dead |
| Remote stream failure | ✅ | Core | `app.rs:1286-1341` | pending_give_up |
| Deterministic duplicate identity | ✅ | Core | `match_local.rs` | — |
| Gapless pre-resolve (next track) | ✅ | Core | `app.rs:688` `preload_next_url` | — |
| Progressive upgrade (fast→premium) | ✅ | Core | `app.rs:1307-1333` | `load_at` resume |
| Cancellation of in-flight resolves on track change | ❌ | Important | playback-recon §6 D6 | No gen id; wasteful not incorrect |
| Process cleanup (mpv/afplay/sidecar) | ✅ | Core | playback-recon §9 | Drop kill+wait |
| Audio format restore on exit/panic/suspend | ✅ | Core | `event.rs:48,114-123` | CAPTURED OnceLock |
| `track_cache` bounded | ❌ | Important | playback-recon §11 D7 | Unbounded HashMap |
| `url_cache` bounded | ✅ | Core | `session.rs:262` | Cap 2 |

## 9. Security & robustness

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Cookie file perms 0600 | ✅ | Core | quality-recon §4 | Rust + Python sides |
| Config file/dir perms 0700 | ✅ | Core | `config.rs:74,78` | — |
| No `/tmp/.config` world-readable fallback | ❌ | Core | quality-recon §6 (P1-4) | When `dirs::config_dir()` None |
| Predictable mpv socket path | ❌ | Core | quality-recon §6 (P1-5) | `/tmp/jukebox-mpv.sock` |
| Temp cookie files cleaned | ❌ | Core | quality-recon §4 (P1-6) | `NamedTemporaryFile(delete=False)` |
| Terminal escape injection (TUI) | ✅ | Core | quality-recon §5 | ratatui sanitizes |
| Terminal escape injection (CLI) | ❌ | Core | quality-recon §5 (P1-2) | `println!` unescaped |
| Secret redaction in error strings | ⚠️ | Core | quality-recon §3 | Residual risk in `Response::Error` |
| No cookie in env on Linux (`/proc` readable) | ⚠️ | Core | quality-recon §3 | Platform concern |
| Panic audit on external data | ⚠️ | Core | quality-recon §2 | `main.rs:35,201`; `sidecar.rs:65-66` expect |
| Migration safety (additive) | ✅ | Core | quality-recon §11 | serde defaults |
| Migration (breaking) path | ❌ | Important | quality-recon §11 (P2-3) | No schema version |
| Corrupt DB recovery | ⚠️ | Core | quality-recon §11 | Falls back to defaults; file persists |
| `rm -rf "$OUT"` guard | ✅ | Core | quality-recon §6 | catalog.json check |
| Subprocess arg construction (no shell) | ✅ | Core | quality-recon §7 | All `Command::new` direct |

## 10. Performance

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| No blocking work in render/input path | ❌ | Core | playback-recon §8 B1-B4 | audio 310ms; discover 3-4s |
| Baseline measurements recorded | ⚠️ | Core | playback-recon §11 | Suspects identified, not measured |
| id→index HashMap (no per-frame O(n×m)) | ❌ | Core | playback-recon §11 D8 | `columns.rs:425`, `app.rs:384` |
| `now_playing_view` called once per frame | ❌ | Core | playback-recon §11 D9 | 4× per frame |
| `clamp_cursors` not cloning per frame | ❌ | Core | playback-recon §11 D10 | — |
| Bounded `track_cache` | ❌ | Core | playback-recon §11 D7 | Unbounded |
| Bounded sidecar channel | ⚠️ | Core | playback-recon §11 D11 | Unbounded mpsc |
| Cancellation/generation-ids for stale bg | ❌ | Core | AUDIT §11 #3 | None |
| No repeated provider requests | ⚠️ | Core | playback-recon §6 D6 | Wasteful resolves |
| No leaked child processes | ✅ | Core | playback-recon §9 | Drop guards |

## 11. Test depth & determinism

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| Unit: state/token/refresh/pagination/cache/identity/cmd/lyrics | ⚠️ | Core | quality-recon §1 | No lyrics/cmd tests (features absent) |
| Integration: restart/refresh/expiry/revocation/offline/mock | ⚠️ | Core | `e2e_yt.rs` | Fake sidecars; no real-API drift tests |
| TUI: keys/focus/overlays/empty/size/snapshots | ✅ | Core | `tests/` | Good but needs more states |
| E2E journeys with fixtures (no creds) | ✅ | Core | `e2e_yt.rs` 18 tests | — |
| `Response::from_line` malformed input tests | ❌ | Core | quality-recon §1 (P2-2) | — |
| No `set_var` race in parallel tests | ⚠️ | Core | quality-recon §1 (P2-1) | Some `JK_FAKE_MAP` set_var remain |
| `cargo fmt --check` passes | ❌ | Core | quality-recon §0 | 42 files, 332 hunks |
| `cargo clippy -D warnings` passes | ❌ | Core | quality-recon §0 | 8 errors |
| `cargo test --all-features` passes | ✅ | Core | playback-recon §0 | 161 pass |
| `bats scripts/test/*.bats` passes | ✅ | Core | quality-recon §1 | — |
| Snapshot diffs inspected | ✅ | Core | `tests/snapshots/` | Committed |

## 12. Release: CI, archive, docs

| Capability | State | Required | Evidence | Notes |
|---|---|---|---|---|
| CI test workflow | ❌ | Core | quality-recon §9 (P0-2) | Only `release.yml` |
| CI runs clippy + fmt + bats | ❌ | Core | quality-recon §9 | — |
| Release archive bundles `scripts/yt/` | ❌ | Core | quality-recon §10 (P0-1) | **YT broken for all installed users** |
| Release builds AND tests | ❌ | Core | quality-recon §9 (P0-3) | Build only |
| README keybindings accurate | ❌ | Core | quality-recon §12 (P1-1) | Almost entirely wrong |
| README YT prereqs accurate for binstall | ❌ | Core | quality-recon §12 (P3-6) | — |
| README "no cookie file written" claim | ❌ | Core | quality-recon §12 (P1-7) | Cookie file IS written |
| `jukebox config` re-runs prompt (README claim) | ❌ | Core | quality-recon §12 (P2-6) | Just prints path |
| Windows support | Out-of-scope | — | quality-recon §8 | Documented unsupported |
| binstall metadata correct | ✅ | Core | quality-recon §10 | pkg-url/bin-dir match |

---

## Priority clusters (root-cause groupings)

1. **Provider-state truthfulness cluster** (M2): false-ready status × 10 sites, auth_status lies, no expiry/refresh, launch-probe suicide, no generation ids, stale-overwrite. *One systemic fix (state machine + generation ids) closes most of §1+§2.*
2. **Missing-features cluster** (M3+M4): lyrics (0%), command history (0%). *Greenfield; no refactor risk.*
3. **Blocking-hot-path cluster** (M9): audio 310ms, discover 3-4s, CONT=YouTube 4s. *Same fire-and-forget pattern fixes B2/B3/B4; audio needs std::thread.*
4. **Release-hygiene cluster** (M10+M8): fmt/clippy/CI/archive/README. *Low risk, unblocks gates.*
5. **TUI-feedback cluster** (M5+M6): status auto-clear, source indicator, empty/loading/error states, diagnostics. *Depends on M2 state machine.*
