//! Visible sidebar (I.6): feature entry points left of the content area.
//!
//! The sidebar subsumes the rail (`columns::render_rail`) at widths ≥100 when
//! `App::sidebar_visible` is true. It mirrors the existing keybindings (`1`-`4`,
//! `H`, `S`, `:radio`, `:gen`, `:publish`, `/`, `D`, `?`) — each entry shows
//! glyph + label + key hint, so features are discoverable without memorizing
//! commands. The rail's `1`-`4` glyphs move into the sidebar's `VIEWS`
//! section.
//!
//! Sections: `VIEWS` (always), `DISCOVER` (always), `TOOLS` (always),
//! `PUBLISH` (only when a playlist is focused in the Playlists view). The
//! active view is highlighted with accent + BOLD + UNDERLINE (matching the tab
//! bar style). Inactive entries are dim. The Queue entry shows `(n)` when the
//! manual queue is non-empty.
//!
//! Width: 24 cols at 100-119, 28 at ≥120. Off at <100 (the content area would
//! drop below the 60-col readability floor). `B` toggles; persisted in
//! `LayoutState.sidebar_visible`.

use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::{App, Overlay, View};
use crate::tui::view::theme::{self, h_line, is_ascii, Theme, ASCII_BORDER_SET};

/// Which target a sidebar entry dispatches to. Mirrors the existing
/// keybindings — clicking the entry (or pressing the key) triggers the same
/// action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidebarTarget {
    ViewArtists,
    ViewPlaylists,
    ViewQueue,
    ViewYoutube,
    Home,
    Discover,
    Radio,
    Generator,
    Publish,
    Search,
    Help,
    Diagnostics,
}

/// A single sidebar row: glyph + label + key hint + dispatch target.
/// Glyphs carry no essential meaning (labels always accompany them) so the
/// sidebar stays readable under `NO_COLOR` + ASCII.
#[derive(Clone, Debug)]
pub struct SidebarEntry {
    pub glyph: &'static str,
    pub label: &'static str,
    pub key: &'static str,
    pub target: SidebarTarget,
}

/// Build the sidebar entries for the current app state. `PUBLISH` only
/// appears when a playlist is focused in the Playlists view (the only place
/// `:publish` makes sense). The Queue entry's `(n)` count is appended by the
/// renderer, not stored here (it changes per-frame).
pub fn sidebar_entries(app: &App) -> Vec<SidebarEntry> {
    let g = glyph_helper();
    let views = [
        SidebarEntry {
            glyph: g.view,
            label: "Artists",
            key: "1",
            target: SidebarTarget::ViewArtists,
        },
        SidebarEntry {
            glyph: g.view,
            label: "Playlists",
            key: "2",
            target: SidebarTarget::ViewPlaylists,
        },
        SidebarEntry {
            glyph: g.view,
            label: "Queue",
            key: "3",
            target: SidebarTarget::ViewQueue,
        },
        SidebarEntry {
            glyph: g.view,
            label: "YouTube",
            key: "4",
            target: SidebarTarget::ViewYoutube,
        },
    ];
    let discover = [
        SidebarEntry {
            glyph: g.home,
            label: "Home",
            key: "H",
            target: SidebarTarget::Home,
        },
        SidebarEntry {
            glyph: g.discover,
            label: "Discover",
            key: "S",
            target: SidebarTarget::Discover,
        },
        SidebarEntry {
            glyph: g.radio,
            label: "Radio",
            key: ":radio",
            target: SidebarTarget::Radio,
        },
        SidebarEntry {
            glyph: g.generator,
            label: "Generator",
            key: ":gen",
            target: SidebarTarget::Generator,
        },
    ];
    let tools = [
        SidebarEntry {
            glyph: g.search,
            label: "Search",
            key: "/",
            target: SidebarTarget::Search,
        },
        SidebarEntry {
            glyph: g.diag,
            label: "Diagnostics",
            key: "D",
            target: SidebarTarget::Diagnostics,
        },
        SidebarEntry {
            glyph: g.help,
            label: "Help",
            key: "?",
            target: SidebarTarget::Help,
        },
    ];
    let mut out: Vec<SidebarEntry> = Vec::new();
    out.extend(views);
    out.extend(discover);
    out.extend(tools);
    // PUBLISH only when a playlist is focused in the Playlists view.
    if app.view == View::Playlists && app.playlists.get(app.cursors.playlist).is_some() {
        out.push(SidebarEntry {
            glyph: g.publish,
            label: "Publish playlist",
            key: ":publish",
            target: SidebarTarget::Publish,
        });
    }
    out
}

/// ASCII fallback glyphs for `JUKEBOX_FONT_MODE=ascii`. All entries use the
/// same `>` glyph (the label carries the meaning) so the sidebar is fully
/// ASCII-readable.
struct GlyphSet {
    view: &'static str,
    home: &'static str,
    discover: &'static str,
    radio: &'static str,
    generator: &'static str,
    search: &'static str,
    diag: &'static str,
    help: &'static str,
    publish: &'static str,
}

fn glyph_helper() -> GlyphSet {
    if is_ascii() {
        GlyphSet {
            view: ">",
            home: "*",
            discover: "*",
            radio: "o",
            generator: "+",
            search: "/",
            diag: "!",
            help: "?",
            publish: "^",
        }
    } else {
        GlyphSet {
            view: "›",
            home: "♫",
            discover: "✦",
            radio: "◎",
            generator: "◆",
            search: "/",
            diag: "!",
            help: "?",
            publish: "↑",
        }
    }
}

/// The sidebar width for the given terminal width. 24 at 100-119, 28 at
/// ≥120, 0 (off) at <100.
pub fn sidebar_width(term_width: u16) -> u16 {
    if term_width < 100 {
        0
    } else if term_width < 120 {
        24
    } else {
        28
    }
}

/// Whether the sidebar should be rendered: `sidebar_visible` AND width
/// ≥100. Exposed so `columns::render` can skip the rail when the sidebar
/// takes over.
pub fn is_visible(app: &App, term_width: u16) -> bool {
    app.sidebar_visible && term_width >= 100
}

/// Render the sidebar into `area`. The caller (`layout::draw`) splits off the
/// sidebar rect first; this function fills it. Active entry = accent + BOLD +
/// UNDERLINE (matches the tab bar style — Phase 3 visual spec H18 / V22);
/// inactive = dim. Sections are separated by a dim `─` rule. The Queue entry
/// appends `(n)` when the manual queue is non-empty.
pub fn render_sidebar(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::default();
    // Phase 3: tabs/sidebar use Theme::tab (accent + BOLD + UNDERLINE),
    // not REVERSED, so they don't collide with row selection.
    let dim = theme.status_description();
    let active = theme.tab(true);
    let text = Style::default().fg(theme.text);
    let header = theme.status_key();

    let block = if is_ascii() {
        Block::default()
            .borders(Borders::RIGHT)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(theme.dim))
    } else {
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(theme.dim))
    };
    let inner = block.inner(area);
    f.render_widget(block, area);

    let col_w = inner.width as usize;
    let entries = sidebar_entries(app);

    let mut lines: Vec<Line> = Vec::new();
    let section_names = ["VIEWS", "DISCOVER", "TOOLS", "PUBLISH"];
    let section_sizes = [4usize, 4, 3, 1]; // entries per section
    let mut emitted: usize = 0;
    for (section_idx, (name, size)) in section_names.iter().zip(section_sizes.iter()).enumerate() {
        // PUBLISH section only renders if there's a publish entry (the last
        // entry when present). Skip the section header if the entries don't
        // include this section.
        if section_idx == 3 && emitted >= entries.len() {
            break;
        }
        if section_idx > 0 {
            // Section separator: a dim horizontal rule.
            let rule_w = col_w.min(24);
            lines.push(Line::from(Span::styled(h_line().repeat(rule_w), dim)));
        }
        lines.push(Line::from(Span::styled((*name).to_string(), header)));
        for _ in 0..*size {
            if emitted >= entries.len() {
                break;
            }
            let entry = &entries[emitted];
            let is_active = is_entry_active(app, entry.target);
            // Queue entry: append `(n)` when manual queue non-empty.
            let queue_count = if matches!(entry.target, SidebarTarget::ViewQueue) {
                app.transport.manual_queue.len()
            } else {
                0
            };
            let label = if queue_count > 0 {
                format!("{} ({})", entry.label, queue_count)
            } else {
                entry.label.to_string()
            };
            // Layout: `glyph key  label`. Key is right-aligned to 3 cols.
            // Reserve room: `glyph ` (2) + `key` (3) + ` label`.
            let key_str = format!("{:>3}", entry.key);
            let row = format!("{} {} {}", entry.glyph, key_str, label);
            let row = theme::clip_to_width(&row, col_w);
            let style = if is_active { active } else { text };
            lines.push(Line::from(Span::styled(row, style)));
            emitted += 1;
        }
    }

    let para = Paragraph::new(lines).alignment(Alignment::Left);
    f.render_widget(para, inner);
}

/// Is the given sidebar target the currently-active one? Used to highlight
/// the matching entry. `View*` targets match `app.view`; overlay-only targets
/// (`Home`, `Discover`, `Radio`, `Generator`, `Publish`) match the active
/// overlay; `Search` matches `Overlay::Search`; `Help` matches `Overlay::Help`;
/// `Diagnostics` matches `Overlay::Diagnostics`.
fn is_entry_active(app: &App, target: SidebarTarget) -> bool {
    match target {
        SidebarTarget::ViewArtists => app.view == View::Artists,
        SidebarTarget::ViewPlaylists => app.view == View::Playlists,
        SidebarTarget::ViewQueue => app.view == View::Queue,
        SidebarTarget::ViewYoutube => app.view == View::Youtube,
        SidebarTarget::Home => matches!(app.overlay, Some(Overlay::Home { .. })),
        SidebarTarget::Discover => matches!(app.overlay, Some(Overlay::Discover { .. })),
        SidebarTarget::Radio => matches!(app.overlay, Some(Overlay::Radio { .. })),
        SidebarTarget::Generator => matches!(app.overlay, Some(Overlay::Generator { .. })),
        SidebarTarget::Publish => matches!(app.overlay, Some(Overlay::Publication { .. })),
        SidebarTarget::Search => matches!(app.overlay, Some(Overlay::Search { .. })),
        SidebarTarget::Help => matches!(app.overlay, Some(Overlay::Help)),
        SidebarTarget::Diagnostics => matches!(app.overlay, Some(Overlay::Diagnostics)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::{App, View};
    use ratatui::{backend::TestBackend, Terminal};

    fn one_track_app() -> App {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("A")).unwrap();
        std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
               "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        let cat = Catalog::load(&p).unwrap();
        std::mem::forget(d);
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    /// I.6: sidebar width is 24 at 100 cols, 28 at 120, 0 at <100.
    #[test]
    fn sidebar_width_breakpoints() {
        assert_eq!(sidebar_width(80), 0, "sidebar off at <100");
        assert_eq!(sidebar_width(99), 0, "sidebar off at 99");
        assert_eq!(sidebar_width(100), 24, "sidebar 24 at 100");
        assert_eq!(sidebar_width(119), 24, "sidebar 24 at 119");
        assert_eq!(sidebar_width(120), 28, "sidebar 28 at 120");
        assert_eq!(sidebar_width(160), 28, "sidebar 28 at 160");
    }

    /// I.6: `is_visible` gates on `sidebar_visible` AND width ≥100.
    #[test]
    fn is_visible_gates_on_width_and_flag() {
        let mut app = one_track_app();
        app.sidebar_visible = true;
        assert!(is_visible(&app, 100), "visible at 100 when flag on");
        assert!(!is_visible(&app, 99), "not visible at 99");
        app.sidebar_visible = false;
        assert!(!is_visible(&app, 100), "not visible when flag off");
    }

    /// I.6: sidebar_entries includes VIEWS, DISCOVER, TOOLS sections.
    #[test]
    fn sidebar_entries_include_core_sections() {
        let app = one_track_app();
        let entries = sidebar_entries(&app);
        let labels: Vec<&str> = entries.iter().map(|e| e.label).collect();
        assert!(labels.contains(&"Artists"), "VIEWS has Artists");
        assert!(labels.contains(&"Playlists"), "VIEWS has Playlists");
        assert!(labels.contains(&"Queue"), "VIEWS has Queue");
        assert!(labels.contains(&"YouTube"), "VIEWS has YouTube");
        assert!(labels.contains(&"Home"), "DISCOVER has Home");
        assert!(labels.contains(&"Discover"), "DISCOVER has Discover");
        assert!(labels.contains(&"Radio"), "DISCOVER has Radio");
        assert!(labels.contains(&"Generator"), "DISCOVER has Generator");
        assert!(labels.contains(&"Search"), "TOOLS has Search");
        assert!(labels.contains(&"Diagnostics"), "TOOLS has Diagnostics");
        assert!(labels.contains(&"Help"), "TOOLS has Help");
    }

    /// I.6: PUBLISH entry only appears when a playlist is focused in the
    /// Playlists view.
    #[test]
    fn publish_entry_only_when_playlist_focused() {
        let mut app = one_track_app();
        app.playlists = vec![crate::tui::app::Playlist {
            name: "Mix".into(),
            track_ids: vec!["t1".into()],
        }];
        // Artists view — no publish entry.
        app.view = View::Artists;
        let entries = sidebar_entries(&app);
        assert!(
            !entries.iter().any(|e| e.target == SidebarTarget::Publish),
            "no Publish in Artists view"
        );
        // Playlists view with a playlist — publish entry present.
        app.view = View::Playlists;
        app.cursors.playlist = 0;
        let entries = sidebar_entries(&app);
        assert!(
            entries.iter().any(|e| e.target == SidebarTarget::Publish),
            "Publish entry present in Playlists view with a focused playlist"
        );
    }

    /// I.6: the active view's entry is marked active by `is_entry_active`.
    #[test]
    fn active_view_entry_marked_active() {
        let mut app = one_track_app();
        app.view = View::Youtube;
        assert!(is_entry_active(&app, SidebarTarget::ViewYoutube));
        assert!(!is_entry_active(&app, SidebarTarget::ViewArtists));
    }

    /// I.6: rendering the sidebar at 100×30 produces visible section headers
    /// + entry labels. Does not panic.
    #[test]
    fn render_sidebar_produces_section_headers() {
        let mut app = one_track_app();
        app.sidebar_visible = true;
        app.view = View::Artists;
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 24, 30);
                render_sidebar(f, area, &app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..30u16 {
            for x in 0..24u16 {
                text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            text.push('\n');
        }
        assert!(text.contains("VIEWS"), "sidebar shows VIEWS header");
        assert!(text.contains("DISCOVER"), "sidebar shows DISCOVER header");
        assert!(text.contains("TOOLS"), "sidebar shows TOOLS header");
        assert!(text.contains("Artists"), "sidebar shows Artists entry");
    }

    /// I.6: Queue entry shows `(n)` count when manual queue non-empty.
    #[test]
    fn queue_entry_shows_count_when_nonempty() {
        let mut app = one_track_app();
        app.transport.manual_queue = vec!["t1".into()];
        let entries = sidebar_entries(&app);
        let queue = entries
            .iter()
            .find(|e| e.target == SidebarTarget::ViewQueue)
            .unwrap();
        // The count is appended by the renderer, not the entries fn. Verify
        // the renderer produces `(1)` via a render.
        let backend = TestBackend::new(24, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 24, 30);
                render_sidebar(f, area, &app);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..30u16 {
            for x in 0..24u16 {
                text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            text.push('\n');
        }
        assert!(
            text.contains("(1)"),
            "Queue entry shows (1) count: {text:?}"
        );
        let _ = queue;
    }
}
