use jukebox::source::{RemoteTrack, StreamFormat, TrackSource};

#[test]
fn track_source_id_returns_opaque_id() {
    let l = TrackSource::Local {
        track_id: "abc".into(),
    };
    let r = TrackSource::Remote {
        video_id: "dQw4w9WgXcQ".into(),
    };
    assert_eq!(l.id(), "abc");
    assert_eq!(r.id(), "dQw4w9WgXcQ");
    assert!(!l.is_remote());
    assert!(r.is_remote());
}

#[test]
fn remote_track_defaults_fmt_none() {
    let t = RemoteTrack {
        video_id: "v".into(),
        title: "S".into(),
        artist: "A".into(),
        album: None,
        dur: None,
        fmt: None,
        isrc: None,
    };
    assert!(t.fmt.is_none());
    assert!(t.isrc.is_none());
}

#[test]
fn stream_format_yt_label() {
    let opus = StreamFormat {
        codec: "Opus".into(),
        abr: 160,
        sample_rate: 48000,
        container: "webm".into(),
        premium: false,
    };
    assert_eq!(opus.yt_label(), "Opus 160k · YT");
    let aac = StreamFormat {
        codec: "AAC".into(),
        abr: 256,
        sample_rate: 48000,
        container: "m4a".into(),
        premium: true,
    };
    assert_eq!(aac.yt_label(), "AAC 256k · YT Premium");
}

use jukebox::catalog::Catalog;
use jukebox::source::match_local::match_local;

fn cat(tracks: &[(&str, &str, &str, Option<&str>)]) -> Catalog {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    // (id, artist, title, isrc)
    let t: Vec<_> = tracks
        .iter()
        .map(|(id, a, t, isrc)| {
            serde_json::json!({
                "id": id, "artists": [a], "primary_artist": a, "title": t,
                "bit_depth": 16, "sample_rate_hz": 44100, "source_path": "x",
                "symlinked_into_artists": [a], "isrc": isrc
            })
        })
        .collect();
    let s = serde_json::json!({
        "version": 1, "built_at": "x", "source_root": "/tmp", "tracks": t
    })
    .to_string();
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("cat-{}-{}.json", std::process::id(), n));
    std::fs::write(&p, &s).unwrap();
    Catalog::load(&p).unwrap()
}

fn rt(title: &str, artist: &str, isrc: Option<&str>) -> RemoteTrack {
    RemoteTrack {
        video_id: "v1".into(),
        title: title.into(),
        artist: artist.into(),
        album: None,
        dur: None,
        fmt: None,
        isrc: isrc.map(String::from),
    }
}

#[test]
fn isrc_exact_match_wins() {
    let c = cat(&[("t1", "Adele", "Hello", Some("GBBKS1500123"))]);
    assert_eq!(
        match_local(&rt("Hello", "Adele", Some("gbbks1500123")), &c),
        Some("t1".into())
    );
}

#[test]
fn isrc_case_insensitive() {
    let c = cat(&[("t1", "Adele", "Hello", Some("GBBKS1500123"))]);
    assert_eq!(
        match_local(&rt("Hello", "ADELE", Some("GBBKS1500123")), &c),
        Some("t1".into())
    );
}

#[test]
fn isrc_absent_falls_back_to_title() {
    let c = cat(&[("t1", "Adele", "Hello", None)]);
    assert_eq!(
        match_local(&rt("Hello", "Adele", None), &c),
        Some("t1".into())
    );
}

#[test]
fn normalized_cjk_title_match() {
    // catalog stores katakana title; remote (YT) gives romaji "burubado"
    let c = cat(&[("t1", "Ado", "ブルーバード", None)]);
    assert_eq!(
        match_local(&rt("burubado", "ado", None), &c),
        Some("t1".into())
    );
}

#[test]
fn feat_token_stripped() {
    let c = cat(&[("t1", "Aimer", "Dawn", None)]);
    assert_eq!(
        match_local(&rt("Dawn feat. Someone", "Aimer", None), &c),
        Some("t1".into())
    );
}

#[test]
fn borderline_rejected() {
    // "Helloing" vs "Hello" — title ratio below gate; must NOT promote to local.
    let c = cat(&[("t1", "Adele", "Helloing", None)]);
    assert_eq!(match_local(&rt("Hello", "Adele", None), &c), None);
}

#[test]
fn no_match_returns_none() {
    let c = cat(&[("t1", "Adele", "Hello", None)]);
    assert_eq!(
        match_local(&rt("Completely Different", "Nobody", None), &c),
        None
    );
}

#[test]
fn artist_mismatch_rejects_even_if_title_close() {
    let c = cat(&[("t1", "Adele", "Hello", None)]);
    assert_eq!(match_local(&rt("Hello", "Beyonce", None), &c), None);
}
