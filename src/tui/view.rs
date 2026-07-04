use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::{App, Pane};

// Palette — one accent (cyan) for focus + selection, dim gray for chrome.
// Picking a single accent keeps the three panes visually coherent instead of
// the prior yellow-on-dark-gray, which read as a warning rather than a calm
// music UI.
const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const HI_FG: Color = Color::Black;

pub fn draw(f: &mut Frame, app: &mut App) {
    // Main 3-pane area + a 2-line footer with all keybindings + queue count.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(f.area());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(50), Constraint::Percentage(25)])
        .split(outer[0]);

    // Artists — rendered with a ListState so the view scrolls to follow the
    // cursor (without this the highlight disappears once it passes the bottom
    // visible row, which reads as "going out of bounds").
    let items: Vec<ListItem> = app.artists.iter()
        .map(|a| ListItem::new(a.as_str())).collect();
    let mut state = list_state(app.artist_cursor, app.artists.len());
    let list = List::new(items)
        .block(border("Artists", matches!(app.focus, Pane::Artists)))
        .highlight_style(selection_style());
    f.render_stateful_widget(list, chunks[0], &mut state);

    // Center column: Search (top) + Now Playing (bottom).
    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[1]);

    // Search pane: input line + ranked results. The result line is
    // "{score}% {marker} {title} — {artist}" on the left and
    // "{bitDepth}/{sampleRate}kHz" right-aligned, so the quality column lines
    // up vertically regardless of title/artist length.
    let pane_w = center[0].width.saturating_sub(2) as usize; // minus borders
    let mut lines: Vec<ListItem> = vec![ListItem::new(format!("/ {}", app.search_input))];
    for (score, tidx) in app.results.iter() {
        let t = &app.catalog.tracks[*tidx];
        let pct = (score * 100.0).clamp(0.0, 100.0);
        // Prefix shows queue state: `▶` playing, `+` enqueued, ` ` neither.
        let prefix = if app.now_playing.as_deref() == Some(&t.id) {
            "▶"
        } else if app.enqueued.contains(&t.id) {
            "+"
        } else {
            " "
        };
        let left = format!("{:>3.0}% {} {} — {}", pct, prefix, t.title, t.primary_artist);
        let right = quality_tag(t.bit_depth, t.sample_rate_hz);
        lines.push(ListItem::new(pad_between(&left, &right, pane_w)));
    }
    let mut rstate = list_state(app.result_cursor, app.results.len());
    let slist = List::new(lines)
        .block(border("Search", matches!(app.focus, Pane::Search)))
        .highlight_style(selection_style());
    f.render_stateful_widget(slist, center[0], &mut rstate);

    // Now Playing panel: track text on top, a 1-line progress gauge below, and
    // a "next up" line so the user knows what's queued after the current track.
    let np = now_playing_text(app);
    let np_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(center[1]);
    f.render_widget(
        Paragraph::new(np).wrap(Wrap { trim: true }).block(border("Now Playing", false)),
        np_area[0],
    );
    let pos = app.player.position();
    let dur = app.player.duration();
    let pct = match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => ((p / d) * 100.0).clamp(0.0, 100.0) as u16,
        _ => 0,
    };
    let label = match (pos, dur) {
        (Some(p), Some(d)) => format!("{} / {}", fmt_time(p), fmt_time(d)),
        (Some(p), None) => fmt_time(p),
        _ => String::new(),
    };
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::NONE))
            .gauge_style(Style::default().fg(ACCENT))
            .percent(pct)
            .label(label),
        np_area[1],
    );

    // Queue pane: items with ▶ on the current; [dead] marks missing sources.
    let cur = app.queue.current_index();
    let qitems: Vec<ListItem> = app.queue().items().iter().enumerate()
        .map(|(i, id)| {
            let prefix = if Some(i) == cur { "▶ " } else { "  " };
            let dead_marker = if app.dead.contains(id) { " [dead]" } else { "" };
            let track = app.catalog.tracks.iter().find(|t| &t.id == id);
            let label = match track {
                Some(t) => format!("{prefix}{} — {}{dead_marker}", t.title, t.primary_artist),
                None => format!("{prefix}{id}{dead_marker}"),
            };
            ListItem::new(label)
        }).collect();
    // The queue's "cursor" is the current play position; scroll to follow it.
    let qcursor = cur.unwrap_or(0);
    let mut qstate = list_state(qcursor, app.queue().items().len());
    let qlist = List::new(qitems)
        .block(border("Queue", matches!(app.focus, Pane::Queue)))
        .highlight_style(selection_style());
    f.render_stateful_widget(qlist, chunks[2], &mut qstate);

    // Footer: all keybindings across two lines + queue count + mode flags,
    // so nothing is hidden behind a per-pane context switch.
    let qcount = app.queue().items().len();
    let now = app.now_playing.as_deref().map(|_| "▶").unwrap_or("■");
    let consume = if app.consume { "consume:on" } else { "consume:off" };
    let line1 = format!(
        " {now} Tab/⇧Tab=pane  /=search  ↑↓=move  q=quit  ←→=±5s  space=play-pause  n/p=next/prev  s=shuf  S=reshuf+play  C=consume  {qcount} queued · {consume}"
    );
    let line2 = " Artists: enter=browse · a=enq all | Search: enter=enq+play | Queue: enter=play · x/r=remove · c=clear   (mouse: click=focus+select · dbl-click=play · wheel=scroll)";
    f.render_widget(
        Paragraph::new(format!("{}\n{}", line1, line2))
            .style(Style::default().fg(DIM)),
        outer[1],
    );
}

/// Build a `ListState` with `selected` set to `cursor`, clamped to the list
/// length. An empty list selects `None` (ratatui panics on `select(0)` for an
/// empty list).
fn list_state(cursor: usize, len: usize) -> ListState {
    let mut s = ListState::default();
    if len == 0 {
        s.select(None);
    } else {
        s.select(Some(cursor.min(len - 1)));
    }
    s
}

/// Selection style: a solid cyan bar with black text — a clear, calm highlight
/// that reads as "current row" without the warning-yellow of the old scheme.
fn selection_style() -> Style {
    Style::default().fg(HI_FG).bg(ACCENT).add_modifier(Modifier::BOLD)
}

/// Right-pad `left` so that `right` sits flush against the pane's right edge.
/// `width` is the inner pane width (borders already subtracted). CJK/wide
/// characters are counted as 2 columns via `disp_width` so alignment holds for
/// Japanese titles (Ado / Aimer / etc.), not just ASCII.
fn pad_between(left: &str, right: &str, width: usize) -> String {
    let lw = disp_width(left);
    let rw = disp_width(right);
    let pad = width.saturating_sub(lw + rw);
    format!("{}{}{}", left, " ".repeat(pad), right)
}

/// Approximate display width: ASCII = 1, CJK + kana + fullwidth = 2. Good
/// enough for terminal alignment of mixed JP/EN titles without pulling in the
/// unicode-width crate.
fn disp_width(s: &str) -> usize {
    s.chars().map(|c| {
        let cp = c as u32;
        if (0x1100..=0x115F).contains(&cp)                    // Hangul Jamo
            || (0x2E80..=0xA4CF).contains(&cp) && cp != 0x303F // CJK radicals / Yi
            || (0xAC00..=0xD7A3).contains(&cp)                // Hangul syllables
            || (0xF900..=0xFAFF).contains(&cp)                // CJK compat ideographs
            || (0xFE30..=0xFE4F).contains(&cp)                // CJK compat forms
            || (0xFF00..=0xFF60).contains(&cp)                // fullwidth forms
            || (0xFFE0..=0xFFE6).contains(&cp)                // fullwidth signs
            || (0x3000..=0x303F).contains(&cp)                // CJK symbols (incl. ・)
            || (0x3040..=0x30FF).contains(&cp)                // Hiragana + Katakana
            || (0x4E00..=0x9FFF).contains(&cp)                // CJK Unified Ideographs
        {
            2
        } else {
            1
        }
    }).sum()
}

/// Format the bit-depth + sample-rate as a compact, right-aligned tag, e.g.
/// `24/96kHz` for 24-bit / 96k, `16/44.1kHz` for CD quality.
fn quality_tag(bit_depth: u32, sample_rate_hz: u32) -> String {
    let khz = if sample_rate_hz.is_multiple_of(1000) {
        format!("{}kHz", sample_rate_hz / 1000)
    } else {
        format!("{:.1}kHz", sample_rate_hz as f64 / 1000.0)
    };
    format!("{}/{}", bit_depth, khz)
}

/// Build the now-playing text: title — artist, album, quality, and the next
/// track queued (so the user knows what's coming). Position/duration is shown
/// by the progress gauge below this panel, not duplicated here.
fn now_playing_text(app: &App) -> String {
    let id = match &app.now_playing {
        Some(id) => id,
        None => return "Nothing playing.".to_string(),
    };
    let t = match app.catalog.tracks.iter().find(|t| &t.id == id) {
        Some(t) => t,
        None => return format!("Playing: {id}"),
    };
    let album = t.album.clone().unwrap_or_else(|| "—".to_string());
    let quality = t.quality_label();
    // When sample-rate switching is on, the output device is switched to match
    // the track's format, so the quality line doubles as the device output rate.
    // Position/duration is rendered by the progress gauge below this panel, not
    // here, to avoid duplication.
    let quality_label = if app.switch_sample_rate { "Output" } else { "Quality" };
    let next = match app.queue.peek_next() {
        Some(nid) => match app.catalog.tracks.iter().find(|t| &t.id == nid) {
            Some(nt) => format!("{} — {}", nt.title, nt.primary_artist),
            None => nid.clone(),
        },
        None => "—".to_string(),
    };
    format!(
        "{} — {}\nAlbum: {}\n{}: {}\nNext: {}",
        t.title, t.primary_artist, album, quality_label, quality, next
    )
}

/// Format seconds as `M:SS` (or `H:MM:SS` past an hour) for the gauge label.
fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 { format!("{}:{:02}:{:02}", h, m, sec) } else { format!("{}:{:02}", m, sec) }
}

fn border<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused { Style::default().fg(ACCENT) } else { Style::default().fg(DIM) };
    Block::default().borders(Borders::ALL).title(title).border_style(style)
}
