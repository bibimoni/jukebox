//! Track metadata component for the Now Playing Deck.
//!
//! Renders the artist / title / album / quality rows with the spec's
//! visual hierarchy:
//!
//! 1. Artist (dim, BOLD) — the "category" header
//! 2. Track title (accent, BOLD) — the visual hero
//! 3. Album (text) — secondary, hidden when `None` or empty
//! 4. Quality badges (dim/muted) — hidden entirely when unknown
//!
//! ## Transparency
//!
//! No `bg` is set on any span — ordinary cells retain the terminal-default
//! background so the user's wallpaper remains visible. Hierarchy comes
//! from foreground color + modifiers + spacing.
//!
//! ## Resume hint
//!
//! When `now_playing.is_none()` and `resume_hint.is_some()`, the metadata
//! component renders the **track title** (resolved from the hint if
//! possible) — NOT the hint text with a `resume:` prefix. The resume
//! action is a separate state row owned by `state.rs` (Stage 4). This
//! fixes spec problem #3 ("resume: is treated as part of the title
//! instead of a playback action").

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::progress::khz;
use crate::tui::view::now_playing_deck::spinner::is_buffering;
use crate::tui::view::now_playing_deck::state::resume_parts;
use crate::tui::view::theme::{clip_to_width, disp_width, ellipsis, sep_dot, Theme};

#[derive(Clone, Debug)]
pub struct DisplayMetadata {
    pub primary_title: String,
    pub artist: String,
    pub secondary_title: Option<String>,
    pub album: Option<String>,
    pub media_type: Option<String>,
    pub is_remote: bool,
    bit_depth: u32,
    sample_rate_hz: u32,
    format: Option<crate::source::StreamFormat>,
}

/// Resolve one display model for playing and resumable tracks. Compact and
/// minimal layouts use this too, so breakpoint changes cannot restore raw
/// source titles or lose structured artist metadata.
pub fn display_metadata(app: &App) -> Option<DisplayMetadata> {
    let (artist, raw_title, album, is_remote, bit_depth, sample_rate_hz, format) =
        if let Some(view) = app.now_playing_view() {
            (
                view.artist,
                view.title,
                view.album,
                view.source.is_remote(),
                view.bit_depth,
                view.sample_rate_hz,
                view.fmt,
            )
        } else {
            let (artist, title, album, is_remote) = resume_metadata(app)?;
            (artist, title, album, is_remote, 0, 0, None)
        };
    let normalized = normalize_source_title(&raw_title, &artist);
    Some(DisplayMetadata {
        primary_title: normalized.primary,
        artist,
        secondary_title: normalized.secondary,
        album,
        media_type: normalized.media_type,
        is_remote,
        bit_depth,
        sample_rate_hz,
        format,
    })
}

/// Render up to 4 metadata rows into `area`. Each row is `Length(1)`.
/// The caller passes a `Rect` of height >= 4; unused rows are left
/// blank (terminal-default). The rows are:
///
/// - Row 0: artist (dim, BOLD)
/// - Row 1: title (accent, BOLD) — wrapped from row 1 if too long
/// - Row 2: album (text) or title-continuation
/// - Row 3: quality badges (dim) — hidden when unknown
///
/// The component hides `album` when `None`/empty and hides the quality
/// row when no metadata is known. It never displays `--bit / -- kHz`.
pub fn render_metadata(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let width = area.width as usize;

    let artist_style = Style::default()
        .fg(theme.text_muted)
        .add_modifier(Modifier::BOLD);
    let title_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let album_style = Style::default().fg(theme.text);
    let quality_style = Style::default().fg(theme.dim);
    let dim_style = Style::default().fg(theme.dim);

    if is_buffering(app) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("Finding the stream{}", ellipsis()),
                title_style,
            ))),
            Rect::new(area.x, area.y, area.width, 1),
        );
        return;
    }
    let Some(metadata) = display_metadata(app) else {
        render_placeholder_rows(f, area, &dim_style);
        return;
    };
    let title = &metadata.primary_title;
    let artist = &metadata.artist;
    let album = &metadata.album;

    let mut row_idx = 0u16;

    // Row 0+: title (accent, BOLD). Wrap across 2 lines when too long.
    if !title.is_empty() && row_idx < area.height {
        let title_w = disp_width(title);
        if title_w <= width {
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(title.clone(), title_style)])),
                Rect::new(area.x, area.y + row_idx, area.width, 1),
            );
            row_idx += 1;
        } else {
            // First line: truncated with ellipsis.
            let first = crate::tui::view::player_bar::truncate_title(title, width);
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(first, title_style)])),
                Rect::new(area.x, area.y + row_idx, area.width, 1),
            );
            row_idx += 1;
            // Second line: the remainder, hard-clipped to width.
            if row_idx < area.height {
                let first_len = width.saturating_sub(1); // the ellipsis
                let remainder = clip_after_width(title, first_len);
                let second = clip_to_width(&remainder, width);
                if !second.is_empty() {
                    f.render_widget(
                        Paragraph::new(Line::from(vec![Span::styled(second, title_style)])),
                        Rect::new(area.x, area.y + row_idx, area.width, 1),
                    );
                    row_idx += 1;
                }
            }
        }
    }

    // Next row: artist (dim, BOLD). Skip if empty.
    if !artist.is_empty() && row_idx < area.height {
        let clipped = crate::tui::view::player_bar::truncate_title(artist, width);
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(clipped, artist_style)])),
            Rect::new(area.x, area.y + row_idx, area.width, 1),
        );
        row_idx += 1;
    }

    // Structured translated/secondary title, when a conservative source-title
    // normalization recognized one. It never displaces the primary title.
    if let Some(secondary) = &metadata.secondary_title {
        if row_idx < area.height {
            let clipped = crate::tui::view::player_bar::truncate_title(secondary, width);
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(clipped, album_style)])),
                Rect::new(area.x, area.y + row_idx, area.width, 1),
            );
            row_idx += 1;
        }
    }

    // Row: album (text). Skip if None or empty.
    if let Some(album_str) = &album {
        if !album_str.is_empty() && row_idx < area.height {
            let clipped = crate::tui::view::player_bar::truncate_title(album_str, width);
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(clipped, album_style)])),
                Rect::new(area.x, area.y + row_idx, area.width, 1),
            );
            row_idx += 1;
        }
    }

    // Row: quality badges. Hide entirely when unknown.
    let quality_line = build_quality_line(&metadata, &quality_style);
    if let Some(line) = quality_line {
        if row_idx < area.height {
            f.render_widget(
                Paragraph::new(line),
                Rect::new(area.x, area.y + row_idx, area.width, 1),
            );
        }
    }
}

struct NormalizedTitle {
    primary: String,
    secondary: Option<String>,
    media_type: Option<String>,
}

/// Normalize only patterns backed by structured artist metadata and a known
/// media marker. Anything uncertain remains the original title.
fn normalize_source_title(raw: &str, artist: &str) -> NormalizedTitle {
    let mut title = raw.trim();
    if !artist.is_empty() {
        for separator in [" - ", " – ", " — "] {
            let prefix = format!("{artist}{separator}");
            if let Some(rest) = title.strip_prefix(&prefix) {
                title = rest.trim();
                break;
            }
        }
    }

    const MEDIA_TYPES: [(&str, &str); 9] = [
        ("Music Video", "VIDEO"),
        ("Official Music Video", "VIDEO"),
        ("Official Video", "VIDEO"),
        ("Lyric Video", "VIDEO"),
        ("Lyrics Video", "VIDEO"),
        ("MV", "VIDEO"),
        ("Official Audio", "AUDIO"),
        ("Audio", "AUDIO"),
        ("Live", "LIVE"),
    ];
    for (marker, display) in MEDIA_TYPES {
        for separator in [" - ", " – ", " — "] {
            let delimiter = format!(" ({marker}){separator}");
            if let Some((primary, secondary)) = title.split_once(&delimiter) {
                if !primary.trim().is_empty() && !secondary.trim().is_empty() {
                    return NormalizedTitle {
                        primary: primary.trim().to_string(),
                        secondary: Some(secondary.trim().to_string()),
                        media_type: Some(display.to_string()),
                    };
                }
            }
        }
    }

    NormalizedTitle {
        primary: title.to_string(),
        secondary: None,
        media_type: None,
    }
}

fn resume_metadata(app: &App) -> Option<(String, String, Option<String>, bool)> {
    if let Some(id) = app.last_played_track_id.as_deref() {
        if let Some(track) = app.track_by_id_fast(id) {
            return Some((
                track.primary_artist.clone(),
                track.title.clone(),
                track.album.clone(),
                false,
            ));
        }
        if let Some(track) = app
            .yt_session
            .as_ref()
            .and_then(|session| session.track_for(id))
        {
            return Some((
                track.artist.clone(),
                track.title.clone(),
                track.album.clone(),
                true,
            ));
        }
        if let Some(track) = app
            .last_played_context_tracks
            .iter()
            .find(|track| track.video_id == id)
        {
            return Some((
                track.artist.clone(),
                track.title.clone(),
                track.album.clone(),
                true,
            ));
        }
    }

    app.resume_hint.as_deref().map(|hint| {
        let (title, _) = resume_parts(hint);
        (
            String::new(),
            title,
            None,
            app.source_mode != crate::mode::SourceMode::Local,
        )
    })
}

/// Render placeholder rows when nothing is playing. The first row is a
/// dim `— nothing playing —` line; subsequent rows are left blank.
fn render_placeholder_rows(f: &mut Frame, area: Rect, dim_style: &Style) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let dash = crate::tui::view::theme::em_dash();
    let placeholder = format!("{dash} nothing playing {dash}");
    let clipped = clip_to_width(&placeholder, width);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(clipped, *dim_style)])),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Build the quality badges line. Returns `None` when no metadata is
/// known (the caller should omit the row entirely — never display
/// `--bit / -- kHz`).
///
/// Known local: `LOCAL · 24-bit · 96 kHz` (codec omitted — not in
/// `NowPlayingView`; the catalog's `Track` has it via the file extension
/// but we don't surface it here to avoid a fragile inference).
/// Known remote: `YOUTUBE · OPUS 160k · 48 kHz` (via `fmt.yt_label()`).
/// Source only: `YOUTUBE` or `LOCAL`.
/// Nothing known: `None` (row omitted).
fn build_quality_line(metadata: &DisplayMetadata, quality_style: &Style) -> Option<Line<'static>> {
    let sd = sep_dot();
    if metadata.is_remote {
        let mut parts = vec!["YOUTUBE".to_string()];
        if let Some(media_type) = metadata.media_type.as_deref() {
            parts.push(media_type.to_string());
        }
        if let Some(format) = metadata.format.as_ref() {
            if !format.codec.is_empty() && format.abr > 0 {
                parts.push(format!("{} {}k", format.codec.to_uppercase(), format.abr));
            }
            if format.sample_rate > 0 {
                parts.push(format!("{} kHz", khz(format.sample_rate)));
            }
        }
        let label = parts.join(&format!(" {sd} "));
        Some(Line::from(vec![Span::styled(label, *quality_style)]))
    } else {
        // Local: show bit_depth + sample_rate when known.
        let mut parts: Vec<String> = vec!["LOCAL".to_string()];
        if let Some(media_type) = metadata.media_type.as_deref() {
            parts.push(media_type.to_string());
        }
        if metadata.bit_depth > 0 {
            parts.push(format!("{}-bit", metadata.bit_depth));
        }
        if metadata.sample_rate_hz > 0 {
            let sr = khz(metadata.sample_rate_hz);
            if !sr.is_empty() {
                parts.push(format!("{} kHz", sr));
            }
        }
        if parts.len() == 1 {
            // Only "LOCAL" — source-only.
            Some(Line::from(vec![Span::styled("LOCAL", *quality_style)]))
        } else {
            let label = parts.join(&format!(" {} ", sd));
            Some(Line::from(vec![Span::styled(label, *quality_style)]))
        }
    }
}

/// Return the remainder of `s` after the first `width` display columns.
/// Used for 2-line title wrapping: the first line is `truncate_title`
/// (with ellipsis), the second line is the remainder after the ellipsis
/// position, hard-clipped to width.
fn clip_after_width(s: &str, width: usize) -> String {
    if width == 0 {
        return s.to_string();
    }
    let mut w = 0;
    for (i, c) in s.char_indices() {
        let cw = crate::tui::view::theme::char_disp_width(c);
        if w + cw > width {
            return s[i..].to_string();
        }
        w += cw;
    }
    String::new()
}
