use jukebox::source::{RemoteTrack, StreamFormat, TrackSource};

#[test]
fn track_source_id_returns_opaque_id() {
    let l = TrackSource::Local { track_id: "abc".into() };
    let r = TrackSource::Remote { video_id: "dQw4w9WgXcQ".into() };
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
