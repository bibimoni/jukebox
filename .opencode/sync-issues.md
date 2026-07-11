# Sync Issues (Unresolved Only)

## SYNC-20
- Severity: HIGH
- Files: src/diagnostics.rs ↔ src/tui/view/diagnostics.rs ↔ src/tui/input.rs ↔ src/tui/app.rs ↔ src/tui/view/layout.rs
- Problem: Diagnostics overlay NOT wired — the buffer (src/diagnostics.rs) and render fn (src/tui/view/diagnostics.rs) exist, but the user has NO way to access the overlay. (1) `:diag` command has no handler in execute_command (input.rs:704-748 — falls through to "unknown command"); (2) No `Overlay::Diagnostics` variant in the Overlay enum (app.rs:107-166); (3) `diagnostics::render()` is never called from the layout/view module; (4) No `D` keybinding. AC-M5.1.2 NOT MET.
- Fix: Add `Overlay::Diagnostics { scroll: u16 }` variant to the Overlay enum; add `"diag" => { app.overlay = Some(Overlay::Diagnostics { scroll: 0 }); }` arm to execute_command; add `D` keybinding to toggle; call `view::diagnostics::render(f, area, &app.diagnostics)` from the layout when `Overlay::Diagnostics` is active; handle Esc/j/k/g/G scroll in input.rs.
- Status: pending

## SYNC-21
- Severity: MEDIUM
- Files: src/yt/sidecar.rs
- Problem: Sidecar stderr STILL uses `Stdio::null()` (line 55) — task S7.4 says "stderr to bounded log (not null)" but sidecar.rs was never modified (not in git diff --stat). AC-M5.3.1: `grep -n 'Stdio::null' src/yt/sidecar.rs` returns 1 match; stderr NOT redirected to bounded log. Errors only reach the user via the JSON protocol (ok:false → yt_error → diagnostics buffer + log_to_file), but sidecar stderr output (Python tracebacks, yt-dlp warnings) is lost.
- Fix: Redirect sidecar stderr to the jukebox cache log file (same as venv install in session.rs:132-157) instead of Stdio::null(). Use a bounded/truncated file (rotation already handled by log_to_file's 1 MiB rotation). OR: capture stderr via a reader thread + redact() + log_to_file, similar to the stdout reader pattern.
- Status: pending

## SYNC-22
- Severity: LOW
- Files: tests/feedback.rs ↔ src/tui/event.rs
- Problem: AC-M5.3.3 (bounded log rotation test) NOT MET — log rotation code exists in event.rs (1 MiB rotation L113-116) but there is NO test `log_rotation_bounded` to verify it. Also AC-M5.4.1 (user errors include "see :diag" or correlation id) NOT MET — no user-facing error string references `:diag` or includes a correlation id.
- Fix: (1) Add a `log_rotation_bounded` test to tests/feedback.rs that writes >1 MiB to a temp log via log_to_file_at, verifies rotation to `.1`, and verifies the active log is fresh. (2) Add "see :diag" hint to yt_error user-facing strings (e.g. in yt_status_text or the footer when yt_error is set).
- Status: pending

## ~~SYNC-23~~: RESOLVED 2026-07-12T03:07 — S4.4 was already implemented: set_output_format_async (audio.rs L53-60) uses std::thread::spawn; test audio_switch_does_not_block_input PASSES; previous review grep was incorrect
- ~~SYNC-24~~: RESOLVED 2026-07-12T03:07 — S4.5 was already implemented: audio_switch_handle field + on_tick is_finished() polling (app.rs L366,1379-1382); previous review grep was incorrect
