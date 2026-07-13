//! Big "Now Playing" player bar (I.2 — DEF-068).
//!
//! A 10-row rectangle that shows the now-playing track with richer metadata
//! than the 2-row mini bar: title, artist · album, quality readout, a
//! `▰▰▰▰▱▱` progress bar with `M:SS / M:SS + pct`, transport controls, a
//! block-bar volume meter, the `SHUF · RPT · CONT · PREF` flags, a `Next:`
//! preview, and a `♫ Lyrics:` first-line preview.
//!
//! RC19-D15: the album-art placeholder (a 4×4 `░▒▓█` grid + "album art" text
//! label) was removed. The label lived in the same row as the transport
//! controls and bled through the gaps between glyphs (`◀◀bu▶ ▶▶t`). The art
//! was decorative-only (terminals can't render real cover art), so dropping
//! it cleans up the transport row. The 10-col left reservation REMAINS so
//! the flags line (rendered into the right sub-rect) doesn't overwrite the
//! transport glyphs.
//!
//! Mini mode (`player_bar::render` / `render_compact`) is byte-identical to
//! the pre-I.2 implementation; this module is purely additive.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, empty_block, filled_block, is_ascii, marker_glyph, next_glyph,
    pause_glyph, play_glyph, prev_glyph, quality_color, sep_dot, stop_glyph, Theme,
    ASCII_BORDER_SET,
};

/// Player bar mode: the 2-row mini bar (current) or the 10-row big rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PlayerBarMode {
    /// 2-row compact bar (current behavior). Default.
    #[default]
    Mini,
    /// 10-row "Now Playing" rectangle.
    Big,
}

impl PlayerBarMode {
    /// Stable string for `LayoutState` persistence (`"mini"` / `"big"`).
    pub fn as_str(self) -> &'static str {
        match self {
            PlayerBarMode::Mini => "mini",
            PlayerBarMode::Big => "big",
        }
    }

    /// Parse a persisted mode string back into the enum. Falls back to `Mini`
    /// for unknown / empty values so a corrupt DB never breaks the bar.
    pub fn parse(s: &str) -> Self {
        match s {
            "big" => PlayerBarMode::Big,
            _ => PlayerBarMode::Mini,
        }
    }
}

/// Track-list layout mode for I.5: classic single-column rows or the
/// multi-column table. Toggled by `T` at wide widths.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrackLayoutMode {
    /// Multi-column table (`# | Title | Artist | Album | Dur | Qual | Src`).
    /// Default at ≥100 cols.
    #[default]
    Table,
    /// 2-3 cards per row (opt-in at ≥140 cols via `T`).
    Cards,
}

impl TrackLayoutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackLayoutMode::Table => "table",
            TrackLayoutMode::Cards => "cards",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "cards" => TrackLayoutMode::Cards,
            _ => TrackLayoutMode::Table,
        }
    }
}

/// Persistent state for the player bar's big/mini toggle + the track-list
/// card/table toggle. Owned by `App`; persisted via `LayoutState`.
#[derive(Clone, Debug, Default)]
pub struct PlayerBarState {
    /// Active mode (Mini or Big). Auto-toggles to Mini when the terminal is
    /// too small for Big (see `effective_mode`); `big_pref` remembers the
    /// user's choice so Big re-enables on resize.
    pub mode: PlayerBarMode,
    /// The user's preference (set by `P`). `effective_mode` may override to
    /// Mini when the terminal is too small, but `big_pref` persists.
    pub big_pref: bool,
    /// Track-list layout preference (set by `T`). Only honored at ≥140 cols.
    pub track_layout: TrackLayoutMode,
    /// Horizontal scroll offset for the lyrics preview line (reserved for
    /// future long-line marquee; currently 0).
    pub lyrics_preview_scroll: u16,
}

impl PlayerBarState {
    /// The mode that should actually render given a terminal size. Big mode
    /// requires ≥100 cols AND ≥30 rows; otherwise Mini is forced (the 10-row
    /// rectangle doesn't fit alongside content below that).
    pub fn effective_mode(&self, width: u16, height: u16) -> PlayerBarMode {
        if self.big_pref && width >= 100 && height >= 30 {
            PlayerBarMode::Big
        } else {
            PlayerBarMode::Mini
        }
    }
}

/// Minimum terminal size for big mode. Below this the bar auto-toggles to
/// mini and `P` shows a toast instead of switching.
pub const BIG_MIN_WIDTH: u16 = 100;
pub const BIG_MIN_HEIGHT: u16 = 30;
/// Rows the big bar occupies when active (replaces the 2-row mini bar).
pub const BIG_BAR_HEIGHT: u16 = 10;

/// Cell rectangles for every clickable control in the big player bar.
/// Extends `player_bar::PlayerBarGeometry` with volume + flag rects so mouse
/// hit-testing can dispatch on the new surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BigPlayerBarGeometry {
    pub previous: Rect,
    pub play_pause: Rect,
    pub next: Rect,
    pub progress: Rect,
    pub volume: Rect,
    pub shuffle: Rect,
    pub repeat: Rect,
    pub continue_mode: Rect,
    pub mode: Rect,
}

/// Compute the deterministic geometry used by [`render_big`] and mouse input
/// for `area`. The big bar lays out controls on row 7 (0-indexed within the
/// 10-row rect): transport on the left, volume mid, flags on row 9.
pub fn geometry_big(area: Rect) -> BigPlayerBarGeometry {
    if area.height < BIG_BAR_HEIGHT {
        return BigPlayerBarGeometry::default();
    }
    let y = area.y;
    // Row 7 (0-indexed): transport controls.
    let transport_y = y + 6;
    // Transport: ◀◀ ▶ ⏸ ⏭ ▶▶  (each ~2 cols + 1 gap). Start at x+1.
    let x0 = area.x + 1;
    let previous = Rect::new(x0, transport_y, 2, 1);
    let play_pause = Rect::new(x0 + 3, transport_y, 2, 1);
    let next = Rect::new(x0 + 6, transport_y, 2, 1);
    // Row 5: progress bar.
    let progress = Rect::new(area.x + 1, y + 4, area.width.saturating_sub(2), 1);
    // Row 7 (right side): volume meter.
    let vol_w = 12u16;
    let volume = Rect::new(
        area.right().saturating_sub(vol_w + 1),
        transport_y,
        vol_w,
        1,
    );
    // Row 9: flags line — each flag gets a hit rect. Approximate widths.
    let flags_y = y + 8;
    let flag_w = 14u16;
    let flags_x = area.x + 1;
    let shuffle = Rect::new(flags_x, flags_y, flag_w, 1);
    let repeat = Rect::new(flags_x + flag_w, flags_y, flag_w, 1);
    let continue_mode = Rect::new(flags_x + flag_w * 2, flags_y, flag_w, 1);
    let mode = Rect::new(flags_x + flag_w * 3, flags_y, flag_w, 1);
    BigPlayerBarGeometry {
        previous,
        play_pause,
        next,
        progress,
        volume,
        shuffle,
        repeat,
        continue_mode,
        mode,
    }
}

/// `(percent, "M:SS / M:SS")` for the progress gauge. When position/duration
/// are unavailable the gauge reads 0% with a `--:--` label. Mirrors
/// `player_bar::progress` but kept local so `player_bar.rs` stays untouched
/// (mini mode byte-identical).
fn progress(app: &App) -> (u16, String) {
    if app.now_playing.is_none() {
        return (0, "--:-- / --:--".to_string());
    }
    let dur = app.player.duration();
    let pos = app.player.position().or_else(|| app.estimated_position());
    match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => {
            let pct = ((p / d) * 100.0).clamp(0.0, 100.0) as u16;
            (pct, format!("{} / {}", fmt_time(p), fmt_time(d)))
        }
        _ => (0, "--:-- / --:--".to_string()),
    }
}

/// `M:SS` (or `H:MM:SS` past an hour). Truncates toward zero.
fn fmt_time(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `96000 -> "96"`, `44100 -> "44.1"`.
fn khz(sample_rate_hz: u32) -> String {
    if sample_rate_hz.is_multiple_of(1000) {
        format!("{}", sample_rate_hz / 1000)
    } else {
        format!("{:.1}", sample_rate_hz as f64 / 1000.0)
    }
}

/// Render a `▰▰▰▰▱▱▱▱ 42%` progress bar line. Local copy of
/// `player_bar::render_progress_bar` (keeps mini mode byte-identical).
fn render_progress_bar(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let (pct, label) = progress(app);
    let pcolor = if crate::tui::view::theme::no_color() {
        Color::Reset
    } else {
        theme.accent
    };
    let dim = Style::default().fg(theme.dim);
    let fill = Style::default().fg(pcolor).add_modifier(Modifier::BOLD);
    let pct_str = format!("{pct}%");
    let label_w = label.chars().count() as u16 + pct_str.chars().count() as u16 + 2;
    let bar_w = area.width.saturating_sub(label_w);
    let line = if bar_w < 3 {
        Line::from(vec![
            Span::styled(pct_str, dim),
            Span::raw(" "),
            Span::styled(label, dim),
        ])
    } else {
        let total = bar_w as usize;
        let filled = (pct as usize * total / 100).min(total);
        let rest = total - filled;
        let mut spans: Vec<Span<'static>> = Vec::new();
        if filled > 0 {
            spans.push(Span::styled(
                filled_block().to_string().repeat(filled),
                fill,
            ));
        }
        if rest > 0 {
            spans.push(Span::styled(empty_block().to_string().repeat(rest), dim));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(pct_str, fill));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, dim));
        Line::from(spans)
    };
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// `SHUF · RPT · CONT · PREF` flags line for row 9. Mirrors
/// `player_bar::build_flags_line` but local so mini mode stays untouched.
/// RC18-D4: appends `· SRC {actual}` when the now-playing track's source
/// differs from `source_mode` (e.g. a YouTube track plays while PREF=local)
/// so the flags line never contradicts the actual playing source — mirrors
/// the mini bar's DEF-013 fix.
/// RC19-D4: the user-preference label was renamed `MODE` → `PREF` so it
/// reads as the user's preference (Local / YouTube / Mixed) rather than
/// the actual playing source. `SRC` (appended when the playing source
/// differs) is the actual source — `PREF local · SRC youtube` is no
/// longer contradictory.
fn build_flags_line(app: &App) -> Line<'static> {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let sd = sep_dot();
    let shuf = match app.transport.shuffle {
        ShuffleMode::Off => "off",
        ShuffleMode::Smart => "smart",
        ShuffleMode::Random => "random",
    };
    let rpt = match app.transport.repeat {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    };
    let cont = match app.transport.continue_mode {
        ContinueMode::Off => "off",
        ContinueMode::NextAlbum => "next",
        ContinueMode::Radio => "radio",
        ContinueMode::YouTube => "youtube",
    };
    let mode = app.source_mode.as_str();
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("SHUF {shuf}"), dim),
        Span::raw(format!(" {sd} ")),
        Span::styled(format!("RPT {rpt}"), dim),
        Span::raw(format!(" {sd} ")),
        Span::styled(format!("CONT {cont}"), dim),
        Span::raw(format!(" {sd} ")),
        Span::styled(format!("PREF {mode}"), dim),
    ];
    // RC18-D4: SRC badge when the playing source differs from the mode.
    if let Some(np) = &app.now_playing {
        let playing_src = if np.is_remote() { "youtube" } else { "local" };
        if playing_src != mode {
            let color = if crate::tui::view::theme::no_color() {
                Color::Reset
            } else if np.is_remote() {
                Color::Yellow
            } else {
                Color::Green
            };
            spans.push(Span::raw(format!(" {sd} ")));
            spans.push(Span::styled(
                format!("SRC {playing_src}"),
                Style::default().fg(color),
            ));
        }
    }
    Line::from(spans)
}

/// The first non-empty line of the now-playing track's lyrics, for the
/// `♫ Lyrics:` preview on row 10. Local tracks: read embedded/sidecar
/// lyrics (fast). YouTube tracks: show `(none)` unless a cached result is
/// available (the bar never fires a sidecar request — that would spam on
/// every frame). Returns `None` when nothing is playing.
fn lyrics_preview_line(app: &App) -> Option<String> {
    let ts = app.now_playing.as_ref()?;
    match ts {
        crate::source::TrackSource::Local { track_id } => {
            let track = app.track_by_id_fast(track_id)?;
            match crate::lyrics::read_embedded(track, &app.catalog.source_root) {
                Some(lyrics) => lyrics
                    .lines
                    .iter()
                    .find(|l| !l.text.trim().is_empty())
                    .map(|first| first.text.clone()),
                None => None,
            }
        }
        crate::source::TrackSource::Remote { video_id } => {
            // Check the on-disk lyrics cache (no sidecar spam).
            let db = crate::state::db_path();
            if let Some(cached) = crate::lyrics::cache::load(&db, video_id) {
                if let Some(first) = cached.lines.iter().find(|l| !l.text.trim().is_empty()) {
                    return Some(first.text.clone());
                }
            }
            None
        }
    }
}

/// The "Next:" preview string for row 10. Reuses `transport.peek_next` +
/// the local/YouTube title resolution (mirrors `player_bar::up_next_preview`
/// without firing sidecar requests).
fn next_preview(app: &App) -> String {
    match app.transport.peek_next(app, &app.catalog) {
        Some(id) => {
            let title = if let Some(t) = app.track_by_id_fast(&id) {
                t.title.clone()
            } else if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(&id)) {
                rt.title.clone()
            } else {
                format!("Loading{}", ellipsis())
            };
            format!("Next: {title}")
        }
        None => {
            if app.now_playing.is_some() {
                "Next: (end)".to_string()
            } else {
                "Next: -".to_string()
            }
        }
    }
}

/// Render the big "Now Playing" bar into `area`. `area` must be at least
/// `BIG_BAR_HEIGHT` rows tall; callers should check `effective_mode`
/// before dispatching here.
pub fn render_big(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let text = Style::default().fg(theme.text);
    let accent = Style::default().fg(theme.accent);
    let nc = crate::tui::view::theme::no_color();

    // Bordered block with a "Now Playing" title. ASCII font mode uses the
    // ASCII border set so the rectangle is fully ASCII under
    // JUKEBOX_FONT_MODE=ascii.
    let title = format!(" Now Playing {} ", em_dash());
    let block = if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(theme.accent))
            .title(Span::styled(
                title.clone(),
                Style::default().fg(theme.accent),
            ))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(theme.accent))
            .title(Span::styled(title, Style::default().fg(theme.accent)))
    };

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserve a 10-col left column for the transport controls so the flags
    // line (rendered into `meta_area` row 6 = rows[5]) doesn't overwrite the
    // transport glyphs (geometry_big places transport at x0 = area.x + 1,
    // cols 1-8). The old `show_art` branch put the album-art grid + label
    // in this column; RC19-D15 removed the art (it was decorative and its
    // "album art" text label bled through the transport gaps as `b`/`u`/`t`
    // stray chars). The column reservation stays so the transport area and
    // the flags text don't collide.
    let split = Layout::horizontal([Constraint::Length(10), Constraint::Min(1)]).split(inner);
    let meta_area = split[1];

    // Split the metadata area into 10 rows per the spec.
    let rows = Layout::vertical([
        Constraint::Length(1), // Row 1: title
        Constraint::Length(1), // Row 2: artist · album
        Constraint::Length(1), // Row 3: quality readout
        Constraint::Length(1), // Row 4: progress bar
        Constraint::Length(1), // Row 5: transport + volume
        Constraint::Length(1), // Row 6: flags
        Constraint::Length(1), // Row 7: Next + Lyrics
        Constraint::Length(1), // Row 8: spare
    ])
    .split(meta_area);

    let np = app.now_playing_view();

    // Row 1: ▶ title (accent, BOLD)
    let playing = app.player.is_playing();
    let has_track = app.now_playing.is_some();
    let state_glyph = if playing && has_track {
        play_glyph()
    } else if has_track {
        pause_glyph()
    } else {
        stop_glyph()
    };
    let title_line = match &np {
        Some(v) => Line::from(vec![
            Span::styled(
                format!("{state_glyph} "),
                accent.add_modifier(Modifier::BOLD),
            ),
            Span::styled(v.title.clone(), accent.add_modifier(Modifier::BOLD)),
        ]),
        None => {
            // RC18-D14: when stopped with a saved last-played track, show the
            // resume hint ("▸ resume: {title} at {M:SS} · R to resume") so the
            // user knows `R` will resume. Mirrors `player_bar::build_info_line`
            // / `render_compact` — the mini bar already renders this hint, but
            // the big bar didn't, so users with `big_pref=true` (persisted)
            // never saw the offer and `R` looked broken. The hint clears on
            // the first successful play (note_play_started).
            if let Some(hint) = app.resume_hint.as_ref() {
                Line::from(Span::styled(
                    format!("{} {}", marker_glyph(), hint),
                    accent.add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(
                    format!("{state_glyph} nothing playing {dash}", dash = em_dash()),
                    dim,
                ))
            }
        }
    };
    f.render_widget(Paragraph::new(title_line), rows[0]);

    // Row 2: artist · album (text, dim separators)
    let row2 = match &np {
        Some(v) => {
            let mut spans: Vec<Span<'static>> = Vec::new();
            spans.push(Span::styled("   ", dim));
            spans.push(Span::styled(v.artist.clone(), text));
            if let Some(album) = &v.album {
                if !album.is_empty() {
                    spans.push(Span::raw(format!(" {} ", sep_dot())));
                    spans.push(Span::styled(album.clone(), text));
                }
            }
            Line::from(spans)
        }
        None => Line::from(Span::styled("   -", dim)),
    };
    f.render_widget(Paragraph::new(row2), rows[1]);

    // Row 3: quality readout (quality_color)
    let row3 = match &np {
        Some(v) if v.source.is_remote() => {
            let label = v
                .fmt
                .as_ref()
                .map(|f| f.yt_label())
                .unwrap_or_else(|| "YT".to_string());
            let color = if nc { Color::Reset } else { Color::Yellow };
            Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(label, Style::default().fg(color)),
            ])
        }
        Some(v) => {
            let q_color = quality_color(v.bit_depth, v.sample_rate_hz);
            let q_text = format!("{}-bit / {} kHz", v.bit_depth, khz(v.sample_rate_hz));
            let mut spans: Vec<Span<'static>> = vec![
                Span::styled("   ", dim),
                Span::styled(q_text, Style::default().fg(q_color)),
            ];
            if app.switch_sample_rate {
                spans.push(Span::styled(
                    format!(" {} bit-perfect", sep_dot()),
                    Style::default().fg(q_color),
                ));
            }
            Line::from(spans)
        }
        None => Line::from(Span::styled("   --bit / -- kHz", dim)),
    };
    f.render_widget(Paragraph::new(row3), rows[2]);

    // Row 4: blank
    // Row 5: progress bar (▰▱) + M:SS / M:SS + pct
    render_progress_bar(f, rows[3], app);

    // Row 6: blank
    // Row 7: transport ◀◀ ▶ ⏸ ⏭ ▶▶ + vol ▰▰▰▰▱ 70% [MUTED]
    {
        let geo = geometry_big(area);
        let controls = Style::default()
            .fg(if nc { Color::Reset } else { theme.accent })
            .add_modifier(Modifier::BOLD);
        f.render_widget(Paragraph::new(prev_glyph()).style(controls), geo.previous);
        // Play/pause glyph reflects the current state (same convention as
        // the mini bar).
        let pp_glyph = if playing && has_track {
            play_glyph()
        } else if has_track {
            pause_glyph()
        } else {
            stop_glyph()
        };
        f.render_widget(
            Paragraph::new(pp_glyph)
                .style(controls)
                .alignment(Alignment::Center),
            geo.play_pause,
        );
        f.render_widget(Paragraph::new(next_glyph()).style(controls), geo.next);

        // Volume meter (right side of row 7).
        let vol_str = {
            let blocks = 4u32;
            let filled = if app.muted {
                0
            } else {
                ((u32::from(app.volume) * blocks + 50) / 100).min(blocks)
            };
            let mut bar = String::new();
            for i in 0..blocks {
                bar.push(if i < filled {
                    filled_block()
                } else {
                    empty_block()
                });
            }
            let pct = if app.muted { 0 } else { app.volume };
            format!("vol {bar} {pct}%")
        };
        let vol_label = if app.muted { " [MUTED]" } else { "" };
        let vol_color = if app.muted { theme.dim } else { theme.text };
        let vol_line = Line::from(vec![
            Span::styled(
                vol_str,
                Style::default().fg(if nc { Color::Reset } else { vol_color }),
            ),
            Span::styled(vol_label.to_string(), Style::default().fg(theme.dim)),
        ]);
        f.render_widget(Paragraph::new(vol_line), geo.volume);
    }

    // Row 8: blank
    // Row 9: SHUF · RPT · CONT · PREF flags
    f.render_widget(Paragraph::new(build_flags_line(app)), rows[5]);

    // Row 10: Next: {title} + ♫ Lyrics: {first line}
    {
        let next = next_preview(app);
        let lyrics_label = if is_ascii() {
            "# Lyrics:"
        } else {
            "♫ Lyrics:"
        };
        let lyrics_text = match &np {
            Some(_) => match lyrics_preview_line(app) {
                Some(line) => {
                    // Clip to available width so the line doesn't overflow.
                    let avail = rows[6].width as usize;
                    let prefix_w = disp_width(&format!("{next}  {lyrics_label} ",));
                    let budget = avail.saturating_sub(prefix_w);
                    crate::tui::view::theme::clip_to_width(&line, budget.max(1))
                }
                None => "(none)".to_string(),
            },
            None => "(none)".to_string(),
        };
        let row7 = Line::from(vec![
            Span::styled(next, dim),
            Span::raw("  "),
            Span::styled(format!("{lyrics_label} "), dim),
            Span::styled(lyrics_text, text),
        ]);
        f.render_widget(Paragraph::new(row7), rows[6]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};

    fn two_track_cat() -> (tempfile::TempDir, Catalog) {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        std::fs::write(lossless.join("A").join("02.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom","album":"Adele","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]},
              {"id":"t2","artists":["Bop"],"primary_artist":"Bop","title":"Long Title Here Yes","album":"Beep","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/02.flac","symlinked_into_artists":["Bop"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        (d, Catalog::load(&p).unwrap())
    }

    fn rendered_big(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, w, h);
        term.draw(|f| render_big(f, area, app)).unwrap();
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

    /// The big bar must show the now-playing title in the first row.
    #[test]
    fn big_bar_shows_title() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("Freedom"),
            "big bar must show now-playing title: {bar}"
        );
    }

    /// The big bar must show the artist + album on row 2.
    #[test]
    fn big_bar_shows_artist_album() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(bar.contains("Ado"), "big bar must show artist: {bar}");
        assert!(bar.contains("Adele"), "big bar must show album: {bar}");
    }

    /// The big bar must show the quality readout on row 3.
    #[test]
    fn big_bar_shows_quality() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("24-bit") || bar.contains("24-bit"),
            "big bar must show bit depth: {bar}"
        );
    }

    /// The big bar must show the progress bar + percentage on row 5.
    #[test]
    fn big_bar_shows_progress() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains('%'),
            "big bar must show progress percentage: {bar}"
        );
    }

    /// The big bar must show transport controls (prev/next glyphs) on row 7.
    #[test]
    fn big_bar_shows_transport() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("◀◀") || bar.contains("<<"),
            "big bar must show prev transport: {bar}"
        );
        assert!(
            bar.contains("▶▶") || bar.contains(">>"),
            "big bar must show next transport: {bar}"
        );
    }

    /// The big bar must show the volume meter on row 7.
    #[test]
    fn big_bar_shows_volume() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(bar.contains("vol"), "big bar must show vol label: {bar}");
        assert!(bar.contains("70%"), "big bar must show volume pct: {bar}");
    }

    /// The big bar must show the SHUF/RPT/CONT/PREF flags on row 9.
    #[test]
    fn big_bar_shows_flags() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(bar.contains("SHUF"), "big bar must show SHUF: {bar}");
        assert!(bar.contains("RPT"), "big bar must show RPT: {bar}");
        assert!(bar.contains("CONT"), "big bar must show CONT: {bar}");
        assert!(bar.contains("PREF"), "big bar must show PREF: {bar}");
    }

    /// The big bar must show the Next: preview on row 10.
    #[test]
    fn big_bar_shows_next() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("Next:"),
            "big bar must show Next: preview: {bar}"
        );
    }

    /// The big bar must show a Lyrics: line (either the first lyric line or
    /// "(none)" when no lyrics are available).
    #[test]
    fn big_bar_shows_lyrics_label() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("Lyrics:"),
            "big bar must show Lyrics: label: {bar}"
        );
    }

    /// `PlayerBarMode::parse` round-trips the persisted string.
    #[test]
    fn player_bar_mode_parse_round_trips() {
        assert_eq!(PlayerBarMode::parse("mini"), PlayerBarMode::Mini);
        assert_eq!(PlayerBarMode::parse("big"), PlayerBarMode::Big);
        assert_eq!(PlayerBarMode::parse("garbage"), PlayerBarMode::Mini);
        assert_eq!(PlayerBarMode::parse(""), PlayerBarMode::Mini);
    }

    /// `effective_mode` forces Mini below the minimum size even when
    /// `big_pref` is true.
    #[test]
    fn effective_mode_forces_mini_below_min() {
        let mut s = PlayerBarState {
            big_pref: true,
            ..Default::default()
        };
        assert_eq!(s.effective_mode(80, 24), PlayerBarMode::Mini);
        assert_eq!(s.effective_mode(100, 24), PlayerBarMode::Mini);
        assert_eq!(s.effective_mode(99, 30), PlayerBarMode::Mini);
        assert_eq!(s.effective_mode(100, 30), PlayerBarMode::Big);
        assert_eq!(s.effective_mode(120, 40), PlayerBarMode::Big);
        s.big_pref = false;
        assert_eq!(s.effective_mode(120, 40), PlayerBarMode::Mini);
    }

    /// `TrackLayoutMode::parse` round-trips the persisted string.
    #[test]
    fn track_layout_mode_parse_round_trips() {
        assert_eq!(TrackLayoutMode::parse("table"), TrackLayoutMode::Table);
        assert_eq!(TrackLayoutMode::parse("cards"), TrackLayoutMode::Cards);
        assert_eq!(TrackLayoutMode::parse(""), TrackLayoutMode::Table);
    }

    /// The album-art placeholder was removed (RC19-D15). The grid function
    /// is gone; this test now verifies the big bar still renders fine
    /// without any album-art content (no regression in title/flags/etc).
    #[test]
    fn album_art_grid_removed_bar_renders() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("Freedom"),
            "big bar must still render title without album art: {bar}"
        );
    }

    /// RC19-D15: the big bar transport row must show `◀◀ ▶ ▶▶` with NO
    /// stray `b`/`u`/`t` chars from the old "album art" text label. The old
    /// layout rendered the label at the same row as the transport controls,
    /// and the gaps between transport rects let the label chars show through.
    /// The fix removes the label (and the whole album-art placeholder).
    #[test]
    fn rc19_d15_big_bar_transport_row_no_stray_chars() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, 100, 10);
        // Find the transport row (the one containing ◀◀ or <<).
        let transport_row = bar
            .lines()
            .find(|l| l.contains("◀◀") || l.contains("<<"))
            .expect("RC19-D15: transport row must exist in big bar");
        // The transport area is the first ~12 cols (border at col 0, then
        // ◀◀ at 1-2, gap at 3, ▶ at 4-5, gap at 6, ▶▶ at 7-8, gap at 9-10).
        // No stray 'b', 'u', 't' should appear in that area.
        let transport_area: String = transport_row.chars().take(12).collect();
        for stray in ['b', 'u', 't'] {
            assert!(
                !transport_area.contains(stray),
                "RC19-D15: stray '{stray}' in transport area \
                 (transport_area={transport_area:?})\nfull row: {transport_row}\nfull bar:\n{bar}"
            );
        }
        // Sanity: transport glyphs are still present.
        assert!(
            transport_row.contains("◀◀") || transport_row.contains("<<"),
            "RC19-D15: transport prev glyph missing: {transport_row}"
        );
        assert!(
            transport_row.contains("▶▶") || transport_row.contains(">>"),
            "RC19-D15: transport next glyph missing: {transport_row}"
        );
    }

    /// RC19-D4: the big bar must show `PREF` (not `MODE`) for the user
    /// source-preference label, and `SRC youtube` when a YouTube track
    /// plays under PREF=local. The old label `MODE local` contradicted the
    /// actual playing source; renaming to `PREF` makes the distinction
    /// clear: PREF = user preference, SRC = actual source.
    #[test]
    fn rc19_d4_big_bar_pref_label_and_src_badge() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // Play a remote (YouTube) track under PREF=local so the SRC badge
        // appears alongside the PREF label.
        app.now_playing = Some(crate::source::TrackSource::Remote {
            video_id: "v1".into(),
        });
        // source_mode defaults to Local.
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("PREF local"),
            "RC19-D4: big bar must show 'PREF local' (not MODE): {bar}"
        );
        assert!(
            !bar.contains("MODE"),
            "RC19-D4: big bar must NOT show 'MODE' label anymore: {bar}"
        );
        assert!(
            bar.contains("SRC youtube"),
            "RC19-D4: big bar must show 'SRC youtube' when a YT track plays under PREF=local: {bar}"
        );
    }

    /// The big bar must render without panicking when nothing is playing.
    #[test]
    fn big_bar_renders_when_stopped() {
        let (_d, cat) = two_track_cat();
        let app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        let bar = rendered_big(&app, 100, 10);
        assert!(
            bar.contains("Now Playing"),
            "big bar must show title: {bar}"
        );
    }

    /// The big bar must render within a `BIG_MIN_WIDTH`×`BIG_MIN_HEIGHT`
    /// terminal without overflowing.
    #[test]
    fn big_bar_renders_at_min_size() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_big(&app, BIG_MIN_WIDTH, BIG_MIN_HEIGHT);
        assert!(
            bar.contains("Freedom"),
            "big bar at min size must render: {bar}"
        );
    }
}
