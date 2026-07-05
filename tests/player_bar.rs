use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::view::player_bar::render as render_bar;
use ratatui::{backend::TestBackend, Terminal};

fn one_track_cat() -> (tempfile::TempDir, Catalog, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
      {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap(), lossless)
}

/// Render the player bar into a string by reading cells out of the
/// `TestBackend`'s buffer. ratatui 0.30's `TestBackend` exposes its buffer via
/// `buffer()`; `Buffer::get(x, y)` returns the `&Cell`, and `Cell::symbol()`
/// yields the glyph string. (There is no `TestBackend::cell(x, y)` accessor.)
fn rendered_bar(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render_bar(f, f.area(), app)).unwrap();
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
fn bar_shows_title_artist_and_quality() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 120, 3);
    assert!(bar.contains("Freedom"), "bar must show the title: {bar}");
    assert!(bar.contains("Ado"), "bar must show the artist: {bar}");
    assert!(bar.contains("24"), "bar must show bit depth: {bar}");
    assert!(bar.contains("96"), "bar must show sample rate: {bar}");
}

#[test]
fn bar_appends_bitperfect_when_switch_sample_rate() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.switch_sample_rate = true;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 120, 3);
    assert!(
        bar.contains("bit-perfect"),
        "bar must flag bit-perfect when switch_sample_rate is on: {bar}"
    );
}

#[test]
fn bar_omits_bitperfect_when_not_switching() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.switch_sample_rate = false;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 120, 3);
    assert!(
        !bar.contains("bit-perfect"),
        "bar must not flag bit-perfect when switch_sample_rate is off: {bar}"
    );
}

#[test]
fn bar_shows_volume_and_mode_flags() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None);
    app.volume = 70;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 120, 3);
    assert!(bar.contains("vol"), "bar must show a volume label: {bar}");
    assert!(bar.contains("70"), "bar must show the volume pct: {bar}");
    assert!(bar.contains("SHUF"), "bar must show the shuffle flag: {bar}");
    assert!(bar.contains("RPT"), "bar must show the repeat flag: {bar}");
}

#[test]
fn bar_renders_without_now_playing() {
    // No track loaded: the bar must still render without panicking and keep
    // its layout (no crash, just empty/dimmed chrome).
    let (_d, cat, _l) = one_track_cat();
    let app = App::new(cat, Box::new(StubPlayer::default()), None);
    let _bar = rendered_bar(&app, 120, 3);
    let _bar = rendered_bar(&app, 80, 3);
}
