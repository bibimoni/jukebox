//! Golden-snapshot tests for the top-level `layout::draw` entry point.
//!
//! Snapshots are stored in `tests/snapshots/`. Regenerate with
//! `INSTA_UPDATE=1 cargo test --test layout`.

use insta::assert_snapshot;
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, LyricsState, Overlay, Playlist, View, YtList};
use jukebox::tui::view::layout::draw;
use ratatui::{backend::TestBackend, style::Modifier, Terminal};

/// Build a fixed 2-artist catalog: "40mP" (album "Cosmic", track "Song1") and
/// "DECO*27" (album "Ghost", track "Ghost Rule"). Each track points at a real
/// file on disk so `App`'s playback helpers don't trip over missing sources.
fn two_artist_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
    std::fs::create_dir_all(lossless.join("DECO")).unwrap();
    std::fs::write(lossless.join("DECO").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1","album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
          {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Ghost Rule","album":"Ghost","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// Build a real `App` over the 2-artist catalog with the first track playing,
/// so the snapshots show meaningful content in both the columns and the player
/// bar (title, artist, quality readout, progress gauge).
fn build_app() -> App {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Start playback on t1 so the player bar shows a now-playing track and a
    // non-zero progress gauge instead of the "— nothing playing —" placeholder.
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app
}

/// Read every cell of the `TestBackend`'s buffer into a flat string. ratatui
/// 0.30 has no `TestBackend::cell(x,y)`; the `Index` impl
/// `term.backend().buffer()[(x,y)]` yields the `&Cell`, whose `symbol()` is
/// the rendered glyph. We collapse runs of trailing spaces per line so the
/// snapshot diffs are legible.
fn buffer_string(term: &Terminal<TestBackend>, w: u16, h: u16) -> String {
    let mut s = String::new();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push(
                term.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        // Trim trailing whitespace so snapshot diffs stay legible at any width
        // (equivalent to the brief's `r" +\n" -> " \n"` filter).
        let trimmed = line.trim_end();
        s.push_str(trimmed);
        s.push('\n');
    }
    s
}

/// Draw `draw` into a `w x h` terminal and snapshot the rendered buffer under
/// `name`. Trailing whitespace is trimmed per line so the snapshots stay
/// readable at any width.
fn snapshot_at(w: u16, h: u16, name: &str) {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    let mut app = build_app();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let s = buffer_string(&term, w, h);
    assert_snapshot!(name, s);
}

#[test]
fn layout_120x24() {
    snapshot_at(120, 24, "wide");
}

#[test]
fn layout_80x24() {
    snapshot_at(80, 24, "standard");
}

#[test]
fn layout_70x24_narrow() {
    snapshot_at(70, 24, "narrow");
}

#[test]
fn layout_too_small() {
    snapshot_at(50, 18, "too_small");
    // Hard invariant: the too-small terminal must show the "terminal too small"
    // message and nothing else. Below the narrow floor (60×20).
    let backend = TestBackend::new(50, 18);
    let mut term = Terminal::new(backend).unwrap();
    let mut app = build_app();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let s = buffer_string(&term, 50, 18);
    assert!(
        s.contains("terminal too small"),
        "too-small render must contain the resize message: {s}"
    );
}

#[test]
fn lyrics_overlay_preserves_responsive_player_and_footer_chrome() {
    for height in [24, 25] {
        let backend = TestBackend::new(120, height);
        let mut term = Terminal::new(backend).unwrap();
        let mut app = build_app();
        app.overlay = Some(Overlay::Lyrics {
            content: None,
            state: LyricsState::NotFound,
            scroll: 0,
            track_id: "t1".into(),
            gen: app.lyrics_gen,
        });
        term.draw(|f| draw(f, &mut app)).unwrap();
        let rendered = buffer_string(&term, 120, height);
        assert!(
            rendered.contains("Song1"),
            "player chrome lost at {height}: {rendered}"
        );
        assert!(
            rendered.contains("VIEW:"),
            "footer chrome lost at {height}: {rendered}"
        );
    }
}

#[test]
fn youtube_breadcrumb_uses_playlist_cursor() {
    let backend = TestBackend::new(120, 25);
    let mut term = Terminal::new(backend).unwrap();
    let mut app = build_app();
    app.view = View::Youtube;
    app.focus_col = 1;
    app.yt_lists = vec![
        YtList {
            id: "a".into(),
            name: "Wrong artist cursor".into(),
            kind: Default::default(),
            track_ids: vec![],
        },
        YtList {
            id: "b".into(),
            name: "Selected playlist".into(),
            kind: Default::default(),
            track_ids: vec![],
        },
    ];
    app.cursors.artist = 0;
    app.cursors.playlist = 1;
    term.draw(|f| draw(f, &mut app)).unwrap();
    let rendered = buffer_string(&term, 120, 25);
    assert!(
        rendered.contains("YouTube › Selected playlist"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("YouTube › Wrong artist cursor"),
        "{rendered}"
    );
}

fn render_app(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|frame| draw(frame, app)).unwrap();
    buffer_string(&term, width, height)
}

#[test]
fn breadcrumbs_cover_all_views_and_artist_depths() {
    let mut app = build_app();
    app.view = View::Artists;
    app.focus_col = 0;
    assert!(render_app(&mut app, 120, 25).contains("Artists"));
    app.focus_col = 1;
    assert!(render_app(&mut app, 120, 25).contains("Artists › 40mP"));
    app.focus_col = 2;
    assert!(render_app(&mut app, 120, 25).contains("Artists › 40mP › Cosmic"));

    app.view = View::Playlists;
    app.playlists = vec![Playlist {
        name: "Roadtrip".into(),
        track_ids: vec![],
    }];
    assert!(render_app(&mut app, 120, 25).contains("Playlists › Roadtrip"));

    app.view = View::Queue;
    assert!(render_app(&mut app, 120, 25).contains("Queue"));

    app.view = View::Youtube;
    app.yt_lists = vec![YtList {
        id: "yt".into(),
        name: "Mix".into(),
        kind: Default::default(),
        track_ids: vec![],
    }];
    app.cursors.playlist = 0;
    assert!(render_app(&mut app, 120, 25).contains("YouTube › Mix"));
}

#[test]
fn active_tab_and_separator_are_rendered_across_full_width() {
    let mut app = build_app();
    app.view = View::Queue;
    let width = 120;
    let backend = TestBackend::new(width, 25);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|frame| draw(frame, &mut app)).unwrap();
    let rendered = buffer_string(&term, width, 25);
    let active = term.backend().buffer()[(26, 0)].modifier;
    assert!(active.contains(Modifier::BOLD), "{rendered}");
    assert!(active.contains(Modifier::REVERSED), "{rendered}");
    assert!(
        rendered
            .lines()
            .any(|line| line.chars().filter(|&c| c == '─').count() >= 100),
        "{rendered}"
    );
}
