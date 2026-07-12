//! Lyrics provider pipeline — parse, read, and stage lyrics for the TUI.
//!
//! Lyrics come from three sources, tried in order (local first so a cold
//! sidecar / offline state never blocks a track that has a sidecar `.lrc`):
//!
//! 1. **Embedded** — FLAC `LYRICS` / `UNSYNCEDLYRICS` / `SYNCEDLYRICS` vorbis
//!    comments, read via a `metaflac` subprocess (the project already relies on
//!    `metaflac` at index time; we reuse it read-only here).
//! 2. **Sidecar file** — a `.lrc` (synced) or `.txt` (plain) file next to the
//!    audio file with the same basename.
//! 3. **ytmusicapi** — the sidecar's `get_lyrics` command (for YouTube tracks
//!    and local tracks without a sidecar/embedded source). Fire-and-forget via
//!    [`crate::yt::session::Session::send_get_lyrics`].
//!
//! ## LRC parsing
//!
//! LRC lines look like `[mm:ss.xx]text` (centiseconds) or `[mm:ss.xxx]text`
//! (milliseconds). Multi-timestamp lines (`[00:01.00][00:15.00]Repeated line`)
//! expand to one [`LyricLine`] per timestamp. Metadata tags (`[ti:]`, `[ar:]`,
//! `[al:]`, `[by:]`, `[offset:]`) are skipped — they aren't lyric lines.
//!
//! `time` is stored in **seconds** (f64) so the renderer can compare directly
//! against `player.position()` (which returns seconds). The sidecar converts
//! ytmusicapi's millisecond timestamps to seconds before sending.
//!
//! ## Non-blocking contract
//!
//! Local reads (`read_embedded` / `read_sidecar_file`) are fast filesystem /
//! one-shot-subprocess calls acceptable at a play boundary. The ytmusicapi path
//! is fire-and-forget (see [`crate::yt::session::Session`]) — it never blocks
//! the TUI; results land in `pending_lyrics` and are drained by `App::on_tick`
//! under a generation guard so stale lyrics can't overwrite a newer track.

use std::path::Path;

use crate::catalog::Track;

pub mod cache;

/// One line of lyrics. `time` is the LRC timestamp in seconds (`None` for
/// plain / unsynchronized lyrics). The renderer highlights the line whose
/// `time` is the greatest `<= player.position()`.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub time: Option<f64>,
    pub text: String,
}

/// Where a [`Lyrics`] payload came from. Drives the overlay's source label
/// ("lyrics: embedded" vs "lyrics: youtube") and the cache-invalidation rule
/// (a `Ytmusicapi` result is re-fetched on track change; an `Embedded` /
/// `SidecarFile` result is re-read from disk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LyricsSource {
    /// Read from a FLAC `LYRICS` / `UNSYNCEDLYRICS` / `SYNCEDLYRICS` tag.
    Embedded,
    /// Read from a `.lrc` / `.txt` file next to the audio file.
    SidecarFile,
    /// Fetched from ytmusicapi via the sidecar (`get_lyrics`).
    Ytmusicapi,
    /// Re-served from an in-memory cache (same source as the original fetch).
    Cached,
}

/// A parsed lyrics payload. `synced` is true when at least one line carries a
/// timestamp (LRC); plain-text lyrics have `synced = false` and every line's
/// `time = None`.
#[derive(Clone, Debug)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    pub source: LyricsSource,
}

impl Lyrics {
    /// An empty payload for the "not found" / "loading" states. The source is
    /// `Cached` (a placeholder doesn't claim a real origin).
    pub fn empty(source: LyricsSource) -> Self {
        Lyrics {
            lines: Vec::new(),
            synced: false,
            source,
        }
    }

    /// True when there are no lines (the "not found" state — never fabricated;
    /// see AC-M3.5.1).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Parse LRC-formatted content into a [`Lyrics`]. Recognizes `[mm:ss.xx]`
/// (centiseconds) and `[mm:ss.xxx]` (milliseconds) timestamps, multi-timestamp
/// lines, and skips LRC metadata tags (`[ti:]`, `[ar:]`, `[al:]`, `[by:]`,
/// `[offset:]`). Lines without a timestamp become plain lines (`time = None`).
/// `synced` is true iff at least one line carried a timestamp.
pub fn parse_lrc(content: &str, source: LyricsSource) -> Lyrics {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut any_synced = false;
    for raw in content.lines() {
        // Collect all leading [mm:ss.xx] timestamps on this line. A line may
        // carry several (`[00:01.00][00:15.00]refrain`) → one LyricLine each.
        let (times, text) = strip_leading_timestamps(raw);
        if times.is_empty() {
            // Metadata tags ([ti:], [ar:], …) and blank lines have no
            // timestamps; metadata is skipped, blank lines kept as spacers
            // only when they carry real text (empty text → skip to avoid a
            // wall of blank LyricLines).
            if text.trim().is_empty() {
                continue;
            }
            // Skip pure metadata tags (text starts with a known tag key).
            if is_lrc_metadata(&text) {
                continue;
            }
            lines.push(LyricLine {
                time: None,
                text: text.trim().to_string(),
            });
        } else {
            any_synced = true;
            let trimmed = text.trim().to_string();
            for t in times {
                lines.push(LyricLine {
                    time: Some(t),
                    text: trimmed.clone(),
                });
            }
        }
    }
    Lyrics {
        lines,
        synced: any_synced,
        source,
    }
}

/// Parse plain-text lyrics (one [`LyricLine`] per line, `time = None`,
/// `synced = false`). Blank lines are kept as spacers so the verse structure
/// is preserved in the overlay.
pub fn parse_plain(content: &str, source: LyricsSource) -> Lyrics {
    let lines = content
        .lines()
        .map(|l| LyricLine {
            time: None,
            text: l.to_string(),
        })
        .collect();
    Lyrics {
        lines,
        synced: false,
        source,
    }
}

/// Read lyrics from the track's source file: first try the embedded FLAC tags
/// (`LYRICS` / `UNSYNCEDLYRICS` / `SYNCEDLYRICS`) via `metaflac`, then fall back
/// to a sidecar `.lrc` / `.txt` file. Returns `None` when neither yields
/// lyrics (so the caller can fire the ytmusicapi path).
///
/// The audio path is resolved via [`Track::resolve_source`] with the catalog's
/// `source_root`. `metaflac` is invoked read-only (`--show-tag`); a missing
/// `metaflac` binary or non-FLAC file falls through to the sidecar lookup
/// (no panic, no surfaced error — lyrics are best-effort, never blocking).
pub fn read_embedded(track: &Track, source_root: &Path) -> Option<Lyrics> {
    let audio = track.resolve_source(source_root);
    if let Some(lyrics) = read_flac_tags(&audio) {
        return Some(lyrics);
    }
    read_sidecar_file(&audio)
}

/// Read a `.lrc` or `.txt` sidecar file next to `audio_path` (same basename).
/// `.lrc` is parsed as LRC (timestamps); `.txt` as plain text. Returns `None`
/// when no sidecar file exists. `.lrc` is preferred (it may carry sync).
pub fn read_sidecar_file(audio_path: &Path) -> Option<Lyrics> {
    // Try .lrc first (may be synced), then .txt (plain).
    for ext in ["lrc", "txt"] {
        let sidecar = audio_path.with_extension(ext);
        if let Ok(content) = std::fs::read_to_string(&sidecar) {
            if ext == "lrc" {
                return Some(parse_lrc(&content, LyricsSource::SidecarFile));
            }
            // A .txt might still be LRC-formatted; detect timestamps.
            if has_lrc_timestamps(&content) {
                return Some(parse_lrc(&content, LyricsSource::SidecarFile));
            }
            return Some(parse_plain(&content, LyricsSource::SidecarFile));
        }
    }
    None
}

/// Read a FLAC file's `LYRICS` / `UNSYNCEDLYRICS` / `SYNCEDLYRICS` vorbis
/// comments via a read-only `metaflac --show-tag=…` subprocess. The first tag
/// that yields non-empty content wins. `SYNCEDLYRICS` is parsed as LRC; the
/// others are parsed as LRC only if they contain timestamps, else plain.
/// Returns `None` if `metaflac` is missing, the file isn't FLAC, or the tags
/// are all empty.
fn read_flac_tags(audio: &Path) -> Option<Lyrics> {
    let output = std::process::Command::new("metaflac")
        .args([
            "--show-tag=LYRICS",
            "--show-tag=UNSYNCEDLYRICS",
            "--show-tag=SYNCEDLYRICS",
        ])
        .arg(audio)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // metaflac prints `<TAG>=<value>` per match, in the order of the --show-tag
    // args, separated by newlines. Group by tag: collect each tag's lines until
    // the next tag's `=` prefix or EOF.
    for tag in ["SYNCEDLYRICS", "LYRICS", "UNSYNCEDLYRICS"] {
        if let Some(value) = extract_tag(&text, tag) {
            if value.trim().is_empty() {
                continue;
            }
            if tag == "SYNCEDLYRICS" || has_lrc_timestamps(&value) {
                return Some(parse_lrc(&value, LyricsSource::Embedded));
            }
            return Some(parse_plain(&value, LyricsSource::Embedded));
        }
    }
    None
}

/// Extract the concatenated value of `tag` from `metaflac` output. metaflac
/// prints `<TAG>=<value>` lines; a multi-line tag value is output as multiple
/// `<TAG>=…` lines (one per continuation), so we join the values with `\n`.
fn extract_tag(metaflac_out: &str, tag: &str) -> Option<String> {
    let prefix = format!("{tag}=");
    let mut values: Vec<&str> = Vec::new();
    for line in metaflac_out.lines() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            values.push(rest);
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values.join("\n"))
    }
}

/// True when `content` contains at least one `[mm:ss.xx]`-style timestamp,
/// i.e. it should be parsed as LRC rather than plain text.
fn has_lrc_timestamps(content: &str) -> bool {
    content.lines().any(|l| {
        let (ts, _) = strip_leading_timestamps(l);
        !ts.is_empty()
    })
}

/// True when `text` (the part after any timestamps) is an LRC metadata tag
/// (`[ti:…]`, `[ar:…]`, `[al:…]`, `[by:…]`, `[offset:…]`). These are skipped —
/// they aren't lyric lines.
fn is_lrc_metadata(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("[ti:")
        || t.starts_with("[ar:")
        || t.starts_with("[al:")
        || t.starts_with("[by:")
        || t.starts_with("[offset:")
        || t.starts_with("[length:")
        || t.starts_with("[re:")
        || t.starts_with("[ve:")
}

/// Strip all leading `[mm:ss.xx]` / `[mm:ss.xxx]` timestamps from `line`,
/// returning the parsed times (in seconds) and the remaining text. Stops at
/// the first non-timestamp `[…]` (e.g. a metadata tag) or any non-`[` char.
fn strip_leading_timestamps(line: &str) -> (Vec<f64>, String) {
    let mut times = Vec::new();
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('[') {
            break;
        }
        // Find the closing bracket.
        let Some(close) = trimmed.find(']') else {
            break;
        };
        let inside = &trimmed[1..close];
        if let Some(t) = parse_timestamp(inside) {
            times.push(t);
            rest = &trimmed[close + 1..];
        } else {
            // Not a timestamp (e.g. "[ti:...]") — stop; the caller treats the
            // remainder as text (and is_lrc_metadata will skip tags).
            break;
        }
    }
    (times, rest.to_string())
}

/// Parse a single LRC timestamp `mm:ss.xx` or `mm:ss.xxx` (or `mm:ss`) into
/// seconds. `mm` may exceed 59 for long tracks (LRC allows `[75:00.00]`).
/// Returns `None` for non-timestamp content.
fn parse_timestamp(s: &str) -> Option<f64> {
    let (mins, rest) = s.split_once(':')?;
    let mm: f64 = mins.parse().ok()?;
    // rest is `ss.xx` or `ss.xxx` or `ss`.
    let (secs, frac) = match rest.split_once('.') {
        Some((ss, frac)) => {
            let ss: f64 = ss.parse().ok()?;
            // Fractional part: centiseconds (2) or milliseconds (3). Pad /
            // truncate to 3 digits so `50` → 0.500s and `5` → 0.500s (LRC spec:
            // the fractional field is centiseconds, but tolerate ms).
            let frac_norm = normalize_frac(frac);
            (ss, frac_norm)
        }
        None => {
            let ss: f64 = rest.parse().ok()?;
            (ss, 0.0)
        }
    };
    Some(mm * 60.0 + secs + frac)
}

/// Normalize a fractional seconds field to a seconds value, tolerating 1–3
/// digits (centiseconds vs milliseconds). `50` → 0.50, `005` → 0.005,
/// `500` → 0.5. Non-digit content → 0.0.
fn normalize_frac(frac: &str) -> f64 {
    let digits: String = frac.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0.0;
    }
    // Interpret as a fixed 3-digit (millisecond) field: pad/truncate to 3.
    let padded = match digits.len() {
        0 => return 0.0,
        1..=3 => format!("{:0<3}", digits),
        _ => digits[..3].to_string(),
    };
    let ms: f64 = padded.parse().unwrap_or(0.0);
    ms / 1000.0
}

/// Build a [`Lyrics`] from the sidecar's wire payload (`LyricLineProto` list +
/// `synced` flag). The sidecar converts ytmusicapi's millisecond timestamps to
/// seconds before sending, so `time` is already in seconds here. Used by
/// `App::on_tick` when draining `pending_lyrics`.
pub fn from_proto(lines: &[crate::yt::proto::LyricLineProto], synced: bool) -> Lyrics {
    let parsed: Vec<LyricLine> = lines
        .iter()
        .map(|l| LyricLine {
            time: l.time,
            text: l.text.clone(),
        })
        .collect();
    Lyrics {
        lines: parsed,
        synced,
        source: LyricsSource::Ytmusicapi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lrc_basic() {
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
        let lrc = "[00:01.00][00:15.00]Refrain";
        let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
        assert!(lyrics.synced);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].time, Some(1.0));
        assert_eq!(lyrics.lines[1].time, Some(15.0));
        assert_eq!(lyrics.lines[0].text, "Refrain");
    }

    #[test]
    fn parse_lrc_skips_metadata_tags() {
        let lrc = "[ti:Song]\n[ar:Artist]\n[al:Album]\n[00:12.50]Real line";
        let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].text, "Real line");
    }

    #[test]
    fn parse_lrc_milliseconds() {
        // Three-digit fractional (milliseconds).
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
    fn parse_plain_keeps_blank_spacers() {
        let text = "Verse 1\n\nVerse 2";
        let lyrics = parse_plain(text, LyricsSource::Embedded);
        assert!(!lyrics.synced);
        assert_eq!(lyrics.lines.len(), 3);
        assert_eq!(lyrics.lines[1].text, "");
    }

    #[test]
    fn parse_lrc_minutes_over_59() {
        let lrc = "[75:00.00]Long track";
        let lyrics = parse_lrc(lrc, LyricsSource::SidecarFile);
        assert_eq!(lyrics.lines[0].time, Some(4500.0));
    }

    #[test]
    fn empty_lyrics_is_empty() {
        let l = Lyrics::empty(LyricsSource::Embedded);
        assert!(l.is_empty());
        assert!(!l.synced);
    }

    #[test]
    fn from_proto_builds_lyrics() {
        let proto = vec![
            crate::yt::proto::LyricLineProto {
                time: Some(1.5),
                text: "hi".into(),
            },
            crate::yt::proto::LyricLineProto {
                time: None,
                text: "plain".into(),
            },
        ];
        let lyrics = from_proto(&proto, true);
        assert!(lyrics.synced);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].time, Some(1.5));
        assert_eq!(lyrics.source, LyricsSource::Ytmusicapi);
    }

    #[test]
    fn read_sidecar_file_returns_none_when_absent() {
        let p = std::env::temp_dir().join("no-such-audio-xyz.flac");
        assert!(read_sidecar_file(&p).is_none());
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
        assert_eq!(lyrics.source, LyricsSource::SidecarFile);
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
}
