# Sync Issues (Unresolved Only)

## SYNC-20
- Severity: HIGH
- Files: src/audio.rs ↔ src/tui/app.rs ↔ tests/
- Problem: Slice 4 S4.4 + S4.5 NOT implemented (false [x] marks reverted). (1) S4.4: audio.rs has NO `std::thread::spawn` for background format switch — `set_output_format` is synchronous, blocks up to 310ms (250ms `verify_format_landed` polling at L326-335 + 60ms settle sleep at L284); called synchronously from `start_playback` (app.rs L794), `load_track` (L957), `load_remote` (L996) on the input handler path; NO test `audio_switch_does_not_block_input`; AC-M9.2.4 NOT MET. (2) S4.5: NO `audio_ready`/`AudioReady`/`audio_signal` field or gating anywhere in codebase (grep=0); `start_playback`/`load_track` call `set_output_format` synchronously with `let _ =` discarding the result; NOT implemented.
- Fix: (1) S4.4 — Add `std::thread::spawn` in audio.rs to move `set_output_format`'s blocking work (verify_format_landed + settle sleep) off the input loop; return a handle/signal the caller can await on tick. (2) S4.5 — Add an `audio_ready: Option<...>` field to App; `start_playback`/`load_track` set it instead of calling `set_output_format` synchronously; `on_tick` drains it. (3) Add test `audio_switch_does_not_block_input` to tests/nonblocking.rs (or tests/perf.rs).
- Status: pending

## RESOLVED (deleted from active list)
- ~~SYNC-3~~: RESOLVED 2026-07-12T02:38 — polling loop fix in e2e_yt.rs
- ~~SYNC-4~~: RESOLVED 2026-07-12T02:34 — cargo fmt --check now PASSES
- ~~SYNC-5~~: RESOLVED 2026-07-12T02:34 — duplicate yt_status_text removed
- ~~SYNC-6~~: RESOLVED 2026-07-12T02:50 — session.rs concurrent edit conflict resolved
- ~~SYNC-7~~: RESOLVED 2026-07-12T02:50 — session.rs Slice 2 refactor completed
- ~~SYNC-8~~: RESOLVED 2026-07-12T03:01 — Slice 10 security hardening completed (config.rs, state.rs, main.rs, tests/security.rs)
- ~~SYNC-19~~: RESOLVED 2026-07-12T03:01 — Slice 8 S8.4/S8.5/S8.6 completed (disp_width zero-width, 20 snapshots, destructive key audit)
