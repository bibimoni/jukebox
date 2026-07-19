#[test]
fn sidebar_smoke_100x30() {
    use jukebox::catalog::Catalog;
    use jukebox::player::StubPlayer;
    use jukebox::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[{"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    std::mem::forget(d);
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.sidebar_visible = true;
    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| jukebox::tui::view::layout::draw(f, &mut app))
        .unwrap();
    let buf = term.backend().buffer();
    let mut text = String::new();
    for y in 0..30u16 {
        for x in 0..100u16 {
            text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        text.push('\n');
    }
    for c in [
        "VIEWS",
        "DISCOVER",
        "TOOLS",
        "Artists",
        "Playlists",
        "Queue",
        "YouTube",
        "Home",
        "Generator",
        "Search",
    ] {
        assert!(
            text.contains(c),
            "sidebar smoke: missing {c} at 100x30:\n{text}"
        );
    }
    assert!(
        text.contains("? help"),
        "help must remain discoverable in the footer at 100x30:\n{text}"
    );
}
