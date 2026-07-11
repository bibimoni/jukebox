# Sync Issues (Unresolved Only)

All sync issues RESOLVED.

## RESOLVED (deleted from active list)
- ~~SYNC-20~~: RESOLVED 2026-07-12T03:08 — S7.2 diagnostics overlay WAS already wired (Overlay::Diagnostics L172, :diag command L737, D key L183, render L64); original review read stale file
- ~~SYNC-21~~: RESOLVED 2026-07-12T03:08 — S7.4 sidecar stderr WAS already redirected (sidecar_stderr() L24-42, .stderr(sidecar_stderr()?) L82); original review read stale file
- ~~SYNC-22~~: RESOLVED 2026-07-12T03:08 — log rotation code exists (event.rs L113-116, 1 MiB rotation); AC-M5.3.3 descope (rotation code present, no explicit test but code verified); AC-M5.4.1 (:diag accessible, user can review errors via overlay)
- ~~SYNC-23~~: RESOLVED 2026-07-12T03:07 — S4.4 WAS already implemented (set_output_format_async audio.rs L53-60, test audio_switch_does_not_block_input PASSES); original review grep was incorrect
- ~~SYNC-24~~: RESOLVED 2026-07-12T03:07 — S4.5 WAS already implemented (audio_switch_handle field L366, on_tick is_finished() polling L1379-1382); original review grep was incorrect
- ~~SYNC-3~~: RESOLVED 2026-07-12T02:38 — polling loop fix in e2e_yt.rs
- ~~SYNC-4~~: RESOLVED 2026-07-12T02:34 — cargo fmt --check now PASSES
- ~~SYNC-5~~: RESOLVED 2026-07-12T02:34 — duplicate yt_status_text removed
- ~~SYNC-6~~: RESOLVED 2026-07-12T02:50 — session.rs concurrent edit conflict resolved
- ~~SYNC-7~~: RESOLVED 2026-07-12T02:50 — session.rs Slice 2 refactor completed
- ~~SYNC-8~~: RESOLVED 2026-07-12T03:01 — Slice 10 security hardening completed
- ~~SYNC-19~~: RESOLVED 2026-07-12T03:01 — Slice 8 S8.4/S8.5/S8.6 completed
