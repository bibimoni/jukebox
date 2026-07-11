# Research: ytmusicapi Lyrics + Pagination API

Date: 2026-07-12
Source: https://github.com/sigma67/ytmusicapi (browsing.py, library.py, models/lyrics.py, test_browsing.py) — VERBATIM from source @ refs/heads/main (SHA 9ba806fa)
Confidence: HIGH (official source code + tests)
Version: ytmusicapi 1.12.2.dev3+g9ba806fa5

## Context
Needed for M2.4 (playlist pagination fix) and M3 (lyrics implementation). Our sidecar `scripts/yt/yt.py` truncates playlists and doesn't implement lyrics.

## 1. get_lyrics — EXACT API (for M3)

**Source:** `ytmusicapi/mixins/browsing.py` (lines ~480-540)

### Signature
```python
def get_lyrics(self, browseId: str, timestamps: bool | None = False) -> Lyrics | TimedLyrics | None:
```

### Parameters
- `browseId` (str): Lyrics browseId obtained from `get_watch_playlist(videoId)` — the response dict has a `"lyrics"` key, a string starting with `MPLYt...`. NOT the videoId directly.
- `timestamps` (bool, default False): if True, returns timed lyrics (if available); if False, returns plain lyrics.

### Return types (from `ytmusicapi/models/lyrics.py` — VERBATIM):

```python
@dataclass
class LyricLine:
    """Represents a line of lyrics with timestamps (in milliseconds)."""
    text: str
    start_time: int    # milliseconds
    end_time: int      # milliseconds
    id: int            # metadata id

class Lyrics(TypedDict):
    lyrics: str           # plain text, \n-separated
    source: str | None   # e.g. "Source: LyricFind"
    hasTimestamps: Literal[False]

class TimedLyrics(TypedDict):
    lyrics: list[LyricLine]
    source: str | None
    hasTimestamps: Literal[True]
```

### Returns `None` if no lyrics are found.

### Plain lyrics example (timestamps=False or no timestamps available):
```python
{
    "lyrics": "Today is gonna be the day\nThat they're gonna throw it back to you\n",
    "source": "Source: LyricFind",
    "hasTimestamps": False
}
```

### Timed lyrics example (timestamps=True and timestamps available):
```python
{
    "lyrics": [
        LyricLine(text="I was a liar", start_time=9200, end_time=10630, id=1),
        LyricLine(text="I gave in to the fire", start_time=10680, end_time=12540, id=2),
    ],
    "source": "Source: LyricFind",
    "hasTimestamps": True
}
```

### Test usage pattern (from `tests/mixins/test_browsing.py` — VERBATIM):
```python
def test_get_lyrics(self, config, yt, sample_video):
    playlist = yt.get_watch_playlist(sample_video)
    # test normal lyrics
    lyrics_song = yt.get_lyrics(playlist["lyrics"])
    assert lyrics_song is not None
    assert isinstance(lyrics_song["lyrics"], str)
    assert lyrics_song["hasTimestamps"] is False

    # test lyrics with timestamps
    lyrics_song = yt.get_lyrics(playlist["lyrics"], timestamps=True)
    assert lyrics_song is not None
    assert len(lyrics_song["lyrics"]) >= 1
    assert lyrics_song["hasTimestamps"] is True

    # check the LyricLine object
    song = lyrics_song["lyrics"][0]
    assert isinstance(song, LyricLine)
    assert isinstance(song.text, str)
    assert song.start_time <= song.end_time
    assert isinstance(song.id, int)
```

### Key insight for our sidecar:
- **Two-step process:** `get_watch_playlist(videoId)` → get `["lyrics"]` browseId → `get_lyrics(browseId, timestamps=True)` → get timed lyrics.
- **Timestamps require mobile client:** internally `get_lyrics(timestamps=True)` uses `self.as_mobile()` context manager to switch the client. This is handled inside ytmusicapi — no extra work for us.
- **Timestamps are in milliseconds** (start_time, end_time), not seconds. Divide by 1000 for comparison with `player.position()` (which returns seconds).

## 2. get_watch_playlist — lyrics browseId source (for M3)

**Source:** `ytmusicapi/mixins/browsing.py` — returns a dict including a `"lyrics"` key.

```python
res = ytm.get_watch_playlist(videoId="vidZ", radio=True)
lyrics_browse_id = res["lyrics"]  # e.g. "MPLYt..."
```

Our sidecar already calls `get_watch_playlist` at `yt.py:415` but ignores the `lyrics` key — it only uses `res.get("tracks", [])`. The `lyrics` browseId is available from the same response.

## 3. get_library_playlists — pagination fix (for M2.4)

**Source:** `ytmusicapi/mixins/library.py` (lines ~17-57)

### Current (BUGGY) code in our sidecar (`yt.py:386`):
```python
ps = ytm.get_library_playlists()  # defaults to limit=25 → TRUNCATION
```

### ytmusicapi signature:
```python
def get_library_playlists(self, limit: int | None = 25) -> JsonList:
    """
    :param limit: Number of playlists to retrieve. ``None`` retrieves them all.
    """
```

### Fix:
```python
ps = ytm.get_library_playlists(limit=None)  # fetch ALL with continuations
```

ytmusicapi handles continuation paging internally when `limit=None` — it loops `get_continuations()` until exhausted. We get all playlists in one call.

### Return format (per item):
```python
{
    'playlistId': 'PLQwVIlKxHM6rz0fDJVv_0UlXGEWf-bFys',
    'title': 'Playlist title',
    'thumbnails': [...],
    'count': 5
}
```

## 4. get_playlist — track pagination fix (for M2.4)

**Source:** `ytmusicapi/mixins/playlists.py` — `get_playlist(playlistId, limit=100)`.

### Current (BUGGY) code in our sidecar (`yt.py:405`):
```python
p = ytm.get_playlist(arg.get("id", ""))  # defaults to limit=100 → TRUNCATION for >100 tracks
```

### Fix:
```python
p = ytm.get_playlist(arg.get("id", ""), limit=None)  # fetch ALL tracks with continuuations
```

Note: passing `limit=None` retrieves all tracks (ytmusicapi handles continuation paging). For very large playlists (>1000 tracks) this may be slow; consider a progress callback or a bounded limit (e.g. 500).

## 5. LRC File Format (for M3 local lyrics)

### Plain LRC format:
```
[ti:Song Title]
[ar:Artist Name]
[al:Album Name]
[00:12.50]First line of lyrics
[00:15.30]Second line of lyrics
[00:18.10]Third line of lyrics
```

### Timestamp format: `[mm:ss.xx]` (minutes:seconds.centiseconds)
- Multi-timestamp lines allowed: `[00:01.00][00:15.00]Repeated line`
- Metadata tags: `[ti:]` title, `[ar:]` artist, `[al:]` album, `[by:]` creator, `[offset:]` time offset in ms

### Enhanced LRC (word-level sync):
```
[00:12.50]First <00:12.50> line <00:12.80> of <00:13.00> lyrics
```

### Parsing approach for our sidecar/Rust:
1. Read `.lrc` file alongside the audio file (same basename, `.lrc` extension)
2. Check embedded FLAC tags: `LYRICS` (plain text), `UNSYNCEDLYRICS` (plain text), `SYNCEDLYRICS` (LRC format)
3. Parse LRC: regex `^\[(\d+):(\d+)(?:[.:](\d+))?\](.*)$` per line
4. Convert to our `LyricLine` equivalent: `{text, start_ms, end_ms}` (end_ms = next line's start_ms, or None for last line)
5. If no timestamps found → plain lyrics (unsynchronized)

## 6. Exact code changes for yt.py (M2.4 + M3)

### M2.4 — Pagination fix:
```python
# Line 386: change from
ps = ytm.get_library_playlists()
# to
ps = ytm.get_library_playlists(limit=None)

# Line 405: change from
p = ytm.get_playlist(arg.get("id", ""))
# to
p = ytm.get_playlist(arg.get("id", ""), limit=None)
```

### M3 — Lyrics handler (new command):
```python
if cmd == "get_lyrics":
    try:
        wp = ytm.get_watch_playlist(videoId=arg.get("video_id", ""), radio=False)
        browse_id = wp.get("lyrics")
        if not browse_id:
            return {"error": "no lyrics browseId for this track"}
        result = ytm.get_lyrics(browse_id, timestamps=True)
        if result is None:
            return {"lyrics": {"lines": [], "synced": False, "source": None}}
        if result["hasTimestamps"]:
            lines = [
                {"text": l.text, "start_ms": l.start_time, "end_ms": l.end_time}
                for l in result["lyrics"]
            ]
            return {"lyrics": {"lines": lines, "synced": True, "source": result.get("source")}}
        else:
            return {"lyrics": {"lines": result["lyrics"], "synced": False, "source": result.get("source")}}
    except Exception as e:
        return {"error": f"lyrics: {e}"}
```

### M2.1 — auth_status validity fix:
```python
# Line 369-371: change from
if cmd == "auth_status":
    ok = _has_auth()
    return {"auth": {"ok": ok, "premium": ok, "account": ok}}
# to (probe a real data call, not just cookie presence)
if cmd == "auth_status":
    ok = _has_auth()
    if not ok:
        return {"auth": {"ok": False, "premium": False, "account": False, "valid": False, "expired": False}}
    # Probe: try a lightweight real call to verify the cookies actually work
    try:
        ytm.get_home(limit=1)  # lightweight; fails fast on invalid/expired cookies
        return {"auth": {"ok": True, "premium": _has_premium(), "account": True, "valid": True, "expired": False}}
    except Exception as e:
        msg = str(e).lower()
        expired = "unauthorized" in msg or "401" in msg or "login" in msg
        return {"auth": {"ok": False, "premium": False, "account": False, "valid": False, "expired": expired, "reason": str(e)}}
```

## 7. Rust proto changes (for M3)

### src/yt/proto.rs — add to Request enum:
```rust
GetLyrics {
    video_id: String,
},
```

### src/yt/proto.rs — add to Response enum + parsing:
```rust
Lyrics(LyricsResponse),
```

```rust
// New payload struct:
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LyricLine {
    pub text: String,
    pub start_ms: u64,  // 0 for unsynced
    pub end_ms: u64,    // 0 if unknown
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LyricsResponse {
    pub lines: Vec<LyricLine>,  // empty if not found
    pub synced: bool,           // true if timestamps present
    pub source: Option<String>,
}
```

### Response parsing (in `from_line`):
```rust
if let Some(val) = o.get("lyrics") {
    return Ok(Response::Lyrics(serde_json::from_value(val.clone())?));
}
```

## Usage Notes
- `get_lyrics(timestamps=True)` internally switches to a mobile client (`as_mobile()` context) — this is automatic in ytmusicapi, no extra setup needed.
- Timestamps are in **milliseconds** (not seconds). Divide by 1000 for comparison with `player.position()` (seconds).
- `get_watch_playlist(radio=True)` already returns a `"lyrics"` browseId — we already call it for CONT=YouTube radio but discard the lyrics key. For lyrics lookup, call `get_watch_playlist(radio=False)` to get the per-song lyrics browseId.
- `get_library_playlists(limit=None)` may be slow for accounts with hundreds of playlists — ytmusicapi does internal continuation fetching. Consider a reasonable limit (e.g. 200) or communicating progress.
- `get_playlist(playlistId, limit=None)` may be slow for large playlists — same consideration.
- `LyricLine.from_raw(raw_lyric)` converts the raw API format: `{"lyricLine": text, "cueRange": {"startTimeMilliseconds": str, "endTimeMilliseconds": str, "metadata": {"id": str}}}` → `LyricLine(text, start_time, end_time, id)`.
