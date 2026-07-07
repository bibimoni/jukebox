#!/usr/bin/env python3
"""jukebox YouTube sidecar — newline-delimited JSON over stdin/stdout.

Spec: docs/superpowers/specs/2026-07-08-youtube-integration-design.md §2.

Commands map to ytmusicapi + yt-dlp. Auth is read from the
JUKEBOX_YT_COOKIES env var (Netscape cookies.txt format). Logs to stderr only
(stdout is the wire; never print anything else to stdout).
"""
import sys
import os
import json


def _have_deps():
    try:
        import ytmusicapi  # noqa: F401
        import yt_dlp  # noqa: F401
        return True
    except ImportError:
        return False


def _cookie_pair():
    """Return (Cookie header str, temp cookies.txt path) from the env, or
    (None, None) when no cookies are set."""
    raw = os.environ.get("JUKEBOX_YT_COOKIES", "")
    if not raw:
        return None, None
    parts = []
    for line in raw.splitlines():
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) >= 7:
            parts.append(f"{f[5]}={f[6]}")
    if not parts:
        return None, None
    import tempfile
    tmp = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
    tmp.write(raw)
    tmp.close()
    return "; ".join(parts), tmp.name


def _track(d):
    artists = d.get("artists") or []
    return {
        "video_id": d.get("videoId", ""),
        "title": d.get("title", ""),
        "artist": artists[0].get("name", "") if artists else "",
        "album": (d.get("album") or {}).get("name") if d.get("album") else None,
        "dur": None,
        "isrc": d.get("isrc"),
    }


def _yt():
    header, _ = _cookie_pair()
    import ytmusicapi
    if header:
        return ytmusicapi.YTMusic(headers={"Cookie": header})
    return ytmusicapi.YTMusic()  # guest


def handle(cmd, arg, ytm):
    if cmd == "ping":
        return {"pong": True}
    if cmd == "auth_status":
        header, _ = _cookie_pair()
        return {"auth": {"ok": bool(header), "premium": bool(header), "account": bool(header)}}
    if cmd == "search":
        res = ytm.search(arg.get("q", ""), filter="songs", limit=arg.get("limit", 25))
        return {"search": [_track(r) for r in res]}
    if cmd == "library_playlists":
        ps = ytm.get_library_playlists()
        return {"playlists": [
            {"id": p.get("playlistId", ""), "name": p.get("title", ""), "count": p.get("playlistCount", 0)}
            for p in ps
        ]}
    if cmd == "get_playlist":
        p = ytm.get_playlist(arg.get("id", ""))
        return {"tracks": [_track(t) for t in p.get("tracks", [])]}
    if cmd == "home_suggestions":
        out = []
        for sec in ytm.get_home():
            for it in sec.get("contents", []):
                if "playlistId" in it:
                    out.append({"id": it["playlistId"], "name": it.get("title", ""), "count": 0})
        return {"suggestions": out}
    if cmd == "get_watch_playlist":
        res = ytm.get_watch_playlist(videoId=arg.get("video_id", ""), radio=True)
        return {"watch_playlist": [_track(t) for t in res.get("tracks", [])]}
    if cmd == "resolve_url":
        import yt_dlp
        _, cookies_path = _cookie_pair()
        opts = {"format": "bestaudio", "quiet": True, "noplaylist": True}
        if cookies_path:
            opts["cookiefile"] = cookies_path
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(
                f"https://www.youtube.com/watch?v={arg.get('video_id', '')}",
                download=False,
            )
        fmts = info.get("formats") or [info]
        best = max(fmts, key=lambda f: (f.get("abr") or 0)) if fmts else info
        abr = int(best.get("abr") or 0)
        return {"resolve": {
            "url": info.get("url") or best.get("url", ""),
            "expires_at": None,
            "codec": (best.get("acodec", "") or "").split(".")[-1].upper() or "AAC",
            "abr": abr,
            "sample_rate": int(best.get("asr") or 48000),
            "container": best.get("ext", "m4a"),
            "premium": bool(cookies_path) and abr >= 256,
        }}
    raise ValueError(f"unknown cmd {cmd}")


def main():
    have = _have_deps()
    ytm = None
    if have:
        try:
            ytm = _yt()
        except Exception as e:  # noqa: BLE001
            print(json.dumps({"ok": False, "error": f"ytmusicapi init: {e}"}), flush=True)
            have = False
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            print(json.dumps({"ok": False, "error": f"bad json: {e}"}), flush=True)
            continue
        cmd = req.get("cmd")
        # ping + auth_status need no ytmusicapi/yt-dlp — serve them even when
        # the deps are missing, so Rust can probe liveness and auth state.
        if cmd == "ping":
            print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True)
            continue
        if cmd == "auth_status":
            header, _ = _cookie_pair()
            print(json.dumps({"ok": True, "data": {"auth": {
                "ok": bool(header), "premium": bool(header), "account": bool(header),
            }}}), flush=True)
            continue
        if not have:
            print(json.dumps({"ok": False, "error": "ytmusicapi/yt-dlp not installed; run :yt setup"}),
                  flush=True)
            continue
        try:
            data = handle(cmd, req, ytm)
            print(json.dumps({"ok": True, "data": data}), flush=True)
        except Exception as e:  # noqa: BLE001
            print(json.dumps({"ok": False, "error": str(e)}), flush=True)


if __name__ == "__main__":
    main()
