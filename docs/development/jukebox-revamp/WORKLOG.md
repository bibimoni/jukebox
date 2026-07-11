# Worklog — Jukebox Revamp

## Baseline (2026-07-12)
- Branch: `revamp/product-polish` @ `0b0977a` (v0.3.0)
- `cargo test --all-features`: **PASS** — 161 tests, 0 fail (13.6s)
- `cargo fmt --check`: **FAIL** — 332 hunks across 42 files
- `cargo clippy --all-targets --all-features -- -D warnings`: **FAIL** — 8 errors
- `bats scripts/test/*.bats`: pending (needs metaflac)
- Recon: 5 specialists completed → AUDIT.md, yt-recon.md, tui-recon.md, playback-recon.md, quality-recon.md

## P0 defects (release-blocking)
1. **Launch probe discards YT session on any error** (main.rs:151) → repeated logins. ROOT CAUSE.
2. **auth_status lies on cookie presence** (yt.py:329-330) → "connected but empty". ROOT CAUSE.
3. **No pagination** (yt.py:345,364) → silent 25/100 truncation.
4. **False-ready status** (main.rs:117/129, app.rs:988/1014/1093) → no verification.
5. **No generation ids** (session.rs:714-720) → stale results overwrite.
6. **Lyrics entirely unimplemented** → release gate.
7. **Command history unimplemented/unpersisted** → release gate.
8. **Queue view non-functional from UI** (enqueue/remove/clear never called).
9. **Playlist manipulation non-functional** (a = display-only picker).
10. **scripts/yt/ NOT in release archive** → YT broken for installed users.
11. **No CI test workflow** → release builds but never tests.
12. **CLI println unescaped** → terminal escape injection.
13. **fmt + clippy FAIL** → baseline gate.

## P1 defects
- CONT=YouTube auto-advance blocks UI ≤4s (app.rs:870); S/Discover block ≤3-4s (app.rs:1495,1638)
- audio.rs:264 blocks ~310ms every rate-switch
- README keybindings wrong; help advertises nonexistent mouse; now-playing can diverge (player.load errors discarded)
- Focus/selection color-only (fails NO_COLOR); no diagnostics/logging (log_to_file dead); no notification timeout/dedup; no empty states for A/P/Q
- /tmp/.config fallback world-readable; predictable mpv socket; temp cookie files leak

## Checkpoint log
| When | Change | Result |
|------|--------|--------|
| 02:06 | baseline | test PASS (161), fmt FAIL (42 files), clippy FAIL (8 err), bats PASS (30), build PASS |
| 02:11 | `cargo fmt` auto-fix | fmt --check PASS, test PASS (161) — no regressions |
| 02:14 | clippy fix (8 errors) | clippy PASS, test PASS — baseline gate clean |
| 02:15 | M1 synthesis complete | JOURNEYS.md, ACCEPTANCE.md, PLAN.md, DECISIONS.md, 4 skills created |
| 02:16 | M2 + S9 launched | M2 (YT provider state) + S9 (release/CI) in parallel |
| 02:18 | Planner research complete | ytmusicapi-research.md (lyrics+pagination API, 10KB) + ratatui-nocolor-accessibility.md (6KB) cached to .opencode/docs/ |
| 02:19 | M4 in-flight (command history) | Worker mid-flight on input.rs — 2 clippy collapsible_if errors at input.rs:327,359 need fixing: collapse nested `if` into single `if` with `&&` |

## Active Worker Guidance (from Planner)
- **M4 clippy fix (input.rs:327,359):** collapse `if !x.is_empty() { if y { ... } }` → `if !x.is_empty() && y { ... }` (clippy::collapsible_if). Two instances in the command_history Up/Down and Enter-submit handlers.
- **M2.4 pagination fix (yt.py:386,405):** change `ytm.get_library_playlists()` → `ytm.get_library_playlists(limit=None)`; `ytm.get_playlist(id)` → `ytm.get_playlist(id, limit=None)`. See .opencode/docs/ytmusicapi-research.md §3-4.
- **M2.1 auth_status fix (yt.py:369-371):** probe `ytm.get_home(limit=1)` to verify cookie validity, not just presence. See .opencode/docs/ytmusicapi-research.md §6.
- **M3 lyrics (yt.py + proto.rs):** add `get_lyrics` command using `get_watch_playlist(videoId)["lyrics"]` → `get_lyrics(browseId, timestamps=True)`. See .opencode/docs/ytmusicapi-research.md §1-2,6-7.
- **M6.1 NO_COLOR fix (columns.rs, footer.rs, player_bar.rs):** add `BorderType::Double` for focused, `Modifier::REVERSED` for selected, `!`/`✓` prefixes for footer. See .opencode/docs/ratatui-nocolor-accessibility.md.
| 02:16 | M2 + S9 launched | M2 (YT provider state) + S9 (release/CI) in parallel |
| 02:53 | Major checkpoint commit | 106 files, 15261 ins, 1010 del; 281 tests PASS; fmt+clippy clean. Slices 0-8,10,11 complete. Slices 4,7 dispatched. |
