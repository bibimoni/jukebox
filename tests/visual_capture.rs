//! Visual capture harness — renders deterministic app states to JSON cell grids.
//!
//! Run: `cargo test --test visual_capture -- --nocapture`
//! Produces JSON grids in `tests/visual-out/` which are then converted to PNGs.

use jukebox::catalog::Catalog;
use jukebox::lyrics::{LyricLine, Lyrics, LyricsSource};
use jukebox::player::StubPlayer;
use jukebox::source::{RemoteTrack, StreamFormat};
use jukebox::tui::app::{App, LyricsState, Overlay, SearchScope, View, YtList, YtListKind};
use jukebox::tui::view::layout::draw;
use jukebox::yt::session::Session;
use jukebox::yt::state::YtState;
use ratatui::{backend::TestBackend, Terminal};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
struct CellOut {
    ch: String,
    fg: String,
    bg: String,
    bold: bool,
    underline: bool,
    reverse: bool,
}

#[derive(Serialize)]
struct GridOut {
    width: u16,
    height: u16,
    cells: Vec<Vec<CellOut>>,
    meta: serde_json::Value,
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/visual-out");
    fs::create_dir_all(&d).unwrap();
    d
}

fn rich_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let mut tracks = Vec::new();
    for (ai, artist) in ["Ado", "Aimer", "BUMP OF CHICKEN"].iter().enumerate() {
        for al in 0..2 {
            let album = format!("Album{}", al + 1);
            let dir = lossless.join(artist).join(&album);
            std::fs::create_dir_all(&dir).unwrap();
            for tr in 0..3 {
                let id = format!("t{}{}{}", ai, al, tr);
                let title = format!("Track{}", tr + 1);
                let path = dir.join(format!("{}.flac", tr + 1));
                std::fs::write(&path, b"x").unwrap();
                tracks.push(serde_json::json!({
                    "id": id, "artists":[artist], "primary_artist": artist,
                    "title": title, "album": album,
                    "bit_depth": if tr%2==0 {24} else {16},
                    "sample_rate_hz": if tr%2==0 {96000} else {44100},
                    "source_path": path.strip_prefix(d.path()).unwrap().to_str().unwrap(),
                    "symlinked_into_artists":[artist]
                }));
            }
        }
    }
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":tracks}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn empty_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn build_populated() -> App {
    let (_d, cat) = rich_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t000".into()], "t000");
    app
}

fn build_empty() -> App {
    let (_d, cat) = empty_cat();
    App::new(cat, Box::new(StubPlayer::default()), None, None)
}

fn build_with_yt(state: YtState, lists: Vec<YtList>) -> App {
    let (_d, cat) = rich_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = state;
    app.yt_lists = lists;
    app
}

/// Create a YouTube session with pre-populated track cache for visual capture.
/// Uses a fake sidecar script that responds to ping, but the track_cache is
/// pre-populated so track_for() returns real titles.
fn mock_yt_session(tracks: Vec<(&str, &str, &str)>) -> Session {
    let script = std::env::temp_dir().join("jk_fake_sidecar.py");
    fs::write(
        &script,
        r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if line == '{"cmd":"ping"}':
        print('{"ok":true,"cmd":"pong"}')
        sys.stdout.flush()
"#,
    )
    .unwrap();
    let mut session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    for (id, title, artist) in tracks {
        session.track_cache.insert(
            id.to_string(),
            RemoteTrack {
                video_id: id.to_string(),
                title: title.to_string(),
                artist: artist.to_string(),
                album: None,
                dur: Some(180.0 + id.len() as f64),
                fmt: Some(StreamFormat {
                    codec: "Opus".to_string(),
                    abr: 160,
                    sample_rate: 48000,
                    container: "webm".to_string(),
                    premium: false,
                }),
                isrc: None,
            },
        );
    }
    session
}

fn build_yt_with_tracks(state: YtState) -> App {
    let (_d, cat) = rich_cat();
    let yt_tracks = vec![
        ("v1", "Kaze no Yukue", "Yoasobi"),
        ("v2", "Gurenge", "LiSA"),
        ("v3", "Yoru ni Kakeru", "Yoasobi"),
        ("v4", "Homura", "LiSA"),
        ("v5", "Shukusei", "Ado"),
        ("v6", "Usse", "Ado"),
        ("v7", "Sparkler", "Yorushika"),
        ("v8", "Sayonara Byebye", "Yorushika"),
    ];
    let session = mock_yt_session(yt_tracks);
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.yt_state = state;
    app.yt_lists = vec![
        YtList {
            id: "pl1".into(),
            name: "J-Pop Favorites".into(),
            kind: YtListKind::Account,
            track_ids: vec![
                "v1".into(),
                "v2".into(),
                "v3".into(),
                "v4".into(),
                "v5".into(),
            ],
        },
        YtList {
            id: "pl2".into(),
            name: "Workout Mix".into(),
            kind: YtListKind::Account,
            track_ids: vec!["v6".into(), "v7".into(), "v8".into()],
        },
        YtList {
            id: "pl3".into(),
            name: "Chill Vibes".into(),
            kind: YtListKind::Suggested,
            track_ids: vec!["v5".into(), "v8".into()],
        },
    ];
    // Focus the first playlist so its tracks show
    app.cursors.playlist = 0;
    app
}

fn build_hybrid() -> App {
    let (_d, cat) = rich_cat();
    let yt_tracks = vec![
        ("v1", "Kaze no Yukue", "Yoasobi"),
        ("v2", "Gurenge", "LiSA"),
    ];
    let session = mock_yt_session(yt_tracks);
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, Some(session));
    app.yt_state = YtState::Ready;
    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.yt_lists = vec![YtList {
        id: "pl1".into(),
        name: "J-Pop Favorites".into(),
        kind: YtListKind::Account,
        track_ids: vec!["v1".into(), "v2".into()],
    }];
    app
}

fn capture(app: &mut App, w: u16, h: u16, name: &str, meta: serde_json::Value) {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let buf = term.backend().buffer();
    let mut cells = Vec::with_capacity(h as usize);
    for y in 0..h {
        let mut row = Vec::with_capacity(w as usize);
        for x in 0..w {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let ch = if sym.is_empty() {
                " ".to_string()
            } else {
                sym.to_string()
            };
            row.push(CellOut {
                ch,
                fg: format!("{:?}", cell.fg),
                bg: format!("{:?}", cell.bg),
                bold: (cell.style().add_modifier.bits() & 0x0001) != 0,
                underline: (cell.style().add_modifier.bits() & 0x0002) != 0,
                reverse: (cell.style().add_modifier.bits() & 0x0004) != 0,
            });
        }
        cells.push(row);
    }
    let grid = GridOut {
        width: w,
        height: h,
        cells,
        meta,
    };
    let path = out_dir().join(format!("{}.json", name));
    let json = serde_json::to_string_pretty(&grid).unwrap();
    fs::write(&path, json).unwrap();
    eprintln!("  captured: {} ({}x{})", name, w, h);
}

#[test]
fn capture_baseline_matrix() {
    eprintln!("=== Baseline capture matrix ===");
    let dims = [(80u16, 24u16), (100, 30), (120, 40), (160, 50)];

    for &(w, h) in &dims {
        let mut app = build_populated();
        app.view = View::Artists;
        capture(
            &mut app,
            w,
            h,
            &format!("local_populated_{}x{}", w, h),
            json!({"state":"local-populated","view":"artists"}),
        );

        let mut app = build_empty();
        app.view = View::Artists;
        capture(
            &mut app,
            w,
            h,
            &format!("local_empty_{}x{}", w, h),
            json!({"state":"local-empty","view":"artists"}),
        );

        let mut app = build_with_yt(YtState::SignedOut, vec![]);
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("yt_signed_out_{}x{}", w, h),
            json!({"state":"yt-signed-out","view":"youtube"}),
        );

        let mut app = build_with_yt(YtState::Authenticating, vec![]);
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("yt_authenticating_{}x{}", w, h),
            json!({"state":"yt-authenticating","view":"youtube"}),
        );

        let mut app = build_with_yt(YtState::Synchronizing, vec![]);
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("yt_synchronizing_{}x{}", w, h),
            json!({"state":"yt-synchronizing","view":"youtube"}),
        );

        let mut app = build_yt_with_tracks(YtState::Ready);
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("yt_ready_{}x{}", w, h),
            json!({"state":"yt-ready","view":"youtube","has_real_titles":true}),
        );

        let mut app = build_with_yt(YtState::Ready, vec![]);
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("yt_no_playlists_{}x{}", w, h),
            json!({"state":"yt-no-playlists","view":"youtube"}),
        );

        let mut app = build_with_yt(
            YtState::ReadyStale,
            vec![YtList {
                id: "pl1".into(),
                name: "Cached Playlist".into(),
                kind: YtListKind::Account,
                track_ids: vec!["v1".into(); 10],
            }],
        );
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("offline_cache_{}x{}", w, h),
            json!({"state":"offline-cache","view":"youtube"}),
        );

        let mut app = build_with_yt(YtState::Failed, vec![]);
        app.yt_error = Some("YT sidecar could not start — run :yt setup".into());
        app.view = View::Youtube;
        capture(
            &mut app,
            w,
            h,
            &format!("provider_failure_{}x{}", w, h),
            json!({"state":"provider-failure","view":"youtube"}),
        );
    }

    // Browsing states at 120x40
    let (w, h) = (120, 40);

    // Hybrid mode
    let mut app = build_hybrid();
    app.view = View::Artists;
    capture(
        &mut app,
        w,
        h,
        "hybrid_mode",
        json!({"state":"hybrid","view":"artists","mode":"mixed"}),
    );

    let mut app = build_populated();
    app.view = View::Playlists;
    capture(
        &mut app,
        w,
        h,
        "playlists_view",
        json!({"state":"playlists","view":"playlists"}),
    );

    let mut app = build_populated();
    app.view = View::Queue;
    capture(
        &mut app,
        w,
        h,
        "queue_empty",
        json!({"state":"queue-empty","view":"queue"}),
    );

    let mut app = build_populated();
    app.view = View::Queue;
    app.enqueue_selected();
    capture(
        &mut app,
        w,
        h,
        "queue_populated",
        json!({"state":"queue-populated","view":"queue"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Search {
        input: String::new(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Local,
        submitted: None,
        searching: false,
    });
    capture(
        &mut app,
        w,
        h,
        "search_empty",
        json!({"state":"search-empty","overlay":"search"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Search {
        input: "Ado".into(),
        results: vec!["AdoTrack1 — Ado".into()],
        cursor: 0,
        scope: SearchScope::Local,
        submitted: Some("Ado".into()),
        searching: false,
    });
    capture(
        &mut app,
        w,
        h,
        "search_populated",
        json!({"state":"search-populated","overlay":"search"}),
    );

    // Playback states
    let mut app = build_populated();
    app.now_playing = None;
    capture(
        &mut app,
        w,
        h,
        "nothing_playing",
        json!({"state":"nothing-playing"}),
    );

    let mut app = build_populated();
    capture(
        &mut app,
        w,
        h,
        "local_playing",
        json!({"state":"local-playing"}),
    );

    let mut app = build_populated();
    let _ = app.player.play_pause();
    capture(&mut app, w, h, "paused", json!({"state":"paused"}));

    // YouTube track playing
    let mut app = build_yt_with_tracks(YtState::Ready);
    app.view = View::Youtube;
    app.play_in_context_ids(vec!["v1".into()], "v1");
    capture(&mut app, w, h, "yt_playing", json!({"state":"yt-playing"}));

    // Lyrics states
    let mut app = build_populated();
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::Loading,
        scroll: 0,
        track_id: "t000".into(),
        gen: 1,
    });
    capture(
        &mut app,
        w,
        h,
        "lyrics_loading",
        json!({"state":"lyrics-loading","overlay":"lyrics"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Lyrics {
        content: Some(Lyrics {
            lines: vec![
                LyricLine {
                    time: Some(0.0),
                    text: "First line of lyrics".into(),
                },
                LyricLine {
                    time: Some(2.0),
                    text: "Second line here".into(),
                },
                LyricLine {
                    time: Some(4.0),
                    text: "Third line of the song".into(),
                },
                LyricLine {
                    time: Some(6.0),
                    text: "Fourth line follows".into(),
                },
                LyricLine {
                    time: Some(8.0),
                    text: "Fifth line of lyrics".into(),
                },
            ],
            synced: true,
            source: LyricsSource::Ytmusicapi,
        }),
        state: LyricsState::Available(true),
        scroll: 0,
        track_id: "t000".into(),
        gen: 1,
    });
    capture(
        &mut app,
        w,
        h,
        "lyrics_available",
        json!({"state":"lyrics-available","overlay":"lyrics","synced":true}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::NotFound,
        scroll: 0,
        track_id: "t000".into(),
        gen: 1,
    });
    capture(
        &mut app,
        w,
        h,
        "lyrics_unavailable",
        json!({"state":"lyrics-unavailable","overlay":"lyrics"}),
    );

    // Interaction states
    let mut app = build_populated();
    app.overlay = Some(Overlay::Help);
    capture(
        &mut app,
        w,
        h,
        "help_overlay",
        json!({"state":"help","overlay":"help"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Command {
        input: String::new(),
        cursor: 0,
    });
    capture(
        &mut app,
        w,
        h,
        "command_empty",
        json!({"state":"command-empty","overlay":"command"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Command {
        input: "yt auth browser chrome".into(),
        cursor: 21,
    });
    capture(
        &mut app,
        w,
        h,
        "command_with_text",
        json!({"state":"command-with-text","overlay":"command"}),
    );

    let mut app = build_populated();
    app.overlay = Some(Overlay::Diagnostics);
    capture(
        &mut app,
        w,
        h,
        "diagnostics",
        json!({"state":"diagnostics","overlay":"diagnostics"}),
    );

    // Narrow / too small
    let mut app = build_populated();
    capture(
        &mut app,
        60,
        20,
        "narrow_60x20",
        json!({"state":"narrow","size":"60x20"}),
    );

    let mut app = build_populated();
    capture(
        &mut app,
        50,
        15,
        "too_small_50x15",
        json!({"state":"too-small","size":"50x15"}),
    );

    eprintln!(
        "=== Baseline capture complete: {} grids ===",
        out_dir().display()
    );
}
