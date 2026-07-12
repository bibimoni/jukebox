//! Persistent bottom player bar.
//!
//! Renders the now-playing line (play glyph + title — artist · album), the
//! transport glyphs, a `▰▰▰▰▱▱▱▱ 42%` block-char progress bar with an
//! `M:SS / M:SS` label, the hi-fi quality readout (`24-bit / 96 kHz`, plus
//! `· bit-perfect` when the output sample-rate is being switched to match the
//! track), a block-bar volume meter, and the shuffle/repeat mode flags as
//! plain text (no emoji — monochrome-safe).
//!
//! Layout: at `area.height >= 2` the info occupies the top row and the
//! progress bar the row beneath; the info content is composed as a single
//! [`Line`] of [`Span`]s so it stays on one visual line at wide widths
//! (≥ ~100 cols) and only wraps below that. At `area.height == 1` only the
//! info line is drawn.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::catalog::Track;
use crate::tui::app::App;
use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, empty_block, filled_block, marker_glyph, next_glyph,
    pause_glyph, play_glyph, prev_glyph, progress_color, quality_color, sep_dot, stop_glyph, Theme,
};

/// Braille spinner frames (U+2800–28FF, width 1) — the same set Claude Code's
/// CLI uses. Animated in `App::on_tick` while a YouTube resolve is in flight.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII spinner frames — a fallback for minimal terminals (often paired with
/// `NO_COLOR`) where the braille dots may not render in the font. 4 frames
/// cycled by `App::on_tick` (`spinner_frame` advances by 1 each tick).
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

/// Cell rectangles for every clickable control rendered by the full player bar.
/// Rendering and input both consume this value, so hit-testing cannot drift
/// from the visible controls as terminal dimensions change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerBarGeometry {
    pub previous: Rect,
    pub play_pause: Rect,
    pub next: Rect,
    pub progress: Rect,
}

/// Return the deterministic geometry used by [`render`] and mouse input.
pub fn geometry(area: Rect) -> PlayerBarGeometry {
    let controls_width = area.width.min(14);
    let controls_x = area.right().saturating_sub(controls_width);
    let info_y = area.y;
    // `render_compact` (height 1) draws no transport controls, so geometry
    // reports zero-size rects for them — `rect_contains` in input.rs then
    // naturally returns false and clicks on the compact bar's right edge do
    // not trigger invisible prev/play/next actions. The full bar (height >= 2)
    // renders all three, so they stay clickable. (Mirrors how `progress` is
    // already zero-sized when `area.height < 2`.)
    let transport_visible = area.height >= 2;
    let progress = if area.height >= 2 {
        let width = area.width.saturating_mul(55) / 100;
        Rect::new(area.x, area.y.saturating_add(1), width, 1)
    } else {
        Rect::new(area.x, area.y, 0, 0)
    };
    if !transport_visible {
        PlayerBarGeometry {
            previous: Rect::new(controls_x, info_y, 0, 0),
            play_pause: Rect::new(controls_x, info_y, 0, 0),
            next: Rect::new(controls_x, info_y, 0, 0),
            progress,
        }
    } else {
        PlayerBarGeometry {
            previous: Rect::new(controls_x, info_y, controls_width.min(4), 1),
            play_pause: Rect::new(
                controls_x.saturating_add(5),
                info_y,
                controls_width.saturating_sub(5).min(3),
                1,
            ),
            next: Rect::new(
                controls_x.saturating_add(9),
                info_y,
                controls_width.saturating_sub(9).min(4),
                1,
            ),
            progress,
        }
    }
}

/// Pick the current spinner glyph: ASCII under `NO_COLOR` (minimal terminals),
/// braille otherwise. `spinner_frame` wraps modulo the active frame count.
fn spinner_glyph(app: &App) -> &'static str {
    let frames = if crate::tui::view::theme::no_color() {
        &SPINNER_ASCII[..]
    } else {
        &SPINNER[..]
    };
    frames[app.spinner_frame as usize % frames.len()]
}

/// The "Up Next" preview string: `▸ Next: {title}` when a successor is queued,
/// or `▸ Next: (end)` when the context is exhausted (so the up-next slot is
/// always visible while a track is playing — GLM: up-next-missing). When
/// nothing is playing, a `▸ Enter to play` hint fills the slot so the up-next
/// area is never empty (GLM: up-next-missing-when-stopped).
fn up_next_preview(app: &App) -> Option<String> {
    match app.up_next_title() {
        Some(title) => {
            let max = 28;
            let trimmed = truncate_title(&title, max);
            Some(format!("{} Next: {trimmed}", marker_glyph()))
        }
        None => {
            if app.now_playing.is_some() {
                Some(format!("{} Next: (end)", marker_glyph()))
            } else {
                Some(format!("{} Enter to play", marker_glyph()))
            }
        }
    }
}

/// Truncate `title` to `max` display columns, appending an ellipsis when it
/// was shortened. CJK/wide characters are counted as 2 via [`disp_width`] so
/// truncation respects terminal alignment for Japanese titles.
pub fn truncate_title(title: &str, max: usize) -> String {
    // max == 0 → no room for any text (avoid `max - 1` underflow below).
    if max == 0 {
        return String::new();
    }
    if disp_width(title) <= max {
        return title.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in title.chars() {
        let cw = disp_width(&c.to_string());
        if w + cw > max - 1 {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push_str(ellipsis());
    out
}

/// Compact 1-row player bar for the narrow (60–80 col) fallback: now-playing +
/// quality + flags all on one line, no gauge (spec §5.6). At very narrow widths
/// the least-important sections drop out so the now-playing text stays visible:
/// below 70 cols the mode flags drop, below 60 cols the quality readout drops
/// too (priority: now-playing > quality > flags).
pub fn render_compact(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let text = Style::default().fg(theme.text);
    let sd = sep_dot(); // ASCII-safe separator for DEF-006
    let resolving = app.is_resolving();
    let has_track = app.now_playing.is_some();
    let playing = app.player.is_playing() && has_track;
    // State convention: ▶ = playing, ⏸ = paused, ■ = stopped (Issue 1: the
    // old logic showed ⏸ while playing and ▶ while paused — backwards — and
    // □ for stopped which reads as a stop button. Now the glyph reflects the
    // CURRENT state, matching the [PLAYING]/[PAUSED]/[STOPPED] label.)
    let state_glyph = if resolving {
        spinner_glyph(app)
    } else if playing {
        play_glyph()
    } else if has_track {
        pause_glyph()
    } else {
        stop_glyph()
    };
    // Accent (Cyan) + BOLD while resolving — an attention/progress signal —
    // else the normal text color + BOLD. The play/pause glyph is bold+colored
    // so it's visually prominent (T2: play-pause-icon-missing — glyph was
    // color-only, now bold+colored = two cues surviving NO_COLOR). Both auto-
    // degrade to Reset under NO_COLOR (theme); bold weight survives mono.
    let glyph_style = if resolving {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if !has_track {
        dim
    } else {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    };
    // State label: [PLAYING]/[PAUSED]/[STOPPED] — the clearest single signal
    // of the playback state, leading the row (GLM: state-label-missing).
    // Colors: Magenta for playing (attention), Yellow for paused (caution),
    // dim for stopped. All collapse to Reset under NO_COLOR (the label text
    // distinguishes states without color).
    let nc = crate::tui::view::theme::no_color();
    let (state_label, state_style) = if playing {
        (
            "[PLAYING]",
            Style::default().fg(if nc { Color::Reset } else { Color::Magenta }),
        )
    } else if has_track {
        (
            "[PAUSED]",
            Style::default().fg(if nc { Color::Reset } else { Color::Yellow }),
        )
    } else {
        ("[STOPPED]", dim)
    };
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(state_label, state_style),
        Span::raw(" "),
        Span::styled(format!("{state_glyph} "), glyph_style),
    ];

    // Resolve the now-playing view ONCE per frame — `now_playing_view()` does
    // an id→track lookup (and for remote tracks a `track_cache` probe). It was
    // called twice here (title/artist + quality) and twice more in
    // `build_info_line`; caching the result cuts 4 lookups/frame to 1 (PB7).
    let np = app.now_playing_view();

    // title — artist (always shown — highest priority)
    match &np {
        Some(v) => {
            spans.push(Span::styled(v.title.clone(), text));
            spans.push(Span::styled(" — ", dim));
            spans.push(Span::styled(v.artist.clone(), text));
        }
        None => spans.push(Span::styled(
            format!("{dash} nothing playing {dash}", dash = em_dash()),
            dim,
        )),
    }
    // Progress text (always visible — essential playback info; T3:
    // status-drops-indicators — keep title + progress + mode at small size).
    // Compact `M:SS / M:SS` label fits on one line where the gauge wouldn't.
    {
        let (_pct, plabel) = progress(app);
        spans.push(Span::styled(format!("  {plabel}"), dim));
    }
    // Up-next preview (width > 60) — so the narrow bar also shows what's
    // queued, matching the full bar's up-next slot (GLM:
    // up-next-missing-in-compact). Grouped right after the now-playing text.
    if area.width > 60 {
        if let Some(next) = up_next_preview(app) {
            spans.push(Span::styled("  ", dim));
            spans.push(Span::styled(next, dim));
        }
    }
    // quality (drop below 80 cols — at narrow widths the bit depth / sample
    // rate readout crowds the status bar; keep title + play glyph + progress
    // text + mode as the essential minimum, drop quality + volume)
    if area.width >= 80 {
        spans.push(Span::raw("  "));
        match &np {
            Some(v) if v.source.is_remote() => {
                let label = v
                    .fmt
                    .as_ref()
                    .map(|f| f.yt_label())
                    .unwrap_or_else(|| "YT".to_string());
                let color = if crate::tui::view::theme::no_color() {
                    Color::Reset
                } else {
                    Color::Yellow
                };
                spans.push(Span::styled(label, Style::default().fg(color)));
            }
            Some(v) => {
                let q_color = quality_color(v.bit_depth, v.sample_rate_hz);
                spans.push(Span::styled(
                    format!("{}bit/{}", v.bit_depth, khz(v.sample_rate_hz)),
                    Style::default().fg(q_color),
                ));
            }
            None => spans.push(Span::styled("--", dim)),
        }
    }
    // MODE always visible (essential — shows playback source local/yt/mixed).
    // CONT drops below 70 cols (lower priority). T3: keep title+progress+mode
    // at small sizes so the status bar never loses essential indicators.
    // DEF-004: SHUF/RPT also visible at >= 80 cols so the compact bar (used
    // at height <= 24) shows playback state indicators alongside MODE+CONT.
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("MODE {}", app.source_mode.as_str()),
        dim,
    ));
    // DEF-011/DEF-012: compact YT auth indicator — always visible at >= 70
    // cols so the YouTube connection state is visible at all terminal sizes,
    // not just in the 2-row footer (which requires >100 width or >24 height).
    // DEF-013: when a track is playing, also show the actual source if it
    // differs from the mode (e.g., "MODE local" while a YT track plays →
    // "MODE local · [Y]" so the label isn't misleading).
    if area.width >= 70 {
        let nc = crate::tui::view::theme::no_color();
        // Playing-source badge (DEF-013): if the now-playing track's source
        // differs from the mode, show [Y] or [L] to disambiguate.
        if let Some(np) = &app.now_playing {
            let playing_src = if np.is_remote() { "youtube" } else { "local" };
            let mode_src = app.source_mode.as_str();
            if playing_src != mode_src {
                let badge = if np.is_remote() { "[Y]" } else { "[L]" };
                let color = if nc {
                    Color::Reset
                } else if np.is_remote() {
                    Color::Yellow
                } else {
                    Color::Green
                };
                spans.push(Span::styled(
                    format!(" {sd} {badge}"),
                    Style::default().fg(color),
                ));
            }
        }
        // YT auth indicator (DEF-011/DEF-012): compact connection-state glyph.
        {
            use crate::yt::state::YtState;
            let yt_relevant = {
                use crate::mode::SourceMode;
                use crate::tui::app::View;
                app.source_mode != SourceMode::Local || app.view == View::Youtube
            };
            if yt_relevant && app.yt_state != YtState::Unconfigured {
                let (label, color) = match app.yt_state {
                    YtState::Ready => ("YT", if nc { Color::Reset } else { Color::Green }),
                    YtState::AuthExpired | YtState::ProviderError | YtState::Failed => {
                        ("YT!", if nc { Color::Reset } else { Color::Red })
                    }
                    YtState::RateLimited | YtState::ReadyStale => {
                        ("YT~", if nc { Color::Reset } else { Color::Yellow })
                    }
                    _ => ("YT…", if nc { Color::Reset } else { Color::Yellow }),
                };
                spans.push(Span::styled(
                    format!(" {sd} {label}"),
                    Style::default().fg(color),
                ));
            }
        }
    }
    if area.width >= 70 {
        let cont = match app.transport.continue_mode {
            ContinueMode::Off => "off",
            ContinueMode::NextAlbum => "next",
            ContinueMode::Radio => "radio",
            ContinueMode::YouTube => "youtube",
        };
        spans.push(Span::styled(format!(" {sd} CONT {cont}"), dim));
    }
    if area.width >= 80 {
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
        spans.push(Span::styled(format!(" {sd} SHUF {shuf}"), dim));
        spans.push(Span::styled(format!(" {sd} RPT {rpt}"), dim));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans).alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
}

/// Render the player bar into `area` using state from `app`. Two rows:
/// row 1 = now-playing + quality + volume; row 2 = progress bar + mode
/// flags (SHUF · RPT · CONT · MODE), `·`-separated, right-anchored.
pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical(if area.height >= 2 {
        vec![Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Fill(1)]
    })
    .split(area);

    let info_area = rows[0];
    let gauge_area = rows.get(1).copied();

    // Reserve the rightmost 14 columns for the transport controls (◀◀ ▶ ▶▶)
    // so a long info line is truncated before it can be overwritten by them.
    // The 14 matches `controls_width = area.width.min(14)` in `geometry()`.
    let line = build_info_line(app, info_area.width.saturating_sub(14) as usize);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        info_area,
    );

    let geo = geometry(area);
    let controls = Style::default()
        .fg(Theme::default().accent)
        .add_modifier(Modifier::BOLD);
    f.render_widget(Paragraph::new(prev_glyph()).style(controls), geo.previous);
    f.render_widget(
        Paragraph::new(play_glyph())
            .style(controls)
            .alignment(Alignment::Center),
        geo.play_pause,
    );
    f.render_widget(Paragraph::new(next_glyph()).style(controls), geo.next);

    if let Some(g) = gauge_area {
        // Row 2: progress bar on the left (~55%) + flags right-anchored. We
        // render the bar into a left sub-rect and the flags into a right
        // sub-rect so the flags stay flush against the right edge and the
        // bar never overruns them.
        //
        // The bar is a custom `▰▰▰▰▱▱▱▱ 42%` Paragraph (not ratatui's `Gauge`)
        // so the bar is ALWAYS visible — even at 0% / when position/duration
        // is unavailable (YouTube / afplay / hybrid mode). The `Gauge` widget's
        // unfilled portion is style-less and invisible at 0%, making the bar
        // "disappear" in hybrid mode (T2 fix). Block chars match the volume
        // meter style; the percentage label gives the exact position.
        // `geo.progress` is the single source of truth for the bar's rect:
        // input hit-testing reads the same value, so rendering and clicks can
        // never drift. `flags_area` is the remaining width to the right of
        // the progress bar on the same row(s). (Previously a
        // `Layout::horizontal([Percentage(55), Min(1)])` + `debug_assert_eq!`
        // was used, but the layout solver may round differently at odd widths
        // and panic debug builds.)
        render_progress_bar(f, geo.progress, app);
        let flags_area = Rect::new(
            geo.progress.right(),
            g.y,
            g.width.saturating_sub(geo.progress.width),
            g.height,
        );
        f.render_widget(
            Paragraph::new(build_flags_line(app, flags_area.width as usize))
                .block(Block::default().borders(Borders::NONE))
                .alignment(Alignment::Right),
            flags_area,
        );
    }
}

/// Row 1: [STATE] source badge + play glyph + title — artist · album (left) +
/// up-next preview + quality + volume. The title is the visual hero (accent
/// style, not dim). Source badge [L]/[Y] makes the source immediately clear.
/// A `[PLAYING]`/`[PAUSED]`/`[STOPPED]` label leads the row so the playback
/// state is unambiguous even without the glyph (GLM: state-label-missing,
/// play-pause-icon-missing). The play glyph reflects the CURRENT state:
/// `▶` playing, `⏸` paused, `■` stopped (Issue 1).
fn build_info_line(app: &App, width: usize) -> Line<'static> {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let text = Style::default().fg(theme.text);
    let accent = Style::default().fg(theme.accent);
    let nc = crate::tui::view::theme::no_color();

    let playing = app.player.is_playing();
    let has_track = app.now_playing.is_some();
    let resolving = app.is_resolving();
    // State convention: ▶ = playing, ⏸ = paused, ■ = stopped (Issue 1: the
    // old logic showed ⏸ while playing and ▶ while paused — backwards — and
    // □ for stopped which reads as a stop button. Now the glyph reflects the
    // CURRENT state, matching the [PLAYING]/[PAUSED]/[STOPPED] label.)
    let state_glyph = if resolving {
        spinner_glyph(app)
    } else if playing && has_track {
        play_glyph()
    } else if has_track {
        pause_glyph()
    } else {
        stop_glyph()
    };
    let glyph_style = if resolving {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if !has_track {
        dim
    } else {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    };

    // State label: [PLAYING]/[PAUSED]/[STOPPED] — the clearest single signal
    // of the playback state. Magenta for playing, Yellow for paused, dim for
    // stopped. All collapse to Reset under NO_COLOR.
    let (state_label, state_style) = if playing && has_track {
        (
            "[PLAYING]",
            Style::default().fg(if nc { Color::Reset } else { Color::Magenta }),
        )
    } else if has_track {
        (
            "[PAUSED]",
            Style::default().fg(if nc { Color::Reset } else { Color::Yellow }),
        )
    } else {
        ("[STOPPED]", dim)
    };

    let mut spans: Vec<Span<'static>> = Vec::new();

    // State label leads the row.
    spans.push(Span::styled(state_label, state_style));
    spans.push(Span::raw(" "));

    // Source badge: [L] or [Y] (or blank when nothing playing).
    let np = app.now_playing_view();
    let src_local = if nc { Color::Reset } else { Color::Green };
    let src_yt = if nc { Color::Reset } else { Color::Yellow };
    match &np {
        Some(v) if v.source.is_remote() => {
            spans.push(Span::styled("[Y] ", Style::default().fg(src_yt)));
        }
        Some(_) => {
            spans.push(Span::styled("[L] ", Style::default().fg(src_local)));
        }
        None => {
            spans.push(Span::styled("   ", dim));
        }
    }

    spans.push(Span::styled(format!("{state_glyph} "), glyph_style));

    // Now-playing: title — artist · album (or a dim placeholder).
    // Title is the visual hero (accent color — bright + prominent).
    match &np {
        Some(v) => {
            spans.push(Span::styled(v.title.clone(), accent));
            spans.push(Span::styled(" — ", dim));
            spans.push(Span::styled(v.artist.clone(), text));
            if let Some(album) = &v.album {
                if !album.is_empty() && width > 60 {
                    spans.push(Span::styled(format!(" {} ", sep_dot()), dim));
                    spans.push(Span::styled(album.clone(), text));
                }
            }
            // Up-next preview right after the now-playing text — grouped
            // with the playback context. Only when there's room (width > 60).
            if width > 60 {
                if let Some(next) = up_next_preview(app) {
                    spans.push(Span::styled("  ", dim));
                    spans.push(Span::styled(next, dim));
                }
            }
        }
        None => {
            spans.push(Span::styled(
                format!("{} nothing playing {}", em_dash(), em_dash()),
                dim,
            ));
            // Up-next hint when nothing is playing.
            if width > 60 {
                if let Some(next) = up_next_preview(app) {
                    spans.push(Span::styled("  ", dim));
                    spans.push(Span::styled(next, dim));
                }
            }
        }
    }

    spans.push(Span::raw("   "));

    // Quality readout: local → `24-bit / 96 kHz` (+`· bit-perfect`);
    // remote → stream format label (`Opus 160k · YT` / `AAC 256k · YT Premium`).
    match &np {
        Some(v) if v.source.is_remote() => {
            let label = v
                .fmt
                .as_ref()
                .map(|f| f.yt_label())
                .unwrap_or_else(|| "YT".to_string());
            let color = if nc { Color::Reset } else { Color::Yellow };
            spans.push(Span::styled(label, Style::default().fg(color)));
        }
        Some(v) => {
            let q_color = quality_color(v.bit_depth, v.sample_rate_hz);
            let q_text = format!("{}-bit / {} kHz", v.bit_depth, khz(v.sample_rate_hz));
            spans.push(Span::styled(q_text, Style::default().fg(q_color)));
            if app.switch_sample_rate {
                spans.push(Span::styled(
                    format!(" {} bit-perfect", sep_dot()),
                    Style::default().fg(q_color),
                ));
            }
        }
        None => {
            spans.push(Span::styled("--bit / -- kHz", dim));
        }
    }

    // Volume (always visible when width >= 70 — the volume meter is a
    // persistent control, not an informational readout, so it shows even when
    // nothing is playing. T2: volume-no-numeric-in-wide — always show
    // `vol ▰▰▰▰▱▱ 70%` with the block bar + numeric percentage.)
    if width >= 70 {
        spans.push(Span::raw("   "));
        spans.push(Span::styled("vol ", dim));
        let blocks = 4u32;
        let filled = ((u32::from(app.volume) * blocks + 50) / 100).min(blocks);
        let mut vol_bar = String::new();
        for i in 0..blocks {
            vol_bar.push(if i < filled {
                filled_block()
            } else {
                empty_block()
            });
        }
        let vol_pct = if app.muted { 0 } else { app.volume };
        let vol_color = if app.muted { theme.dim } else { theme.text };
        spans.push(Span::styled(
            format!("{vol_bar} {vol_pct}%"),
            Style::default().fg(vol_color),
        ));
    }

    Line::from(spans).alignment(Alignment::Left)
}

/// Row 2 right-anchored flags: `SHUF off · RPT off · CONT off · MODE local`.
/// `·`-separated and right-anchored so they read as one rhythm; `MODE` last
/// (spec §5.1 cut #4). DEF-013: when a track is playing and its source
/// differs from `source_mode`, a `· SRC youtube` or `· SRC local` badge is
/// appended so the label doesn't contradict the actual playing source.
fn build_flags_line(app: &App, _width: usize) -> Line<'static> {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);

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
    let nc = crate::tui::view::theme::no_color();
    // DEF-013: if the now-playing track's source differs from the mode,
    // append "· SRC {actual}" so the label isn't misleading.
    let src_badge = if let Some(np) = &app.now_playing {
        let playing_src = if np.is_remote() { "youtube" } else { "local" };
        if playing_src != mode {
            let color = if nc {
                Color::Reset
            } else if np.is_remote() {
                Color::Yellow
            } else {
                Color::Green
            };
            Some(vec![
                Span::raw(format!(" {} ", sep_dot())),
                Span::styled(format!("SRC {playing_src}"), Style::default().fg(color)),
            ])
        } else {
            None
        }
    } else {
        None
    };
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("SHUF {shuf}"), dim),
        Span::raw(format!(" {} ", sep_dot())),
        Span::styled(format!("RPT {rpt}"), dim),
        Span::raw(format!(" {} ", sep_dot())),
        Span::styled(format!("CONT {cont}"), dim),
        Span::raw(format!(" {} ", sep_dot())),
        Span::styled(format!("MODE {mode}"), dim),
    ];
    if let Some(src) = src_badge {
        spans.extend(src);
    }
    Line::from(spans).alignment(Alignment::Right)
}

/// `(percent, "M:SS / M:SS")` for the progress gauge. When position/duration
/// are unavailable the gauge reads 0% with a `--:--` label.
fn progress(app: &App) -> (u16, String) {
    let pos = app.player.position();
    let dur = app.player.duration();
    match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => {
            let pct = ((p / d) * 100.0).clamp(0.0, 100.0) as u16;
            (pct, format!("{} / {}", fmt_time(p), fmt_time(d)))
        }
        _ => (0, "--:-- / --:--".to_string()),
    }
}

/// Render a `▰▰▰▰▱▱▱▱ 42%` style progress bar into `area`. The filled portion
/// (`▰`) uses the theme accent color (bold); the unfilled portion (`▱`) is
/// dim. A `42%` percentage label follows the bar so the user gets a numeric
/// readout alongside the visual bar (T2: progress-bar-low-res — block chars
/// give finer-grained resolution than the old `━━●───` line+marker style,
/// and the percentage label makes the exact position unambiguous).
///
/// Always visible — even when position/duration is unavailable (the bar is
/// all-empty `▱` and the label shows `--:-- / --:--` with `0%`), so the bar
/// never disappears in hybrid/YouTube mode. This replaces ratatui's `Gauge`
/// widget, whose unfilled portion is style-less and invisible at 0%,
/// making the bar vanish when playing YouTube tracks (T2 fix:
/// progress-bar-missing-in-hybrid). Block chars (`▰`/`▱`) match the volume
/// meter style for visual consistency across the player bar.
fn render_progress_bar(f: &mut Frame, area: Rect, app: &App) {
    let theme = Theme::default();
    let (pct, label) = progress(app);
    let pcolor = progress_color(&theme);
    let dim = Style::default().fg(theme.dim);
    let fill = Style::default().fg(pcolor).add_modifier(Modifier::BOLD);

    // Layout: [bar][space][pct%][space][label]. Reserve the right edge for
    // " {pct}% {label}" so the percentage and time label stay flush right.
    let pct_str = format!("{pct}%");
    let label_w = label.len() as u16 + pct_str.len() as u16 + 2;
    let bar_w = area.width.saturating_sub(label_w);

    let line = if bar_w < 3 {
        // Too narrow for a bar — just pct + label.
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

/// Find the currently-playing [`Track`] by `app.now_playing` id.
#[allow(dead_code)]
fn now_playing_track(app: &App) -> Option<&Track> {
    let id = app.now_playing.as_ref().map(|s| s.id())?;
    app.track_by_id_fast(id)
}
