#!/usr/bin/env python3
"""Standalone stdlib-only tests for scripts/yt/yt.py handle() arms.

Run with: python3 scripts/yt/test_yt.py
No test runner required (uses unittest from the stdlib).
"""
import os
import sys
import unittest

# Make yt.py importable from this directory (insert at front so our yt.py
# shadows any same-named module on the search path).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from yt import handle  # noqa: E402


_DEFAULT_HOME = [
    {
        "title": "Listen again",
        "contents": [
            {
                "title": "Mix 1",
                "subtitle": "Playlist",
                "playlistId": "PL1",
                "videoId": "v1",
                "artists": [{"name": "Artist A"}],
                "browseId": "UCM1",
            },
            {
                "title": "Song 2",
                "subtitle": "Video",
                "videoId": "v2",
                "artists": [{"name": "Artist B"}],
            },
        ],
    },
    {
        "title": "Top picks",
        "contents": [],
    },
]

# ytmusicapi.get_mood_categories() returns a dict of section -> list of
# {params, title} categories.
_DEFAULT_MOOD_CATEGORIES = {
    "For you": [
        {"params": "p1", "title": "1980s"},
        {"params": "p2", "title": "Feel Good"},
    ],
    "Genres": [
        {"params": "p3", "title": "Dance & Electronic"},
    ],
    "Moods & moments": [
        {"params": "p4", "title": "Chill"},
    ],
}

# ytmusicapi.get_mood_playlists(params) returns a list of playlists (same
# shape as get_library_playlists). One canned list per params value.
_DEFAULT_MOOD_PLAYLISTS = {
    "p1": [
        {"playlistId": "PL1", "title": "80s Pop Hits", "playlistCount": 42},
        {"playlistId": "PL2", "title": "80s Rock", "playlistCount": 30},
    ],
    "p2": [
        {"playlistId": "PL3", "title": "Feel Good Mix", "playlistCount": 25},
    ],
    "p3": [
        {"playlistId": "PL4", "title": "Dance Anthems", "playlistCount": 50},
    ],
    "p4": [
        {"playlistId": "PL5", "title": "Chill Vibes", "playlistCount": 18},
        {"playlistId": "PL6", "title": "Late Night", "playlistCount": 22},
    ],
}

# ytmusicapi.get_charts() returns a dict like:
#   {"countries": {...}, "videos": [...], "artists": [...], "genres": [...]}
# Items have different shapes per chart (artists have subscribers/rank, not
# subtitle; videos/genres have playlistId).
_DEFAULT_CHARTS = {
    "countries": {"selected": {"text": "United States"}, "options": ["ZZ"]},
    "videos": [
        {"title": "Daily Top Music Videos", "playlistId": "PLV1", "thumbnails": []},
        {"title": "Weekly Top Music Videos", "playlistId": "PLV2", "thumbnails": []},
    ],
    "artists": [
        {"title": "Artist X", "browseId": "UCX", "subscribers": "9.62M", "rank": "1", "trend": "neutral"},
        {"title": "Artist Y", "browseId": "UCY", "subscribers": "5.00M", "rank": "2", "trend": "up"},
    ],
    "genres": [
        {"title": "Top Pop Videos", "playlistId": "PLG1", "thumbnails": []},
    ],
}


class MockYTM:
    """Minimal ytmusicapi stub returning canned fixtures for get_home,
    get_mood_categories, get_mood_playlists, get_charts. Shape mirrors
    ytmusicapi's real responses. Pass overrides to customize per-test."""

    def __init__(self, home=None, mood_categories=None, mood_playlists=None,
                 charts=None):
        self._home = home if home is not None else _DEFAULT_HOME
        self._mood_cats = (mood_categories if mood_categories is not None
                           else _DEFAULT_MOOD_CATEGORIES)
        self._mood_pls = (mood_playlists if mood_playlists is not None
                          else _DEFAULT_MOOD_PLAYLISTS)
        self._charts = charts if charts is not None else _DEFAULT_CHARTS

    def get_home(self):
        return self._home

    def get_mood_categories(self):
        return self._mood_cats

    def get_mood_playlists(self, params):
        return self._mood_pls.get(params, [])

    def get_charts(self):
        return self._charts


class HomeTests(unittest.TestCase):
    def test_home_returns_home_sections_key(self):
        result = handle("home", {}, MockYTM())
        self.assertIn("home_sections", result)
        self.assertNotIn("home", result)
        sections = result["home_sections"]
        self.assertIsInstance(sections, list)
        for sec in sections:
            self.assertIn("title", sec)
            self.assertIn("items", sec)
            self.assertIsInstance(sec["items"], list)

    def test_home_item_shape(self):
        result = handle("home", {}, MockYTM())
        item = result["home_sections"][0]["items"][0]
        self.assertEqual(item["title"], "Mix 1")
        self.assertEqual(item["subtitle"], "Playlist")
        self.assertEqual(item["playlist_id"], "PL1")
        self.assertEqual(item["video_id"], "v1")
        self.assertEqual(item["artist"], "Artist A")
        self.assertEqual(item["browse_id"], "UCM1")

    def test_home_skips_missing_fields(self):
        home = [{"title": "sec", "contents": [{"title": "x"}]}]
        result = handle("home", {}, MockYTM(home=home))
        item = result["home_sections"][0]["items"][0]
        self.assertEqual(item["title"], "x")
        self.assertEqual(item["subtitle"], "")
        self.assertIsNone(item["playlist_id"])
        self.assertIsNone(item["video_id"])
        self.assertIsNone(item["artist"])
        self.assertIsNone(item["browse_id"])

    def test_home_empty_section_no_crash(self):
        home = [{"title": "empty", "contents": []}]
        result = handle("home", {}, MockYTM(home=home))
        self.assertEqual(result, {"home_sections": [{"title": "empty", "items": []}]})


class ExploreTests(unittest.TestCase):
    def test_explore_flattens_categories(self):
        result = handle("explore", {}, MockYTM())
        self.assertIn("explore_playlists", result)
        playlists = result["explore_playlists"]
        # 2 + 1 + 1 + 2 = 6 playlists across all categories.
        self.assertEqual(len(playlists), 6)
        self.assertEqual(playlists[0]["id"], "PL1")
        self.assertEqual(playlists[0]["title"], "80s Pop Hits")
        # subtitle carries the category title ("1980s").
        self.assertEqual(playlists[0]["subtitle"], "1980s")
        self.assertEqual(playlists[0]["count"], 42)
        self.assertEqual(playlists[5]["id"], "PL6")
        self.assertEqual(playlists[5]["title"], "Late Night")

    def test_explore_skips_categories_without_params(self):
        cats = {
            "For you": [
                {"params": "p1", "title": "good"},
                {"title": "no params here"},  # skipped
            ],
            "Bad section": "not a list",  # skipped
        }
        pls = {"p1": [{"playlistId": "PL1", "title": "ok"}]}
        result = handle("explore", {}, MockYTM(mood_categories=cats,
                                               mood_playlists=pls))
        playlists = result["explore_playlists"]
        self.assertEqual(len(playlists), 1)
        self.assertEqual(playlists[0]["id"], "PL1")

    def test_explore_handles_empty_categories(self):
        cats = {"Empty": []}
        result = handle("explore", {}, MockYTM(mood_categories=cats))
        self.assertEqual(result, {"explore_playlists": []})


class ChartsTests(unittest.TestCase):
    def test_charts_groups_by_chart_name(self):
        result = handle("charts", {}, MockYTM())
        self.assertIn("charts", result)
        entries = result["charts"]
        # 2 videos + 2 artists + 1 genre = 5 entries.
        self.assertEqual(len(entries), 5)
        chart_names = {e["chart"] for e in entries}
        self.assertEqual(chart_names, {"videos", "artists", "genres"})

    def test_charts_video_item_shape(self):
        result = handle("charts", {}, MockYTM())
        videos = [e for e in result["charts"] if e["chart"] == "videos"]
        self.assertEqual(len(videos), 2)
        first = videos[0]
        self.assertEqual(first["title"], "Daily Top Music Videos")
        self.assertEqual(first["playlist_id"], "PLV1")
        self.assertIsNone(first["video_id"])
        self.assertEqual(first["subtitle"], "")

    def test_charts_artist_item_uses_subscribers_as_subtitle(self):
        result = handle("charts", {}, MockYTM())
        artists = [e for e in result["charts"] if e["chart"] == "artists"]
        self.assertEqual(len(artists), 2)
        first = artists[0]
        self.assertEqual(first["title"], "Artist X")
        self.assertEqual(first["subtitle"], "9.62M")  # subscribers field
        self.assertIsNone(first["video_id"])
        self.assertIsNone(first["playlist_id"])

    def test_charts_skips_non_list_values(self):
        charts = {
            "videos": [{"title": "V1", "playlistId": "PL1"}],
            "countries": {"selected": "not a list"},  # skipped
            "Bad": "also not a list",  # skipped
            "Empty": [],  # skipped (no entries)
        }
        result = handle("charts", {}, MockYTM(charts=charts))
        entries = result["charts"]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["title"], "V1")
        self.assertEqual(entries[0]["chart"], "videos")

    def test_charts_skips_non_dict_items(self):
        charts = {"videos": ["not a dict", {"title": "ok", "playlistId": "PL1"}]}
        result = handle("charts", {}, MockYTM(charts=charts))
        entries = result["charts"]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["title"], "ok")


if __name__ == "__main__":
    unittest.main()
