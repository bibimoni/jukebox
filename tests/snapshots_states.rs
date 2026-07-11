//! Snapshot tests for all important TUI states (AC-M6.3.1).
//!
//! ≥20 insta snapshot tests covering: local-populated, local-empty,
//! YT-signed-out, YT-authenticating, YT-synchronizing, YT-ready,
//! YT-no-playlists, offline-cache, provider-failure, search-empty,
//! search-populated, queue-empty, queue-populated, lyrics-loading,
//! lyrics-available, lyrics-unavailable, help, command+history,
//! confirmation, too-small.
//!
//! Snapshots stored in `tests/snapshots/`. Regenerate with
//! `INSTA_UPDATE=1 cargo test --test snapshots_states`.

use insta::assert_snapshot;
use jukebox::catalog::Catalog;
use jukebox::lyrics::{LyricLine, Lyrics, LyricsSource};
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, LyricsState, Overlay, SearchScope, View, YtList, YtListKind};
use jukebox::tui::view::layout::draw;
use jukebox::yt::state::YtState;
use ratatui::{backend::TestBackend, Terminal};

// ---------------------------------------------------------------------------
// Helpers (mirrors tests/layout.rs)
// ---------------------------------------------------------------------------

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

fn empty_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn build_app_populated() -> App {
    let (_d, cat) = two_artist_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into()], "t1");
    app
}

fn build_app_empty() -> App {
    let (_d, cat) = empty_cat();
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

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
        s.push_str(line.trim_end());
        s.push('\n');
    }
    s
}

fn snapshot_state(app: &mut App, name: &str) {
    let w = 120;
    let h = 24;
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let s = buffer_string(&term, w, h);
    assert_snapshot!(name, s);
}

fn sample_yt_lists() -> Vec<YtList> {
    vec![
        YtList {
            id: "PL1".into(),
            name: "Liked Music".into(),
            kind: YtListKind::Account,
            track_ids: vec!["yt1".into(), "yt2".into()],
        },
        YtList {
            id: "PL2".into(),
            name: "Chill".into(),
            kind: YtListKind::Account,
            track_ids: vec![],
        },
        YtList {
            id: "SG1".into(),
            name: "Mood: Focus".into(),
            kind: YtListKind::Suggested,
            track_ids: vec!["yt3".into()],
        },
    ]
}

// ---------------------------------------------------------------------------
// Local states
// ---------------------------------------------------------------------------

#[test]
fn local_populated() {
    let mut app = build_app_populated();
    snapshot_state(&mut app, "local_populated");
}

#[test]
fn local_empty() {
    let mut app = build_app_empty();
    snapshot_state(&mut app, "local_empty");
}

// ---------------------------------------------------------------------------
// YouTube provider states
// ---------------------------------------------------------------------------

#[test]
fn yt_signed_out() {
    let mut app = build_app_populated();
    app.yt_state = YtState::SignedOut;
    app.view = View::Youtube;
    snapshot_state(&mut app, "yt_signed_out");
}

#[test]
fn yt_authenticating() {
    let mut app = build_app_populated();
    app.yt_state = YtState::Authenticating;
    app.view = View::Youtube;
    snapshot_state(&mut app, "yt_authenticating");
}

#[test]
fn yt_synchronizing() {
    let mut app = build_app_populated();
    app.yt_state = YtState::Synchronizing;
    app.yt_lists_loading = true;
    app.view = View::Youtube;
    snapshot_state(&mut app, "yt_synchronizing");
}

#[test]
fn yt_ready() {
    let mut app = build_app_populated();
    app.yt_state = YtState::Ready;
    app.yt_lists = sample_yt_lists();
    app.view = View::Youtube;
    snapshot_state(&mut app, "yt_ready");
}

#[test]
fn yt_no_playlists() {
    let mut app = build_app_populated();
    app.yt_state = YtState::Ready;
    app.yt_lists.clear();
    app.view = View::Youtube;
    snapshot_state(&mut app, "yt_no_playlists");
}

#[test]
fn offline_cache() {
    let mut app = build_app_populated();
    app.yt_session = None;
    app.yt_state = YtState::ReadyStale;
    app.yt_lists = sample_yt_lists();
    app.view = View::Youtube;
    snapshot_state(&mut app, "offline_cache");
}

#[test]
fn provider_failure() {
    let mut app = build_app_populated();
    app.yt_state = YtState::ProviderError;
    app.view = View::Youtube;
    snapshot_state(&mut app, "provider_failure");
}

// ---------------------------------------------------------------------------
// Search overlay states
// ---------------------------------------------------------------------------

#[test]
fn search_empty() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::Search {
        input: "zzz".into(),
        results: vec![],
        cursor: 0,
        scope: SearchScope::Local,
        submitted: Some("zzz".into()),
        searching: false,
    });
    snapshot_state(&mut app, "search_empty");
}

#[test]
fn search_populated() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::Search {
        input: "Song".into(),
        results: vec!["t1".into()],
        cursor: 0,
        scope: SearchScope::Local,
        submitted: Some("Song".into()),
        searching: false,
    });
    snapshot_state(&mut app, "search_populated");
}

// ---------------------------------------------------------------------------
// Queue states
// ---------------------------------------------------------------------------

#[test]
fn queue_empty() {
    let mut app = build_app_populated();
    app.view = View::Queue;
    app.transport.manual_queue.clear();
    snapshot_state(&mut app, "queue_empty");
}

#[test]
fn queue_populated() {
    let mut app = build_app_populated();
    app.view = View::Queue;
    app.transport.manual_queue.clear();
    app.transport.enqueue("t1".into());
    app.transport.enqueue("t2".into());
    snapshot_state(&mut app, "queue_populated");
}

// ---------------------------------------------------------------------------
// Lyrics overlay states
// ---------------------------------------------------------------------------

#[test]
fn lyrics_loading() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::Loading,
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_state(&mut app, "lyrics_loading");
}

#[test]
fn lyrics_available() {
    let mut app = build_app_populated();
    let lyrics = Lyrics {
        lines: vec![
            LyricLine {
                time: Some(0.0),
                text: "[00:00] First line".into(),
            },
            LyricLine {
                time: Some(5.0),
                text: "[00:05] Second line".into(),
            },
        ],
        synced: true,
        source: LyricsSource::Embedded,
    };
    app.overlay = Some(Overlay::Lyrics {
        content: Some(lyrics),
        state: LyricsState::Available(true),
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_state(&mut app, "lyrics_available");
}

#[test]
fn lyrics_unavailable() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::NotFound,
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_state(&mut app, "lyrics_unavailable");
}

// ---------------------------------------------------------------------------
// Other overlay states
// ---------------------------------------------------------------------------

#[test]
fn help_overlay() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::Help);
    snapshot_state(&mut app, "help_overlay");
}

#[test]
fn command_with_history() {
    let mut app = build_app_populated();
    app.command_history = vec!["sync".into(), "yt auth browser chrome".into()];
    app.overlay = Some(Overlay::Command {
        input: "sync".into(),
        cursor: 4,
    });
    snapshot_state(&mut app, "command_with_history");
}

#[test]
fn confirmation_playlist_picker() {
    let mut app = build_app_populated();
    app.overlay = Some(Overlay::PlaylistPicker {
        track_id: "t1".into(),
        cursor: 0,
    });
    snapshot_state(&mut app, "confirmation_playlist_picker");
}

// ---------------------------------------------------------------------------
// Edge case: too-small terminal
// ---------------------------------------------------------------------------

#[test]
fn too_small_state() {
    let mut app = build_app_populated();
    let w = 50;
    let h = 18;
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();
    let s = buffer_string(&term, w, h);
    assert_snapshot!("too_small_state", s);
}
