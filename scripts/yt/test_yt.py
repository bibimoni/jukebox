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

_DEFAULT_EXPLORE = [
    {
        "items": [
            {"playlistId": "PL1", "title": "Chill", "subtitle": "mood", "itemCount": 42},
            {"playlistId": "PL2", "title": "Focus", "subtitle": "mood", "itemCount": 25},
        ]
    },
    {
        "items": [
            {"playlistId": "PL3", "title": "Workout", "subtitle": "energy", "itemCount": 30},
            {"playlistId": "PL4", "title": "Party", "subtitle": "energy", "itemCount": 18},
        ]
    },
]

_DEFAULT_CHARTS = {
    "Top songs": [
        {"title": "Song A", "subtitle": "Artist A", "videoId": "v1", "artists": [{"name": "Artist A"}]},
        {"title": "Song B", "subtitle": "Artist B", "videoId": "v2", "artists": [{"name": "Artist B"}]},
    ],
    "Trending": [
        {"title": "Trend 1", "subtitle": "Artist C", "videoId": "v3", "artists": [{"name": "Artist C"}]},
        {"title": "Trend 2", "subtitle": "Artist D", "videoId": "v4", "artists": [{"name": "Artist D"}]},
    ],
}


class MockYTM:
    """Minimal ytmusicapi stub returning canned fixtures for get_home,
    get_explore, get_charts. Shape mirrors ytmusicapi's responses. Pass
    overrides to customize per-test."""

    def __init__(self, home=None, explore=None, charts=None):
        self._home = home if home is not None else _DEFAULT_HOME
        self._explore = explore if explore is not None else _DEFAULT_EXPLORE
        self._charts = charts if charts is not None else _DEFAULT_CHARTS

    def get_home(self):
        return self._home

    def get_explore(self):
        return self._explore

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
        self.assertEqual(len(playlists), 4)
        self.assertEqual(playlists[0]["id"], "PL1")
        self.assertEqual(playlists[0]["title"], "Chill")
        self.assertEqual(playlists[0]["subtitle"], "mood")
        self.assertEqual(playlists[0]["count"], 42)
        self.assertEqual(playlists[3]["id"], "PL4")
        self.assertEqual(playlists[3]["title"], "Party")


class ChartsTests(unittest.TestCase):
    def test_charts_groups_by_chart_name(self):
        result = handle("charts", {}, MockYTM())
        self.assertIn("charts", result)
        entries = result["charts"]
        self.assertEqual(len(entries), 4)
        chart_names = {e["chart"] for e in entries}
        self.assertEqual(chart_names, {"Top songs", "Trending"})
        first = entries[0]
        self.assertEqual(first["title"], "Song A")
        self.assertEqual(first["subtitle"], "Artist A")
        self.assertEqual(first["video_id"], "v1")
        self.assertIsNone(first["playlist_id"])
        self.assertEqual(first["artist"], "Artist A")
        self.assertEqual(first["chart"], "Top songs")

    def test_charts_skips_non_list_values(self):
        charts = {
            "Top songs": [{"title": "S1", "videoId": "v1"}],
            "Bad entry": "not a list",
            "Another bad": {"also": "not a list"},
            "Empty": [],
        }
        result = handle("charts", {}, MockYTM(charts=charts))
        entries = result["charts"]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["title"], "S1")
        self.assertEqual(entries[0]["chart"], "Top songs")


if __name__ == "__main__":
    unittest.main()
