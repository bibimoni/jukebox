# Jukebox Quality, Security & Release Reconnaissance Report

**Date:** 2026-07-12  
**Scope:** Read-only audit of test coverage, panic hazards, secret leakage, token storage, escape injection, path handling, subprocess safety, dependencies, CI, release packaging, migration safety, and documentation accuracy.  
**Repo:** `/Users/distiled/Dev/jukebox` @ v0.3.0

---

## Executive Summary

| Check | Status |
|-------|--------|
| `cargo fmt --check` | **FAIL** — 42 of ~42 source/test files have formatting diffs (332 hunks) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **FAIL** — 8 errors (lib + test compilation) |
| Test count | ~120 Rust tests across 22 files + 4 snapshot files + 4 bats files |
| Network-dependent tests | 0 (all YT tests use fake Python sidecars) |
| Credential-dependent tests | 0 |
| CI test workflow | **MISSING** — only `release.yml` exists; it builds but never tests |
| Release archive completeness | **MISSING `scripts/yt/`** — YT sidecar not bundled |
| README keybinding accuracy | **WRONG** — keybindings are stale/incorrect |

---

## 1. Test Coverage Map

### Rust integration tests (`tests/*.rs`)

| File | Lines | Covers | Determinism | Network/Creds | Gaps / False Confidence |
|------|-------|-------|-------------|---------------|--------------------------|
| `e2e_yt.rs` | 847 | YT integration via fake Python sidecars: search, resolve, two-tier cache, progressive upgrade, error-scoping, search-overlay concurrency, radio cursor | High — fake sidecars with canned responses, `tick_until` with sleeps | Needs `python3` (no creds, no network) | **False confidence risk:** tests prove wiring against *canned* responses, not real ytmusicapi/yt-dlp output. Real sidecar JSON shape drift is undetected. `std::env::set_var("JK_FAKE_MAP", ...)` at lines 181, 214, etc. is a **parallel-test race** (shared env var) — the comment at L4-6 claims it was fixed with per-test map files, but several tests still call `set_var("JK_FAKE_MAP")` (L181, L214, L257, L305, L565). |
| `yt_sidecar.rs` | 129 | Proto serialization/deserialization, sidecar spawn + ping, session spawn | High | Needs `python3` | No test for `Response::from_line` on malformed/truncated JSON (e.g. missing `data` key, non-UTF8, huge payloads). No test for sidecar stderr (it's null'd — can't verify error surfacing). |
| `app.rs` | 392 | App state: play_selected, dead-track skip, shuffle/repeat/volume cycling, prev-across-context, collaboration albums, cursor clamping, continue modes, mixed-mode local match, discover | High — StubPlayer, tempdir catalogs | None | No test for `play_in_context_ids` with an empty ids vec. No test for concurrent `on_tick` + key handling (single-threaded so likely fine, but untested). |
| `input.rs` | 383 | Key→action dispatch: navigation, search, overlay, command, volume, mouse | High | None | No test for `:yt auth` cookie paste overlay. No test for `:yt auth browser`. No test for `:yt setup`. No test for `:yt logout`. |
| `transport.rs` | 158 | Transport engine: shuffle (smart/random), repeat, manual queue, continue, prev/next | High | None | Good coverage of transport mechanics. |
| `tui.rs` | 168 | App construction, artist index, play-in-context, auto-advance, dead-track all-dead, cycle_shuffle integration | High | None | `EndAfterN` test player is a nice seam. |
| `columns.rs` | 171 | Column rendering: artist/album/track/playlist/queue/yt lists, filters, headers | High — snapshot tests | None | Snapshot tests cover layout but not content correctness for YT rows (needs fake session). |
| `layout.rs` | 116 | Layout snapshots: standard, wide, narrow, too-small | High — insta snapshots | None | Good. |
| `player.rs` | 100 | StubPlayer, RecordingPlayer, volume/mute push-to-player | High | None | No test for mpv IPC (would need mpv installed). No test for afplay kill/reap (would need macOS). |
| `source_match.rs` | 129 | ISRC + normalized artist+title matching for local-vs-remote | High | None | Good. |
| `player_bar.rs` | 98 | Player bar rendering: play glyph, quality label, volume bar, time, transport flags | High | None | Good. |
| `state_ext.rs` | 69 | State DB: layout save/load, playlists save/load, version compat | High — tempdir DB | None | No test for corrupt DB recovery. No test for schema migration. |
| `search.rs` | 63 | Searcher: build, open, search, fuzzy, cross-script | High | None | Good. |
| `catalog.rs` | 106 | Catalog load, quality_label, resolve_source | High | None | No test for malformed catalog.json (serde error path). |
| `context.rs` | 101 | Context building: albums_by_artist, context track_ids | High | None | Good. |
| `config.rs` | 62 | Config: default_for, validate_source_dir, save/load roundtrip, perms 0700 | High — env lock for parallel safety | None | **Good:** verifies file permissions are 0700. No test for the hand-rolled YAML parser with edge cases (quoted paths, special chars, missing fields). |
| `source_device_rate.rs` | 59 | DeviceRateState: CoreAudio re-clock cadence | High | None | macOS-specific logic tested on the dev platform. |
| `transport.rs` | 158 | (listed above) | | | |
| `theme.rs` | 28 | Theme color/gradient rendering | High | None | Minimal. |
| `translit.rs` | 33 | Kana↔romaji transliteration | High | None | Good — covers edge cases. |
| `mode.rs` | 22 | SourceMode cycling: Local→YouTube→Mixed | High | None | Good. |
| `audio_restore.rs` | 12 | CoreAudio format restore on exit | High | None | Minimal. |
| `cli.rs` | 20 | First-run prompt via stdin | High | None | Only tests `prompt_source_dir_with`, not `ensure_config` end-to-end. |

### Snapshot tests (`tests/snapshots/*.snap`)
- `layout__standard.snap`, `layout__wide.snap`, `layout__narrow.snap`, `layout__too_small.snap` — insta RON snapshots of the TUI layout at various terminal sizes. Committed to the repo. Good.

### Bash tests (`scripts/test/*.bats`)
| File | Covers |
|------|--------|
| `normalize.bats` | Artist normalization, filename sanitization, khz labels, title normalization, canonical artist sorting, tag reading |
| `integration.bats` | standardize.sh end-to-end: single-artist, collab, dedup, catalog.json, idempotency, corrupt-file skip |
| `dedup.bats` | (not read — likely dedup logic) |
| `inputs.bats` | (not read — likely input edge cases) |

**All bash tests skip if `metaflac` is unavailable** (`skip_if_no_metaflac`), so they degrade gracefully on CI without audio tools.

### False-confidence tests
1. **`e2e_yt.rs` fake sidecar tests** — Prove the Rust↔sidecar wiring works against canned JSON, but:
   - Don't test against real ytmusicapi/yt-dlp output (field presence, type drift, null handling).
   - The fake sidecar scripts are written inline per-test, so they test "Rust handles the JSON we wrote" not "Rust handles the JSON the real sidecar produces."
   - **The `Response::from_line` parser is untested for malformed/garbage input** (what if the sidecar prints a Python traceback to stdout instead of JSON?).

2. **`yt_sidecar.rs` `session_spawn_and_auth_status_no_cookies`** (L120-128) — Spawns the fake sidecar, calls `Session::spawn`, asserts `is_ok()`. The fake returns `pong` for *any* input, so `auth_status` would get a `Pong` response (not `Auth`). The test name says "auth_status" but it only proves spawn doesn't panic — **it doesn't test auth_status at all**.

3. **`state.rs` inline `tmp_db()` (L337-344)** — Leaks the TempDir (`d` is dropped at end of `tmp_db`, but the comment says "keep `d` alive by leaking it"). On some platforms the temp directory may be cleaned by the OS before the test finishes. Minor.

---

## 2. Panic / Unwrap Hazards on External Data

Every `.unwrap()` / `.expect()` on data that could originate from an external source (file I/O, JSON parse, subprocess output, provider response):

| Location | Code | External trigger | Panic? |
|----------|------|-------------------|--------|
| `src/main.rs:35` | `std::env::current_exe()?.parent().unwrap()` | If `current_exe()` returns a path with no parent (root `/`) — theoretically impossible on normal systems but possible on exotic setups | **Yes — panic on launch** |
| `src/main.rs:201` | `std::env::current_exe()?.parent().unwrap()` | Same as above, for `jukebox sync` | **Yes — panic on sync** |
| `src/yt/sidecar.rs:65` | `child.stdin.take().expect("stdin piped")` | If `Command::spawn()` succeeds but stdin pipe isn't available (fd exhaustion, ulimit) | **Yes — panic** |
| `src/yt/sidecar.rs:66` | `child.stdout.take().expect("stdout piped")` | Same — fd exhaustion | **Yes — panic** |
| `src/yt/proto.rs:39` | `serde_json::to_string(self).expect("request serializes")` | Serializing a `Request` (owned struct) — safe | No (safe by construction) |
| `src/yt/session.rs:392` | `self.url_cache.iter_mut().find(...).expect("just inserted")` | After `push_back` — safe by construction | No |
| `src/config.rs:126` | `line.split('#').next().unwrap()` | `split` always returns ≥1 element | No (safe) |
| `src/tui/app.rs:1038` | `*self.yt_session.as_mut().unwrap() = new` | Guarded by `if let Some(session) = self.yt_session.as_mut()` on L1035 | No (guarded) |
| `src/tui/app.rs:1092` | `*self.yt_session.as_mut().unwrap() = new` | Guarded by `if let Some(session) = self.yt_session.as_mut()` context | No (guarded) |
| `src/audio.rs:373,381,389,399` | `match_format(&avail, ...).unwrap()` | **TEST CODE ONLY** — in `#[cfg(test)]` | No (test-only) |
| `src/state.rs:338,349-363` | `.unwrap()` on tempdir/db ops | **TEST CODE ONLY** | No (test-only) |

### Summary of production panics on external data
- **`src/main.rs:35,201`** — `current_exe().parent().unwrap()` — panics if the binary is at the filesystem root. Low probability but unhandled.
- **`src/yt/sidecar.rs:65-66`** — `expect("stdin/stdout piped")` — panics on fd exhaustion. The `spawn()` succeeded (so the process exists), but the pipe handles weren't set. On a system hitting ulimit, this would panic instead of falling back gracefully.

---

## 3. Secret Leakage Analysis

### Cookies / auth tokens in environment
- Cookies are passed to the sidecar via `JUKEBOX_YT_COOKIES` env var (`sidecar.rs:56`). **Env vars are visible to any process via `/proc/<pid>/environ` on Linux** (macOS restricts this). On Linux, a local user could read the cookie material of the jukebox process.
- Browser name is passed via `JUKEBOX_YT_BROWSER` (`sidecar.rs:57`) — not sensitive.
- Persistent cookies file path via `JUKEBOX_YT_COOKIES_FILE` (`sidecar.rs:59`) — not sensitive (just a path).

### Can tokens appear in logs / error strings / panic messages?
- **Sidecar stderr is `Stdio::null()`** (`sidecar.rs:55`) — sidecar errors never reach the terminal. Good.
- **Sidecar errors are surfaced via `Response::Error(String)`** (`proto.rs:121`). The error string comes from the Python sidecar's `str(e)`. If `ytmusicapi` or `yt-dlp` include cookie values in exception messages (unlikely but not impossible), those would be surfaced to the user via `app.yt_error`.
- **`app.rs:978`** — `format!("auth failed: {e}")` — surfaces sidecar spawn/auth errors to the user. If the error message contains a cookie value (e.g. a malformed cookie header in a ytmusicapi exception), it would be visible in the TUI footer.
- **`app.rs:152` (main.rs)** — `format!("YouTube unreachable: {e}")` — surfaces network errors. No cookie content expected.
- **`session.rs:147`** — `format!("installed YT deps into {} (log: {})", dir.display(), log_path.display())` — no secrets.
- **`event.rs:97-105` `log_to_file`** — writes to `~/.cache/jukebox/jukebox.log`. Currently `#[allow(dead_code)]` and never called — no logging occurs. If it were called with error strings containing cookie material, the log file would persist secrets.

### Verdict
- **No explicit logging of cookies/tokens found.** Cookies are passed via env, not logged.
- **Residual risk:** sidecar error strings (`Response::Error`) are opaque to the Rust side — if a ytmusicapi/yt-dlp exception includes a cookie value, it would be shown in the TUI footer (`yt_error`). This is a low-probability but unmitigated path.
- **Linux env var visibility** is a platform-specific concern (cookies readable via `/proc`).

---

## 4. Token Storage

| Aspect | Status | Location |
|--------|--------|----------|
| Cookie file location | `<config_dir>/jukebox/yt-cookies.txt` | `session.rs:58-66` |
| Cookie file permissions | **0600** — set after write | `session.rs:452-453` (`std::fs::set_permissions(&p, Permissions::from_mode(0o600))`) |
| Python-side cookie file permissions | **0600** — `os.chmod(out_path, 0o600)` | `yt.py:177` |
| Cookie file created on browser auth | Yes, sidecar writes decrypted cookies to persistent path | `yt.py:144-180` |
| Cookie file deleted on logout | **Yes** — `std::fs::remove_file(&p)` | `app.rs:1386` |
| Session cookies cleared on logout | **Yes** — `clear_cookies` respawns guest | `session.rs:435-439` |
| Browser choice cleared on logout | **Yes** | `app.rs:1387` |
| Config file permissions | **0700** | `config.rs:78` |
| Config dir permissions | **0700** | `config.rs:74` |
| Temp cookie file (pasted, non-persistent) | Created via `tempfile.NamedTemporaryFile` — default perms (0600 on most systems) | `yt.py:50-53` |
| Temp cookie file cleanup | **Not explicitly cleaned** — `delete=False` on NamedTemporaryFile means it persists until OS temp cleanup | `yt.py:50,153` |

### Issues
- **Temp cookie files from pasted-cookies path are not cleaned up** (`yt.py:50-53`). Each `_cookie_pair()` call creates a new temp file that persists in `/tmp` until the OS cleans it. The file contains the full cookie material. On a multi-user system, `/tmp` cleanup may be delayed, leaving cookie material accessible.
- **Config dir fallback to `/tmp/.config`** (`config.rs:35`, `session.rs:62`, `state.rs:26`) — if `XDG_CONFIG_HOME` is unset AND `dirs::config_dir()` returns None (rare, but possible in headless/container environments), cookies and state are written to `/tmp/.config/jukebox/` which is world-readable/writable on multi-user systems. No permission restriction on the `/tmp/.config` fallback path.

---

## 5. Terminal Escape Injection

### How external metadata reaches the TUI

| Source | Path to screen | Sanitized? |
|--------|---------------|------------|
| Local track title/artist/album | `catalog.rs` → `columns.rs:430` `format!("{glyph} {num} {} — {album}", t.title)` → `Span::styled(line, style)` → ratatui renders | **Yes** — ratatui escapes text in `Span::styled` (no raw byte writes) |
| YouTube track title/artist | `session.rs` track_cache → `overlay.rs:151` `format!("{} — {}", rt.title, rt.artist)` → `ListItem::new(track_label(...))` → ratatui renders | **Yes** — `ListItem::new` takes a `String` and ratatui handles it safely |
| YouTube playlist name | `columns.rs:175` `format!("{g} {}", l.name)` → `Span::styled` | **Yes** |
| Search query (user input) | `overlay.rs:199` `Span::styled(input.to_string(), ...)` | **Yes** — ratatui renders |
| YT error messages | `footer.rs:24` `format!("YT: {e}")` → `Span::styled` | **Yes** |
| Filter text | `columns.rs:62` `format!("{base} (filter: {}▏)", f.text)` | **Yes** |

### `Span::raw` usage (14 occurrences)
All `Span::raw` calls use **static string literals** (separators like `"  "`, `" · "`, `"  "`), never external data. **No injection vector.**

### Direct stdout writes
- `main.rs:210,218,231` — `println!` for CLI search results and sync/index output. Track titles/artists from the catalog are printed. On a terminal that interprets escape sequences, a malicious track title containing `\x1b[2J` (clear screen) would clear the terminal. **However, this is the CLI path, not the TUI** — the TUI uses ratatui which sanitizes. The CLI `println!` path is unescaped.

### Verdict
- **TUI is safe** — all external data goes through ratatui's safe rendering.
- **CLI search output (`main.rs:231-239`) is unescaped** — `println!` with track titles could inject terminal escape sequences. Low risk (the user's own catalog), but a maliciously tagged FLAC file could trigger it.

---

## 6. Path Handling

### Config / state / cookie path resolution

| Platform | Mechanism | Fallback | Risk |
|----------|-----------|----------|------|
| macOS | `dirs::config_dir()` → `~/Library/Application Support` | `/tmp/.config` | Fallback is world-readable |
| Linux | `dirs::config_dir()` → `~/.config` | `/tmp/.config` | Fallback is world-readable |
| Windows | `dirs::config_dir()` → `%APPDATA%` | `/tmp/.config` | Fallback invalid (no `/tmp` on Windows) |

The `/tmp/.config` fallback (`config.rs:35`, `session.rs:62`, `state.rs:26`) is used when both `XDG_CONFIG_HOME` is unset and `dirs::config_dir()` returns `None`. This is a **shared, world-readable location** on multi-user systems. Cookie material and state written here would be accessible to other users.

### mpv socket path
- Default: `/tmp/jukebox-mpv.sock` (`config.rs:52`) — **predictable, world-writable location**. An attacker could pre-create a socket at this path to hijack the IPC connection, or symlink it to another file. The code does `std::fs::remove_file(socket)` before spawning mpv (`player.rs:189`), which mitigates the symlink attack but not a race condition.

### standardize.sh path handling
- `rm -rf "$OUT"` at line 59 — **dangerous** if `$OUT` is misconfigured. The guard at lines 53-56 checks for `catalog.json` or `_build.log` before wiping, which prevents wiping a random directory. Good.
- `--source` and `--out` args are quoted properly (`"$SOURCE"`, `"$OUT"`). Safe.
- `rel_target()` and `truncate_bytes()` use `python3 -` (stdin) with `sys.argv` — args are passed via argv, not shell interpolation. Safe.

### yt.py path handling
- `JUKEBOX_YT_COOKIES_FILE` path from env — `_os.makedirs(_os.dirname(out_path) or ".")` (`yt.py:149`) — if the path is empty, it writes to `.` (current dir). No path traversal risk since it's the user's own env var.

---

## 7. Shell / Subprocess Invocation

Every `Command::new` call site:

| Location | Command | Args from | Shell? | Injection risk |
|----------|---------|----------|--------|----------------|
| `main.rs:203` | `standardize.sh` | `--source <cfg.source_dir> --out <cfg.filtered_dir` | No (direct exec) | **Low** — paths from user's own config file; no shell interpolation |
| `player.rs:108` | `afplay` | `path` (the audio file) | No | **None** — arg is a file path, passed directly |
| `player.rs:198` | `mpv` | `--no-video --no-terminal --idle --gapless-audio=yes --input-ipc-server=<socket>` | No | **None** — all args are static or from config |
| `session.rs:126` | `python3` | `-m venv <dir>` | No | **None** — dir is from `venv_dir()` (config path) |
| `session.rs:137` | `<venv>/pip` | `install -q -r <requirements>` | No | **None** — requirements path from `yt_script.parent()` |
| `sidecar.rs:51` | `python` (interpreter) | `script` (yt.py path) | No | **None** — paths resolved at launch |

### Verdict
**No shell injection risk.** All subprocess invocations use `Command::new` with direct args (no `sh -c`). Cookies are passed via env var, not args. Safe.

### standardize.sh internal subprocess calls
- `metaflac`, `ffprobe`, `jq`, `yq`, `awk`, `sed`, `tr`, `grep`, `sort`, `cut`, `shasum` — all called with properly quoted arguments. The script uses `set -euo pipefail`. Filenames are passed as shell variables (quoted). Safe against injection via filenames.

---

## 8. Dependency / Feature Audit

### Dependencies (`Cargo.toml`)

| Dep | Version | Features | Risk | Notes |
|-----|---------|----------|------|-------|
| `anyhow` | 1.0 | — | Low | Standard error handling |
| `clap` | 4.6 | `derive` | Low | CLI parsing |
| `crossterm` | 0.29 | — | Low | Terminal I/O |
| `dirs` | 6.0 | — | Low | Platform path resolution |
| `ratatui` | 0.30 | — | Low | TUI framework |
| `signal-hook` | 0.3 | — | Low | Signal handling |
| `rusqlite` | 0.31 | `bundled` | Low-moderate | Bundles SQLite — avoids system dep, but adds build complexity. Good for portability. |
| `serde` | 1.0 | `derive` | Low | Serialization |
| `serde_json` | 1.0 | — | Low | JSON |
| `tantivy` | 0.25 | — | Moderate | Heavy dep, but core to the app |
| `lindera` | 4.0 | `embed-ipadic` | **Heavy** — embeds ~40MB IPADIC dictionary at build time | Documented in Cargo.toml and README |
| `lindera-tantivy` | 4.0 | `embed-ipadic` | **Heavy** — same dictionary | Required for Japanese tokenization |
| `wana_kana` | 5.0 | — | Low | Kana↔romaji |
| `unicode-normalization` | 0.1 | — | Low | Unicode NFKC |
| `coreaudio-sys` | 0.2 | macOS only | Low | Platform-correct (`cfg(target_os = "macos")`) |
| `libc` | 0.2 | unix only | Low | Platform-correct (`cfg(unix)`) |

### Dev dependencies
| `tempfile` | 3 | — | Low | Test temp dirs |
| `insta` | 1 | `ron` | Low | Snapshot testing |

### Unused deps
No obviously unused dependencies found. All are referenced in source.

### Feature flags
- `rusqlite` `bundled` — correct for avoiding system sqlite dependency.
- `lindera` / `lindera-tantivy` `embed-ipadic` — heavy but documented and necessary.
- No feature flags for optional functionality (all features are always on).

### Cross-platform cfg correctness
- `coreaudio-sys` is correctly gated behind `cfg(target_os = "macos")`.
- `libc` is correctly gated behind `cfg(unix)`.
- `player.rs` uses `#[cfg(unix)]` for `signal_child` (SIGSTOP/SIGCONT) and `#[cfg(not(unix))]` for the no-op fallback. Correct.
- `event.rs` uses `#[cfg(unix)]` for `handle_sigtstp` and `#[cfg(not(unix))]` for the no-op. Correct.
- **Windows is not supported** (no Windows release target in release.yml, `/tmp` fallback for paths, Unix-only signal handling) — but this is documented (release.yml only builds for macOS + Linux).

---

## 9. CI Gaps

### What exists
- `.github/workflows/release.yml` — the **ONLY** workflow. Triggers on `v*.*.*` tags + `workflow_dispatch`. Builds release binaries for 3 targets (aarch64-darwin, x86_64-darwin, x86_64-linux-gnu), creates a GitHub Release.

### What's missing
1. **No test workflow.** `cargo test` is never run in CI. Tests exist (~120 Rust tests + 4 bats files) but are only run manually (`cargo test` / `bats scripts/test/*.bats` per README).
2. **No clippy in CI.** `cargo clippy` is never run. Currently **8 errors** that would fail CI if a clippy gate existed.
3. **No fmt check in CI.** `cargo fmt --check` is never run. Currently **42 files** have formatting diffs.
4. **No bats tests in CI.** The bash tests are never run in CI.
5. **release.yml builds but doesn't test.** `cargo build --release --locked` (L64) compiles but doesn't run `cargo test`. A release could be published with broken tests.
6. **No matrix test job.** No `ubuntu-22.04` test job, no `macos-14` test job.
7. **Snapshots are committed** (good) but never verified in CI.

### Risk
- **Regressions can be released without detection.** The fmt and clippy failures are pre-existing — any PR would inherit them. No CI gate prevents merging broken code.
- The release workflow uses `--locked` (good for reproducibility) but doesn't verify the lock file is up to date (`cargo update --lock` check).

---

## 10. Release Packaging

### binstall metadata (`Cargo.toml:52-57`)
```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/v{ version }/jukebox-{ target }{ archive-suffix }"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```
- `pkg-url` matches `release.yml`'s artifact naming (`jukebox-<target>.tar.gz`). **Correct.**
- `bin-dir` = `{ bin }{ binary-ext }` — the binary is at the archive root. **Correct** (matches release.yml L72-73: `tar czf ... .` from `staging/` where `jukebox` is at root).
- `pkg-fmt` = `tgz` — matches `tar czf`. **Correct.**

### Archive layout (release.yml L67-73)
```
staging/
  jukebox              # binary
  scripts/
    standardize.sh
    lib/
  README.md
  LICENSE
  Cargo.toml
```

### **P0 BUG: `scripts/yt/` is NOT bundled**

The release archive stages `scripts/standardize.sh` and `scripts/lib/` (L70) but **NOT** `scripts/yt/yt.py` or `scripts/yt/requirements.txt`.

`main.rs:30-37` resolves the YT sidecar script:
```rust
let yt_script = {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/yt/yt.py");
    if p.exists() { p }
    else { std::env::current_exe()?.parent().unwrap().join("scripts/yt/yt.py") }
};
```

For a binstall user, `CARGO_MANIFEST_DIR` points to the build-time path (not available at runtime), so it falls through to `current_exe().parent()/scripts/yt/yt.py` — which **doesn't exist** in the archive. YouTube integration is **completely broken** for all installed users.

### Other packaging issues
- `Cargo.toml` is bundled in the archive but not used at runtime (it's metadata). Harmless.
- `scripts/lib/` is bundled but `scripts/yt/requirements.txt` is not — so `:yt setup` (which reads `requirements.txt`) would also fail for installed users.

---

## 11. Migration Safety

### state.db schema
- **No schema version column.** The table is `CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY, value TEXT NOT NULL)` (`state.rs:39-43`). No `version` or `schema_version` key.
- The `value` column stores JSON for `layout` and `playlists` keys. The JSON has `#[serde(default)]` on all fields (`LayoutState`, `LayoutWidths`), so adding fields is backward-compatible.
- **Corrupt DB recovery:** `Connection::open` will fail on a corrupt SQLite file. `main.rs:68` wraps `load_layout()` in `if let Ok(layout)`, so a corrupt DB falls back to defaults. **OK.**
- **Older-version compat:** If a future version adds a new key to `LayoutState`, old DBs will deserialize with defaults. If a future version removes a field, old DBs will ignore the extra JSON key. **OK for additive changes.**
- **No migration path for breaking changes.** If the schema needs to change fundamentally, there's no version check + migration logic.

### config.yml
- Has a `version: 1` field (`config.rs:15`). The parser (`serde_yaml_compat`, L118) reads `version` but **doesn't use it** — there's no `if cfg.version > 1 { migrate() }` logic.
- The parser ignores unknown keys (L138 `_ => {}`), so forward-compat for additive fields works.
- **No migration path** from v1 to v2 if the format changes.

### Verdict
- **Additive changes are safe** (serde defaults handle new fields).
- **Breaking changes have no migration path** (no version check, no migration logic).
- **Corrupt DB/config falls back to defaults** (graceful degradation).
- A corrupt `state.db` would cause `load_layout()` to return Err, which main.rs handles by falling back to defaults — but the corrupt DB file persists and would fail on every launch. No auto-recovery (delete + recreate).

---

## 12. Documentation Accuracy (README vs actual behavior)

### Keybindings — **WRONG / STALE**

| README says | Code (Help overlay `overlay.rs:267-300`) says | Verdict |
|-------------|----------------------------------------------|---------|
| `space` enqueue artist | `Space` play / pause | **WRONG** |
| `enter` enqueue result / play-now | `Enter` play selected in context | **Misleading** |
| `s`/`S` shuffle (S also jumps) | `z`/`Z` cycle shuffle / reshuffle; `s` instant random; `S` discover overlay | **WRONG** |
| `r` remove | `r` cycle repeat (off→all→one) | **WRONG** |
| `c` clear queue | `c` cycle continue (mode-dependent) | **WRONG** |
| `n`/`p` next/prev | `>` / `<` next / previous track | **WRONG** |
| `←/→` seek ±5s | `,` / `.` seek −5s / +5s | **WRONG** |
| Not mentioned | `1 2 3 4` view switching, `M` mode cycling, `f` filter, `a` add to playlist, `:yt` commands, mouse, `gg/G`, `m` mute, `+/-` volume, `?` help, `:` command | **Missing** |

The README keybindings section is **almost entirely incorrect**. The Help overlay in the code is the source of truth.

### `jukebox config` command
- README says: "`jukebox config` — show / re-run the config prompt"
- Code (`main.rs:11-18`): `Cmd::Config` calls `ensure_config()` (which only runs first-run if no config exists), then either prints "config edits are not yet supported; edit <path>" or "config: <path>". **It does NOT re-run the prompt if config already exists.** **Misleading.**

### YouTube prerequisites
- README says: "install with `pip install -r scripts/yt/requirements.txt` (or run `:yt setup` from inside the TUI)"
- For binstall users, `scripts/yt/requirements.txt` is **not in the release archive** (see §10). **The README instruction is broken for installed users.**

### YouTube integration description
- README describes `:yt auth browser chrome`, `:yt auth` (paste), `:yt logout`, `:yt setup` — all match the code. **Correct.**
- README says "no cookie file is written" for browser auth — but the code DOES write a decrypted cookies file to `cookies_file()` (`yt.py:144-180`, `session.rs:300`). The README is **misleading** — a cookie file IS written (0600, persistent), just not the raw pasted cookies. The comment in `session.rs:296-299` explains this is intentional (to avoid re-reading the Keychain), but the README doesn't mention it.

### Other README claims
- "lyrics" — not mentioned in README, not in code. **OK.**
- "command history" — not mentioned, not in code. **OK.**
- "states" — not mentioned, but state.db persists layout/playlists/focus. **OK (not claimed).**
- Search description (Tantivy + Lindera + kana↔romaji) — matches code. **Correct.**
- `jukebox sync` description — matches code. **Correct.**
- Config path resolution — matches code. **Correct.**
- `cargo binstall` — matches binstall metadata. **Correct** (but the resulting archive is incomplete — see §10).

---

## 13. Defect List

### P0 — Critical (release-breaking or security-critical)

| ID | Location | Description | Repro | Acceptance |
|----|----------|-------------|-------|------------|
| P0-1 | `release.yml:70` | **`scripts/yt/` not bundled in release archive** — YouTube integration is completely broken for all binstall/binary-download users | `cargo binstall jukebox && jukebox` → `:yt setup` → "could not find requirements.txt"; any YT command → "could not find scripts/yt/yt.py" | Release archive includes `scripts/yt/yt.py` + `scripts/yt/requirements.txt` |
| P0-2 | `.github/workflows/` | **No CI test workflow** — tests exist but are never run in CI; broken code can be released | Check `.github/workflows/` — only `release.yml` exists | A `ci.yml` workflow runs `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, and `bats scripts/test/*.bats` on PRs |
| P0-3 | `release.yml:64` | **Release builds but doesn't test** — `cargo build --release --locked` with no `cargo test` step | Read release.yml L63-64 | Add `cargo test --release` before the build step, or gate the release on a separate test job |

### P1 — High (functional bugs or significant risk)

| ID | Location | Description | Repro | Acceptance |
|----|----------|-------------|-------|------------|
| P1-1 | `README.md:142-145` | **README keybindings are almost entirely wrong** — `space` is play/pause not enqueue, `s`/`S` is random/discover not shuffle, `r` is repeat not remove, `c` is continue not clear, `n`/`p` doesn't exist, `←/→` doesn't seek | Compare README keybindings section to `overlay.rs:267-300` Help overlay | README keybindings match the Help overlay |
| P1-2 | `src/main.rs:231-239` | **CLI search output is unescaped** — `println!` with track titles from the catalog could inject terminal escape sequences (e.g. `\x1b[2J` in a malicious track title) | Tag a FLAC with `TITLE=\x1b[2J;rm -rf /;` → `jukebox sync` → `jukebox search test` | Escape control characters in CLI output, or use a safe printer |
| P1-3 | `src/yt/sidecar.rs:65-66` | **`expect("stdin/stdout piped")` on sidecar spawn** — panics on fd exhaustion instead of falling back to guest mode | Hit `ulimit -n` limit → launch jukebox with YT → panic | Return `Err` and let the caller degrade to guest mode |
| P1-4 | `src/config.rs:35`, `session.rs:62`, `state.rs:26` | **`/tmp/.config` fallback for config/cookie/state** — world-readable on multi-user systems if `dirs::config_dir()` returns None (headless/container) | Unset `XDG_CONFIG_HOME`, run in a container where `dirs::config_dir()` returns None → cookies written to `/tmp/.config/jukebox/yt-cookies.txt` (world-readable) | Refuse to start or require explicit `XDG_CONFIG_HOME` when `dirs::config_dir()` is None |
| P1-5 | `src/player.rs:52` | **Predictable mpv socket path `/tmp/jukebox-mpv.sock`** — symlink/race attack possible on multi-user systems | Attacker creates symlink `/tmp/jukebox-mpv.sock` → sensitive file → jukebox removes it (`player.rs:189`) and mpv creates a new socket | Use a per-user runtime dir (`$XDG_RUNTIME_DIR` or `dirs::runtime_dir()`) or a random suffix |
| P1-6 | `scripts/yt/yt.py:50-53,153` | **Temp cookie files not cleaned up** — `NamedTemporaryFile(delete=False)` leaks cookie material to `/tmp` on every pasted-cookie resolve | Use `:yt auth` (paste cookies) → resolve a track → check `/tmp` for leftover `.txt` files with cookie content | Clean up temp cookie files after use, or use `delete=True` with a context manager |
| P1-7 | `README.md:67-69` | **README claims "no cookie file is written" for browser auth** — but the code DOES write a decrypted cookies file (0600, persistent) to `cookies_file()` | Run `:yt auth browser chrome` → check `<config>/jukebox/yt-cookies.txt` | Update README to mention the persistent decrypted cookie cache |
| P1-8 | 42 files | **`cargo fmt --check` fails on 42/42 source+test files** — 332 diff hunks | `cargo fmt --check` | `cargo fmt --check` passes with zero diffs |
| P1-9 | 8 errors | **`cargo clippy -D warnings` fails with 8 errors** | `cargo clippy --all-targets --all-features -- -D warnings` | Clippy passes with zero warnings |

### P2 — Medium (correctness, robustness, maintainability)

| ID | Location | Description | Acceptance |
|----|----------|-------------|------------|
| P2-1 | `tests/e2e_yt.rs:181,214,257,305,565` | **`std::env::set_var("JK_FAKE_MAP", ...)` in parallel tests** — despite the per-test map file pattern, several tests still set a shared env var, which races under `cargo test -- --test-threads=N` | Remove all `set_var("JK_FAKE_MAP")` calls; the map path is already interpolated into the fake script |
| P2-2 | `src/yt/proto.rs:111-153` | **`Response::from_line` untested for malformed input** — no test for non-UTF8, truncated JSON, missing `data` key, or a Python traceback printed to stdout | Add tests for malformed/garbage sidecar lines |
| P2-3 | `src/state.rs` | **No schema versioning** — `state.db` has no version key; breaking changes have no migration path | Add a `schema_version` key and migration logic |
| P2-4 | `src/config.rs:118-142` | **Hand-rolled YAML parser** — no test for edge cases (quoted paths with special chars, multiline values, missing fields beyond the known set) | Add tests for edge cases, or switch to `serde_yaml` |
| P2-5 | `src/yt/session.rs:108` | **`log_path.parent().unwrap_or(Path::new("."))`** — if `setup_log_path()` returns a relative path, `parent()` returns `None` and the fallback writes to `.` (current dir) | Ensure the log path is absolute before computing parent |
| P2-6 | `src/main.rs:11-18` | **`jukebox config` doesn't re-run the prompt** — README says it does; code just prints the path | Either re-run the prompt (as README claims) or update README |
| P2-7 | `tests/yt_sidecar.rs:120-128` | **`session_spawn_and_auth_status_no_cookies` is a false-confidence test** — the fake returns `pong` for any input, so `auth_status` gets `Pong` not `Auth`; the test only proves spawn doesn't panic | Fix the fake to return an `auth` response, or rename the test |
| P2-8 | `src/tui/app.rs:1038,1092` | **`self.yt_session.as_mut().unwrap()`** — guarded by `if let Some(session)` but the unwrap is on a separate line, making it fragile if the guard is refactored | Use the `session` binding from the `if let Some(session)` guard instead of re-unwraping |

### P3 — Low (polish, hardening, minor inaccuracies)

| ID | Location | Description | Acceptance |
|----|----------|-------------|------------|
| P3-1 | `src/main.rs:35,201` | **`current_exe().parent().unwrap()`** — panics if the binary is at filesystem root (nearly impossible but unhandled) | Use `unwrap_or(Path::new("."))` or return an error |
| P3-2 | `src/yt/sidecar.rs:55` | **Sidecar stderr is `Stdio::null()`** — debugging sidecar failures is very difficult; errors only surface via the wire protocol | Optionally redirect stderr to the jukebox log file |
| P3-3 | `src/tui/event.rs:96` | **`log_to_file` is `#[allow(dead_code)]` and never called** — no file logging exists, making production debugging impossible | Wire `log_to_file` into error paths, or remove the dead code |
| P3-4 | `scripts/standardize.sh:59` | **`rm -rf "$OUT"`** — guarded by catalog.json/_build.log check, but still risky if `$OUT` is `/` or empty | Add an explicit guard: `[[ "$OUT" == /* && "$OUT" != "/" ]]` |
| P3-5 | `Cargo.toml:29` | **`lindera` + `lindera-tantivy` embed ~40MB IPADIC** — build is slow and RAM-heavy | Consider a feature flag for Japanese support (off by default) |
| P3-6 | `README.md:48` | **YouTube prereqs instruction broken for binstall users** — `pip install -r scripts/yt/requirements.txt` won't work (file not in archive) | Fix P0-1 first, then this is resolved |
| P3-7 | `release.yml:36-38` | **No Windows release target** — only macOS + Linux | Document Windows as unsupported, or add a Windows target |
| P3-8 | `src/yt/proto.rs:152` | **`Err(anyhow!("unrecognized sidecar response: {line}"))`** — the raw sidecar line is included in the error, which could contain cookie material if the sidecar is buggy | Truncate or sanitize the line before including in the error |
| P3-9 | `tests/state.rs:337-344` | **`tmp_db()` leaks TempDir** — comment acknowledges this; the temp dir is never cleaned up | Return the TempDir from the test function to keep it alive properly |

---

## Appendix: Clippy Errors (8)

```
error: method `from_str` can be confused for the standard trait method
  --> src/mode.rs:36:5  [should_implement_trait]

error: this function has too many arguments (9/7)
  --> src/state.rs:207:1  [too_many_arguments — save_layout_at]

error: this function has too many arguments (8/7)
  --> src/state.rs:305:1  [too_many_arguments — save_layout]

error: this function has too many arguments (9/7)
  --> src/tui/view/overlay.rs:160:1  [too_many_arguments — render_search]

error: this `if` can be collapsed into the outer `match`
  --> src/yt/session.rs:680:33  [collapsible_match]

error: this `impl` can be derived
  --> src/yt/session.rs:929:1  [derivable_impls — RadioCursor Default]

error: doc list item without indentation
  --> src/yt/sidecar.rs:43:9  [doc_lazy_continuation]

error: this `if` statement can be collapsed
  --> src/yt/sidecar.rs:77:25  [collapsible_if]
```

---

*End of report.*
