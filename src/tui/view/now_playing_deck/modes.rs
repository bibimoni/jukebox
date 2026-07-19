//! Playback modes component for the Now Playing Deck.
//!
//! Renders the shuffle / repeat / continue modes as a secondary group
//! using **explicit words** (not `SHUF`/`RPT`/`CONT` — spec problem #6):
//!
//! ```text
//! Shuffle: Random   Repeat: One   Continue: Off
//! ```
//!
//! At compact widths:
//! ```text
//! Random · Repeat One · Continue Off
//! ```

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;
use crate::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};
use crate::tui::view::theme::{sep_dot, Theme};

/// Render the modes row into `area` (1 row). Uses explicit words at
/// normal widths; collapses to the `·`-separated compact form when the
/// explicit form doesn't fit.
pub fn render_modes(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_modes_line(app, &theme, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Render the compact vocabulary regardless of whether the normal labels
/// technically fit. Breakpoint semantics, not incidental spare columns,
/// determine the wording in the Compact deck.
pub fn render_modes_compact(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    let line = build_modes_line_impl(app, &theme, area.width as usize, true);
    f.render_widget(
        Paragraph::new(line),
        Rect::new(area.x, area.y, area.width, 1),
    );
}

/// Build the modes line. Exposed for layout composers.
pub fn build_modes_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    build_modes_line_impl(app, theme, width, false)
}

fn build_modes_line_impl(
    app: &App,
    theme: &Theme,
    width: usize,
    force_compact: bool,
) -> Line<'static> {
    let enabled_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let disabled_style = Style::default().fg(theme.dim);
    let key_style = Style::default().fg(theme.text_muted);
    let sd = sep_dot();

    let (shuf_label, shuf_on) = match app.transport.shuffle {
        ShuffleMode::Off => ("Off", false),
        ShuffleMode::Random => ("Random", true),
        ShuffleMode::Smart => ("Smart", true),
    };
    let (rpt_label, rpt_on) = match app.transport.repeat {
        RepeatMode::Off => ("Off", false),
        RepeatMode::All => ("All", true),
        RepeatMode::One => ("One", true),
    };
    let (cont_label, cont_on) = match app.transport.continue_mode {
        ContinueMode::Off => ("Off", false),
        ContinueMode::NextAlbum => ("Next", true),
        ContinueMode::Radio => ("Radio", true),
        ContinueMode::YouTube => ("YouTube", true),
    };

    // Explicit form: "Shuffle: Random   Repeat: One   Continue: Off"
    let explicit = format!(
        "Shuffle: {}   Repeat: {}   Continue: {}",
        shuf_label, rpt_label, cont_label
    );
    let explicit_w = crate::tui::view::theme::disp_width(&explicit);

    if !force_compact && width >= explicit_w {
        let shuf_style = if shuf_on {
            enabled_style
        } else {
            disabled_style
        };
        let rpt_style = if rpt_on {
            enabled_style
        } else {
            disabled_style
        };
        let cont_style = if cont_on {
            enabled_style
        } else {
            disabled_style
        };
        Line::from(vec![
            Span::styled("Shuffle: ", key_style),
            Span::styled(shuf_label.to_string(), shuf_style),
            Span::raw("   "),
            Span::styled("Repeat: ", key_style),
            Span::styled(rpt_label.to_string(), rpt_style),
            Span::raw("   "),
            Span::styled("Continue: ", key_style),
            Span::styled(cont_label.to_string(), cont_style),
        ])
    } else {
        // Compact form: "Random · Repeat One · Continue Off"
        let compact = format!("{shuf_label} {sd} Repeat {rpt_label} {sd} Continue {cont_label}");
        let compact_w = crate::tui::view::theme::disp_width(&compact);
        let _ = compact_w;
        let shuf_style = if shuf_on {
            enabled_style
        } else {
            disabled_style
        };
        let rpt_style = if rpt_on {
            enabled_style
        } else {
            disabled_style
        };
        let cont_style = if cont_on {
            enabled_style
        } else {
            disabled_style
        };
        Line::from(vec![
            Span::styled(shuf_label.to_string(), shuf_style),
            Span::raw(format!(" {sd} Repeat ")),
            Span::styled(rpt_label.to_string(), rpt_style),
            Span::raw(format!(" {sd} Continue ")),
            Span::styled(cont_label.to_string(), cont_style),
        ])
    }
}
