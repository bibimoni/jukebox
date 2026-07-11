# Plain Text

> Source: https://raw.githubusercontent.com/sigma67/ytmusicapi/main/ytmusicapi/exceptions.py
> Cached: 2026-07-11T19:21:15.825Z

---

"""custom exception classes for ytmusicapi"""


class YTMusicError(Exception):
    """base error class

    shall only be raised if none of the subclasses below are fitting
    """


class YTMusicUserError(YTMusicError):
    """error caused by invalid usage of ytmusicapi"""


class YTMusicServerError(YTMusicError):
    """error caused by the YouTube Music backend"""
