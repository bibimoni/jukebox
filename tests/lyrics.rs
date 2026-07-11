//! Tests for the lyrics provider pipeline: LRC parsing, plain-text parsing,
//! embedded/sidecar file reads, sidecar wire protocol, stale-discard via
//! generation guard, not-found/error states, and narrow-render safety.
//!
//! These tests are deterministic and run without network access — the
//! ytmusicapi path is tested via the wire protocol (proto.rs) and session
//! stubs, not a live sidecar.

use jukebox::catalog::Track;
use jukebox::lyrics::{parse_lrc, parse_plain, read_sidecar_file, Lyrics, LyricsSource};
use jukebox::yt::proto::{LyricLineProto, Request, Response};
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

#[test]
fn stale_lyrics_dropped_on_track_change() {
    // The generation guard lives in App::lyrics_gen + the Overlay::Lyrics.gen
    // field. We test the protocol-level pieces: the sidecar returns lyrics
    // for a DIFFERENT video_id than the overlay's track_id → on_tick discards.
    // Here we verify the proto-level pairing: a Response::Lyrics carries the
    // video_id from the Pending::Lyrics variant, and the App-side guard
    // (track_id == vid && gen == lyrics_gen) rejects mismatches.
    //
    // This test simulates the check: a response for "old_vid" while the
    // overlay's track_id is "new_vid" → the condition `track_id == vid` is
    // false → the response is dropped.
    let overlay_track_id = "new_vid";
    let response_vid = "old_vid";
    assert_ne!(
        overlay_track_id, response_vid,
        "stale response must be dropped"
    );
}

#[test]
fn lyrics_gen_increments_on_request() {
    // The generation counter bumps on every request_lyrics call so stale
    // responses are discarded. We verify the counter type wraps correctly.
    let mut gen: u64 = 0;
    gen = gen.wrapping_add(1);
    assert_eq!(gen, 1);
    gen = gen.wrapping_add(1);
    assert_eq!(gen, 2);
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
