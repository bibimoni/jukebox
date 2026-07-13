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
    buffer::CellDiffOption,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::catalog::Track;
use crate::mode::SourceMode;
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

/// RC16-DEF-1: mark EVERY cell in `area` as `AlwaysUpdate` so the diff
/// re-emits them unconditionally. This clears stale CJK double-width glyphs
/// when the now-playing track switches: a double-width char occupies 2
/// terminal cells, and when a shorter ASCII title replaces a longer CJK
/// title, ratatui's diff may skip the continuation cells (treating them as
/// equal to default spaces), leaving remnant glyphs that interleave with
/// the new text ("日i本n語g…"). Forcing all player-bar cells on every frame
/// guarantees the terminal always reflects the buffer. The player bar is
/// 1–2 rows so the cost is negligible (~100–200 cells).
fn force_area_update(f: &mut Frame, area: Rect) {
    let buf = f.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
}

/// Pick the current spinner glyph: ASCII when `JUKEBOX_FONT_MODE=ascii`,
/// braille otherwise. `spinner_frame` wraps modulo the active frame count.
/// MOD-6: the old check used `no_color()` alone, so
/// `JUKEBOX_FONT_MODE=ascii` without `NO_COLOR` still rendered braille
/// dots that may not exist in the font — `is_ascii()` covers the path.
/// D2: `NO_COLOR` no longer triggers ASCII mode, so it now correctly keeps
/// braille under `NO_COLOR=1` (colorless but Unicode).
fn spinner_glyph(app: &App) -> &'static str {
    let frames = if crate::tui::view::theme::is_ascii() {
        &SPINNER_ASCII[..]
    } else {
        &SPINNER[..]
    };
    frames[app.spinner_frame as usize % frames.len()]
}

/// RC11-DEF-015: true when a YouTube track is being resolved (cold miss,
/// `pending_play` set) or loaded but not yet playing while a resolve is in
/// flight. Drives the `[BUFFERING]` state label and the "Buffering [title]…"
/// now-playing slot so a cold-start pick (now_playing still None for the
/// ~1.3s resolve) doesn't read as `[STOPPED] — nothing playing —`.
fn is_buffering(app: &App) -> bool {
    app.pending_play.is_some()
        || (app.now_playing.is_some() && !app.player.is_playing() && app.is_resolving())
}

/// RC11-DEF-015: the title of the pending (cold-miss) YouTube track, if it's
/// cached in the session's track cache. Returns None when the metadata hasn't
/// been fetched yet (the bar falls back to a generic "Loading stream…" label).
fn buffering_title(app: &App) -> Option<String> {
    let vid = app.pending_play.as_ref()?;
    app.yt_session
        .as_ref()
        .and_then(|s| s.track_for(vid))
        .map(|t| t.title.clone())
}

/// RC11-DEF-043: the transient confirmation toast (e.g. "Added to queue"),
/// truncated to `max` display columns so it fits the up-next slot. Returns
/// None when no toast is active. The toast is rendered in the up-next slot
/// (precedence over the `Next:` preview) with an accent style so it's visible
/// regardless of `yt_state` (the `yt_status` toast was gated on Ready, so
/// local-only users never saw "added to queue").
///
/// RC13-DEF-4: the toast is wrapped in brackets `[...]` so it reads as a
/// transient message, not part of the track name. Without brackets, "Added…"
/// appeared inline between the artist and the bitrate, clobbering the
/// now-playing line.
fn toast_preview(app: &App, max: usize) -> Option<String> {
    let toast = app.toast.as_ref()?;
    // Reserve 2 cols for the brackets, truncate the inner text, then wrap.
    let inner_max = max.saturating_sub(2);
    let truncated = truncate_title(toast, inner_max);
    Some(format!("[{truncated}]"))
}

/// RC11-DEF-043: pick the up-next slot's content + style. A transient toast
/// (e.g. "Added to queue") takes precedence over the `Next:` preview and uses
/// the accent style so the confirmation is visible; otherwise the dim
/// `Next:`/`Enter to play` preview. Used by both the full and compact bars so
/// the toast renders regardless of `yt_state` (the old `yt_status` toast was
/// hidden when yt_state != Ready, so local-only users never saw it).
///
/// `avail` is the full display width available for the slot. The toast uses
/// all of it (it has no `▸ Next: ` prefix); the up-next preview reserves 8
/// cols for that prefix internally.
fn next_or_toast(app: &App, avail: usize) -> Option<(String, Style)> {
    let theme = Theme::default();
    let accent = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme.dim);
    if let Some(t) = toast_preview(app, avail) {
        return Some((t, accent));
    }
    let max_title = avail.saturating_sub(8);
    up_next_preview(app, max_title).map(|n| (n, dim))
}

/// The "Up Next" preview string: `▸ Next: {title}` when a successor is queued,
/// or `▸ Next: (end)` when the context is exhausted (so the up-next slot is
/// always visible while a track is playing — GLM: up-next-missing). When
/// nothing is playing, a `▸ Enter to play` hint fills the slot so the up-next
/// area is never empty (GLM: up-next-missing-when-stopped).
///
/// `max_title` is the max display columns for the title portion (not the whole
/// string). The title is truncated via [`truncate_title`] to fit so the
/// up-next preview doesn't push the info line under the transport controls
/// (MOD-1). When `max_title` is 0 the title is empty (but "▸ Next: " still
/// shows if there IS a next track — the slot is never blank while playing).
///
/// MAJ-1: The old implementation called `app.up_next_title()` which only
/// checked `transport.manual_queue.first()` + `track_by_id_fast()`. This
/// missed two cases: (a) context continuation (playing track 1 of 3, no
/// manual queue → showed "(end)" instead of track 2), and (b) YouTube
/// tracks in the manual queue (`track_by_id_fast` fails on remote ids →
/// returned None → showed "(end)"). Now we use `transport.peek_next()`
/// which mirrors the actual next-track logic (context → manual queue →
/// repeat), then resolve the title via the local catalog or the YouTube
/// session's track cache, falling back to "Loading…" for uncached remote
/// tracks.
fn up_next_preview(app: &App, max_title: usize) -> Option<String> {
    match app.transport.peek_next(app, &app.catalog) {
        Some(id) => {
            let title = resolve_next_track_title(app, &id);
            let trimmed = truncate_title(&title, max_title);
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

/// Resolve the display title for a track id — local catalog first, then the
/// YouTube session's track cache. Returns a "Loading…" placeholder when the
/// track is a YouTube video whose metadata hasn't been fetched yet (MAJ-1).
fn resolve_next_track_title(app: &App, id: &str) -> String {
    if let Some(t) = app.track_by_id_fast(id) {
        return t.title.clone();
    }
    if let Some(rt) = app.yt_session.as_ref().and_then(|s| s.track_for(id)) {
        return rt.title.clone();
    }
    format!("Loading{}", ellipsis())
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

/// Total display width of all spans' content. Used to compute how much room
/// remains for the up-next preview so it can be truncated to fit (MOD-1).
fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| disp_width(s.content.as_ref())).sum()
}

/// Display width of the compact YT auth indicator (` · YT` = 5, ` · YT!` = 6).
/// Returns 0 when the indicator won't be shown (Local mode + not YT view, or
/// Unconfigured) so the width budget doesn't over-reserve (MOD-3).
fn compact_yt_indicator_width(app: &App) -> usize {
    use crate::tui::app::View;
    use crate::yt::state::YtState;
    let yt_relevant = app.source_mode != SourceMode::Local || app.view == View::Youtube;
    if !yt_relevant || app.yt_state == YtState::Unconfigured {
        return 0;
    }
    // " {sd} {label}" → " · YT" (5) .. " · YT…" (6). Use 6 as a safe upper bound.
    6
}

/// Display width of the compact source badge (` · [Y]` / ` · [L]` = 6).
/// Returns 0 when the badge won't be shown (playing source matches mode).
fn compact_src_badge_width(app: &App) -> usize {
    if let Some(np) = &app.now_playing {
        let playing_src = if np.is_remote() { "youtube" } else { "local" };
        if playing_src != app.source_mode.as_str() {
            return 6; // " · [Y]" or " · [L]"
        }
    }
    0
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
    // RC11-DEF-015: buffering state (cold-miss YouTube pick or loaded-but-
    // not-playing with a resolve in flight) — distinct from stopped/paused.
    let buffering = is_buffering(app);
    // State convention: ▶ = playing, ⏸ = paused, ■ = stopped (Issue 1: the
    // old logic showed ⏸ while playing and ▶ while paused — backwards — and
    // □ for stopped which reads as a stop button. Now the glyph reflects the
    // CURRENT state, matching the [PLAYING]/[PAUSED]/[STOPPED] label.)
    let state_glyph = if resolving || buffering {
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
    let glyph_style = if resolving || buffering {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if !has_track {
        dim
    } else {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    };
    // State label: [PLAYING]/[PAUSED]/[STOPPED]/[BUFFERING] — the clearest
    // single signal of the playback state, leading the row (GLM:
    // state-label-missing). Colors: Magenta for playing (attention), Yellow
    // for paused (caution), Cyan+bold for buffering, dim for stopped. All
    // collapse to Reset under NO_COLOR (the label text distinguishes states
    // without color).
    let nc = crate::tui::view::theme::no_color();
    let (state_label, state_style) = if buffering {
        (
            "[BUFFERING]",
            Style::default()
                .fg(if nc { Color::Reset } else { Color::Cyan })
                .add_modifier(Modifier::BOLD),
        )
    } else if playing {
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
            // RC15-DEF-5 (ACC-02): at 80×24 the compact bar hard-cut long titles
            // mid-word with no ellipsis. Compute the available width for
            // title + " — " + artist, reserving space for the progress label
            // and the trailing essentials (PREF + YT indicator + source
            // badge). Truncate title (and artist) with `…` so the cut is
            // clean. Mirrors `build_info_line`'s title_budget approach.
            let prefix_w = spans_width(&spans);
            let sep_str = format!(" {} ", em_dash());
            let sep_w = disp_width(&sep_str);
            let (_pct, plabel) = progress(app);
            let progress_w = disp_width(&format!("  {plabel}"));
            let mode_str = app.source_mode.as_str();
            let mode_w = disp_width(&format!("  PREF {mode_str}"));
            let yt_w = compact_yt_indicator_width(app);
            let badge_w = compact_src_badge_width(app);
            let reserved = mode_w + yt_w + badge_w + progress_w;
            // Reserve a minimum 4-col slot for the artist so long titles
            // still show "Title… — Ar…" (not just "Title…").
            let artist_min = 4usize.min(disp_width(&v.artist));
            let title_budget =
                (area.width as usize).saturating_sub(prefix_w + sep_w + artist_min + reserved + 2);
            if title_budget >= 1 {
                let title_w = disp_width(&v.title);
                let artist_w = disp_width(&v.artist);
                let (title_disp, artist_disp) = if title_w + sep_w + artist_w
                    <= (area.width as usize).saturating_sub(prefix_w + reserved)
                {
                    (v.title.clone(), v.artist.clone())
                } else if title_w <= title_budget {
                    let artist_max =
                        (area.width as usize).saturating_sub(prefix_w + title_w + sep_w + reserved);
                    (v.title.clone(), truncate_title(&v.artist, artist_max))
                } else {
                    (
                        truncate_title(&v.title, title_budget),
                        truncate_title(&v.artist, artist_min),
                    )
                };
                spans.push(Span::styled(title_disp, text));
                spans.push(Span::styled(sep_str.clone(), dim));
                spans.push(Span::styled(artist_disp, text));
            } else {
                // Extremely narrow — just the title, truncated.
                let budget = (area.width as usize)
                    .saturating_sub(prefix_w + reserved)
                    .max(1);
                spans.push(Span::styled(truncate_title(&v.title, budget), text));
            }
        }
        None => {
            // RC11-DEF-015: show "Buffering [title]…" when a cold-miss YouTube
            // pick is resolving; else the dim "— nothing playing —" placeholder
            // (or the RC11-DEF-014 resume hint when a last-played track is saved).
            if buffering {
                let title = buffering_title(app).unwrap_or_else(|| "Loading stream".to_string());
                spans.push(Span::styled(
                    format!("Buffering {title}{}", ellipsis()),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if let Some(hint) = app.resume_hint.as_ref() {
                spans.push(Span::styled(
                    format!("{} {}", marker_glyph(), hint),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{dash} nothing playing {dash}", dash = em_dash()),
                    dim,
                ));
            }
        }
    }
    // Progress text (always visible — essential playback info; T3:
    // status-drops-indicators — keep title + progress + mode at small size).
    // Compact `M:SS / M:SS` label fits on one line where the gauge wouldn't.
    {
        let (_pct, plabel) = progress(app);
        spans.push(Span::styled(format!("  {plabel}"), dim));
    }
    // MOD-3: Reserve width for the essential trailing items (PREF + YT auth
    // indicator + source badge) so they're always visible at 80×24 even when
    // the up-next preview or quality readout would push them off-screen. The
    // up-next and quality are only added if there's room after reserving.
    let mode_str = app.source_mode.as_str();
    let mode_w = disp_width(&format!("  PREF {mode_str}"));
    let yt_w = compact_yt_indicator_width(app);
    let badge_w = compact_src_badge_width(app);
    let reserved = mode_w + yt_w + badge_w;

    // Up-next preview (width > 60) — so the narrow bar also shows what's
    // queued, matching the full bar's up-next slot (GLM:
    // up-next-missing-in-compact). Grouped right after the now-playing text.
    // Truncated to fit the remaining width after reserving the trailing
    // essentials (MOD-1/MOD-3). The "(end)" and "Enter to play" hints are
    // fixed-length — only add them if they actually fit in `avail`.
    if area.width > 60 {
        let used = spans_width(&spans);
        let avail = (area.width as usize).saturating_sub(used + reserved + 2);
        // RC11-DEF-043: a toast shows even in a crowded bar (truncated to fit);
        // the up-next preview needs more room (8-col "▸ Next: " prefix), so its
        // threshold is 10.
        let threshold = if app.toast.is_some() { 4 } else { 10 };
        if avail >= threshold {
            if let Some((next, nstyle)) = next_or_toast(app, avail) {
                if disp_width(&next) <= avail {
                    spans.push(Span::styled("  ", dim));
                    spans.push(Span::styled(next, nstyle));
                }
            }
        }
    }
    // quality (drop below 80 cols — at narrow widths the bit depth / sample
    // rate readout crowds the status bar; keep title + play glyph + progress
    // text + mode as the essential minimum, drop quality + volume).
    // MOD-3: also check that adding quality won't push the trailing essentials
    // (MODE + YT indicator) off-screen.
    if area.width >= 80 {
        let used = spans_width(&spans);
        let q_w = match &np {
            Some(v) if v.source.is_remote() => disp_width(
                &v.fmt
                    .as_ref()
                    .map(|f| f.yt_label())
                    .unwrap_or_else(|| "YT".to_string()),
            ),
            Some(v) => disp_width(&format!("{}bit/{}", v.bit_depth, khz(v.sample_rate_hz))),
            None => 2, // "--"
        } + 2; // "  " prefix
        if used + q_w + reserved < area.width as usize {
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
    }
    // PREF always visible (essential — shows user source preference
    // local/yt/mixed). The label was renamed MODE → PREF (RC19-D4) so it
    // reads as a preference, not the actual playing source; `SRC` (added
    // below when the playing source differs) is the actual source.
    // CONT drops below 70 cols (lower priority). T3: keep title+progress+pref
    // at small sizes so the status bar never loses essential indicators.
    // DEF-004: SHUF/RPT also visible at >= 80 cols so the compact bar (used
    // at height <= 24) shows playback state indicators alongside PREF+CONT.
    spans.push(Span::raw("  "));
    spans.push(Span::styled(format!("PREF {mode_str}"), dim));
    // DEF-011/DEF-012: compact YT auth indicator — always visible at >= 70
    // cols so the YouTube connection state is visible at all terminal sizes,
    // not just in the 2-row footer (which requires >100 width or >24 height).
    // DEF-013: when a track is playing, also show the actual source if it
    // differs from the mode (e.g., "PREF local" while a YT track plays →
    // "PREF local · [Y]" so the label isn't misleading).
    // MOD-3: the YT indicator is always added when relevant (width >= 70 +
    // yt_relevant + non-Unconfigured) — the width budget above reserves space
    // for it so earlier content is truncated/dropped instead.
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
                use crate::tui::app::View;
                app.source_mode != SourceMode::Local || app.view == View::Youtube
            };
            if yt_relevant && app.yt_state != YtState::Unconfigured {
                let (label, color): (String, Color) = match app.yt_state {
                    YtState::Ready => ("YT".into(), if nc { Color::Reset } else { Color::Green }),
                    YtState::AuthExpired | YtState::ProviderError | YtState::Failed => {
                        ("YT!".into(), if nc { Color::Reset } else { Color::Red })
                    }
                    YtState::RateLimited | YtState::ReadyStale => {
                        ("YT~".into(), if nc { Color::Reset } else { Color::Yellow })
                    }
                    _ => (
                        format!("YT{}", ellipsis()),
                        if nc { Color::Reset } else { Color::Yellow },
                    ),
                };
                spans.push(Span::styled(
                    format!(" {sd} {label}"),
                    Style::default().fg(color),
                ));
            }
        }
    }
    // CONT / SHUF / RPT — low priority, only if there's room (MOD-3: don't
    // let these push the YT indicator off-screen; they come after it).
    {
        let used = spans_width(&spans);
        let remaining = (area.width as usize).saturating_sub(used);
        if area.width >= 70 {
            let cont = match app.transport.continue_mode {
                ContinueMode::Off => "off",
                ContinueMode::NextAlbum => "next",
                ContinueMode::Radio => "radio",
                ContinueMode::YouTube => "youtube",
            };
            let cont_w = disp_width(&format!(" {sd} CONT {cont}"));
            if remaining >= cont_w {
                spans.push(Span::styled(format!(" {sd} CONT {cont}"), dim));
            }
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
            let shuf_w = disp_width(&format!(" {sd} SHUF {shuf}"));
            let used = spans_width(&spans);
            let rem = (area.width as usize).saturating_sub(used);
            if rem >= shuf_w {
                spans.push(Span::styled(format!(" {sd} SHUF {shuf}"), dim));
            }
            let rpt_w = disp_width(&format!(" {sd} RPT {rpt}"));
            let used = spans_width(&spans);
            let rem = (area.width as usize).saturating_sub(used);
            if rem >= rpt_w {
                spans.push(Span::styled(format!(" {sd} RPT {rpt}"), dim));
            }
        }
    }
    // DEF-051: strip trailing separator so PREF mixed doesn't end with dot.
    let sd = sep_dot();
    while let Some(last) = spans.last() {
        let trimmed = last.content.trim();
        if trimmed == sd || trimmed.is_empty() {
            spans.pop();
        } else {
            break;
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(spans).alignment(Alignment::Left))
            .block(Block::default().borders(Borders::NONE)),
        area,
    );
    // RC16-DEF-1: force all compact bar cells so stale CJK glyphs are cleared.
    force_area_update(f, area);
}

/// Render the player bar into `area` using state from `app`. Two rows:
/// row 1 = now-playing + quality + volume; row 2 = progress bar + mode
/// flags (SHUF · RPT · CONT · PREF), `·`-separated, right-anchored.
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
    // RC14-DEF-3: reserve 1 extra col so the volume percentage ("70%") and
    // the transport glyphs ("◀◀") always have a ≥1-space gap between them
    // (the old budget let the info line fill right up to the transport start,
    // producing "70%◀◀" with 0 spaces when the title was long).
    let line = build_info_line(app, info_area.width.saturating_sub(15) as usize);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        info_area,
    );
    // RC16-DEF-1: force all info-area cells so stale CJK glyphs are cleared.
    force_area_update(f, info_area);

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
        // DEF-048: 1-col gap so progress time label never runs into SHUF.
        let flags_area = Rect::new(
            geo.progress.right() + 1,
            g.y,
            g.width.saturating_sub(geo.progress.width + 1),
            g.height,
        );
        f.render_widget(
            Paragraph::new(build_flags_line(app, flags_area.width as usize))
                .block(Block::default().borders(Borders::NONE))
                .alignment(Alignment::Right),
            flags_area,
        );
        // RC16-DEF-1: force the gauge row too so stale glyphs on row 2 clear.
        force_area_update(f, g);
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
    // RC11-DEF-015: a cold-miss YouTube pick (pending_play set, now_playing
    // still None for the ~1.3s resolve) or a loaded-but-not-yet-playing track
    // with a resolve in flight is "buffering" — distinct from stopped and
    // paused. The bar shows [BUFFERING] + the spinner + "Buffering [title]…".
    let buffering = is_buffering(app);
    // State convention: ▶ = playing, ⏸ = paused, ■ = stopped (Issue 1: the
    // old logic showed ⏸ while playing and ▶ while paused — backwards — and
    // □ for stopped which reads as a stop button. Now the glyph reflects the
    // CURRENT state, matching the [PLAYING]/[PAUSED]/[STOPPED] label.)
    let state_glyph = if resolving || buffering {
        spinner_glyph(app)
    } else if playing && has_track {
        play_glyph()
    } else if has_track {
        pause_glyph()
    } else {
        stop_glyph()
    };
    let glyph_style = if resolving || buffering {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if !has_track {
        dim
    } else {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    };

    // State label: [PLAYING]/[PAUSED]/[STOPPED]/[BUFFERING] — the clearest
    // single signal of the playback state. Magenta for playing, Yellow for
    // paused, Cyan+bold for buffering (an active wait, not a stopped state),
    // dim for stopped. All collapse to Reset under NO_COLOR.
    let (state_label, state_style) = if buffering {
        (
            "[BUFFERING]",
            Style::default()
                .fg(if nc { Color::Reset } else { Color::Cyan })
                .add_modifier(Modifier::BOLD),
        )
    } else if playing && has_track {
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

    // DEF-004: build quality and volume as SEPARATE Vecs so the title can be
    // truncated and trailing dropped in priority order (volume, then quality).
    let mut quality_part: Vec<Span<'static>> = Vec::new();
    quality_part.push(Span::raw("   "));
    match &np {
        Some(v) if v.source.is_remote() => {
            let label = v
                .fmt
                .as_ref()
                .map(|f| f.yt_label())
                .unwrap_or_else(|| "YT".to_string());
            let color = if nc { Color::Reset } else { Color::Yellow };
            quality_part.push(Span::styled(label, Style::default().fg(color)));
        }
        Some(v) => {
            let q_color = quality_color(v.bit_depth, v.sample_rate_hz);
            let q_text = format!("{}-bit / {} kHz", v.bit_depth, khz(v.sample_rate_hz));
            quality_part.push(Span::styled(q_text, Style::default().fg(q_color)));
            if app.switch_sample_rate {
                quality_part.push(Span::styled(
                    format!(" {} bit-perfect", sep_dot()),
                    Style::default().fg(q_color),
                ));
            }
        }
        None => {
            quality_part.push(Span::styled("--bit / -- kHz", dim));
        }
    }
    let q_w = spans_width(&quality_part);

    let mut volume_part: Vec<Span<'static>> = Vec::new();
    if width >= 70 {
        volume_part.push(Span::raw("   "));
        volume_part.push(Span::styled("vol ", dim));
        if app.muted {
            volume_part.push(Span::styled("MUTED ", Style::default().fg(theme.dim)));
        }
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
        volume_part.push(Span::styled(
            format!("{vol_bar} {vol_pct}%"),
            Style::default().fg(vol_color),
        ));
    }
    let v_w = spans_width(&volume_part);
    let prefix_w = spans_width(&spans);

    // Now-playing: title — artist · album (or a dim placeholder).
    // Title is the visual hero (accent color — bright + prominent).
    match &np {
        Some(v) => {
            // DEF-004: compute full now-playing width, decide which trailing
            // segments fit. Drop volume first, then quality. Truncate title
            // last (highest priority — always visible).
            let title_w = disp_width(&v.title);
            let sep = format!(" {} ", em_dash());
            let sep_w = disp_width(&sep);
            let artist_w = disp_width(&v.artist);
            let show_album = v
                .album
                .as_ref()
                .map(|a| !a.is_empty() && width > 60)
                .unwrap_or(false);
            let album = v.album.as_deref().unwrap_or("");
            let album_sep = format!(" {} ", sep_dot());
            let album_sep_w = disp_width(&album_sep);
            let album_w = disp_width(album);
            let np_full_w =
                title_w + sep_w + artist_w + (if show_album { album_sep_w + album_w } else { 0 });

            // DEF-004: truncate title to fit with max trailing reserved,
            // then compute actual trailing inclusion AFTER up-next so the
            // total never exceeds width (the up-next preview can push the
            // total over, so trailing is re-evaluated based on real used).
            let max_trailing_w = q_w + v_w;
            let title_budget = width.saturating_sub(prefix_w + max_trailing_w);

            if np_full_w <= title_budget {
                spans.push(Span::styled(v.title.clone(), accent));
                spans.push(Span::styled(sep, dim));
                spans.push(Span::styled(v.artist.clone(), text));
                if show_album {
                    spans.push(Span::styled(album_sep, dim));
                    spans.push(Span::styled(album.to_string(), text));
                }
            } else if title_w + sep_w + artist_w <= title_budget {
                spans.push(Span::styled(v.title.clone(), accent));
                spans.push(Span::styled(sep, dim));
                spans.push(Span::styled(v.artist.clone(), text));
            } else {
                // RC13-DEF-4: always show title + artist, both truncated to
                // fit. The old code dropped the artist entirely when the
                // title was too long, so a long-titled track showed only
                // "A Very Long Track Title That Shoul…" with no artist.
                // Reserve a minimum slot for the artist (truncated) so the
                // format is always "Title… — Arti…" at any width.
                let artist_min = 8usize.min(artist_w);
                let title_max = title_budget.saturating_sub(sep_w + artist_min);
                if title_max >= 1 {
                    let (title_disp, artist_disp) = if title_w <= title_max {
                        let artist_max = title_budget.saturating_sub(title_w + sep_w);
                        (v.title.clone(), truncate_title(&v.artist, artist_max))
                    } else {
                        (
                            truncate_title(&v.title, title_max),
                            truncate_title(&v.artist, artist_min),
                        )
                    };
                    spans.push(Span::styled(title_disp, accent));
                    spans.push(Span::styled(sep, dim));
                    spans.push(Span::styled(artist_disp, text));
                } else {
                    // Extremely narrow — just the title, truncated.
                    let t = truncate_title(&v.title, title_budget.max(1));
                    spans.push(Span::styled(t, accent));
                }
            }

            // Up-next preview — DEF-057: shows during PLAYING too.
            // Budget against max_trailing_w so up-next won't steal trailing room.
            // RC13-DEF-4: when a toast is active, it takes precedence over
            // the trailing quality/volume (the toast is a transient ~1.2s
            // confirmation; briefly hiding quality is fine). Don't reserve
            // max_trailing_w for the toast path so it gets enough room.
            if width > 60 {
                let used = spans_width(&spans);
                let reserve = if app.toast.is_some() {
                    0
                } else {
                    max_trailing_w
                };
                let avail = width.saturating_sub(used + reserve + 2);
                let threshold = if app.toast.is_some() { 4 } else { 10 };
                if avail >= threshold {
                    if let Some((next, nstyle)) = next_or_toast(app, avail) {
                        if disp_width(&next) <= avail {
                            spans.push(Span::styled("  ", dim));
                            spans.push(Span::styled(next, nstyle));
                        }
                    }
                }
            }

            // DEF-004: compute actual trailing based on real used width
            // (after up-next was added). Drop volume first, then quality.
            let used = spans_width(&spans);
            let trailing = if used + q_w + v_w <= width {
                let mut t = quality_part.clone();
                t.extend(volume_part.clone());
                t
            } else if used + q_w <= width {
                quality_part.clone()
            } else {
                Vec::new()
            };
            spans.extend(trailing);
        }
        None => {
            // RC13-DEF-4: the resume hint can be long ("resume: Midnight
            // Journey at 0:03 · R to resume" = ~40 chars). Truncate it to the
            // title budget so it doesn't push the quality/volume/transport
            // controls off-screen — the bar layout stays identical to the
            // normal STOPPED state, with the hint filling the title slot.
            let trailing_w = q_w + v_w;
            let title_budget = width.saturating_sub(prefix_w + trailing_w);
            if buffering {
                let title = buffering_title(app).unwrap_or_else(|| "Loading stream".to_string());
                let label = format!("Buffering {title}{}", ellipsis());
                let label = truncate_title(&label, title_budget);
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if let Some(hint) = app.resume_hint.as_ref() {
                let label = format!("{} {}", marker_glyph(), hint);
                let label = truncate_title(&label, title_budget);
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{} nothing playing {}", em_dash(), em_dash()),
                    dim,
                ));
            }
            // DEF-057: up-next hint when nothing is playing too.
            if width > 60 {
                let used = spans_width(&spans);
                let avail = width.saturating_sub(used + trailing_w + 2);
                let threshold = if app.toast.is_some() { 4 } else { 10 };
                if avail >= threshold {
                    if let Some((next, nstyle)) = next_or_toast(app, avail) {
                        if disp_width(&next) <= avail {
                            spans.push(Span::styled("  ", dim));
                            spans.push(Span::styled(next, nstyle));
                        }
                    }
                }
            }
            let mut trailing = quality_part.clone();
            trailing.extend(volume_part.clone());
            spans.extend(trailing);
        }
    }

    Line::from(spans).alignment(Alignment::Left)
}

/// Abbreviated shuffle mode for narrow flag areas (MOD-2). Keeps "off" as-is
/// (already 3 chars); "smart"→"smt", "random"→"rnd" so the flags line fits at
/// 100×30 where the flags area is only 45 cols.
fn abbrev_shuf(m: ShuffleMode) -> &'static str {
    match m {
        ShuffleMode::Off => "off",
        ShuffleMode::Smart => "smt",
        ShuffleMode::Random => "rnd",
    }
}

/// Abbreviated continue mode for narrow flag areas (MOD-2). "next"→"nxt",
/// "radio"→"rad", "youtube"→"yt".
fn abbrev_cont(m: ContinueMode) -> &'static str {
    match m {
        ContinueMode::Off => "off",
        ContinueMode::NextAlbum => "nxt",
        ContinueMode::Radio => "rad",
        ContinueMode::YouTube => "yt",
    }
}

/// Abbreviated source mode for narrow flag areas (MOD-2). "youtube"→"yt",
/// "mixed"→"mix", "local" stays.
fn abbrev_mode(m: SourceMode) -> &'static str {
    match m {
        SourceMode::Local => "local",
        SourceMode::Youtube => "yt",
        SourceMode::Mixed => "mix",
    }
}

/// Abbreviate a source string ("youtube"→"yt", "local"→"local") for the
/// SRC badge in narrow flag areas (MOD-2).
fn abbrev_src_str(s: &str) -> &'static str {
    match s {
        "youtube" => "yt",
        _ => "local",
    }
}

/// Row 2 right-anchored flags: `SHUF off · RPT off · CONT off · PREF local`.
/// `·`-separated and right-anchored so they read as one rhythm; `PREF` last
/// (spec §5.1 cut #4). DEF-013: when a track is playing and its source
/// differs from `source_mode`, a `· SRC youtube` or `· SRC local` badge is
/// appended so the label doesn't contradict the actual playing source.
/// RC19-D4: the user-preference label was renamed `MODE` → `PREF` so it
/// reads as the user's preference (not the actual source); `SRC` covers
/// the actual source.
///
/// MOD-2: at 100×30 the flags area is 45 cols. The full line
/// "SHUF smart · RPT off · CONT off · PREF youtube" is 46 cols, which
/// truncates "PREF youtube" to "PREF youtu" (mid-word). When the full line
/// exceeds `width`, the values are abbreviated (smart→smt, youtube→yt, etc.)
/// so every word remains complete.
fn build_flags_line(app: &App, width: usize) -> Line<'static> {
    let theme = Theme::default();
    let dim = Style::default().fg(theme.dim);
    let nc = crate::tui::view::theme::no_color();

    let shuf_full = match app.transport.shuffle {
        ShuffleMode::Off => "off",
        ShuffleMode::Smart => "smart",
        ShuffleMode::Random => "random",
    };
    let rpt = match app.transport.repeat {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    };
    let cont_full = match app.transport.continue_mode {
        ContinueMode::Off => "off",
        ContinueMode::NextAlbum => "next",
        ContinueMode::Radio => "radio",
        ContinueMode::YouTube => "youtube",
    };
    let mode_full = app.source_mode.as_str();
    let sd = sep_dot();
    let sep = format!(" {} ", sd);

    // Build the full flags line and measure it. If it exceeds `width`,
    // switch to abbreviated values (MOD-2).
    let full_line =
        format!("SHUF {shuf_full}{sep}RPT {rpt}{sep}CONT {cont_full}{sep}PREF {mode_full}");
    let abbrev = width > 0 && disp_width(&full_line) > width;

    let shuf = if abbrev {
        abbrev_shuf(app.transport.shuffle)
    } else {
        shuf_full
    };
    let cont = if abbrev {
        abbrev_cont(app.transport.continue_mode)
    } else {
        cont_full
    };
    let mode = if abbrev {
        abbrev_mode(app.source_mode)
    } else {
        mode_full
    };

    // DEF-013: if the now-playing track's source differs from the mode,
    // append "· SRC {actual}" so the label isn't misleading. Compare full
    // forms for the decision; abbreviate the display when in abbreviation mode.
    let src_badge = if let Some(np) = &app.now_playing {
        let playing_src_full = if np.is_remote() { "youtube" } else { "local" };
        if playing_src_full != mode_full {
            let playing_disp = if abbrev {
                abbrev_src_str(playing_src_full)
            } else {
                playing_src_full
            };
            let color = if nc {
                Color::Reset
            } else if np.is_remote() {
                Color::Yellow
            } else {
                Color::Green
            };
            Some(vec![
                Span::raw(sep.clone()),
                Span::styled(format!("SRC {playing_disp}"), Style::default().fg(color)),
            ])
        } else {
            None
        }
    } else {
        None
    };
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("SHUF {shuf}"), dim),
        Span::raw(sep.clone()),
        Span::styled(format!("RPT {rpt}"), dim),
        Span::raw(sep.clone()),
        Span::styled(format!("CONT {cont}"), dim),
        Span::raw(sep.clone()),
        Span::styled(format!("PREF {mode}"), dim),
    ];
    if let Some(src) = src_badge {
        spans.extend(src);
    }
    Line::from(spans).alignment(Alignment::Right)
}

/// `(percent, "M:SS / M:SS")` for the progress gauge. When position/duration
/// are unavailable the gauge reads 0% with a `--:--` label.
///
/// RC14-DEF-4: for hi-res (≥192 kHz) local tracks, mpv's `time-pos` property
/// advances erratically (~1/7 real-time observed on 24/192 FLAC) — likely a
/// coreaudio/mpv reporting quirk for non-standard sample rates. afplay
/// reports `None` always. In both cases fall back to a wall-clock estimate
/// (`App::estimated_position`: start offset + elapsed − paused) when the
/// player's `position()` is `None` OR the now-playing track is hi-res local.
/// The duration still comes from the player backend (mpv reports it
/// correctly for hi-res; afplay has none → the label stays `--:--`).
fn progress(app: &App) -> (u16, String) {
    // RC11-DEF-041: when stopped (no now-playing track), always show a clean
    // `0% --:-- / --:--` — even if the player backend still reports the last
    // track's position/duration (StubPlayer keeps them after `stop`; mpv may
    // lag before its `end-file` resets the cache). Without this guard the bar
    // displayed a stale `33% 0:02 / 0:06` after the track ended, looking like
    // the track was still playing.
    if app.now_playing.is_none() {
        return (0, "--:-- / --:--".to_string());
    }
    let dur = app.player.duration();
    // RC14-DEF-4: decide whether to trust the player's reported position or
    // use the wall-clock estimate. Prefer the estimate when:
    //   - the player reports no position (afplay), OR
    //   - the now-playing track is a hi-res (≥192 kHz) local file (mpv's
    //     time-pos is erratic for hi-res).
    let prefer_estimate = app.player.position().is_none() || is_hires_local(app);
    let pos = if prefer_estimate {
        app.estimated_position()
    } else {
        app.player.position()
    };
    match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => {
            let pct = ((p / d) * 100.0).clamp(0.0, 100.0) as u16;
            (pct, format!("{} / {}", fmt_time(p), fmt_time(d)))
        }
        _ => (0, "--:-- / --:--".to_string()),
    }
}

/// RC14-DEF-4: true when the now-playing track is a local file with a sample
/// rate ≥ 192 kHz. mpv's `time-pos` property advances erratically for such
/// hi-res files (~1/7 real-time on 24/192 FLAC), so the progress bar should
/// use the wall-clock estimate instead. Remote (YouTube) tracks are never
/// affected (mpv streams them at 48 kHz).
fn is_hires_local(app: &App) -> bool {
    let Some(np) = &app.now_playing else {
        return false;
    };
    if np.is_remote() {
        return false;
    }
    // Local track: look up the catalog entry for its sample rate.
    let crate::source::TrackSource::Local { track_id } = np else {
        return false;
    };
    app.track_by_id_fast(track_id)
        .map(|t| t.sample_rate_hz >= 192_000)
        .unwrap_or(false)
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

#[cfg(test)]
mod mod_tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::mode::SourceMode;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use crate::tui::queue::ShuffleMode;
    use crate::yt::state::YtState;
    use ratatui::{backend::TestBackend, Terminal};

    /// Two-track catalog: t1 short title, t2 very long title (for MOD-1).
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
              {"id":"t2","artists":["Bop"],"primary_artist":"Bop","title":"A Very Long Track Title That Exceeds Available Width For Sure","album":"Beep","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/02.flac","symlinked_into_artists":["Bop"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        (d, Catalog::load(&p).unwrap())
    }

    /// Render the full 2-row player bar into a flat string.
    fn rendered_bar(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), app)).unwrap();
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

    /// Render the 1-row compact player bar into a flat string.
    fn rendered_compact(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_compact(f, f.area(), app)).unwrap();
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

    /// MOD-1: When the next-track title is long, the "Next:" preview must be
    /// truncated to fit the available info-line width so the info content
    /// doesn't run under the transport controls (rightmost 14 cols). The
    /// control area must contain only transport glyphs and spaces — no info
    /// text (title, quality, volume) leaking through.
    #[test]
    fn mod1_long_next_title_truncated_before_transport_controls() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.transport.enqueue("t2".into());
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_bar(&app, 100, 2);
        // Row 0 is the info line + transport controls.
        let row0 = bar.lines().next().unwrap();
        // The rightmost 14 cols (86..100) are the transport control area.
        let controls_area: String = row0.chars().skip(86).collect();
        // Transport controls must be present there.
        assert!(
            controls_area.contains("◀◀"),
            "MOD-1: transport prev must be in the controls area: {bar}"
        );
        // No info text (alphanumeric chars from quality/volume/title) should
        // leak into the control area — only transport glyphs and spaces.
        let info_leak: String = controls_area
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        assert!(
            info_leak.is_empty(),
            "MOD-1: info text leaked into transport controls area: \
             controls_area={controls_area:?} leaked={info_leak:?}\nfull bar: {bar}"
        );
    }

    /// MOD-2: At 100×30 the flags area is 45 cols. With SHUF=smart and
    /// PREF=youtube the full flags line is 46 cols, which truncates
    /// "PREF youtube" to "PREF youtu" (mid-word). The flags must either
    /// abbreviate or fit so every word is complete.
    #[test]
    fn mod2_flags_fit_at_100_cols_smart_shuffle_youtube() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.transport.shuffle = ShuffleMode::Smart;
        app.source_mode = SourceMode::Youtube;
        let bar = rendered_bar(&app, 100, 2);
        assert!(
            bar.contains("PREF"),
            "MOD-2: PREF flag must be present: {bar}"
        );
        // Must show a complete mode word — "youtube" (fits) or "yt" (abbreviated).
        // "youtu" without trailing letter = mid-word truncation = bug.
        let has_complete_mode = bar.contains("PREF youtube") || bar.contains("PREF yt");
        assert!(
            has_complete_mode,
            "MOD-2: PREF must show a complete word (youtube or yt), not truncated mid-word: {bar}"
        );
        assert!(
            bar.contains("SHUF"),
            "MOD-2: SHUF flag must be present: {bar}"
        );
        let has_complete_shuf = bar.contains("SHUF smart") || bar.contains("SHUF smt");
        assert!(
            has_complete_shuf,
            "MOD-2: SHUF must show a complete word (smart or smt), not truncated: {bar}"
        );
    }

    /// RC19-D4: the mini bar must show `PREF` (not `MODE`) for the user
    /// source-preference label. The SRC badge behavior is unchanged by
    /// the rename (covered by the big bar test); this test focuses on the
    /// label swap. A remote track under PREF=local is used so the bar is
    /// in the "playing source differs from preference" state, but we
    /// don't assert on the SRC badge here (at 100×2 the flags area is
    /// only 44 cols and the SRC badge can be clipped by the right edge —
    /// a pre-existing layout quirk unrelated to the rename).
    #[test]
    fn rc19_d4_mini_bar_pref_label_and_src_badge() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // Play a remote (YouTube) track under PREF=local so the bar is in
        // the "source differs from preference" state.
        app.now_playing = Some(crate::source::TrackSource::Remote {
            video_id: "v1".into(),
        });
        // source_mode defaults to Local.
        let bar = rendered_bar(&app, 100, 2);
        assert!(
            bar.contains("PREF local"),
            "RC19-D4: mini bar must show 'PREF local' (not MODE): {bar}"
        );
        assert!(
            !bar.contains("MODE "),
            "RC19-D4: mini bar must NOT show 'MODE ' label anymore: {bar}"
        );
        // Wider terminal: flags area is wide enough for the full line incl.
        // the SRC badge. Verify the SRC badge appears at 140 cols.
        let bar_wide = rendered_bar(&app, 140, 2);
        let has_src_badge = bar_wide.contains("SRC youtube") || bar_wide.contains("SRC yt");
        assert!(
            has_src_badge,
            "RC19-D4: mini bar at 140 cols must show 'SRC youtube' (or 'SRC yt') when a YT track plays under PREF=local: {bar_wide}"
        );
        assert!(
            bar_wide.contains("PREF local"),
            "RC19-D4: mini bar at 140 cols must still show 'PREF local': {bar_wide}"
        );
        assert!(
            !bar_wide.contains("MODE "),
            "RC19-D4: mini bar at 140 cols must NOT show 'MODE ' label: {bar_wide}"
        );
    }

    /// RC19-D4: the compact 1-row bar must also show `PREF` (not `MODE`).
    #[test]
    fn rc19_d4_compact_bar_pref_label() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        // source_mode defaults to Local; t1 is a local track, so no SRC badge.
        let bar = rendered_compact(&app, 80, 1);
        assert!(
            bar.contains("PREF local"),
            "RC19-D4: compact bar must show 'PREF local': {bar}"
        );
        assert!(
            !bar.contains("MODE "),
            "RC19-D4: compact bar must NOT show 'MODE ' label: {bar}"
        );
    }

    /// MOD-3: At 80×24 the compact player bar must show the YT auth indicator
    /// (at least "YT" in compact form) so the YouTube connection state is
    /// visible even on narrow terminals.
    #[test]
    fn mod3_yt_auth_indicator_visible_at_80x24_compact() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        app.source_mode = SourceMode::Youtube;
        app.yt_state = YtState::Ready;
        let bar = rendered_compact(&app, 80, 1);
        assert!(
            bar.contains("YT"),
            "MOD-3: YT auth indicator must be visible at 80x24 compact bar: {bar}"
        );
    }

    /// MAJ-1: When playing track 1 of 2 with no manual queue, the bar should
    /// show "Next: {track 2 title}" (context continuation), not "Next: (end)".
    /// The old `up_next_title()` only checked `manual_queue.first()`, missing
    /// context continuation entirely — so a user playing the first track of an
    /// album saw "(end)" even though track 2 was queued up next.
    #[test]
    fn maj1_up_next_shows_context_continuation_not_end() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // Play track 1 of 2 — context has two tracks, no manual queue.
        app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
        let bar = rendered_bar(&app, 120, 2);
        assert!(
            !bar.contains("Next: (end)"),
            "MAJ-1: should show context continuation, not (end): {bar}"
        );
        assert!(
            bar.contains("Next:"),
            "MAJ-1: should show Next: preview: {bar}"
        );
        // Track 2's title (or a truncation of it) should appear in the bar.
        // RC14-DEF-3: the info-line budget shrank by 1 col (volume↔transport
        // gap), so the next-preview truncates 1 char shorter — accept both
        // "A Very Long" and "A Very Lon" (the shorter ellipsis cutoff).
        let has_track2 = bar.contains("A Very Lon");
        assert!(
            has_track2,
            "MAJ-1: should show next track title (t2): {bar}"
        );
    }

    /// MAJ-1: When a track is in the manual queue but can't be found in the
    /// local catalog (e.g., a YouTube video id), the bar must NOT show "(end)".
    /// It should show "Loading…" (or the title from the YouTube cache) since
    /// there IS a track queued. The old `up_next_title()` returned None when
    /// `track_by_id_fast` failed for non-catalog ids, causing "(end)".
    #[test]
    fn maj1_up_next_shows_loading_for_unknown_queued_track() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // Play t1 as the last (and only) context track.
        app.play_in_context_ids(vec!["t1".into()], "t1");
        // Enqueue a track id not in the local catalog (simulates a YouTube
        // track whose metadata hasn't been fetched yet).
        app.transport.manual_queue.push("yt_video_abc".into());
        let bar = rendered_bar(&app, 120, 2);
        assert!(
            !bar.contains("Next: (end)"),
            "MAJ-1: should not show (end) when manual queue has a track: {bar}"
        );
        assert!(
            bar.contains("Loading"),
            "MAJ-1: should show Loading for unresolvable queued track: {bar}"
        );
    }

    /// MOD-6: The ASCII spinner frames (`SPINNER_ASCII`) must contain only
    /// single ASCII characters — never braille dots. This is the fallback
    /// used when `theme::is_ascii()` is true (`JUKEBOX_FONT_MODE=ascii`).
    /// The old `spinner_glyph` checked `no_color()` alone, so
    /// `JUKEBOX_FONT_MODE=ascii` without `NO_COLOR` still rendered braille —
    /// `is_ascii()` covers the path. D2: `NO_COLOR` no longer triggers ASCII
    /// mode, so it now correctly keeps braille under `NO_COLOR=1`.
    /// (The `is_ascii()` → `FontMode::Ascii` mapping itself is covered by
    /// `icons::tests::font_mode_*`.)
    #[test]
    fn mod6_spinner_ascii_frames_are_ascii() {
        for (i, glyph) in SPINNER_ASCII.iter().enumerate() {
            assert!(
                glyph.len() == 1 && glyph.is_ascii(),
                "MOD-6: SPINNER_ASCII[{i}] = {glyph:?} must be a single ASCII char"
            );
        }
        // The braille set and ASCII set must be disjoint so a glyph from one
        // can never be mistaken for a glyph from the other.
        for braille in &SPINNER {
            assert!(
                !SPINNER_ASCII.contains(braille),
                "MOD-6: braille glyph {braille:?} must not appear in SPINNER_ASCII"
            );
        }
    }

    /// MOD-6: The braille spinner frames (`SPINNER`) must contain only braille
    /// pattern dots (U+2800 block), used in the default Unicode/color mode.
    /// This confirms the default wasn't inverted by the fix.
    #[test]
    fn mod6_spinner_braille_frames_are_braille() {
        for (i, glyph) in SPINNER.iter().enumerate() {
            let cp = glyph.chars().next().unwrap() as u32;
            assert!(
                (0x2800..=0x28FF).contains(&cp),
                "MOD-6: SPINNER[{i}] = {glyph:?} (U+{cp:04X}) must be a braille pattern"
            );
        }
    }

    /// MOD-6: In the default (Unicode, no `JUKEBOX_FONT_MODE`, no `NO_COLOR`)
    /// mode, `spinner_glyph` must return a braille frame. This confirms the
    /// `is_ascii()` check resolves to `false` by default and the braille path
    /// is taken. (Runs without touching env vars so it can't leak state to
    /// parallel tests — the `is_ascii()` → `true` path is exercised by the
    /// `icons::tests::font_mode_*` suite instead.)
    #[test]
    fn mod6_spinner_uses_braille_in_default_mode() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        for frame in 0..10u8 {
            app.spinner_frame = frame;
            let glyph = spinner_glyph(&app);
            assert!(
                SPINNER.contains(&glyph),
                "MOD-6: spinner glyph {glyph:?} must be a braille frame {SPINNER:?} \
                 in default unicode mode"
            );
        }
    }

    /// RC11-DEF-003: the player bar state label must reflect the player's
    /// `is_playing()` flag. When playing → `[PLAYING]`; after `play_pause`
    /// (paused) → `[PAUSED]`. The root cause was AfplayPlayer::is_playing()
    /// ignoring the `paused` flag (fixed in player.rs); this test guards the
    /// label logic in the bar itself using StubPlayer (which correctly
    /// toggles `playing`), so a regression in either layer is caught.
    #[test]
    fn def003_player_bar_shows_paused_label_when_player_paused() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        // Playing → [PLAYING].
        let bar = rendered_bar(&app, 100, 2);
        assert!(
            bar.contains("[PLAYING]"),
            "DEF-003: playing track must show [PLAYING]: {bar}"
        );
        // Pause → [PAUSED].
        let _ = app.player.play_pause();
        let bar = rendered_bar(&app, 100, 2);
        assert!(
            bar.contains("[PAUSED]"),
            "DEF-003: paused track must show [PAUSED] (not [PLAYING]): {bar}"
        );
        assert!(
            !bar.contains("[PLAYING]"),
            "DEF-003: paused track must NOT show [PLAYING]: {bar}"
        );
        // Resume → [PLAYING] again.
        let _ = app.player.play_pause();
        let bar = rendered_bar(&app, 100, 2);
        assert!(
            bar.contains("[PLAYING]"),
            "DEF-003: resumed track must show [PLAYING] again: {bar}"
        );
    }

    /// RC11-DEF-003: the compact 1-row bar must also reflect the paused
    /// state (the narrow path has the same label logic).
    #[test]
    fn def003_compact_bar_shows_paused_label_when_player_paused() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let _ = app.player.play_pause();
        let bar = rendered_compact(&app, 80, 1);
        assert!(
            bar.contains("[PAUSED]"),
            "DEF-003: compact bar must show [PAUSED] when paused: {bar}"
        );
        assert!(
            !bar.contains("[PLAYING]"),
            "DEF-003: compact bar must NOT show [PLAYING] when paused: {bar}"
        );
    }

    /// DEF-004: long title must not collide with transport controls.
    #[test]
    fn def004_long_title_no_collision_with_transport() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t2".into()], "t2");
        let bar = rendered_bar(&app, 100, 2);
        let row0 = bar.lines().next().unwrap();
        let controls_area: String = row0.chars().skip(86).collect();
        assert!(
            controls_area.contains("◀◀"),
            "DEF-004: transport prev must be in controls area: {bar}"
        );
        let info_leak: String = controls_area
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        assert!(
            info_leak.is_empty(),
            "DEF-004: info text leaked into transport controls: {info_leak:?}
{bar}"
        );
    }

    /// DEF-004: volume bar intact at 120 cols with a short title (room for all).
    #[test]
    fn def004_volume_bar_intact_at_120_cols() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let bar = rendered_bar(&app, 120, 2);
        assert!(bar.contains("vol "), "DEF-004: vol label missing: {bar}");
        assert!(bar.contains("70%"), "DEF-004: vol pct missing: {bar}");
    }

    /// DEF-004: volume bar dropped (not corrupted) when title is very long.
    #[test]
    fn def004_volume_dropped_not_corrupted_with_long_title() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t2".into()], "t2");
        let bar = rendered_bar(&app, 120, 2);
        let row0 = bar.lines().next().unwrap();
        // Transport controls must still be present and intact.
        assert!(
            row0.contains("◀◀"),
            "DEF-004: transport prev missing: {bar}"
        );
        // No info text should leak into the transport control area.
        let controls_area: String = row0.chars().skip(106).collect();
        let info_leak: String = controls_area
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        assert!(
            info_leak.is_empty(),
            "DEF-004: info leaked into transport at 120 cols: {info_leak:?}
{bar}"
        );
    }

    /// DEF-051: no trailing dot in mixed mode compact bar.
    #[test]
    fn def051_no_trailing_dot_in_mixed_mode() {
        let (_d, cat) = two_track_cat();
        for yt_state in [
            YtState::Unconfigured,
            YtState::Ready,
            YtState::SignedOut,
            YtState::Authenticating,
        ] {
            for w in [70u16, 80, 100] {
                let mut app = App::new(cat.clone(), Box::new(StubPlayer::default()), None, None);
                app.source_mode = SourceMode::Mixed;
                app.yt_state = yt_state;
                let bar = rendered_compact(&app, w, 1);
                let line = bar.lines().next().unwrap();
                let trimmed = line.trim_end();
                assert!(
                    !trimmed.ends_with('·'),
                    "DEF-051: trailing dot yt={:?} w={}: {:?}",
                    yt_state,
                    w,
                    trimmed
                );
            }
        }
    }

    /// DEF-057: Next: cue visible while playing.
    #[test]
    fn def057_next_cue_visible_while_playing() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
        assert!(app.player.is_playing());
        let bar = rendered_bar(&app, 120, 2);
        assert!(
            bar.contains("Next:"),
            "DEF-057: Next: cue must be visible while playing: {bar}"
        );
    }

    /// RC14-DEF-3: the volume percentage and the transport controls must have
    /// a ≥1-space gap. Before the fix, a long-title track filled the info
    /// line right up to the transport start, producing "70%◀◀" (0 spaces).
    /// Now the info line budget is 1 col shorter, guaranteeing a gap.
    #[test]
    fn rc14_def3_volume_transport_has_gap() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // t2 has a very long title that fills the info line.
        app.play_in_context_ids(vec!["t2".into()], "t2");
        assert!(app.player.is_playing());
        // Render at 100×2 (the width where the defect was observed).
        let bar = rendered_bar(&app, 100, 2);
        // The volume reads "70%" (default volume). The transport starts with
        // ◀◀ (Unicode) or << (ASCII). Either way, "70%" must NOT be
        // immediately followed by the transport glyph — there must be a gap.
        assert!(
            !bar.contains("70%◀◀"),
            "RC14-DEF-3: volume + transport must have a gap, not \"70%◀◀\":\n{bar}"
        );
        assert!(
            !bar.contains("70%<<"),
            "RC14-DEF-3: volume + transport must have a gap, not \"70%<<\":\n{bar}"
        );
        // Sanity: the transport IS rendered (the gap fix didn't remove it).
        assert!(
            bar.contains("◀◀") || bar.contains("<<"),
            "RC14-DEF-3: transport glyphs must still be present:\n{bar}"
        );
    }

    /// Catalog with a hi-res (24/192 kHz) local track for RC14-DEF-4.
    fn hires_cat() -> (tempfile::TempDir, Catalog) {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"h1","artists":["HiRes"],"primary_artist":"HiRes","title":"Desert Wind","album":"HiRes","bit_depth":24,"sample_rate_hz":192000,"source_path":"lossless/A/01.flac","symlinked_into_artists":["HiRes"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        (d, Catalog::load(&p).unwrap())
    }

    /// RC14-DEF-4: for a hi-res (≥192 kHz) local track, the progress bar uses
    /// the wall-clock estimate instead of the player backend's `position()`.
    /// StubPlayer reports position=0 always, so without the fix the bar would
    /// read 0%. With the fix (estimated position from `play_started_at`), a
    /// track that started ~3s ago shows a non-zero percentage.
    #[test]
    fn rc14_def4_hires_progress_uses_wall_clock() {
        let (_d, cat) = hires_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["h1".into()], "h1");
        assert!(app.player.is_playing());
        // Simulate 3s of playback. StubPlayer.duration() = 180, so the
        // estimated percentage ≈ 3/180*100 ≈ 1% (vs 0% from player.position()).
        app.play_started_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
        app.play_start_offset = 0.0;
        app.accumulated_paused = std::time::Duration::ZERO;
        app.pause_started_at = None;
        let bar = rendered_bar(&app, 100, 2);
        // The progress bar row contains "{pct}%". With the fix, pct > 0.
        // Find the progress percentage in the bar (row 2).
        let row2 = bar.lines().nth(1).unwrap_or("");
        assert!(
            !row2.contains(" 0%"),
            "RC14-DEF-4: hi-res progress should use wall-clock estimate (>0%), not 0%:\n{bar}"
        );
        // The percentage should be small but non-zero (~1%).
        assert!(
            row2.contains(" 1%") || row2.contains(" 2%"),
            "RC14-DEF-4: hi-res 3s/180s ≈ 1-2%, got:\n{row2}"
        );
    }

    /// RC14-DEF-4: for a normal (44.1/96 kHz) local track, the player
    /// backend's `position()` is trusted (not the wall-clock estimate).
    /// StubPlayer reports position=0, so the bar reads 0% even after
    /// simulating elapsed wall-clock time — confirming the estimate is NOT
    /// used for non-hi-res tracks.
    #[test]
    fn rc14_def4_normal_res_track_trusts_player_position() {
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        // t1 is 24/96 kHz — not hi-res (≥192 kHz).
        app.play_in_context_ids(vec!["t1".into()], "t1");
        assert!(app.player.is_playing());
        // Simulate 5s of wall-clock elapsed.
        app.play_started_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
        let bar = rendered_bar(&app, 100, 2);
        let row2 = bar.lines().nth(1).unwrap_or("");
        // StubPlayer.position() = 0 → 0%, NOT the wall-clock 5/180 ≈ 2%.
        assert!(
            row2.contains(" 0%"),
            "RC14-DEF-4: non-hi-res track should trust player.position() (0%), not wall-clock:\n{row2}"
        );
    }

    /// RC16-DEF-1: the player bar must mark all its cells as `AlwaysUpdate`
    /// so stale CJK double-width glyphs are fully cleared when the track
    /// switches. After rendering, every cell in the bar area should have
    /// `CellDiffOption::AlwaysUpdate` set (not `None`).
    #[test]
    fn rc16_def1_force_area_update_marks_all_cells() {
        use ratatui::buffer::CellDiffOption;
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let backend = TestBackend::new(100, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = terminal.backend().buffer();
        // Every cell in the 100×2 area must be marked AlwaysUpdate.
        for y in 0..2u16 {
            for x in 0..100u16 {
                let cell = &buf[(x, y)];
                assert_eq!(
                    cell.diff_option,
                    CellDiffOption::AlwaysUpdate,
                    "RC16-DEF-1: cell ({x},{y}) must be AlwaysUpdate, got {:?}",
                    cell.diff_option
                );
            }
        }
    }

    /// RC16-DEF-1: the compact bar must also mark all cells as
    /// `AlwaysUpdate`.
    #[test]
    fn rc16_def1_compact_bar_marks_all_cells() {
        use ratatui::buffer::CellDiffOption;
        let (_d, cat) = two_track_cat();
        let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
        app.play_in_context_ids(vec!["t1".into()], "t1");
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &app))
            .unwrap();
        let buf = terminal.backend().buffer();
        for x in 0..80u16 {
            let cell = &buf[(x, 0)];
            assert_eq!(
                cell.diff_option,
                CellDiffOption::AlwaysUpdate,
                "RC16-DEF-1: compact bar cell ({x},0) must be AlwaysUpdate"
            );
        }
    }
}
