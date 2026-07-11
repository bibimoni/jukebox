//! State-snapshot tests for all important TUI states (AC-M6.3.1).
//!
//! Renders the full `layout::draw` for each of the 20+ required states and
//! snapshots the buffer via insta. Snapshots live in `tests/snapshots/`.
//! Regenerate with `INSTA_UPDATE=1 cargo test --test state_snapshots`.

use insta::assert_snapshot;
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, View, YtList, YtListKind};
use jukebox::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
use jukebox::tui::view::layout::draw;
use jukebox::yt::state::YtState;
use ratatui::{backend::TestBackend, Terminal};

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
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

/// An empty catalog (no tracks) — for the local-empty state.
fn empty_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":d.path().to_str().unwrap(),
        "tracks":[]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn build_app() -> App {
    let (_d, cat) = two_artist_cat();
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

fn build_empty_app() -> App {
    let (_d, cat) = empty_cat();
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

/// Read every cell of the `TestBackend`'s buffer into a flat string.
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
        let trimmed = line.trim_end();
        s.push_str(trimmed);
        s.push('\n');
    }
    s
}

fn snapshot_at(name: &str, w: u16, h: u16, app: &mut App) {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let s = buffer_string(&term, w, h);
    assert_snapshot!(name, s);
}

// --- Local states (Artists view) ---

#[test]
fn state_local_populated() {
    let mut app = build_app();
    app.view = View::Artists;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    snapshot_at("local_populated", 120, 24, &mut app);
}

#[test]
fn state_local_empty() {
    let mut app = build_empty_app();
    app.view = View::Artists;
    snapshot_at("local_empty", 120, 24, &mut app);
}

// --- YouTube states ---

#[test]
fn state_yt_signed_out() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::SignedOut;
    app.yt_session = None;
    snapshot_at("yt_signed_out", 120, 24, &mut app);
}

#[test]
fn state_yt_authenticating() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::Authenticating;
    snapshot_at("yt_authenticating", 120, 24, &mut app);
}

#[test]
fn state_yt_synchronizing() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::Synchronizing;
    app.yt_lists_loading = true;
    snapshot_at("yt_synchronizing", 120, 24, &mut app);
}

#[test]
fn state_yt_ready() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::Ready;
    app.yt_lists = vec![
        YtList {
            id: "PL1".into(),
            name: "Liked Songs".into(),
            kind: YtListKind::Account,
            track_ids: vec![],
        },
        YtList {
            id: "RD1".into(),
            name: "Focus Flow".into(),
            kind: YtListKind::Suggested,
            track_ids: vec![],
        },
    ];
    snapshot_at("yt_ready", 120, 24, &mut app);
}

#[test]
fn state_yt_no_playlists() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::Ready;
    app.yt_lists = vec![];
    snapshot_at("yt_no_playlists", 120, 24, &mut app);
}

#[test]
fn state_offline_cache() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::ReadyStale;
    app.yt_lists = vec![YtList {
        id: "PL1".into(),
        name: "Liked Songs".into(),
        kind: YtListKind::Account,
        track_ids: vec![],
    }];
    snapshot_at("offline_cache", 120, 24, &mut app);
}

#[test]
fn state_provider_failure() {
    let mut app = build_app();
    app.view = View::Youtube;
    app.yt_state = YtState::ProviderError;
    app.yt_error = Some("connection refused".into());
    snapshot_at("provider_failure", 120, 24, &mut app);
}

// --- Search states ---

#[test]
fn state_search_empty() {
    let mut app = build_app();
    app.view = View::Artists;
    app.filter = Some(jukebox::tui::app::FilterState {
        col: 0,
        text: "zzznomatch".into(),
    });
    snapshot_at("search_empty", 120, 24, &mut app);
}

#[test]
fn state_search_populated() {
    let mut app = build_app();
    app.view = View::Artists;
    app.filter = Some(jukebox::tui::app::FilterState {
        col: 0,
        text: "40".into(),
    });
    snapshot_at("search_populated", 120, 24, &mut app);
}

// --- Queue states ---

#[test]
fn state_queue_empty() {
    let mut app = build_app();
    app.view = View::Queue;
    snapshot_at("queue_empty", 120, 24, &mut app);
}

#[test]
fn state_queue_populated() {
    let mut app = build_app();
    app.view = View::Queue;
    app.transport.manual_queue.push("t1".into());
    app.transport.manual_queue.push("t2".into());
    snapshot_at("queue_populated", 120, 24, &mut app);
}

// --- Help + Command overlays ---

#[test]
fn state_help() {
    let mut app = build_app();
    app.overlay = Some(jukebox::tui::app::Overlay::Help);
    snapshot_at("help", 120, 24, &mut app);
}

#[test]
fn state_command_history() {
    let mut app = build_app();
    app.overlay = Some(jukebox::tui::app::Overlay::Command {
        input: ":queue".into(),
        cursor: 6,
    });
    snapshot_at("command_history", 120, 24, &mut app);
}

// --- Playlist picker (confirmation) ---

#[test]
fn state_confirmation() {
    let mut app = build_app();
    app.overlay = Some(jukebox::tui::app::Overlay::PlaylistPicker {
        track_id: "t1".into(),
        cursor: 0,
    });
    snapshot_at("confirmation", 120, 24, &mut app);
}

// --- Too-small ---

#[test]
fn state_too_small() {
    let mut app = build_app();
    snapshot_at("state_too_small", 50, 18, &mut app);
}

// --- Lyrics states ---

#[test]
fn state_lyrics_loading() {
    let mut app = build_app();
    app.overlay = Some(jukebox::tui::app::Overlay::Lyrics {
        content: None,
        state: jukebox::tui::app::LyricsState::Loading,
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_at("lyrics_loading", 120, 24, &mut app);
}

#[test]
fn state_lyrics_available() {
    let mut app = build_app();
    let lyrics = jukebox::lyrics::Lyrics {
        lines: vec![
            jukebox::lyrics::LyricLine {
                time: Some(0.0),
                text: "First line".into(),
            },
            jukebox::lyrics::LyricLine {
                time: Some(5.0),
                text: "Second line".into(),
            },
        ],
        synced: true,
        source: jukebox::lyrics::LyricsSource::Embedded,
    };
    app.overlay = Some(jukebox::tui::app::Overlay::Lyrics {
        content: Some(lyrics),
        state: jukebox::tui::app::LyricsState::Available(true),
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_at("lyrics_available", 120, 24, &mut app);
}

#[test]
fn state_lyrics_unavailable() {
    let mut app = build_app();
    app.overlay = Some(jukebox::tui::app::Overlay::Lyrics {
        content: None,
        state: jukebox::tui::app::LyricsState::NotFound,
        scroll: 0,
        track_id: "t1".into(),
        gen: 0,
    });
    snapshot_at("lyrics_unavailable", 120, 24, &mut app);
}

// --- Extra: shuffled + repeat states ---

#[test]
fn state_shuffled_repeat() {
    let mut app = build_app();
    app.view = View::Artists;
    app.transport.shuffle = ShuffleMode::Smart;
    app.transport.repeat = RepeatMode::All;
    app.transport.continue_mode = ContinueMode::NextAlbum;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    snapshot_at("shuffled_repeat", 120, 24, &mut app);
}
