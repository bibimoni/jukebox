# Jukebox Revamp — Decisions

**Date:** 2026-07-12 · **Source:** Synthesized from recon reports. Each decision cites the recon evidence that drove it.

Decisions are LOCKED for this planning phase. Reopening requires new evidence.

---

## D1: Launch probe — degrade, don't suicide the session

**Decision:** Replace the blocking `library_playlists()` probe at `main.rs:142-166` with a fire-and-forget `send_refresh` after setting state `Authenticating → Synchronizing`. **Never set `yt_session = None` on a transient probe error.** Degrade to `Offline` or `AuthExpired` state; fall back the VIEW to Artists if desired, but keep the session alive. Only `:yt logout` destroys the session.

**Rationale:** The probe's stated purpose (`main.rs:140` comment) is "don't strand the user on the persisted Y view staring at empty loading." Falling back the *view* achieves that without destroying the *session*. Nuking the session forces re-login (Keychain re-prompt) on any transient blip — the #1 reported symptom.

**Alternatives considered:**
- **Retry with backoff inside the probe:** keeps blocking the launch 3-9s; worse UX.
- **Remove the probe entirely, rely on Y-view-enter refresh:** risks stranding on a persisted Y view with stale/empty data until the user navigates. Worse for the "connected but empty" perception.
- **Keep probe but only fall back the view (don't None the session):** closest to chosen, but the probe still blocks 3s at launch. Fire-and-forget is strictly better for responsiveness.

**Recon evidence:** yt-recon §3 (`main.rs:151` root cause #1), §10; playback-recon §8 B6 (3s block); AUDIT §11 #1; JOURNEYS C4/K2.

**Confidence:** HIGH.

---

## D2: Truthful provider state — explicit state machine, not freeform strings

**Decision:** Introduce a `ProviderState` enum (Unconfigured / SignedOut / Authenticating / AuthenticatedUnsynced / Synchronizing / Ready / ReadyStale / Offline / RateLimited / AuthExpired / Failed) in a new `src/yt/state.rs`. Replace the `yt_status: Option<String>` + `yt_error: Option<String>` pair with `provider_state: ProviderState` + `provider_msg: Option<String>`. The footer and Y-view render the state, not a freeform string. `auth_status` from the sidecar distinguishes `ok` (cookie present) from `valid` (cookie not expired) and reports real `premium`/`account`.

**Rationale:** The recon shows `yt_status` and `yt_error` can be set simultaneously and desync — `yt_status="connected"` persists after `yt_error` is set, and `on_tick`'s auto-respawn can't recover a `None` session (AUDIT §11 #1, yt-recon §8). A single enum with explicit transitions is provably consistent: only one state at a time, and the footer renders exactly that. This collapses "token exists," "API succeeded," "sync done," and "data ready" — which the prompt (line 445) explicitly forbids collapsing into one boolean.

**Alternatives considered:**
- **Keep two fields but add consistency invariants:** fragile; every assignment site must be audited forever; the 10 false-ready sites (yt-recon §10) prove this doesn't scale.
- **A single `connected: bool`:** explicitly forbidden by the prompt (line 445).

**Recon evidence:** yt-recon §8 (5 false-ready sites + `auth_status` lies), §10 (root cause #2), AUDIT §4 (`yt_status`/`yt_error` desync), prompt lines 429-445.

**Confidence:** HIGH.

---

## D3: Lyrics architecture — sidecar command + overlay, reusing the fire-and-forget pattern

**Decision:** Add a `get_lyrics` command to the sidecar protocol (ytmusicapi `get_lyrics` where available) plus local `.lrc`/embedded parsing in Rust. Render lyrics in a new `Overlay::Lyrics` variant (not a persistent panel). Request is fire-and-forget (`send_get_lyrics`) + `on_tick` drain — the same proven pattern as `send_resolve`/`send_refresh`. The request carries a track-id + generation tag; `on_tick` applies results only if `track_id == now_playing && gen == current_gen`. Lyrics states: Loading / Available(synced) / Available(plain) / NotFound / Offline / Error.

**Rationale:** The app has no async runtime (playback-recon §7) and the fire-and-forget + `on_tick` drain pattern is already proven correct for resolve/search/refresh. A separate panel would compete for screen space at narrow terminals (tui-recon §4); an overlay is consistent with Search/Help/Command and dismissible with `Esc`. The generation tag reuses D5's design, guaranteeing stale lyrics can't overwrite a newer track (prompt line 506).

**Alternatives considered:**
- **Persistent lyrics panel (split-screen):** competes with Miller columns at 80×24; tui-recon §4 shows narrow mode already compresses to one pane. Overlay is more flexible.
- **Separate lyrics thread with channel:** adds concurrency to a single-thread app; the sidecar reader thread already provides async drainage. Unnecessary.
- **Introduce tokio/async runtime:** overkill for a poll-based TUI; playback-recon §7 confirms no async runtime exists and the existing pattern works.
- **Scraping lyrics from unaffiliated sites:** prompt line 518 forbids "questionable scraping merely to make the panel non-empty."

**Recon evidence:** AUDIT §13 Q5 (lyrics entirely missing); playback-recon §7 (one background thread, no async runtime); tui-recon §2 (overlay pattern); prompt lines 489-514; ytmusicapi `get_lyrics` noted at `docs/superpowers/specs/2026-07-08-youtube-integration-design.md:342`.

**Confidence:** HIGH (architecture); MEDIUM (ytmusicapi `get_lyrics` availability — may be region/account limited; document as external limitation if so, per prompt line 972).

---

## D4: Command history — persisted to `state.db`, bounded, dedup-adjacent

**Decision:** Store command history in `state.db` under a `'command_history'` key as a JSON array, bounded at 100 entries (configurable), with adjacent-identical dedup. In-memory `Vec<String>` for the session, persisted on clean exit (same pattern as `LayoutState`). `Up`/`Down` recall in the `Overlay::Command`, preserving the unfinished command in a separate `unsaved` field.

**Rationale:** The prompt (line 528) requires "persistent history across restart when appropriate" — command history is appropriate (it's user-authored, small, and the user expects `:` → `Up` to recall like a shell). `state.db` is the existing persistence layer (AUDIT §6) with safe additive serde defaults (quality-recon §11). Bounding at 100 prevents unbounded growth (prompt line 531). Dedup-adjacent matches shell behavior (prompt line 530). The `unsaved` field ensures recalling history doesn't destroy a half-typed command (prompt line 529).

**Alternatives considered:**
- **In-memory only (no persistence):** violates prompt line 528 ("persistent history across restart").
- **Separate history file (`~/.config/jukebox/command_history.json`):** adds a file I/O path; `state.db` already handles JSON KV and is the established location.
- **Unbounded:** prompt line 531 requires bounded; unbounded risks bloat on long sessions.

**Recon evidence:** AUDIT §13 Q6 (no history, no 'command_history' key); AUDIT §6 (`state.db` pattern, serde defaults); prompt lines 527-541.

**Confidence:** HIGH.

---

## D5: Generation/cancellation — per-category generation counter + inflight guard, not true cancellation

**Decision:** Add a `u64` generation counter per background category (Playlists / Suggestions / Tracks / Lyrics / Watch / Resolve) in `Session`. Each `send_*` increments the category's gen and records it on the `Pending` entry. `apply_pair` tags the response with the paired request's gen; `on_tick` applies a response only if `resp_gen == current_gen`. Add an inflight guard to `send_refresh` (matching `playlist_inflight`). Do NOT attempt true cancellation of in-flight sidecar requests (the sidecar is single-threaded sequential; cancelling mid-flight isn't supported by the protocol).

**Rationale:** The recon confirms no generation/cancellation id exists (AUDIT §11 #3, playback-recon §6) and stale results CAN overwrite newer state, most dangerously for `yt_lists` (yt-recon §7). True cancellation would require a sidecar protocol change (cancel + interrupt) and the single-threaded Python sidecar can't interrupt a running ytmusicapi call. A generation counter achieves the correctness property ("stale can't overwrite fresh") with minimal protocol change (additive `gen` field) and no sidecar changes beyond echoing the gen back. The inflight guard on `send_refresh` prevents stacking multiple refreshes (yt-recon §7).

**Alternatives considered:**
- **True cancellation (cancel + interrupt sidecar):** requires protocol + Python changes; ytmusicapi calls aren't interruptible; high complexity for marginal benefit (wasteful resolves are harmless per playback-recon §6).
- **Global single epoch (one counter for all):** too coarse — a lyrics request shouldn't supersede a playlist refresh.
- **No change (rely on single-slot staging):** proven insufficient (yt-recon §7: stale refresh regresses `yt_lists`).

**Recon evidence:** AUDIT §11 #3; yt-recon §7 (`send_refresh` no guard, stale overwrite); playback-recon §6 (no cancellation, wasteful not incorrect); AUDIT §12 (`session.rs` `Pending` enum).

**Confidence:** HIGH.

---

## D6: app.rs god object — split into a directory, incrementally after behavior slices

**Decision:** Split `src/tui/app.rs` (1819 lines / 83KB) into a directory `src/tui/app/` with `mod.rs` (struct + `new` + re-exports), `state.rs` (browse/cursors), `playback.rs` (play/next/prev/on_track_ended), `yt.rs` (YT lifecycle/refresh/auth/discover), and `tick.rs` (`on_tick`). Do this AFTER the behavior slices (1-9) stabilize, so the split moves already-refactored code rather than mid-flight logic. Keep the public `App` type in `mod.rs` so callers (`main.rs`, `event.rs`, `input.rs`) are unaffected.

**Rationale:** The prompt (line 412) says "Do not perform a rewrite merely because a cleaner architecture is imaginable. Preserve working components unless evidence shows they prevent correctness, testability, responsiveness, or coherent UX." The recon provides that evidence: AUDIT §12 calls app.rs a "god object" holding all state + all update logic + source resolution + YT lifecycle; the module doc (`app.rs:1-9`) falsely claims "pure update methods, no I/O" while calling `player.load()`, `audio::set_output_format()`, `session.home_suggestions()` (blocking). This blocks testability (hard to unit-test `on_tick` in isolation) and coherent UX (state scattered across 31 fields). However, doing the split FIRST would cause merge conflicts with every behavior slice. Doing it LAST (after behavior) means each move is mechanical and verified by the full test suite.

**Alternatives considered:**
- **Don't split; leave the god object:** violates the maintainability rubric (5pts) and the prompt's evidence-based rewrite bar is met.
- **Big-bang split first:** high merge-conflict risk with all 13 behavior slices; each touches app.rs.
- **Split into separate top-level modules (not a directory):** breaks the `App` ownership model; the fields must stay together.

**Recon evidence:** AUDIT §12 (`app.rs` 1819 lines, 31 fields, false module doc); AUDIT §4 (state distribution); playback-recon §8 (blocking calls live in app.rs methods).

**Confidence:** MEDIUM (the split is correct; the *timing* — last — is the judgment call to minimize merge risk).

---

## D7: Blocking call removal — fire-and-forget + on_tick, plus a one-shot std::thread for audio

**Decision:** Convert the three remaining blocking sidecar roundtrips on the hot path (B2 `get_playlist` in Discover-Enter, B3 `home_suggestions` in `S`, B4 `get_watch_playlist` in CONT=YouTube auto-advance) to fire-and-forget `send_*` + `on_tick` merge — the same pattern already used for `send_refresh`/`send_resolve`/`send_search`. For B1 (audio format switch ~310ms), move `set_output_format` + `verify_format_landed` + the 60ms sleep to a `std::thread::spawn` one-shot that signals readiness via an `mpsc` channel; `on_tick` checks readiness before `load`. Do NOT introduce tokio/async runtime.

**Rationale:** playback-recon §7 confirms there is no async runtime and exactly one background thread (sidecar reader). The fire-and-forget pattern is proven correct (search, resolve, refresh all work this way). Converting B2/B3/B4 is mechanical — add a `pending_*` slot + `on_tick` merge, mirroring existing slots. The audio switch (B1) can't use the sidecar (it's CoreAudio, in-process) and can't be fire-and-forget into the sidecar; a one-shot `std::thread` is the minimal concurrency addition, bounded (single shot, joins via channel drain on next switch). Introducing tokio for a poll-based TUI would be a large dependency and architectural change for narrow benefit.

**Alternatives considered:**
- **Introduce tokio:** large dependency, architectural shift, playback-recon §7 shows the existing pattern suffices.
- **Keep audio synchronous but reduce the 60ms sleep:** playback-recon §8 notes the sleep is unconditional; reducing it helps but the 250ms `verify_format_landed` poll remains. Background thread fully removes the input block.
- **Move audio to the sidecar:** CoreAudio is in-process Rust (`audio.rs`); moving to Python adds IPC latency and a platform boundary. Wrong direction.
- **Accept the blocks:** violates prompt line 666 ("TUI must remain responsive during playback startup").

**Recon evidence:** playback-recon §8 (B1-B4 with file:line + durations), §7 (no async runtime, one thread), §6 (fire-and-forget proven); prompt lines 664-692.

**Confidence:** HIGH (B2/B3/B4 — proven pattern); MEDIUM (B1 audio thread — adds concurrency, needs careful join-on-switch to avoid races with rapid track changes; mitigate by gating `load` on the ready signal and discarding stale signals via D5 gen).

---

## D8: Skills — build the 4 repo skills (prompt lines 118-160)

**Decision:** Create `.agents/skills/jukebox-product-audit/`, `jukebox-provider-reliability/`, `jukebox-tui-visual-qa/`, `jukebox-isolated-release-judge/` with focused `SKILL.md` files and large checklists in `references/`. Do not duplicate existing superpowers skills (brainstorming, TDD, debugging, etc.).

**Rationale:** The prompt (lines 118-160) explicitly requires these four skills. They encode the domain-specific audit/reliability/visual-QA/judge workflows so future sessions and judges can apply them consistently. `jukebox-isolated-release-judge` is needed for M11's two-judge protocol (prompt lines 889-937).

**Alternatives considered:**
- **Skip skill creation, rely on ad-hoc prompts:** violates the explicit prompt requirement; judges would lack a consistent rubric.

**Recon evidence:** prompt lines 118-160, 889-937; no existing equivalent skills found in `available_skills`.

**Confidence:** HIGH (required by prompt).

---

## Summary table

| ID | Decision | Risk | Confidence | Drives slice |
|---|---|---|---|---|
| D1 | Launch probe degrades, doesn't suicide | H | HIGH | 1 |
| D2 | Explicit ProviderState state machine | H | HIGH | 1 |
| D3 | Lyrics: sidecar command + overlay, fire-and-forget | M | HIGH/MEDIUM | 5 |
| D4 | Command history: `state.db`, bounded, dedup | M | HIGH | 6 |
| D5 | Per-category generation counter + inflight guard | M | HIGH | 2 |
| D6 | Split app.rs into directory, incrementally last | M | MEDIUM | 13 |
| D7 | Fire-and-forget for sidecar; std::thread for audio | H/M | HIGH/MEDIUM | 4 |
| D8 | Build 4 repo skills | L | HIGH | 14 |
