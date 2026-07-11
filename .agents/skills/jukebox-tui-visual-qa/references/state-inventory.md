# TUI State Inventory (Jukebox)

Every state below must be rendered at every size in the size matrix (80×24, 100×30, 120×40, 160×50, narrow 60–80, too-small) with a deterministic fixture. Mark each with the snapshot file or a recorded defect.

## Local library
- [ ] Local library populated (artists + albums + tracks columns)
- [ ] Local library empty (truthful empty state + next action)

## YouTube provider states
- [ ] YT signed out (unconfigured / logged out)
- [ ] YT authenticating (probe in flight)
- [ ] YT synchronizing (playlist fetch in flight)
- [ ] YT ready with playlists
- [ ] YT authenticated but no playlists (truthful "no playlists", not "loading…" forever)
- [ ] YT offline with cache (stale/offline indicator)
- [ ] YT provider failure (unavailable / failed with reason)
- [ ] YT auth expired (re-auth action shown, no false "connected")
- [ ] YT rate-limited (backoff indicator)

## Hybrid
- [ ] Hybrid duplicate / source-selection view (local + remote shown coherently)
- [ ] Hybrid fallback after preferred source fails

## Search
- [ ] Search empty (before query)
- [ ] Search populated (results)
- [ ] Search no results (truthful)
- [ ] Search in-flight (loading indicator)

## Queue
- [ ] Queue empty
- [ ] Queue populated (up-next)
- [ ] Queue reorder / add / remove affordances visible

## Now-playing / player bar
- [ ] Playing (progress, time, source/quality label)
- [ ] Paused
- [ ] Loading / buffering
- [ ] Error during playback

## Lyrics
- [ ] Lyrics loading (async lookup in flight)
- [ ] Lyrics available (timestamped)
- [ ] Lyrics available (plain)
- [ ] Lyrics unavailable (truthful state, not blank)
- [ ] Lyrics provider error
- [ ] Lyrics stale-result discarded on track change

## Overlays / modals
- [ ] Help overlay (full keymap, scrollable)
- [ ] Command mode + command history
- [ ] YT auth input overlay
- [ ] Confirm dialog (e.g. destructive action)
- [ ] Search overlay

## Terminal / responsive
- [ ] Too-small terminal (clear message, no corrupt rendering)
- [ ] Narrow (60–80 wide) — columns still usable
- [ ] Resized repeatedly — focus/overlays/status stay understandable
- [ ] No-color mode — meaning survives without color

## Accessibility
- [ ] Focused control is indicated in text (not color alone)
- [ ] Status messages readable as plain text (screen-reader meaning)
- [ ] Keyboard reaches every action; no mouse-only path
- [ ] Mouse hitboxes align with rendered targets

## Size matrix
Apply each state above to: 80×24, 100×30, 120×40, 160×50, narrow 60–80, too-small. Add sizes at observed breakpoints.
