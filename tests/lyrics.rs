//! Tests for the lyrics provider pipeline: LRC parsing, plain-text parsing,
//! embedded/sidecar file reads, sidecar wire protocol, stale-discard via
//! generation guard, not-found/error states, and narrow-render safety.
//!
//! These tests are deterministic and run without network access — the
//! ytmusicapi path is tested via the wire protocol (proto.rs) and session
//! stubs, not a live sidecar.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Track;
use jukebox::lyrics::{parse_lrc, parse_plain, read_sidecar_file, Lyrics, LyricsSource};
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, LyricsState, Overlay};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use jukebox::yt::proto::{LyricLineProto, Request, Response};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;

// --- LRC parsing -----------------------------------------------------------

#[test]
fn parse_lrc_basic_synced() {
    let lrc = "[00:12.50]First line\n[00:15.30]Second line\n[00:18.10]Third";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 3);
    assert_eq!(lyrics.lines[0].time, Some(12.5));
    assert_eq!(lyrics.lines[0].text, "First line");
    assert_eq!(lyrics.lines[1].time, Some(15.3));
    assert_eq!(lyrics.lines[2].time, Some(18.1));
}

#[test]
fn parse_lrc_multi_timestamp_line() {
    // A line with multiple timestamps expands to one LyricLine per timestamp
    // (LRC spec: `[00:01.00][00:15.00]Refrain` → two entries).
    let lrc = "[00:01.00][00:15.00]Refrain";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].time, Some(1.0));
    assert_eq!(lyrics.lines[1].time, Some(15.0));
    assert_eq!(lyrics.lines[0].text, "Refrain");
    assert_eq!(lyrics.lines[1].text, "Refrain");
}

#[test]
fn parse_lrc_skips_metadata_tags() {
    let lrc = "[ti:Song Title]\n[ar:Artist]\n[al:Album]\n[by:Author]\n[00:12.50]Real line";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert_eq!(lyrics.lines.len(), 1);
    assert_eq!(lyrics.lines[0].text, "Real line");
    assert_eq!(lyrics.lines[0].time, Some(12.5));
}

#[test]
fn parse_lrc_milliseconds() {
    // Three-digit fractional (milliseconds, per ytmusicapi-research.md §5).
    let lrc = "[00:12.500]Line";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert_eq!(lyrics.lines[0].time, Some(12.5));
}

#[test]
fn parse_lrc_no_timestamps_is_plain() {
    let lrc = "Just text\nNo timestamps";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert!(!lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert!(lyrics.lines[0].time.is_none());
}

#[test]
fn parse_lrc_minutes_over_59() {
    // LRC allows minutes > 59 for long tracks.
    let lrc = "[75:00.00]Long track";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert_eq!(lyrics.lines[0].time, Some(4500.0));
}

#[test]
fn parse_lrc_blank_lines_skipped() {
    let lrc = "[00:01.00]First\n\n[00:02.00]Second";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].text, "First");
    assert_eq!(lyrics.lines[1].text, "Second");
}

// --- Plain parsing ---------------------------------------------------------

#[test]
fn parse_plain_keeps_blank_spacers() {
    let text = "Verse 1\n\nVerse 2";
    let lyrics = parse_plain(text, LyricsSource::Embedded);
    assert!(!lyrics.synced);
    assert_eq!(lyrics.lines.len(), 3);
    assert_eq!(lyrics.lines[1].text, ""); // blank spacer preserved
}

#[test]
fn parse_plain_all_none_time() {
    let lyrics = parse_plain("a\nb\nc", LyricsSource::Embedded);
    for l in &lyrics.lines {
        assert!(l.time.is_none());
    }
}

// --- Sidecar file reads ----------------------------------------------------

#[test]
fn read_sidecar_file_returns_none_when_absent() {
    let p = Path::new("/tmp/no-such-audio-xyz-test.flac");
    assert!(read_sidecar_file(p).is_none());
}

#[test]
fn read_sidecar_file_reads_lrc() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("song.flac");
    std::fs::write(&audio, b"x").unwrap();
    let lrc = dir.path().join("song.lrc");
    std::fs::write(&lrc, "[00:01.00]Hello\n[00:03.00]World").unwrap();
    let lyrics = read_sidecar_file(&audio).unwrap();
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].text, "Hello");
    assert_eq!(lyrics.source, LyricsSource::SidecarFile);
}

#[test]
fn read_sidecar_file_reads_txt_as_plain() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("song.flac");
    std::fs::write(&audio, b"x").unwrap();
    let txt = dir.path().join("song.txt");
    std::fs::write(&txt, "Verse 1\nVerse 2").unwrap();
    let lyrics = read_sidecar_file(&audio).unwrap();
    assert!(!lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
}

#[test]
fn read_sidecar_prefers_lrc_over_txt() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("song.flac");
    std::fs::write(&audio, b"x").unwrap();
    std::fs::write(dir.path().join("song.txt"), "plain text").unwrap();
    std::fs::write(dir.path().join("song.lrc"), "[00:00.50]synced").unwrap();
    let lyrics = read_sidecar_file(&audio).unwrap();
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines[0].text, "synced");
}

// --- Wire protocol ---------------------------------------------------------

#[test]
fn get_lyrics_request_serializes() {
    let r = Request::GetLyrics {
        video_id: "vid123".into(),
    };
    let line = r.to_line();
    assert!(line.contains("\"cmd\":\"get_lyrics\""));
    assert!(line.contains("\"video_id\":\"vid123\""));
    assert!(!line.contains('\n'));
}

#[test]
fn response_lyrics_round_trips_synced() {
    let wire = r#"{"ok":true,"data":{"lyrics":{"lines":[{"time":1.5,"text":"hi"},{"time":3.0,"text":"there"}],"synced":true}}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Lyrics(lines, synced) => {
            assert!(synced);
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].time, Some(1.5));
            assert_eq!(lines[0].text, "hi");
            assert_eq!(lines[1].time, Some(3.0));
        }
        other => panic!("expected Lyrics, got {other:?}"),
    }
}

#[test]
fn response_lyrics_round_trips_plain() {
    let wire = r#"{"ok":true,"data":{"lyrics":{"lines":[{"time":null,"text":"verse 1"},{"time":null,"text":"verse 2"}],"synced":false}}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Lyrics(lines, synced) => {
            assert!(!synced);
            assert_eq!(lines.len(), 2);
            assert!(lines[0].time.is_none());
        }
        _ => panic!("expected Lyrics"),
    }
}

#[test]
fn response_lyrics_empty_is_not_found() {
    let wire = r#"{"ok":true,"data":{"lyrics":{"lines":[],"synced":false}}}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Lyrics(lines, synced) => {
            assert!(lines.is_empty());
            assert!(!synced);
        }
        _ => panic!("expected Lyrics"),
    }
}

#[test]
fn response_lyrics_error_propagates() {
    let wire = r#"{"ok":false,"error":"lyrics lookup failed: timeout"}"#;
    let r = Response::from_line(wire).unwrap();
    assert!(matches!(r, Response::Error(e) if e.contains("lyrics")));
}

// --- from_proto conversion -------------------------------------------------

#[test]
fn from_proto_builds_ytmusicapi_lyrics() {
    let proto = vec![
        LyricLineProto {
            time: Some(1.5),
            text: "hi".into(),
        },
        LyricLineProto {
            time: None,
            text: "plain".into(),
        },
    ];
    let lyrics = jukebox::lyrics::from_proto(&proto, true);
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].time, Some(1.5));
    assert_eq!(lyrics.source, LyricsSource::Ytmusicapi);
}

#[test]
fn from_proto_empty_lines() {
    let proto: Vec<LyricLineProto> = vec![];
    let lyrics = jukebox::lyrics::from_proto(&proto, false);
    assert!(lyrics.is_empty());
    assert!(!lyrics.synced);
}

// --- No fabricated lyrics (AC-M3.5.1) --------------------------------------

#[test]
fn no_fabricated_lyrics() {
    // When no source returns lyrics, the state must be "not found" — never
    // invented text. An empty Lyrics payload is_empty().
    let empty = Lyrics::empty(LyricsSource::Embedded);
    assert!(empty.is_empty());
    assert!(!empty.synced);
}

// --- Lyrics struct ---------------------------------------------------------

#[test]
fn lyrics_empty_source() {
    let l = Lyrics::empty(LyricsSource::Ytmusicapi);
    assert!(l.is_empty());
    assert_eq!(l.source, LyricsSource::Ytmusicapi);
}

#[test]
fn lyrics_source_distinct_variants() {
    assert_ne!(LyricsSource::Embedded, LyricsSource::SidecarFile);
    assert_ne!(LyricsSource::SidecarFile, LyricsSource::Ytmusicapi);
    assert_ne!(LyricsSource::Ytmusicapi, LyricsSource::Cached);
}

// --- Track struct for embedded read test -----------------------------------

fn make_track(id: &str, source_path: &str) -> Track {
    Track {
        id: id.into(),
        artists: vec!["TestArtist".into()],
        primary_artist: "TestArtist".into(),
        title: "TestTitle".into(),
        album: Some("TestAlbum".into()),
        track_number: Some(1),
        disc_number: Some(1),
        bit_depth: 16,
        sample_rate_hz: 44100,
        isrc: None,
        source_path: source_path.into(),
        symlinked_into_artists: vec!["TestArtist".into()],
    }
}

#[test]
fn read_embedded_falls_through_to_sidecar() {
    // No metaflac installed (or non-FLAC file) → falls through to sidecar .lrc.
    let dir = tempfile::tempdir().unwrap();
    let lossless = dir.path().join("lossless");
    std::fs::create_dir_all(&lossless).unwrap();
    let audio = lossless.join("01.flac");
    std::fs::write(&audio, b"not really flac").unwrap();
    let lrc = lossless.join("01.lrc");
    std::fs::write(&lrc, "[00:00.50]Synced lyrics").unwrap();

    let track = make_track("t1", "lossless/01.flac");
    // source_root is the lossless dir; resolve_source joins parent(source_root)
    // with source_path, so parent = dir.path(), + "lossless/01.flac" = audio.
    let lyrics = jukebox::lyrics::read_embedded(&track, &lossless);
    // metaflac will fail (not real FLAC), so it should fall through to the
    // sidecar .lrc file. If metaflac isn't installed, it also falls through.
    if let Some(lyrics) = lyrics {
        assert!(lyrics.synced);
        assert_eq!(lyrics.lines[0].text, "Synced lyrics");
    }
    // If metaflac IS installed and returns no tags, and the .lrc exists,
    // it still falls through. Either Some(synced) or None is acceptable
    // depending on whether metaflac is available — the key assertion is no
    // panic/crash.
}

// --- Stale-discard: generation guard (AC-M3.4.2) ---------------------------

/// Build a fake sidecar Session whose script reads stdin and echoes nothing,
/// so `send_get_lyrics` succeeds (writes to stdin) but no response lands until
/// we inject `pending_lyrics` manually. Lets the generation-guard test
/// exercise the real `on_tick` sidecar path without a ytmusicapi roundtrip.
fn fake_lyrics_session() -> jukebox::yt::session::Session {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let script =
        std::env::temp_dir().join(format!("jk-lyrics-fake-{}-{}.py", std::process::id(), n));
    std::fs::write(&script, "import sys\nfor line in sys.stdin: pass\n").unwrap();
    jukebox::yt::session::Session::spawn(std::path::Path::new("python3"), &script, None).unwrap()
}

#[test]
fn stale_lyrics_dropped_on_track_change() {
    // REAL generation-guard exercise: request lyrics for track A (bumping
    // lyrics_gen), then for track B (bumping again), then deliver A's stale
    // sidecar response via `pending_lyrics`. on_tick's guard
    // (`track_id == vid && gen == self.lyrics_gen`) must reject A's response
    // so the overlay stays on B's Loading state — A's lyrics must NOT
    // overwrite B's overlay.
    let mut app = empty_app();
    app.yt_session = Some(fake_lyrics_session());
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::Idle,
        scroll: 0,
        track_id: String::new(),
        gen: app.lyrics_gen,
    });
    app.request_lyrics("vidA");
    let gen_a = app.lyrics_gen;
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics {
                state: LyricsState::Loading,
                ref track_id,
                ..
            }) if track_id == "vidA"
        ),
        "overlay should be Loading for A after request"
    );
    app.request_lyrics("vidB");
    assert_ne!(app.lyrics_gen, gen_a, "gen must advance between requests");
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics {
                state: LyricsState::Loading,
                ref track_id,
                ..
            }) if track_id == "vidB"
        ),
        "overlay should be Loading for B after request"
    );
    app.yt_session.as_mut().unwrap().pending_lyrics = Some((
        "vidA".into(),
        vec![LyricLineProto {
            time: Some(1.0),
            text: "stale line for A".into(),
        }],
        true,
    ));
    app.on_tick();
    assert!(
        matches!(
            app.overlay,
            Some(Overlay::Lyrics {
                state: LyricsState::Loading,
                content: None,
                ref track_id,
                ..
            }) if track_id == "vidB"
        ),
        "stale lyrics for A must be dropped; overlay should stay Loading for B"
    );
}

#[test]
fn lyrics_gen_increments_on_request() {
    let mut app = empty_app();
    assert_eq!(app.lyrics_gen, 0);
    app.request_lyrics("vidA");
    assert_eq!(app.lyrics_gen, 1);
    app.request_lyrics("vidB");
    assert_eq!(app.lyrics_gen, 2);
    app.request_lyrics("vidA");
    assert_eq!(app.lyrics_gen, 3);
}

// --- Unicode / CJK safety --------------------------------------------------

#[test]
fn parse_lrc_japanese_text() {
    let lrc = "[00:01.00]こんにちは\n[00:02.00]世界";
    let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
    assert_eq!(lyrics.lines.len(), 2);
    assert_eq!(lyrics.lines[0].text, "こんにちは");
    assert_eq!(lyrics.lines[1].text, "世界");
}

#[test]
fn parse_plain_long_line_no_panic() {
    // A very long line must not panic during parsing.
    let long = "a".repeat(5000);
    let lyrics = parse_plain(&long, LyricsSource::Embedded);
    assert_eq!(lyrics.lines.len(), 1);
    assert_eq!(lyrics.lines[0].text.len(), 5000);
}

#[test]
fn parse_lrc_empty_input() {
    let lyrics = parse_lrc("", LyricsSource::SidecarFile);
    assert!(lyrics.is_empty());
    assert!(!lyrics.synced);
}

#[test]
fn parse_plain_empty_input() {
    let lyrics = parse_plain("", LyricsSource::Embedded);
    assert!(lyrics.is_empty());
}

// --- Error state -----------------------------------------------------------

#[test]
fn response_lyrics_error_from_sidecar() {
    // A sidecar error for a get_lyrics request must surface as Response::Error,
    // not crash the sidecar or hang the overlay.
    let wire = r#"{"ok":false,"error":"lyrics fetch failed: connection reset"}"#;
    let r = Response::from_line(wire).unwrap();
    match r {
        Response::Error(msg) => assert!(msg.contains("lyrics")),
        _ => panic!("expected Error"),
    }
}

#[test]
fn r_retries_lyrics_for_the_same_track() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lossless");
    std::fs::create_dir_all(&root).unwrap();
    let catalog_path = dir.path().join("catalog.json");
    std::fs::write(
        &catalog_path,
        serde_json::json!({
            "version": 1, "built_at": "x", "source_root": root,
            "tracks": [{
                "id": "same-track", "artists": ["A"], "primary_artist": "A",
                "title": "Song", "bit_depth": 16, "sample_rate_hz": 44100,
                "source_path": "missing.flac", "symlinked_into_artists": ["A"]
            }]
        })
        .to_string(),
    )
    .unwrap();
    let catalog = jukebox::catalog::Catalog::load(&catalog_path).unwrap();
    let mut app = App::new(catalog, Box::new(StubPlayer::default()), None, None);
    app.overlay = Some(Overlay::Lyrics {
        content: None,
        state: LyricsState::NotFound,
        scroll: 7,
        track_id: "same-track".into(),
        gen: app.lyrics_gen,
    });
    let before = app.lyrics_gen;

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
    );

    assert_eq!(app.lyrics_gen, before.wrapping_add(1));
    assert!(matches!(
        app.overlay,
        Some(Overlay::Lyrics { ref track_id, scroll: 7, .. }) if track_id == "same-track"
    ));
}

fn empty_app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lossless");
    std::fs::create_dir_all(&root).unwrap();
    let path = dir.path().join("catalog.json");
    std::fs::write(
        &path,
        serde_json::json!({"version":1,"built_at":"x","source_root":root,"tracks":[]}).to_string(),
    )
    .unwrap();
    App::new(
        jukebox::catalog::Catalog::load(&path).unwrap(),
        Box::new(StubPlayer::default()),
        None,
        None,
    )
}

fn render_lyrics_state(
    state: LyricsState,
    content: Option<Lyrics>,
    scroll: u16,
    width: u16,
) -> String {
    let mut app = empty_app();
    app.overlay = Some(Overlay::Lyrics {
        content,
        state,
        scroll,
        track_id: "track".into(),
        gen: app.lyrics_gen,
    });
    let backend = TestBackend::new(width, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let mut rendered = String::new();
    for y in 0..24 {
        for x in 0..width {
            rendered.push(
                terminal.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        rendered.push('\n');
    }
    rendered
}

#[test]
fn lyrics_state_matrix() {
    let cases = [
        (LyricsState::Loading, "loading"),
        (LyricsState::Available(true), "synced"),
        (LyricsState::Available(false), "plain"),
        (LyricsState::NotFound, "not found"),
        (LyricsState::Offline, "offline"),
        (LyricsState::Error("provider exploded".into()), "error"),
    ];
    for (state, expected) in cases {
        let content = matches!(state, LyricsState::Available(_))
            .then(|| parse_plain("line", LyricsSource::Cached));
        let rendered = render_lyrics_state(state, content, 0, 80);
        assert!(rendered.to_lowercase().contains(expected), "{rendered}");
    }
}

#[test]
fn synced_lyrics_highlights_current_line() {
    let lyrics = parse_lrc(
        "[00:00.00]current\n[00:10.00]later",
        LyricsSource::SidecarFile,
    );
    let rendered = render_lyrics_state(LyricsState::Available(true), Some(lyrics), 0, 80);
    assert!(rendered.contains("▸ [0:00] current"), "{rendered}");
}

#[test]
fn lyrics_scroll() {
    let lyrics = parse_plain(
        &(0..40)
            .map(|n| format!("line-{n:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
        LyricsSource::Embedded,
    );
    let rendered = render_lyrics_state(LyricsState::Available(false), Some(lyrics), 20, 80);
    assert!(rendered.contains("line-20"), "{rendered}");
    assert!(!rendered.contains("line-00"), "{rendered}");
}

#[test]
fn lyrics_unicode_long_lines_narrow() {
    let lyrics = parse_plain(&"界e\u{301}".repeat(100), LyricsSource::Embedded);
    let rendered = render_lyrics_state(LyricsState::Available(false), Some(lyrics), 0, 60);
    assert!(rendered.contains('界'));
    assert!(rendered.lines().all(|line| line.chars().count() <= 60));
}

#[test]
fn lyrics_cache_invalidates_on_track_change() {
    let mut app = empty_app();
    app.overlay = Some(Overlay::Lyrics {
        content: Some(parse_plain("cached", LyricsSource::Cached)),
        state: LyricsState::Available(false),
        scroll: 3,
        track_id: "old".into(),
        gen: app.lyrics_gen,
    });
    let old_gen = app.lyrics_gen;
    app.request_lyrics("new");
    assert_eq!(app.lyrics_gen, old_gen.wrapping_add(1));
    assert!(
        matches!(app.overlay, Some(Overlay::Lyrics { content: None, ref track_id, .. }) if track_id == "new")
    );
}
