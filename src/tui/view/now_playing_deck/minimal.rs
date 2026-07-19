//! Minimal Now Playing layout for tiny terminals and narrow sub-rects.
//!
//! Renders 1-2 lines with no border, no controls, no flags — just the
//! essential playback state. This is the deck's last-resort fallback when
//! the rect is too small for the Compact layout (`< 60` cols or `< 2`
//! rows of content). It is also the layout used for sub-rects in a
//! pane-embedded Now Playing module when the pane is too narrow for the
//! big bar.
//!
//! ## Layout
//!
//! ```text
//! ▶ Title — Artist
//! 1:42 ━━●━━━ 3:40  VOL 70%
//! ```
//!
//! Row 1 is always rendered when `area.height >= 1`. Row 2 is rendered
//! only when `area.height >= 2`. The border is never drawn (Minimal has
//! no border per the spec).
//!
//! ## Drop schedule as width shrinks
//!
//! Per the spec, never drop: playback-state icon, title, artist, elapsed
//! time, play/pause affordance. The duration (`3:40`) and `VOL 70%` are
//! dropped as width shrinks:
//!
//! | Width | Row 1 | Row 2 |
//! |-------|-------|-------|
//! | >= 40 | `▶ Title — Artist` | `1:42 ━━●━━━ 3:40  VOL 70%` |
//! | 30-39 | `▶ Title — Artist` | `1:42 ━━●━━━━━━━  VOL 70%` (no duration) |
//! | 20-29 | `▶ Title — Art…` | `1:42 ━━●━━━━━━━` (no VOL) |
//! | 10-19 | `▶ T… — A…` | `1:42` (elapsed only) |
//! | 1-9 | `▶ T…` (title only; artist kept per spec via truncation) | — |
//!
//! ## No-panic guarantee
//!
//! Every `Layout` call is guarded by an `area.width > 0 && area.height > 0`
//! early return. Every `truncate_title` call guards `max == 0` (returns
//! empty string). Every `saturating_sub` is used for width math. Tested
//! by `tests/now_playing_deck.rs::no_panic_sweep` over every `Rect` from
//! `(0,0)` to `(180,40)`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::tui::app::App;
use crate::tui::view::now_playing_deck::metadata;
use crate::tui::view::now_playing_deck::progress::progress;
use crate::tui::view::now_playing_deck::spinner::{is_buffering, spinner_glyph};
use crate::tui::view::theme::{
    disp_width, ellipsis, em_dash, marker_glyph, pause_glyph, play_glyph, progress_color,
    progress_fill, progress_track, stop_glyph, Theme,
};

/// Render the minimal Now Playing deck into `area`. No border, no
/// controls, no flags. See the module docs for the layout and drop
/// schedule.
pub fn render(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let row1 = build_row1(app, &theme, area.width as usize, focused);
    f.render_widget(row1, Rect::new(area.x, area.y, area.width, 1));

    if area.height >= 2 {
        let row2 = build_row2(app, &theme, area.width as usize);
        f.render_widget(row2, Rect::new(area.x, area.y + 1, area.width, 1));
    }
    if area.height >= 3
        && app.resume_hint.is_some()
        && app.now_playing.is_none()
        && !is_buffering(app)
    {
        f.render_widget(
            Line::from(Span::styled("[Space] Resume", theme.status_key())),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
    }
}

/// Row 1: `▶ Title — Artist` (or the paused / stopped / buffering glyph).
/// The state glyph is always rendered first (survives truncation). The
/// title is the visual hero (accent + BOLD). The artist is truncated to
/// fit but never dropped (per spec).
fn build_row1(app: &App, theme: &Theme, width: usize, focused: bool) -> Line<'static> {
    let glyph = state_glyph(app);
    let glyph_w = disp_width(glyph);

    let mut title_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    if focused {
        title_style = title_style.add_modifier(Modifier::UNDERLINED);
    }
    let artist_style = Style::default().fg(theme.text);
    let dim_style = Style::default().fg(theme.dim);

    // "▶ " = glyph_w + 1 (space). " — " = 3. Reserve 4 for the separator
    // and the trailing ellipsis budget.
    let prefix_w = glyph_w + 1;
    let sep_w = 3; // " — " (em-dash with spaces)

    let (title, artist) = if is_buffering(app) {
        (format!("Finding the stream{}", ellipsis()), String::new())
    } else {
        match metadata::display_metadata(app) {
            Some(metadata) => (metadata.primary_title, metadata.artist),
            None => {
                let dash = em_dash();
                let placeholder = format!("{dash} nothing playing {dash}");
                let placeholder_w = width.saturating_sub(prefix_w);
                let truncated =
                    crate::tui::view::player_bar::truncate_title(&placeholder, placeholder_w);
                return Line::from(vec![
                    Span::styled(glyph.to_string(), dim_style),
                    Span::raw(" "),
                    Span::styled(truncated, dim_style),
                ]);
            }
        }
    };

    // If we have both title and artist, split the budget. Per the spec,
    // never drop the artist when there's room for both (even truncated).
    // The title is the hero (gets priority), but the artist must always
    // be visible at >= 10 cols (the 10-19 col band of the drop schedule:
    // `▶ T… — A…`). Below 10 cols the artist may be dropped (the 1-9 col
    // band: `▶ T…`).
    let total_text_w = width.saturating_sub(prefix_w);
    let title_w = disp_width(&title);
    let artist_w = disp_width(&artist);

    if total_text_w == 0 {
        // Just the glyph.
        return Line::from(vec![Span::styled(glyph.to_string(), title_style)]);
    }

    // Budget split:
    // - Both fit fully → use full widths (with separator).
    // - Both don't fit but there's room for `T… — A…` (>= 7 cols text)
    //   → split the remaining space 50/50 (title gets the extra col on
    //   odd widths), keep the separator.
    // - Extreme narrow (< 7 cols text) → title gets priority; artist is
    //   shown only if at least 1 col remains after the title. This
    //   matches the spec's 1-9 col band where the artist may be absent
    //   (`▶ T…`).
    let both_fit = title_w + sep_w + artist_w <= total_text_w;
    let (title_budget, sep_budget, artist_budget) = if both_fit {
        (title_w, sep_w, artist_w)
    } else if total_text_w >= sep_w + 4 {
        // Enough room for: title(2) + sep(3) + artist(2) = 7. Split the
        // remaining space fairly (title gets the extra col on odd).
        let remaining = total_text_w - sep_w;
        let title_half = remaining.div_ceil(2);
        let artist_half = remaining / 2;
        (title_half.min(title_w), sep_w, artist_half.min(artist_w))
    } else {
        // Extreme narrow: title gets priority; artist is shown only if
        // at least 1 col remains after the title (rendered as `…` by
        // truncate_title when budget is 1).
        let title_budget = total_text_w.min(title_w);
        let artist_budget = total_text_w.saturating_sub(title_budget);
        (title_budget, 0, artist_budget)
    };

    let title_span = if title_budget == title_w {
        Span::styled(title, title_style)
    } else {
        Span::styled(
            crate::tui::view::player_bar::truncate_title(&title, title_budget),
            title_style,
        )
    };

    let mut spans = vec![
        Span::styled(glyph.to_string(), title_style),
        Span::raw(" "),
        title_span,
    ];

    if artist_budget > 0 {
        let artist_span = if artist_budget == artist_w {
            Span::styled(artist, artist_style)
        } else {
            Span::styled(
                crate::tui::view::player_bar::truncate_title(&artist, artist_budget),
                artist_style,
            )
        };
        if sep_budget > 0 {
            spans.push(Span::styled(format!(" {} ", em_dash()), dim_style));
        }
        spans.push(artist_span);
    }

    Line::from(spans)
}

/// Row 2: `1:42 ━━●━━━ 3:40  VOL 70%`. The progress bar uses the shared
/// `progress` component for clamping. The thumb (`●`) marks the playhead.
/// The duration is dropped below 40 cols; the VOL label is dropped below
/// 30 cols; the bar itself shrinks to 0 below ~10 cols (leaving just the
/// elapsed time).
fn build_row2(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let (pct, label) = progress(app);
    let fill_color = progress_color(theme);
    let fill_style = Style::default().fg(fill_color).add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.dim);

    // Parse the label to get elapsed and total separately.
    // label is "M:SS / M:SS" or "M:SS / --:--" or "--:-- / --:--".
    let (elapsed_str, total_str) = label.split_once(" / ").unwrap_or(("--:--", "--:--"));
    let elapsed_w = disp_width(elapsed_str);

    // Reserve: elapsed + 1 gap + bar(min 3) + 1 gap + total + 2 + VOL(8)
    let total_w = disp_width(total_str);
    let vol_str = format!("VOL {}%", app.volume);
    let vol_w = disp_width(&vol_str);

    // Decide what to include based on width.
    let show_total = width >= 40;
    let show_vol = width >= 30;

    let reserved_for_text = elapsed_w
        + 1
        + if show_total { total_w + 1 } else { 0 }
        + if show_vol { vol_w + 2 } else { 0 };
    let bar_w = width.saturating_sub(reserved_for_text);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(8);
    spans.push(Span::styled(elapsed_str.to_string(), dim_style));
    spans.push(Span::raw(" "));

    if bar_w >= 3 {
        let filled = (pct as usize * bar_w).div_ceil(100).min(bar_w);
        let rest = bar_w.saturating_sub(filled);
        let mut bar = String::with_capacity(bar_w);
        for _ in 0..filled {
            bar.push(progress_fill());
        }
        for _ in 0..rest {
            bar.push(progress_track());
        }
        spans.push(Span::styled(bar, fill_style));
        spans.push(Span::raw(" "));
    }

    if show_total {
        spans.push(Span::styled(total_str.to_string(), dim_style));
        if show_vol {
            spans.push(Span::raw("  "));
        }
    }

    if show_vol {
        spans.push(Span::styled(vol_str, dim_style));
    }

    Line::from(spans)
}

/// Pick the state glyph for the now-playing row: `▶` (playing), `⏸`
/// (paused), `■` (stopped), or the spinner frame (resolving). Mirrors
/// `player_bar.rs:327-335` but uses the deck's shared spinner.
fn state_glyph(app: &App) -> &'static str {
    if is_buffering(app) {
        spinner_glyph(app)
    } else if app.player.is_playing() {
        play_glyph()
    } else if app.now_playing.is_some() {
        pause_glyph()
    } else if app.resume_hint.is_some() {
        // Stopped with resume available — show the marker glyph (the
        // resume hint text follows in row 1).
        marker_glyph()
    } else {
        stop_glyph()
    }
}
