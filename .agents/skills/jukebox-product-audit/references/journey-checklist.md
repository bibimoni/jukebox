# Journey & Capability Checklist

## Critical acceptance journeys (Jukebox)

Verify each journey end-to-end with deterministic fixtures. Mark pass/fail with `file:line` evidence.

### Journey A — local-only first use
- Configure or discover a local library.
- Launch successfully without network access.
- Browse and search.
- Start playback.
- Use all transport controls.
- Manage queue/context.
- View metadata and lyrics or a truthful unavailable state.
- Restart and retain appropriate state.

### Journey B — YouTube first login
- Start signed out.
- Begin authentication.
- Complete login.
- Observe truthful intermediate states.
- Load every expected playlist page.
- Browse and play content.
- Restart the application.
- Confirm the session is restored without another normal login.
- Confirm playlists appear after restoration.

### Journey C — expired or revoked YouTube authorization
- Simulate expiry.
- Refresh successfully.
- Simulate invalid refresh credentials.
- Show a specific reauthentication action.
- Avoid false ready/connected status.
- Recover after signing in again.

### Journey D — offline YouTube use
- Launch with previously cached provider data while offline.
- Display cached content with a stale/offline indicator.
- Avoid blocking or repeated noisy retries.
- Recover when connectivity returns.
- Refresh without restarting.

### Journey E — hybrid playback
- Show local and remote items coherently.
- Deduplicate or distinguish equivalent recordings according to documented policy.
- Prefer the configured source.
- Fall back predictably when the preferred source fails.
- Preserve queue, history, and now-playing correctness.

### Journey F — command workflow
- Open command mode.
- Execute known and unknown commands.
- Traverse history.
- Edit a historical command without corrupting history.
- Restart and confirm persistence if enabled.
- Use help and completion.
- Exit cleanly.

### Journey G — lyrics
- Play a track with timestamped local lyrics.
- Play a track with plain lyrics.
- Play a track requiring asynchronous provider lookup.
- Change tracks during lookup.
- Confirm stale results are discarded.
- Handle no-result and provider-error cases.
- Scroll and use the view at narrow dimensions.

### Journey H — degraded terminal
- Launch at supported minimum dimensions.
- Resize repeatedly.
- Confirm focus, overlays, status, and now-playing remain understandable.
- Confirm below-minimum behavior gives a clear message instead of corrupt rendering.

## Capability matrix template

Build a matrix for Local / YouTube / Hybrid modes. For each capability, record status + `file:line` evidence.

Capabilities (evaluate at least):
1. First-run setup
2. Authentication and logout
3. Session persistence
4. Library and playlist loading
5. Search and filtering
6. Browse by artist, album, playlist, and source
7. Play, pause, resume, stop
8. Previous and next
9. Seeking
10. Volume and mute
11. Shuffle and repeat
12. Up-next queue
13. Add, remove, and reorder queue items
14. Playlist creation, editing, and deletion where supported
15. Favorites or liked tracks where supported
16. Playback history
17. Resume state
18. Lyrics
19. Track metadata and quality/source information
20. Mode switching
21. Offline behavior
22. Refresh and synchronization
23. Help and discoverability
24. Command palette
25. Configuration and diagnostics
26. Error recovery
27. Keyboard and mouse support
28. Responsive terminal behavior
29. Persistence across restart

### Classification (exactly one per capability)
- **Core and required** — without this, a normal music app is broken.
- **Important and required for this release** — needed to meet release gates.
- **Existing but defective** — implemented but wrong/buggy; cite `file:line`.
- **Intentionally out of scope with a clear rationale** — document why.
- **Dependent on an external provider limitation** — local behavior still complete.

Do not treat social feeds, recommendation algorithms, casting, or unrelated web-service features as automatically required. The objective is a complete **core listening experience**, not a clone of every commercial service.

## Per-journey audit template

```
Journey X: [name]
Mode: [Local/YouTube/Mixed] | Network: [Required/Not required] | Status: [✅ Works / ⚠️ Defective / ❌ Missing]

| Step | Expected state | Actual behavior | Defect ID | Severity |
|------|---------------|-----------------|-----------|----------|
| X1   | ...           | ...             | ...       | P?       |

Blockers: [steps that block the journey]
```

### State consistency check (per journey)
Verify these three agree — any disagreement is a **false-ready** defect:
1. **Footer status** (`yt_status`/`yt_error` in `footer.rs`)
2. **View body** (col2 status text)
3. **Now-playing** (`now_playing` in `player_bar.rs`)

### Severity
- **P0** — breaks a required journey / false success / data loss / security.
- **P1** — degrades an important journey; no clean recovery.
- **P2** — polish; confusing but recoverable.
