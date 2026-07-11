# Performance Plan & Measurements — Slice 11

> **Status:** Baseline analysis complete (from `playback-recon.md` §11). Post-optimization measurements pending Slice 11 implementation (PB7/PB8/PB9 fixes). Fill in the "After" columns once `src/tui/app.rs` id_index, `track_cache` LRU, and `now_playing_view` caching land.

## Hot Path Inventory (before optimization)

Catalogued in `playback-recon.md` §11 (PB7-PB20). The three P1 performance defects this slice addresses:

| ID | Hot path | Frequency | Cost | Root cause |
|----|----------|-----------|------|------------|
| PB7 | `track_by_id` — linear scan `catalog.tracks.iter().find(|t| t.id == id)` | per visible track × per frame | O(n=1594) each | No `HashMap<id, usize>` index |
| PB8 | `clamp_cursors` → `current_context_ids` → `tracks_for_album` — full `catalog.tracks.iter().enumerate().filter()` | per frame | O(n=1594) | `albums_by_artist` has `track_indices` but `tracks_for_album` re-scans instead of using them |
| PB9 | `track_cache: HashMap<String, RemoteTrack>` + `transport.history: Vec` | unbounded growth | memory leak | No LRU cap / no history cap |

**Aggregate impact (1594-track library, 20 visible tracks, 30fps render):**
- PB7: 20 × 1594 ≈ **32k comparisons/frame** → ~960k/sec
- PB8: 1594 comparisons/frame → ~48k/sec
- PB9: grows linearly with session length (hours of browsing = thousands of cached RemoteTracks)

## Planned Optimizations (Slice 11 tasks)

### S11.1 — `id_index: HashMap<String, usize>` in `App::new`
Build a HashMap from `track.id` → catalog index at catalog load time. O(n) one-time cost, O(1) per lookup thereafter.
- **Fixes:** PB7 (track_by_id), PB8 (via track_indices reuse)
- **Before:** `self.catalog.tracks.iter().find(|t| t.id == id)` — O(n) per call
- **After:** `self.track_index.get(id).and_then(|&i| self.catalog.tracks.get(i))` — O(1) per call

### S11.2 — O(1) `track_by_id` + `track_rows` + `now_playing_view`
Route all per-frame track lookups through `track_index` instead of linear scans.
- **Fixes:** PB7 in columns.rs (track_rows) and player_bar.rs (now_playing_view)
- **Before:** 32k + 48k comparisons/sec at 30fps
- **After:** 20 + 1 HashMap lookups/frame — O(1) each

### S11.3 — `track_cache` LRU cap 256
Bound the YouTube RemoteTrack cache with a simple LRU (or size-cap with eviction).
- **Fixes:** PB9 (unbounded track_cache)
- **Before:** grows without bound (thousands of entries after long sessions)
- **After:** max 256 entries, oldest evicted

### S11.4 — `mpsc::sync_channel(64)` for sidecar reader → main thread
Bound the channel between the sidecar reader thread and the main event loop.
- **Fixes:** PB9 (unbounded channel backpressure)
- **Before:** unbounded `mpsc::channel()` — reader can flood main thread
- **After:** `sync_channel(64)` — reader blocks when buffer full (backpressure)

### S11.5 — Cache focused album `track_ids` in `layout.rs`
`clamp_cursors` currently re-scans the catalog every frame. Cache the focused album's track_ids so cursor clamping is O(1).
- **Fixes:** PB8
- **Before:** `tracks_for_album` → O(n=1594) full scan per frame
- **After:** cached `Vec<usize>` from `albums_by_artist` — O(1) lookup

## Measurement Methodology

### Manual timing (before/after)
```bash
# Build a release binary with the optimization:
cargo build --release
# Run with a 1594-track library:
./target/release/jukebox play
# Open a 50-track album, press j 20 times, measure frame time.
# Open the Y view, browse playlists, measure on_tick latency.
```

### Test assertions (tests/perf.rs)
```rust
// Assert track_by_id is O(1) — no linear scan:
// - Build a 2000-track catalog
// - Call track_by_id for the LAST track 1000 times
// - Assert total time < 1ms (HashMap lookup, not linear scan)
```

### Benchmark (optional, if criterion is added)
```
# If criterion benchmarks are added:
cargo bench -- track_by_id
```

## Before / After Measurements

| Metric | Before (baseline) | After (Slice 11) | Improvement |
|--------|-------------------|------------------|-------------|
| `track_by_id` (last track, 1594 catalog) | _TODO: measure_ | _TODO: measure_ | _TODO_ |
| `track_rows` render (20 visible tracks) | _TODO: measure_ | _TODO: measure_ | _TODO_ |
| `clamp_cursors` per frame | _TODO: measure_ | _TODO: measure_ | _TODO_ |
| `now_playing_view` per frame | _TODO: measure_ | _TODO: measure_ | _TODO_ |
| `track_cache` memory (1hr session) | unbounded | 256 entries | bounded |
| Sidecar channel depth | unbounded | 64 | bounded |

## Verification

```bash
cargo test --test perf --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Acceptance criteria (from ACCEPTANCE.md M9):
- M9.1.1: PERF.md exists with before/after timings
- M9.4.1: `track_cache` bounded (LRU 256)
- M9.4.2: `track_by_id` is O(1) via HashMap
- M9.4.3: `now_playing_view` called ≤1 time/frame
- M9.4.4: sidecar channel is `sync_channel(64)`
