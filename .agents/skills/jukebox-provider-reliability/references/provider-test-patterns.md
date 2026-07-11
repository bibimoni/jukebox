# Provider Test Patterns

## Fake sidecar pattern (from tests/e2e_yt.rs)

```python
# fake_sidecar.py — echoes canned responses per cmd
import sys, json
m = json.load(open(MAP_PATH))
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    req = json.loads(line)
    cmd = req.get("cmd")
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        # Echo a canned resolve response keyed on vid
        print(json.dumps({"ok": True, "data": {"resolve": {"url": "https://x/" + vid, ...}}}), flush=True)
    elif cmd in m:
        print(m[cmd], flush=True)
```

```rust
// Rust test: spawn fake sidecar, pump on_tick until condition
let session = Session::spawn(Path::new("python3"), &script, None).unwrap();
let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
app.on_tick(); // pump
tick_until(&mut app, 100, |a| a.now_playing.is_some());
```

## Key test scenarios

1. **Cold-miss resolve:** fake returns resolve on the next tick → `pending_play` swaps.
2. **Progressive upgrade:** fast URL playing → premium URL lands → `load_at(pos)` swaps.
3. **Search concurrency:** two searches in flight → each tagged with query → no misattribution.
4. **Stale refresh:** rapid refresh → generation id discards stale response.
5. **Probe failure:** `library_playlists` errors → session preserved (not None'd).
6. **Empty vs failed:** `Ok([])` → `AuthenticatedNotSynced`; `Err` → `ProviderError`.
7. **Pagination:** >25 playlists → continuation loop loads all.
8. **Logout:** all caches cleared (`yt_lists`, `track_cache`, `url_cache`, `pending_*`).
9. **Offline:** network drop → `ReadyStale(offline)` → cached lists shown.
10. **Expiry:** expired cookie → `AuthExpired` → re-auth action shown.

## No-credential rule

Tests must NOT use real YouTube credentials. Use fake sidecars with canned JSON. The fake sidecar scripts are written inline per-test (no shared env var — per-test map files to avoid parallel races).
