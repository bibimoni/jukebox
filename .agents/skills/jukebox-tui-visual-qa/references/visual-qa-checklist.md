# Visual QA Checklist

## Per-state audit

For each TUI state, verify:

```
State: [name]
Dimensions: [160x50 / 80x24 / 60x20 / 40x15]
Snapshot: [exists? new? updated?]

[ ] Renders without panic
[ ] Focus column has accent border
[ ] Now-playing track has ▶ glyph
[ ] Quality label right-anchored, not clipped
[ ] No text overlap or truncation of critical info
[ ] Footer shows status/error (not stale)
[ ] Esc closes overlay (if open)
[ ] NO_COLOR: selection still visible
[ ] CJK title: alignment correct
```

## States to cover

1. Empty catalog (Artists view, no tracks)
2. Artist → Album → Track column navigation
3. Now-playing (local, 24-bit/96kHz)
4. Now-playing (YT, Opus 160k)
5. Now-playing (YT Premium, AAC 256k)
6. Paused (⏸ glyph)
7. Resolving (spinner ⠋)
8. Search overlay (Local, results)
9. Search overlay (YouTube, searching…)
10. Search overlay (no results)
11. Help overlay (scrolled to top + bottom)
12. Command overlay (typing)
13. YtAuth overlay (paste box)
14. Discover overlay (local albums)
15. Discover overlay (YT playlists)
16. Inline filter active (filter: ade▏)
17. Queue view (manual queue)
18. YT view (loading…)
19. YT view (error)
20. YT view (not configured)
21. Terminal too small
22. Narrow mode (60×20)
