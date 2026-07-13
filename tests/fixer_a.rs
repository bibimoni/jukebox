//! Regression tests for Fixer A defects (Batch A: Home overlay + Discover).
//!
//! - RC11-DEF-030: YT view playlists not type-distinguishable (uniform > glyph)
//! - RC11-DEF-040: Add-to-playlist overlay title truncated
//! - RC11-DEF-045: Home "Track Radio — radio" placeholder text
//! - RC11-DEF-052: No source labels in search results
//! - RC11-DEF-059: YouTube search returns a single result (no count indicator)
//!
//! DEF-001 / DEF-012 / DEF-013 / DEF-028 / DEF-029 / DEF-035 are covered by
//! lib tests in `src/tui/app.rs` and `src/tui/view/home.rs` (they don't need
//! the full layout::draw harness).

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, SearchScope, View, YtList, YtListKind};
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, Terminal};

fn one_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":"Local Song","album":"Al",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac",
        "symlinked_into_artists":["A"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn rendered(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let mut buf = String::new();
    for y in 0..h {
        for x in 0..w {
            let c = &term.backend().buffer()[(x, y)];
            buf.push(c.symbol().chars().next().unwrap_or(' '));
        }
        buf.push('\n');
    }
    buf
}

// ---------------------------------------------------------------------------
// RC11-DEF-030: YT view playlists not type-distinguishable
// ---------------------------------------------------------------------------

#[test]
fn def030_yt_view_account_playlist_uses_song_glyph() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "My Playlist".into(),
        kind: YtListKind::Account,
        track_ids: vec![],
    }];
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains('\u{266B}'),
        "DEF-030: account playlist should use the ♫ glyph: {buf}"
    );
}

#[test]
fn def030_yt_view_suggested_playlist_uses_star_glyph() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_lists = vec![YtList {
        id: "RD1".into(),
        name: "Suggested Mix".into(),
        kind: YtListKind::Suggested,
        track_ids: vec![],
    }];
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains('\u{2726}'),
        "DEF-030: suggested playlist should use the ✦ glyph: {buf}"
    );
}

#[test]
fn def030_yt_view_generated_playlist_uses_diamond_glyph() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Youtube;
    app.yt_lists = vec![YtList {
        id: "GEN1".into(),
        name: "Daily Mix".into(),
        kind: YtListKind::Generated,
        track_ids: vec![],
    }];
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains('\u{25C6}'),
        "DEF-030: generated playlist should use the ◆ glyph: {buf}"
    );
}

// ---------------------------------------------------------------------------
// RC11-DEF-040: Add-to-playlist overlay title truncated
// ---------------------------------------------------------------------------

#[test]
fn def040_playlist_picker_title_truncates_long_track_name() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Use a long track id that resolves to a long label. The catalog's only
    // track is "Local Song — A" (short), so synthesize a long id and let
    // track_label fall back to the raw id.
    let long_id = "VeryLongTrackNameThatWouldOverflowTheTitle";
    app.overlay = Some(Overlay::PlaylistPicker {
        track_id: long_id.into(),
        cursor: 0,
    });
    let buf = rendered(&mut app, 80, 24);
    // The title must still contain the "Enter confirm" suffix (not truncated
    // off the right edge by the long track name).
    assert!(
        buf.contains("Enter confirm"),
        "DEF-040: playlist picker title must keep 'Enter confirm' suffix when the track name is long: {buf}"
    );
}

// ---------------------------------------------------------------------------
// RC11-DEF-045: Home "Track Radio — radio" placeholder text
// ---------------------------------------------------------------------------

#[test]
fn def045_home_radio_seed_subtitle_not_placeholder_radio() {
    use jukebox::tui::view::home::HomeItem;
    let item = HomeItem::radio_seed("Track Radio".into());
    let subtitle = item.subtitle.expect("radio_seed must have a subtitle");
    assert_ne!(
        subtitle, "radio",
        "DEF-045: radio_seed subtitle must not be the bare placeholder 'radio': {subtitle:?}"
    );
    assert!(
        !subtitle.is_empty(),
        "DEF-045: radio_seed subtitle must be a real description, not empty"
    );
}

// ---------------------------------------------------------------------------
// RC11-DEF-052: No source labels in search results
// ---------------------------------------------------------------------------

#[test]
fn def052_search_results_have_source_labels() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // A local catalog track in the results → [L] badge.
    app.overlay = Some(Overlay::Search {
        input: "Song".into(),
        results: vec!["t1".into()],
        cursor: 0,
        scope: SearchScope::Local,
        submitted: Some("Song".into()),
        searching: false,
    });
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("[L]"),
        "DEF-052: local search result must carry the [L] source badge: {buf}"
    );
}

#[test]
fn def052_search_results_youtube_badge_for_non_catalog_id() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // A YouTube video id (not in the local catalog) → [Y] badge.
    app.overlay = Some(Overlay::Search {
        input: "vid".into(),
        results: vec!["vXYZ123".into()],
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("vid".into()),
        searching: false,
    });
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("[Y]"),
        "DEF-052: YouTube search result must carry the [Y] source badge: {buf}"
    );
}

// ---------------------------------------------------------------------------
// RC11-DEF-059: YouTube search returns a single result (no count indicator)
// ---------------------------------------------------------------------------

#[test]
fn def059_youtube_search_shows_result_count() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Search {
        input: "Blinding".into(),
        results: vec!["v001".into()],
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("Blinding".into()),
        searching: false,
    });
    let buf = rendered(&mut app, 100, 30);
    // The status line must show the result count ("1 result") so a fixture-
    // bound single-result search is visibly a count, not a truncated list.
    assert!(
        buf.contains("1 result"),
        "DEF-059: YouTube search must show the result count: {buf}"
    );
}

#[test]
fn def059_youtube_search_shows_plural_result_count() {
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Search {
        input: "song".into(),
        results: vec!["v001".into(), "v002".into(), "v003".into()],
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("song".into()),
        searching: false,
    });
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("3 results"),
        "DEF-059: YouTube search with 3 results must show '3 results' (plural): {buf}"
    );
}
