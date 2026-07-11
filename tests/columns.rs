use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::view::columns::render as render_cols;
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

/// Build a 2-artist catalog: "40mP" (album "Cosmic", track "Song1") and
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
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// Render `app`'s columns into a flat string by reading every cell out of the
/// `TestBackend`'s buffer. ratatui 0.30 has no `TestBackend::cell(x,y)`; instead
/// use the `Index` impl: `term.backend().buffer()[(x,y)]` -> `&Cell` ->
/// `Cell::symbol()` (`Buffer::get` is deprecated).
fn rendered(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, w, h);
    term.draw(|f| render_cols(f, area, app)).unwrap();
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

#[test]
fn columns_show_artists_and_albums_and_tracks() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // cursors default to 0 -> 40mP / Cosmic / Song1.
    let buf = rendered(&mut app, 120, 30);
    assert!(buf.contains("40mP"), "artist column must show the artist: {buf}");
    assert!(buf.contains("Cosmic"), "album column must show the album: {buf}");
    assert!(buf.contains("Song1"), "track column must show the track: {buf}");
}

#[test]
fn rail_highlights_active_view() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = jukebox::tui::app::View::Queue;
    let buf = rendered(&mut app, 120, 30);
    // Rail must still render its labels in every view; Queue view must show
    // the (empty) queue column without panicking.
    assert!(buf.contains('Q'), "rail must render the Q label: {buf}");
}

#[test]
fn queue_view_lists_manual_queue_titles() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = jukebox::tui::app::View::Queue;
    app.transport.manual_queue.push("t1".into());
    app.transport.manual_queue.push("t2".into());
    let buf = rendered(&mut app, 120, 30);
    assert!(buf.contains("Song1"), "queue column must resolve t1 -> Song1: {buf}");
    assert!(buf.contains("Ghost Rule"), "queue column must resolve t2 -> Ghost Rule: {buf}");
}

#[test]
fn now_playing_track_marked_with_glyph() {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let buf = rendered(&mut app, 120, 30);
    assert!(buf.contains('▶'), "now-playing track must be marked with the play glyph: {buf}");
}

#[test]
fn youtube_view_renders_account_and_suggested_lists() {
    let (_d, cat) = cat_albums_for_yt();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = jukebox::tui::app::View::Youtube;
    app.yt_lists = vec![
        jukebox::tui::app::YtList { id: "PL1".into(), name: "Liked Songs".into(),
            kind: jukebox::tui::app::YtListKind::Account, track_ids: vec![] },
        jukebox::tui::app::YtList { id: "RD1".into(), name: "Focus Flow".into(),
            kind: jukebox::tui::app::YtListKind::Suggested, track_ids: vec![] },
    ];
    let s = render(&app);
    assert!(s.contains("♫ Liked Songs"), "{s}");
    assert!(s.contains("✦ Focus Flow"), "{s}");
    assert!(s.contains("Up Next") || s.contains("Suggested"), "missing up-next: {s}");
}

#[test]
fn youtube_view_shows_setup_hint_when_no_session() {
    let (_d, cat) = cat_albums_for_yt();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = jukebox::tui::app::View::Youtube;
    let s = render(&app);
    assert!(s.contains(":yt auth") || s.contains(":yt setup"), "missing setup hint: {s}");
}

fn cat_albums_for_yt() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":"S","album":"Al",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac",
        "symlinked_into_artists":["A"]}]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

use jukebox::tui::view::layout::draw;
fn render(app: &App) -> String {
    // draw needs &mut App, but render is read-only for our assertion; clone
    // state into a mut app via a fresh build to avoid mutating the test's app.
    let mut app2 = clone_for_render(app);
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| draw(f, &mut app2)).unwrap();
    let buf = term.backend().buffer();
    let mut s = String::new();
    for y in 0..30 {
        for x in 0..120 {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}
fn clone_for_render(app: &App) -> App {
    // Simplest: rebuild an app with the same yt_lists. App isn't Clone, so
    // rebuild from a trivial catalog (the renderer only reads app state we set).
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":"S","album":"Al",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac","symlinked_into_artists":["A"]}]})
        .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut a = App::new(cat, Box::new(StubPlayer::default()), None, None);
    a.view = app.view;
    a.yt_lists = app.yt_lists.clone();
    a.yt_error = app.yt_error.clone();
    a.yt_session = None; // tests have no session
    a
}
