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


def _browser_name():
    """The browser profile to read cookies from, or None."""
    b = os.environ.get("JUKEBOX_YT_BROWSER", "").strip().lower()
    return b or None


def _browser_cookie_header():
    """Build a `Cookie:` header from the configured browser's profile
    (for ytmusicapi). Returns None if no browser is set or reading fails."""
    name = _browser_name()
    if not name:
        return None
    try:
        import browser_cookie3 as bc3
    except ImportError:
        return None
    # browser_cookie3's load fn name per browser:
    loaders = {
        "chrome": bc3.chrome,
        "firefox": bc3.firefox,
        "safari": bc3.safari,
        "edge": bc3.edge,
        "brave": bc3.brave,
        "opera": bc3.opera,
        "chromium": bc3.chromium,
    }
    load = loaders.get(name)
    if load is None:
        return None
    try:
        cj = load(domain_name="youtube.com")
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"browser cookies: {e}\n")
        return None
    parts = []
    for c in cj:
        # only youtube.com / .google.com cookies are auth-relevant
        d = c.domain.lower()
        if "youtube.com" in d or "google.com" in d:
            parts.append(f"{c.name}={c.value}")
    return "; ".join(parts) if parts else None


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


def _auth_header():
    """Resolve the Cookie header for ytmusicapi: browser profile first, else
    the pasted cookies.txt file. None → guest."""
    if _browser_name():
        return _browser_cookie_header()
    header, _ = _cookie_pair()
    return header


def _has_auth():
    """True when we can build full ytmusicapi browser-auth headers (SAPISID
    present) — either from the browser profile or a pasted cookies.txt."""
    if _browser_name():
        return _browser_auth_headers() is not None
    header, _ = _cookie_pair()
    if not header:
        return False
    from http.cookies import SimpleCookie
    sc = SimpleCookie()
    sc.load(header.replace('"', ""))
    return any(k in sc for k in ("__Secure-3PAPISID", "SAPISID"))


def _browser_auth_headers():
    """Build the full ytmusicapi browser-auth headers dict from the configured
    browser profile: Cookie (from browser_cookie3) + Authorization (SAPISIDHASH
    built from __Secure-3PAPISID) + User-Agent + Origin. Returns None if no
    browser is set or the SAPISID cookie is missing."""
    cookie_header = _browser_cookie_header()
    if not cookie_header:
        return None
    # Extract __Secure-3PAPISID (or SAPISID) for the SAPISIDHASH.
    from http.cookies import SimpleCookie
    sc = SimpleCookie()
    sc.load(cookie_header.replace('"', ""))
    sapisid = None
    for k in ("__Secure-3PAPISID", "SAPISID"):
        if k in sc:
            sapisid = sc[k].value
            break
    if not sapisid:
        return None
    from ytmusicapi.helpers import get_authorization, USER_AGENT
    origin = "https://music.youtube.com"
    authz = get_authorization(sapisid + " " + origin)
    return {
        "Cookie": cookie_header,
        "User-Agent": USER_AGENT,
        "Origin": origin,
        "authorization": authz,
        "X-Goog-AuthUser": "0",
    }


def _yt():
    """Construct a YTMusic client. Browser profile auth (preferred — no file
    written) or pasted cookies.txt, else guest."""
    headers = None
    if _browser_name():
        headers = _browser_auth_headers()
    if headers is None:
        header, _ = _cookie_pair()
        if header:
            # Pasted cookies.txt → write a minimal headers file for ytmusicapi.
            import json as _json
            import tempfile
            from http.cookies import SimpleCookie
            from ytmusicapi.helpers import get_authorization, USER_AGENT
            sc = SimpleCookie()
            sc.load(header.replace('"', ""))
            sapisid = None
            for k in ("__Secure-3PAPISID", "SAPISID"):
                if k in sc:
                    sapisid = sc[k].value
                    break
            if sapisid:
                origin = "https://music.youtube.com"
                headers = {
                    "Cookie": header,
                    "User-Agent": USER_AGENT,
                    "Origin": origin,
                    "authorization": get_authorization(sapisid + " " + origin),
                    "X-Goog-AuthUser": "0",
                }
    import ytmusicapi
    if headers:
        import json as _json
        import tempfile
        tf = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        _json.dump(headers, tf)
        tf.close()
        return ytmusicapi.YTMusic(tf.name)
    return ytmusicapi.YTMusic()  # guest


def handle(cmd, arg, ytm):
    if cmd == "ping":
        return {"pong": True}
    if cmd == "auth_status":
        ok = _has_auth()
        return {"auth": {"ok": ok, "premium": ok, "account": ok}}
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
        # Prefer AAC 256k (Premium ad-free) → Opus 160k → best audio. The
        # `ios` client needs no PO token; with cookies it yields Premium
        # formats. `format` accepts a list so yt-dlp falls through gracefully.
        opts = {
            "format": "bestaudio[acodec^=mp4a]/bestaudio/m4a/best",
            "quiet": True,
            "noplaylist": True,
            "extractor_args": {"youtube": {"player_client": ["tv", "mweb", "web"]}},
        }
        authed = False
        if _browser_name():
            # Read cookies straight from the browser profile — no file written.
            opts["cookiesfrombrowser"] = (_browser_name(),)
            authed = True
        else:
            _, cookies_path = _cookie_pair()
            if cookies_path:
                opts["cookiefile"] = cookies_path
                authed = True
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(
                f"https://www.youtube.com/watch?v={arg.get('video_id', '')}",
                download=False,
            )
        fmts = info.get("formats") or [info]
        best = max(fmts, key=lambda f: (f.get("abr") or 0)) if fmts else info
        abr = int(best.get("abr") or 0)
        # acodec is like "mp4a.40.5" (AAC), "opus", "vorbis". Map to a friendly
        # codec name; fall back to the raw acodec.
        acodec = (best.get("acodec") or "").lower()
        if acodec.startswith("mp4a") or acodec.startswith("aac"):
            codec = "AAC"
        elif acodec.startswith("opus"):
            codec = "Opus"
        elif acodec:
            codec = acodec.split(".")[0].upper()
        else:
            codec = "AAC"
        return {"resolve": {
            "url": info.get("url") or best.get("url", ""),
            "expires_at": None,
            "codec": codec,
            "abr": abr,
            "sample_rate": int(best.get("asr") or 48000),
            "container": best.get("ext", "m4a"),
            "premium": authed and abr >= 256,
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
            ok = _has_auth()
            print(json.dumps({"ok": True, "data": {"auth": {
                "ok": ok, "premium": ok, "account": ok,
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
