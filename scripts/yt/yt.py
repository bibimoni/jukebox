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
import time

# Process-lifetime cache for the browser cookie read. On macOS, reading the
# browser cookie store triggers a Keychain password prompt (Chromium browsers
# encrypt cookies with a Keychain key). We read ONCE and cache both the jar
# (for the ytmusicapi Cookie header) and the Netscape content string (for a
# short-lived yt-dlp cookiefile), so the single prompt happens at first use —
# not on every resolve_url (play).
_BC3_JAR = None        # http.cookiejar.CookieJar, or None if not yet/unreadable
_BC3_JAR_ERR = None    # str error if the read failed (no retry)
# Persistent cookies.txt path once written (0600), or None if not yet written.
# When JUKEBOX_YT_COOKIES_FILE is set the decrypted jar is written there ONCE
# and reused across calls; the next app launch loads it without re-reading the
# Keychain. When no persistent path is set, _browser_cookies_file returns the
# content and the caller writes a short-lived temp file (0600) per resolve_url,
# unlinking it immediately after yt-dlp reads it — no long-lived temp cookie
# file can leak on SIGKILL/crash (defense-in-depth).
_BC3_FILE = None


def _have_deps():
    try:
        import ytmusicapi  # noqa: F401
        import yt_dlp  # noqa: F401
        return True
    except ImportError:
        return False


def _cookie_pair():
    """Return (Cookie header str, Netscape cookies.txt content) from the env,
    or (None, None) when no cookies are set. The content is the raw env value
    (Netscape format); the caller writes it to a short-lived 0600 temp file
    for yt-dlp and unlinks it immediately after use, so no cookie file
    persists beyond a single resolve_url call (defense-in-depth: a
    SIGKILL/crash can't leak a long-lived temp cookie file)."""
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
    return "; ".join(parts), raw


def _browser_name():
    """The browser profile to read cookies from, or None."""
    b = os.environ.get("JUKEBOX_YT_BROWSER", "").strip().lower()
    return b or None


def _browser_cookie_jar():
    """Read the configured browser's cookie jar ONCE and cache it for the
    process lifetime. On macOS, Chrome (and other Chromium browsers) encrypt
    cookies with a key stored in the Keychain, so every read prompts for the
    user's password. Reading once (at first use) means a single password
    prompt, not one per play. Returns (cookiejar, None) or (None, errstr)."""
    global _BC3_JAR, _BC3_JAR_ERR
    if _BC3_JAR is not None or _BC3_JAR_ERR is not None:
        return _BC3_JAR, _BC3_JAR_ERR
    name = _browser_name()
    if not name:
        _BC3_JAR_ERR = "no browser"
        return None, _BC3_JAR_ERR
    try:
        import browser_cookie3 as bc3
    except ImportError:
        _BC3_JAR_ERR = "browser_cookie3 not installed"
        return None, _BC3_JAR_ERR
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
        _BC3_JAR_ERR = f"unsupported browser: {name}"
        return None, _BC3_JAR_ERR
    try:
        # Filter to youtube.com: YouTube sets SAPISID/__Secure-3PAPISID on
        # .youtube.com (as well as .google.com), so this keeps the auth cookies
        # while dropping the ~3000 irrelevant google.com cookies — a full jar
        # produces a 27KB Cookie header that ytmusicapi rejects (returns empty).
        cj = load(domain_name="youtube.com")
        _BC3_JAR = cj
        return _BC3_JAR, None
    except Exception as e:  # noqa: BLE001
        _BC3_JAR_ERR = f"browser cookies: {e}"
        sys.stderr.write(f"{_BC3_JAR_ERR}\n")
        return None, _BC3_JAR_ERR


def _browser_cookie_header():
    """Build a `Cookie:` header from the cached browser cookie jar. Returns
    None if no browser is set or reading failed. The jar is read once (see
    `_browser_cookie_jar`) so the Keychain prompt happens once, not per play."""
    cj, _err = _browser_cookie_jar()
    if not cj:
        return None
    parts = []
    for c in cj:
        d = c.domain.lower()
        if "youtube.com" in d or "google.com" in d:
            parts.append(f"{c.name}={c.value}")
    return "; ".join(parts) if parts else None


def _netscape_cookie_content(cj):
    """Build the Netscape cookies.txt content string from a CookieJar,
    filtered to youtube.com / google.com domains. Shared by the persistent
    path (written once to JUKEBOX_YT_COOKIES_FILE) and the per-call temp file
    (written + unlinked per resolve_url)."""
    lines = ["# Netscape HTTP Cookie File\n"]
    for c in cj:
        d = (c.domain or "").lower()
        if "youtube.com" not in d and "google.com" not in d:
            continue
        flag = "TRUE" if (c.domain or "").startswith(".") else "FALSE"
        secure = "TRUE" if c.secure else "FALSE"
        expires = str(int(c.expires)) if c.expires else "0"
        lines.append("\t".join([
            c.domain or "",
            flag,
            c.path or "/",
            secure,
            expires,
            c.name or "",
            c.value or "",
        ]) + "\n")
    return "".join(lines)


def _write_cookie_temp(content):
    """Write `content` to a fresh 0600 temp file and return its path. The
    caller MUST unlink it (via _cleanup_temp) as soon as yt-dlp has read it —
    the file exists only for the duration of a single resolve_url call so a
    SIGKILL/crash can't leak a long-lived decrypted cookie file to /tmp
    (defense-in-depth; the previous atexit-only cleanup didn't fire on
    SIGKILL)."""
    import tempfile
    fd, path = tempfile.mkstemp(suffix=".txt")
    with os.fdopen(fd, "w") as f:
        f.write(content)
    os.chmod(path, 0o600)
    return path


def _browser_cookies_file():
    """Return (persistent_path, netscape_content) for the cached browser
    cookie jar. When JUKEBOX_YT_COOKIES_FILE is set, writes the jar to that
    PERSISTENT 0600 path (once; cached) and returns (path, None) — the path
    is reused across calls and loaded by the next app launch without
    re-reading the Keychain. When no persistent path is set, returns
    (None, content) — the caller writes content to a short-lived 0600 temp
    file for yt-dlp and unlinks it immediately after use, so no decrypted
    cookie file persists beyond a single resolve_url call (defense-in-depth
    against SIGKILL/crash leaving a temp file in /tmp).

    Returns (None, None) if no browser / read failed.
    """
    global _BC3_FILE
    cj, _err = _browser_cookie_jar()
    if not cj:
        return None, None
    out_path = os.environ.get("JUKEBOX_YT_COOKIES_FILE", "").strip()
    if out_path:
        # Persistent path: write ONCE (cached via _BC3_FILE), then reuse.
        if _BC3_FILE != out_path:
            import os as _os
            _os.makedirs(_os.path.dirname(out_path) or ".", exist_ok=True)
            with open(out_path, "w") as tf:
                tf.write(_netscape_cookie_content(cj))
            try:
                os.chmod(out_path, 0o600)
            except OSError:
                pass
            _BC3_FILE = out_path
        return out_path, None
    # No persistent path: return the content for a short-lived temp file.
    return None, _netscape_cookie_content(cj)


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
        ytm = ytmusicapi.YTMusic(tf.name)
        # Unlink the headers temp file immediately — ytmusicapi reads it once
        # at construction and holds the parsed headers in memory. Defense in
        # depth: no auth file (Cookie + authorization) lingers in /tmp to leak
        # on SIGKILL/crash.
        try:
            os.unlink(tf.name)
        except OSError:
            pass
        return ytm
    return ytmusicapi.YTMusic()  # guest


def _cleanup_temp(path):
    """Remove a temp file (called from a finally block after yt-dlp has
    read the cookie file). Best-effort: a missing file is not an error."""
    try:
        os.unlink(path)
    except OSError:
        pass


def _is_transient(e):
    """True if `e` looks like a transient network error worth retrying:
    a dropped TLS connection (SSL EOF), timeout, connection reset, or an
    incomplete read — not a logic error like 'format not available'."""
    msg = str(e).lower()
    return any(k in msg for k in (
        "ssl", "eof", "timeout", "timed out",
        "connection reset", "connection aborted", "connectionerror",
        "incomplete read", "incompletereading", "remotedisconnected",
        "temporarily", "10054", "10060",
    ))


def _extract_with_retry(ydl_opts, url, attempts=2):
    """Run yt-dlp extract_info, retrying transient network errors (SSL EOF,
    timeout, connection reset) with a short linear backoff. YouTube / the
    CDN will drop a TLS connection mid-handshake every so often — retrying
    the same client once usually succeeds, whereas falling through to the
    next client_set won't (a network-level error isn't client-specific).
    Reduced from 3 to 2 attempts so a DRM-protected video doesn't block
    the sidecar for 30+ seconds (each yt-dlp call takes 5-10s)."""
    opts = {**ydl_opts, "socket_timeout": 3}
    last = None
    for attempt in range(attempts):
        try:
            import yt_dlp
            with yt_dlp.YoutubeDL(opts) as ydl:
                return ydl.extract_info(url, download=False)
        except Exception as e:  # noqa: BLE001
            last = e
            if attempt < attempts - 1 and _is_transient(e):
                time.sleep(0.4 * (attempt + 1))  # 0.4s
                continue
            raise
    raise last  # unreachable — loop always returns or re-raises


def handle(cmd, arg, ytm):
    if cmd == "ping":
        return {"pong": True}
    if cmd == "auth_status":
        # Cookie presence (backwards compat). The real validity probe lives in
        # main()'s auth_status handler (which intercepts this cmd before
        # dispatching to handle()). This is kept for direct handle() callers.
        # premium/account require a data fetch to detect; presence-only is a
        # known limitation — we report False (not ok) so the Rust side never
        # acts on a false "premium" claim.
        ok = _has_auth()
        return {"auth": {
            "ok": ok, "premium": False, "account": False,
            "valid": False, "expired": False,
            "reason": "not probed (direct handle call)",
        }}
    if cmd == "search":
        res = ytm.search(arg.get("q", ""), filter="songs", limit=arg.get("limit", 25))
        return {"search": [_track(r) for r in res]}
    if cmd == "library_playlists":
        # Load the first 50 library playlists (2 pages, ~1-2s). The previous
        # limit=None made ytmusicapi loop through ALL continuation pages for
        # users with >25 playlists, which blocked the single-threaded sidecar
        # for 10-30s+ — the "stuck on syncing" bug. The first 50 cover the
        # vast majority of users; larger libraries load their first 50
        # instantly and the user can browse them immediately. On failure (both
        # the primary path and the get_library fallback raise), re-raise so
        # main() returns {"ok": false, "error": "..."} — distinguishing a
        # genuinely empty library (Ok([])) from a failed fetch (Err).
        #
        # The fallback (get_library) stays because ytmusicapi's
        # get_library_playlists can raise on an intermittent alternate browse
        # layout (singleColumnBrowseResultsRenderer) that its parser doesn't
        # expect — an account/region-dependent response. get_library uses a
        # more tolerant path.
        try:
            ps = ytm.get_library_playlists(limit=50)
        except Exception:  # noqa: BLE001
            # Fallback: get_library returns mixed sections; keep only entries
            # that look like playlists (have a playlistId). If THIS also
            # fails (or doesn't exist in the installed ytmusicapi version),
            # let the exception propagate so the caller sees a real error
            # instead of a silent empty list.
            if hasattr(ytm, "get_library"):
                lib = ytm.get_library()
            else:
                raise
            if isinstance(lib, dict):
                lib = lib.get("items", lib)
            ps = [
                it for it in (lib or [])
                if isinstance(it, dict) and it.get("playlistId")
            ]
        return {"playlists": [
            {"id": p.get("playlistId", ""), "name": p.get("title", ""), "count": p.get("playlistCount", 0)}
            for p in ps
        ]}
    if cmd == "get_playlist":
        # Load the first 100 tracks (one page, ~1-2s). The previous limit=None
        # made ytmusicapi loop through ALL continuation pages for large
        # playlists, which blocked the single-threaded sidecar for 10-30s+
        # and queued every subsequent request behind it — the "forever
        # loading" bug. The first 100 tracks cover the vast majority of
        # playlists; larger playlists load their first 100 instantly and
        # the user can browse them immediately. On failure the exception
        # propagates to main() which returns {"ok": false, ...}.
        p = ytm.get_playlist(arg.get("id", ""), limit=100)
        return {"tracks": [_track(t) for t in p.get("tracks", [])]}
    if cmd == "home_suggestions":
        # NOTE: home_suggestions is no longer sent by send_refresh (it was
        # removed because get_home() can hang in guest mode, blocking the
        # single-threaded sidecar). This handler stays for the `S` discover
        # overlay, which sends it on explicit user action. The timeout guard
        # prevents a hang: if get_home() doesn't return in 5s, we return an
        # empty list instead of blocking the sidecar forever.
        import signal as _sig
        def _timeout_handler(signum, frame):
            raise TimeoutError("get_home() timed out after 5s")
        _old_handler = _sig.signal(_sig.SIGALRM, _timeout_handler)
        _sig.alarm(5)
        try:
            out = []
            for sec in ytm.get_home():
                for it in sec.get("contents", []):
                    pid = it.get("playlistId")
                    if pid:  # skip None/empty ids — they can't be fetched
                        out.append({"id": pid, "name": it.get("title", "") or "", "count": 0})
            return {"suggestions": out}
        except TimeoutError:
            return {"suggestions": []}
        finally:
            _sig.alarm(0)
            _sig.signal(_sig.SIGALRM, _old_handler)
    if cmd == "get_watch_playlist":
        res = ytm.get_watch_playlist(videoId=arg.get("video_id", ""), radio=True)
        return {"watch_playlist": [_track(t) for t in res.get("tracks", [])]}
    if cmd == "resolve_url":
        # Two tiers, selected by the request's "quality" field:
        #   "fast" (default) → tv_embedded/mweb, ~1.3s, NO nsig solver, caps at
        #     itag 140 (AAC 129k). Used for instant starts when premium isn't
        #     ready yet.
        #   "premium" → tv/web + the deno EJS nsig solver (remote_components),
        #     ~10-15s, reaches itag 141 (AAC 256k, Premium). Used for preloading
        #     the next track ahead of time (gapless Premium) + progressive
        #     upgrade of a playing fast stream to 256k.
        # The `format` string is identical for both — the player_client gates
        # which formats YouTube OFFERS (tv_embedded caps at 140; tv/web reach
        # 141); the format string merely picks among what's offered.
        quality = (arg.get("quality") or "fast").lower()
        # Try several client sets in order. YouTube rotates which clients work
        # (the n-challenge / PO-token situation shifts), so a single set is
        # flaky — "no video available" on some videos/regions. Each is tried
        # with a permissive format that accepts ANY audio (mp4a OR opus); a
        # set that yields no audio formats falls through to the next.
        if quality == "premium":
            client_sets = [
                {"player_client": ["tv", "web"], "remote_components": ["ejs:github"]},
                {"player_client": ["tv_embedded", "mweb"], "remote_components": ["ejs:github"]},
            ]
        else:
            # Fast tier: only try ONE client set. The old code tried 3 sets
            # (tv_embedded, web_embedded, mweb) which meant a DRM-protected
            # video blocked the sidecar for 3×5=15s. One set is enough for
            # pre-resolve (cache warming); if it fails, the play path will
            # retry. This keeps home_suggestions and other quick requests
            # from waiting behind a stuck resolve.
            client_sets = [
                {"player_client": ["tv_embedded", "mweb"]},
            ]
        authed = False
        cookiefile = None
        # Short-lived temp cookie file (0600), unlinked in the finally below so
        # the file exists only during yt-dlp's read — defense-in-depth against
        # a SIGKILL/crash leaving a decrypted cookie file in /tmp (the old
        # atexit-only cleanup didn't fire on SIGKILL). None when the persistent
        # path is used (browser cookies file already written) or no cookies.
        cookie_temp = None
        if _browser_name():
            # Use the cached cookies.txt (read once from the browser profile,
            # not per-play) so the macOS Keychain prompt happens at most once
            # per sidecar lifetime, not on every resolve_url. BOTH tiers go
            # through this same cached read — never a second cookie read
            # (which would re-prompt the Keychain).
            fpath, fcontent = _browser_cookies_file()
            if fpath:
                cookiefile = fpath
                authed = True
            elif fcontent is not None:
                cookiefile = _write_cookie_temp(fcontent)
                cookie_temp = cookiefile
                authed = True
        else:
            _, cookies_content = _cookie_pair()
            if cookies_content:
                cookiefile = _write_cookie_temp(cookies_content)
                cookie_temp = cookiefile
                authed = True

        try:
            vid = arg.get("video_id", "")
            info = None
            last_err = None
            for yt_args in client_sets:
                opts = {
                    # Permissive: AAC preferred, else any audio (opus/m4a), else best.
                    # Don't restrict to acodec^=mp4a only — opus is fine for audio.
                    "format": "bestaudio[acodec^=mp4a]/bestaudio/m4a/bestaudio/best",
                    "quiet": True,
                    "noplaylist": True,
                    "extractor_args": {"youtube": yt_args},
                }
                if cookiefile:
                    opts["cookiefile"] = cookiefile
                try:
                    info = _extract_with_retry(
                        opts, f"https://www.youtube.com/watch?v={vid}"
                    )
                    # Confirm we got at least one AUDIO format; else keep trying
                    # other client sets (some return only video on a given video).
                    fmts = info.get("formats") or [info]
                    if any((f.get("acodec") or "") != "none" and f.get("vcodec") in (None, "none") for f in fmts):
                        break  # got audio
                    # info may itself be an audio-only format (single, no list).
                    if (info.get("acodec") or "") != "none" and info.get("vcodec") in (None, "none"):
                        break
                except Exception as e:  # noqa: BLE001
                    last_err = e
                    info = None
                    continue
            if info is None:
                msg = str(last_err) if last_err else "no audio formats available for this video"
                # Strip the noisy yt-dlp prefix the user can't act on.
                if "Requested format is not available" in msg:
                    msg = "YouTube returned no playable audio for this video (try another track, or it may be region/age-restricted)"
                elif last_err is not None and _is_transient(last_err):
                    # Already retried inside _extract_with_retry and still failed
                    # across every client_set — a real network block, not a :yt
                    # setup issue. Say so plainly (mirrors the init-path message).
                    msg = (
                        f"can't reach YouTube to resolve this track ({last_err}) — "
                        "likely a network block, VPN/proxy, or YouTube rate-limiting "
                        "this IP. Check your connection / VPN; retry in a moment; "
                        "this is not fixed by :yt setup."
                    )
                raise RuntimeError(msg)

            fmts = info.get("formats") or [info]
            # AUDIO-only formats only: vcodec is none/None and acodec is not none.
            audio = [f for f in fmts if (f.get("vcodec") in (None, "none")) and (f.get("acodec") or "none") != "none"]
            if not audio:
                # info itself might be the audio format (single-format result).
                audio = [info] if (info.get("vcodec") in (None, "none") and (info.get("acodec") or "none") != "none") else []
            if not audio:
                raise RuntimeError("no audio-only formats available for this video")
            # Prefer AAC (mp4a) for best mpv compatibility, then opus, then anything;
            # within a codec prefer higher abr (fall back to tbr / a heuristic).
            def _rank(f):
                codec = (f.get("acodec") or "").lower()
                codec_pref = 2 if codec.startswith("mp4a") or codec.startswith("aac") else (1 if codec.startswith("opus") else 0)
                br = f.get("abr") or f.get("tbr") or 0
                return (codec_pref, br)
            best = max(audio, key=_rank) if audio else info
            abr = int(best.get("abr") or best.get("tbr") or 0)
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
            # `premium` is quality-aware: only the premium tier can reach itag 141
            # (AAC 256k). The fast tier always reports False (it caps at 129k even
            # for a Premium account, since itag 141 isn't offered to tv_embedded).
            is_premium = (quality == "premium") and authed and abr >= 256
            return {"resolve": {
                "url": best.get("url") or info.get("url", ""),
                "expires_at": None,
                "codec": codec,
                "abr": abr,
                "sample_rate": int(best.get("asr") or 48000),
                "container": best.get("ext", "m4a"),
                "premium": is_premium,
            }}
        finally:
            # Unlink the short-lived temp cookie file as soon as yt-dlp has
            # read it (the resolve loop above is the only reader). The file
            # existed only for this call — no long-lived temp cookie file can
            # leak to /tmp on SIGKILL/crash (defense-in-depth). No-op when the
            # persistent path was used (cookie_temp is None).
            if cookie_temp is not None:
                _cleanup_temp(cookie_temp)
    if cmd == "get_lyrics":
        # Two-step ytmusicapi flow (research: ytmusicapi-research.md §1):
        #   1. get_watch_playlist(videoId, radio=False) → lyrics browseId
        #      (a string starting with "MPLYt…", NOT the videoId directly).
        #   2. get_lyrics(browseId, timestamps=True) → timed lyrics if
        #      available, else plain text. Timestamps come back in
        #      MILLISECONDS; convert to SECONDS here so the Rust side compares
        #      directly against player.position() (seconds).
        # Returns a not-found payload (empty lines, synced=False) when the
        # track has no lyrics browseId or ytmusicapi returns None — so the UI
        # shows a truthful "lyrics unavailable" state instead of an error.
        # Real errors (network, parse) propagate as {"ok": false, "error":…}.
        try:
            wp = ytm.get_watch_playlist(
                videoId=arg.get("video_id", ""), radio=False, limit=1
            )
        except Exception as e:  # noqa: BLE001
            # A network/parse failure on the browseId lookup is a real error —
            # surface it so the UI can show "lyrics error" (not a silent
            # not-found, which would hide a transient provider failure).
            raise RuntimeError(f"lyrics lookup failed: {e}")
        browse_id = wp.get("lyrics")
        if not browse_id:
            return {"lyrics": {"lines": [], "synced": False}}
        try:
            result = ytm.get_lyrics(browse_id, timestamps=True)
        except Exception as e:  # noqa: BLE001
            raise RuntimeError(f"lyrics fetch failed: {e}")
        if result is None:
            return {"lyrics": {"lines": [], "synced": False}}
        if result.get("hasTimestamps"):
            # Timed lyrics: each line has start_time/end_time in MILLISECONDS.
            lines = []
            for l in result.get("lyrics", []):
                start_ms = l.start_time if hasattr(l, "start_time") else None
                # ytmusicapi's LyricLine dataclass exposes .text/.start_time;
                # tolerate a plain dict shape too (defensive across versions).
                if start_ms is None and isinstance(l, dict):
                    start_ms = l.get("start_time") or l.get("start_ms")
                text = l.text if hasattr(l, "text") else l.get("text", "")
                time_s = (start_ms / 1000.0) if start_ms is not None else None
                lines.append({"time": time_s, "text": text})
            return {"lyrics": {"lines": lines, "synced": True}}
        # Plain text: lyrics is a single \n-separated string. Split into lines
        # so the Rust side gets one LyricLine per row (time=None → unsynced).
        raw = result.get("lyrics", "")
        if isinstance(raw, list):
            # Defensive: some versions return a list of strings even when
            # hasTimestamps is False.
            text_lines = [l if isinstance(l, str) else getattr(l, "text", "") for l in raw]
        else:
            text_lines = str(raw).split("\n")
        lines = [{"time": None, "text": t} for t in text_lines]
        return {"lyrics": {"lines": lines, "synced": False}}
    if cmd == "create_playlist":
        # Create a new YouTube playlist. ytmusicapi.create_playlist returns the
        # new playlist id. `privacy` defaults to PRIVATE (the safest option;
        # ytmusicapi's own default). `video_ids` is optional — pass a list to
        # seed the playlist at creation, or None for an empty playlist. On
        # failure the exception propagates to main() which returns
        # {"ok": false, "error": "..."}.
        title = arg.get("title", "")
        description = arg.get("description", "")
        privacy = arg.get("privacy", "PRIVATE")
        video_ids = arg.get("video_ids", None)
        playlist_id = ytm.create_playlist(
            title, description, privacy_status=privacy, video_ids=video_ids
        )
        return {"created_playlist": {"id": playlist_id, "title": title, "privacy": privacy}}
    if cmd == "add_playlist_items":
        # Add tracks to an existing playlist. `duplicates=True` makes the call
        # idempotent (ytmusicapi skips already-present items instead of erroring),
        # so retry-on-failure won't double-add. ytmusicapi returns a dict with a
        # "status" field ("STATUS_SUCCEEDED" on success). On failure the
        # exception propagates.
        playlist_id = arg.get("playlist_id", "")
        video_ids = arg.get("video_ids", [])
        duplicates = arg.get("duplicates", True)
        result = ytm.add_playlist_items(playlist_id, video_ids, duplicates=duplicates)
        return {"added_items": {"status": result.get("status", ""), "count": len(video_ids)}}
    if cmd == "get_liked_songs":
        # The user's liked-songs playlist (ytmusicapi.get_liked_songs). `limit`
        # caps the fetch (default 100) so a huge liked-songs library doesn't
        # block the single-threaded sidecar. Returns tracks via the shared
        # `_track` mapper.
        limit = arg.get("limit", 100)
        result = ytm.get_liked_songs(limit)
        return {"liked_songs": [_track(t) for t in result.get("tracks", [])]}
    if cmd == "get_artist":
        # Artist info: name, channel id, shuffleId/radioId (for radio seeding),
        # subscriber counts, description, top songs, and related artists.
        # ytmusicapi.get_artist returns a nested dict; we extract the fields the
        # Rust side needs and flatten songs/related into our wire types. Wire
        # keys are snake_case (matching _track's `video_id` convention) so the
        # Rust serde structs deserialize without rename attributes.
        channel_id = arg.get("channel_id", "")
        artist = ytm.get_artist(channel_id)
        return {"artist_info": {
            "name": artist.get("name", ""),
            "channel_id": artist.get("channelId", ""),
            "shuffle_id": artist.get("shuffleId", ""),
            "radio_id": artist.get("radioId", ""),
            "subscribers": artist.get("subscribers", ""),
            "description": artist.get("description", ""),
            "songs_browse_id": artist.get("songs", {}).get("browseId", ""),
            "songs": [_track(t) for t in artist.get("songs", {}).get("results", [])],
            "related": [
                {"name": r.get("title", ""), "browse_id": r.get("browseId", "")}
                for r in artist.get("related", {}).get("results", [])
            ],
        }}
    if cmd == "get_song_related":
        # Related content for a song (ytmusicapi.get_song_related). The response
        # is a list of sections, each with "contents" — items with a "videoId"
        # are tracks, items with a "playlistId" are playlists. We flatten both
        # into separate lists so the Rust side gets a clean tracks/playlists split.
        browse_id = arg.get("browse_id", "")
        related = ytm.get_song_related(browse_id)
        tracks = []
        playlists = []
        for section in related:
            for item in section.get("contents", []):
                if "videoId" in item:
                    tracks.append(_track(item))
                elif "playlistId" in item:
                    playlists.append({
                        "id": item["playlistId"],
                        "name": item.get("title", ""),
                        "count": 0,
                    })
        return {"related_content": {"tracks": tracks, "playlists": playlists}}
    if cmd == "get_album":
        # Album info: title, artists, year, and tracks. ytmusicapi.get_album
        # returns a dict with "artists" (list of {name, id}) and "tracks" (list
        # of track dicts). We map through `_track` for consistent track shape.
        # The artist `id` field is a browse/channel id, so we name it
        # `browse_id` on the wire to match the Rust `RelatedArtist` struct.
        browse_id = arg.get("browse_id", "")
        album = ytm.get_album(browse_id)
        return {"album_info": {
            "title": album.get("title", ""),
            "artists": [
                {"name": a.get("name", ""), "browse_id": a.get("id", "")}
                for a in album.get("artists", [])
            ],
            "year": album.get("year", ""),
            "tracks": [_track(t) for t in album.get("tracks", [])],
        }}
    raise ValueError(f"unknown cmd {cmd}")


def main():
    have = _have_deps()
    ytm = None
    # Eagerly persist the browser cookies to the configured file (if any) so
    # the next app launch is prompt-free. This reads the browser jar ONCE
    # (the single Keychain prompt) and writes the decrypted cookies.txt to
    # JUKEBOX_YT_COOKIES_FILE (0600). Subsequent launches load that file
    # directly via the pasted-cookies path — no browser, no Keychain.
    if have and _browser_name() and os.environ.get("JUKEBOX_YT_COOKIES_FILE", "").strip():
        try:
            _browser_cookies_file()
        except Exception:  # noqa: BLE001
            pass
    if have:
        try:
            ytm = _yt()
        except Exception as e:  # noqa: BLE001
            # A failed init is almost always network: YouTube blocking the IP
            # (rate-limit after a burst of resolves), a VPN/proxy, or a captive
            # portal. Say so plainly so the user doesn't chase :yt setup
            # (which only installs deps — it can't fix a blocked connection).
            msg = str(e)
            if "SSL" in msg or "Timeout" in msg or "Connection" in msg or "EOF" in msg:
                msg = (
                    f"can't reach music.youtube.com ({e}) — likely a network block, "
                    "VPN/proxy, or YouTube rate-limiting this IP. Check your connection "
                    "/ VPN; this is not fixed by :yt setup."
                )
            else:
                msg = f"ytmusicapi init: {e}"
            print(json.dumps({"ok": False, "error": msg}), flush=True)
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
        _t0 = time.time()
        print(f"[sidecar] {cmd} start at {_t0:.3f}", file=sys.stderr, flush=True)
        # ping + auth_status need no ytmusicapi/yt-dlp — serve them even when
        # the deps are missing, so Rust can probe liveness and auth state.
        if cmd == "ping":
            print(json.dumps({"ok": True, "data": {"pong": True}}), flush=True)
            continue
        if cmd == "auth_status":
            # Cookie presence (backwards compat) + a lightweight data probe
            # (get_home(limit=1)) to verify the credential actually works.
            # An expired/revoked cookie has ok=True (the string exists) but
            # valid=False (the probe fails with an auth error). The probe runs
            # on every auth_status call, so it must be fast — get_home(limit=1)
            # fetches a single row of suggestions.
            ok = _has_auth()
            if not ok:
                print(json.dumps({"ok": True, "data": {"auth": {
                    "ok": False, "premium": False, "account": False,
                    "valid": False, "expired": False, "reason": None,
                }}}), flush=True)
                continue
            # Cookie present — probe to verify it actually works. ytm may be
            # None when ytmusicapi isn't installed (have=False); in that case
            # we can't probe, so valid=False.
            if ytm is None:
                print(json.dumps({"ok": True, "data": {"auth": {
                    "ok": True, "premium": False, "account": False,
                    "valid": False, "expired": False,
                    "reason": "ytmusicapi not initialized",
                }}}), flush=True)
                continue
            try:
                ytm.get_home(limit=1)
                print(json.dumps({"ok": True, "data": {"auth": {
                    "ok": True, "premium": False, "account": False,
                    "valid": True, "expired": False, "reason": None,
                }}}), flush=True)
            except Exception as e:  # noqa: BLE001
                msg = str(e).lower()
                expired = any(k in msg for k in (
                    "unauthorized", "401", "login", "auth", "forbidden", "403",
                ))
                print(json.dumps({"ok": True, "data": {"auth": {
                    "ok": True, "premium": False, "account": False,
                    "valid": False, "expired": expired, "reason": str(e),
                }}}), flush=True)
            continue
        if not have:
            print(json.dumps({"ok": False, "error": "ytmusicapi/yt-dlp not installed; run :yt setup"}),
                  flush=True)
            continue
        # Only resolve_url is slow (yt-dlp retries across multiple client sets
        # for DRM-protected videos, 10-30s). All other commands (home_suggestions,
        # library_playlists, search, get_lyrics) are quick ytmusicapi calls (<2s).
        # We only wrap resolve_url in a thread with timeout; other commands run
        # directly in the main thread (ytmusicapi uses signal.alarm which only
        # works in the main thread).
        if cmd == "resolve_url":
            try:
                import threading
                result_box = [None]
                error_box = [None]
                def _run():
                    try:
                        result_box[0] = handle(cmd, req, ytm)
                    except Exception as e:  # noqa: BLE001
                        error_box[0] = e
                t = threading.Thread(target=_run, daemon=True)
                t.start()
                t.join(timeout=5)
                if t.is_alive():
                    print(json.dumps({"ok": False, "error": "request timed out (5s) — the video may be DRM-protected or YouTube is rate-limiting; try another track"}), flush=True)
                    continue
                if error_box[0] is not None:
                    raise error_box[0]
                print(json.dumps({"ok": True, "data": result_box[0]}), flush=True)
            except Exception as e:  # noqa: BLE001
                print(json.dumps({"ok": False, "error": str(e)}), flush=True)
            print(f"[sidecar] {cmd} done in {time.time()-_t0:.2f}s at {time.time():.3f}", file=sys.stderr, flush=True)
        else:
            try:
                data = handle(cmd, req, ytm)
                print(json.dumps({"ok": True, "data": data}), flush=True)
            except Exception as e:  # noqa: BLE001
                print(json.dumps({"ok": False, "error": str(e)}), flush=True)
            print(f"[sidecar] {cmd} done in {time.time()-_t0:.2f}s at {time.time():.3f}", file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
