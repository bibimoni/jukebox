use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;
use jukebox::tui::view::player_bar::{geometry, render as render_bar, truncate_title};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
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
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.volume = 70;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 120, 3);
    assert!(bar.contains("vol"), "bar must show a volume label: {bar}");
    assert!(bar.contains("70"), "bar must show the volume pct: {bar}");
    assert!(
        bar.contains("SHUF"),
        "bar must show the shuffle flag: {bar}"
    );
    assert!(bar.contains("RPT"), "bar must show the repeat flag: {bar}");
}

#[test]
fn bar_renders_without_now_playing() {
    // No track loaded: the bar must still render without panicking and keep
    // its layout (no crash, just empty/dimmed chrome).
    let (_d, cat, _l) = one_track_cat();
    let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let _bar = rendered_bar(&app, 120, 3);
    let _bar = rendered_bar(&app, 80, 3);
}

#[test]
fn rendered_controls_match_the_exported_hit_regions() {
    let (_d, cat, _l) = one_track_cat();
    let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let area = Rect::new(0, 10, 120, 2);
    let geo = geometry(area);
    let bar = rendered_bar(&app, 120, 2);

    assert!(
        bar.contains("◀◀"),
        "previous control must be rendered: {bar}"
    );
    assert!(bar.contains("▶"), "play control must be rendered: {bar}");
    assert!(bar.contains("▶▶"), "next control must be rendered: {bar}");
    assert_eq!(geo.previous.y, area.y);
    assert_eq!(geo.play_pause.y, area.y);
    assert_eq!(geo.next.y, area.y);
    assert_eq!(geo.progress.y, area.y + 1);
    assert_eq!(geo.progress.width, area.width * 55 / 100);
    assert!(geo.previous.right() <= geo.play_pause.x);
    assert!(geo.play_pause.right() <= geo.next.x);
}

#[test]
fn title_truncation_respects_cjk_combining_and_boundaries() {
    assert_eq!(truncate_title("abc", 3), "abc");
    assert_eq!(truncate_title("日本語", 5), "日本…");
    assert_eq!(truncate_title("e\u{301}clair", 3), "e\u{301}c…");
}

#[test]
fn player_state_labels_cover_stopped_playing_and_paused() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(rendered_bar(&app, 120, 2).contains("[STOPPED]"));
    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert!(rendered_bar(&app, 120, 2).contains("[PLAYING]"));
    app.player.play_pause().unwrap();
    assert!(rendered_bar(&app, 120, 2).contains("[PAUSED]"));
}

#[test]
fn up_next_preview_covers_idle_title_and_end() {
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert!(rendered_bar(&app, 120, 2).contains("Enter to play"));
    app.transport.enqueue("t1".into());
    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert!(rendered_bar(&app, 120, 2).contains("Next: Freedom"));
    app.transport.manual_queue.clear();
    assert!(rendered_bar(&app, 120, 2).contains("Next: (end)"));
}

#[test]
fn geometry_reserves_transport_columns_from_info_line() {
    // Bug 1: transport controls occupy the rightmost `min(14, width)` columns
    // (per `geometry()`), so `render()` passes `info_area.width.saturating_sub(14)`
    // to `build_info_line` — info content is truncated before the controls so
    // it is not overwritten by them. Assert the controls start 14 cols from the
    // right edge (the same 14 `render()` reserves).
    let area = Rect::new(0, 0, 120, 2);
    let geo = geometry(area);
    assert_eq!(
        geo.previous.x,
        area.right().saturating_sub(14),
        "transport controls must start 14 cols from the right edge"
    );
    // Narrow bar: controls clamp to the available width, still anchored right.
    let narrow = Rect::new(0, 0, 10, 2);
    let ng = geometry(narrow);
    assert_eq!(ng.previous.x, narrow.right().saturating_sub(10));
}

#[test]
fn geometry_compact_bar_has_zero_transport_rects() {
    // Bug 2: `render_compact` (height 1) draws no transport controls, so
    // `geometry()` reports zero-size rects for them — `rect_contains` in
    // input.rs then naturally returns false and clicks on the compact bar's
    // right edge do not trigger invisible prev/play/next actions.
    let area = Rect::new(0, 0, 80, 1);
    let g = geometry(area);
    assert_eq!(g.previous.width, 0);
    assert_eq!(g.previous.height, 0);
    assert_eq!(g.play_pause.width, 0);
    assert_eq!(g.play_pause.height, 0);
    assert_eq!(g.next.width, 0);
    assert_eq!(g.next.height, 0);
    assert_eq!(g.progress.width, 0, "progress is already zero in compact");

    // Full bar (height 2): transport controls are visible and clickable.
    let area2 = Rect::new(0, 0, 80, 2);
    let g2 = geometry(area2);
    assert!(
        g2.previous.width > 0,
        "previous control visible in full bar"
    );
    assert!(
        g2.play_pause.width > 0,
        "play/pause control visible in full bar"
    );
    assert!(g2.next.width > 0, "next control visible in full bar");
}

#[test]
fn progress_render_rect_matches_geometry_contract() {
    // Bug 4: rendering uses `geo.progress` (the same rect input hit-testing
    // uses) as the single source of truth — no `Layout::horizontal` split +
    // `debug_assert_eq!` that could round differently and panic at odd widths.
    let area = Rect::new(0, 0, 101, 2);
    let geo = geometry(area);
    assert_eq!(geo.progress.width, (101 * 55) / 100); // 55
                                                      // Rendering at an odd width must not panic (the old debug_assert could).
    let (_d, cat, _l) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    let bar = rendered_bar(&app, 101, 2); // must not panic
    assert!(
        bar.contains('▰') || bar.contains('▱'),
        "progress bar must render at odd width: {bar}"
    );
    assert!(
        bar.contains("SHUF"),
        "flags must render at odd width: {bar}"
    );
    // The flags area is the remaining width to the right of the progress bar.
    assert_eq!(area.width.saturating_sub(geo.progress.width), 101 - 55);
}

#[test]
fn truncate_title_zero_max_returns_empty() {
    // Bug 5: `max == 0` used to underflow `max - 1` to usize::MAX, returning
    // the full title + ellipsis. Now it returns an empty string.
    assert_eq!(truncate_title("Hello", 0), String::new());
    assert_eq!(truncate_title("", 0), String::new());
    // Non-zero max still works (regression guard).
    assert_eq!(truncate_title("Hello", 5), "Hello");
    assert_eq!(truncate_title("Hello", 3), "He…");
}
