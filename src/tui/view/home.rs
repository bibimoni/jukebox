//! YouTube Home view — multi-section discovery and personalization.
//!
//! Replaces the playlist-only YouTube view with a complete discovery product:
//! - Continue Listening: recent contexts (playlists, mixes, radio, albums)
//! - Quick Picks: strong recent/session signals
//! - Made for You: generated mixes (Daily Mix, Discover, On Repeat, etc.)
//! - Start Radio: radio seeds from various sources
//! - New and Relevant: new content from followed artists
//! - Your YouTube Library: existing playlists, liked content, subscriptions
//! - Explore: intentional discovery by genre, mood, activity, decade, region

use crate::reco::mixes::MixType;
use crate::tui::view::icons::{Icon, IconRenderer};
use crate::tui::view::theme::{
    bullet, ellipsis, em_dash, h_line, is_ascii, marker_glyph, play_glyph, sep_dot, Theme,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

/// A section in the YouTube Home view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeSection {
    /// Recent listening contexts (playlists, mixes, radio sessions, albums).
    ContinueListening,
    /// Strong recent and session signals — stable long enough to select.
    QuickPicks,
    /// Generated mixes (Daily Mix, Discover, On Repeat, Rediscover, etc.).
    MadeForYou,
    /// Radio seeds from various sources.
    StartRadio,
    /// New content from artists the user meaningfully follows.
    NewRelevant,
    /// Existing playlists, liked content, subscriptions.
    Library,
    /// Intentional discovery by genre, mood, activity, decade, region.
    Explore,
}

impl HomeSection {
    /// The display title for this section.
    pub fn title(&self) -> &'static str {
        match self {
            HomeSection::ContinueListening => "Continue Listening",
            HomeSection::QuickPicks => "Quick Picks",
            HomeSection::MadeForYou => "Made for You",
            HomeSection::StartRadio => "Start Radio",
            HomeSection::NewRelevant => "New and Relevant",
            HomeSection::Library => "Your YouTube Library",
            HomeSection::Explore => "Explore",
        }
    }

    /// A short description for this section.
    pub fn description(&self) -> &'static str {
        match self {
            HomeSection::ContinueListening => "Pick up where you left off",
            HomeSection::QuickPicks => "Tracks based on your recent listening",
            HomeSection::MadeForYou => "Generated mixes tailored to your taste",
            HomeSection::StartRadio => "Start a radio session from any seed",
            HomeSection::NewRelevant => "New from artists you listen to",
            HomeSection::Library => "Your playlists, liked songs, and subscriptions",
            HomeSection::Explore => "Discover by genre, mood, activity, and more",
        }
    }

    /// All sections in display order.
    pub fn all() -> Vec<HomeSection> {
        vec![
            HomeSection::ContinueListening,
            HomeSection::QuickPicks,
            HomeSection::MadeForYou,
            HomeSection::StartRadio,
            HomeSection::NewRelevant,
            HomeSection::Library,
            HomeSection::Explore,
        ]
    }

    /// True if this section requires listening history to be meaningful.
    pub fn requires_history(&self) -> bool {
        matches!(
            self,
            HomeSection::ContinueListening | HomeSection::QuickPicks | HomeSection::NewRelevant
        )
    }
}

/// An item in a Home section. Each item is a selectable, playable entry.
#[derive(Clone, Debug)]
pub struct HomeItem {
    /// The display title.
    pub title: String,
    /// Optional subtitle (artist name, track count, etc.).
    pub subtitle: Option<String>,
    /// The kind of item (for dispatching on selection).
    pub kind: HomeItemKind,
    /// Optional provenance/explanation text.
    pub explanation: Option<String>,
}

/// What kind of item a HomeItem is — determines what happens on Enter.
#[derive(Clone, Debug)]
pub enum HomeItemKind {
    /// A playlist (local or YouTube) — Enter starts playing it.
    Playlist {
        id: String,
        name: String,
        is_local: bool,
    },
    /// A track — Enter plays it immediately.
    Track {
        id: String,
        title: String,
        artist: String,
        is_local: bool,
    },
    /// A generated mix — Enter opens/plays the mix.
    Mix { mix_type: MixType },
    /// A radio seed — Enter starts a radio session.
    RadioSeed { description: String },
    /// An explore category — Enter opens the category.
    Explore { category: String },
    /// A liked-songs entry — Enter opens liked songs.
    LikedSongs,
    /// A subscription — Enter opens the artist/channel.
    Subscription { channel_id: String, name: String },
}

impl HomeItem {
    /// Create a playlist item.
    pub fn playlist(id: String, name: String, is_local: bool) -> Self {
        HomeItem {
            title: name.clone(),
            subtitle: None,
            kind: HomeItemKind::Playlist { id, name, is_local },
            explanation: None,
        }
    }

    /// Create a track item.
    pub fn track(id: String, title: String, artist: String, is_local: bool) -> Self {
        HomeItem {
            title: title.clone(),
            subtitle: Some(artist.clone()),
            kind: HomeItemKind::Track {
                id,
                title,
                artist,
                is_local,
            },
            explanation: None,
        }
    }

    /// Create a mix item.
    pub fn mix(mix_type: MixType) -> Self {
        HomeItem {
            title: mix_type.label().to_string(),
            subtitle: Some(mix_type.description().to_string()),
            kind: HomeItemKind::Mix { mix_type },
            explanation: None,
        }
    }

    /// Create a radio seed item.
    pub fn radio_seed(description: String) -> Self {
        HomeItem {
            title: description.clone(),
            subtitle: Some("radio".into()),
            kind: HomeItemKind::RadioSeed { description },
            explanation: None,
        }
    }

    /// Create an explore category item.
    pub fn explore(category: String) -> Self {
        HomeItem {
            title: category.clone(),
            subtitle: Some("explore".into()),
            kind: HomeItemKind::Explore { category },
            explanation: None,
        }
    }

    /// Create a liked-songs item.
    pub fn liked_songs() -> Self {
        HomeItem {
            title: "Liked Songs".into(),
            subtitle: Some("your liked music".into()),
            kind: HomeItemKind::LikedSongs,
            explanation: None,
        }
    }

    /// Create a subscription item.
    pub fn subscription(channel_id: String, name: String) -> Self {
        HomeItem {
            title: name.clone(),
            subtitle: Some("subscription".into()),
            kind: HomeItemKind::Subscription { channel_id, name },
            explanation: None,
        }
    }

    /// Set the explanation/provenance.
    pub fn with_explanation(mut self, explanation: String) -> Self {
        self.explanation = Some(explanation);
        self
    }
}

/// The state of the Home view: which sections are visible, which is focused,
/// and the cursor within each section.
#[derive(Clone, Debug, Default)]
pub struct HomeState {
    /// The section the cursor is currently in.
    pub focused_section: usize,
    /// The cursor within the focused section.
    pub cursor: usize,
    /// Whether the Home is loading (first render before data arrives).
    pub loading: bool,
    /// Whether the user has listening history (affects which sections show).
    pub has_history: bool,
    /// The sections + items to display. Populated by `App::open_home()` from
    /// the catalog, reco mixes, yt_lists, etc. Empty on cold start before
    /// `open_home` runs; the renderer shows the welcome screen only when this
    /// is empty.
    pub sections: Vec<(HomeSection, Vec<HomeItem>)>,
}

impl HomeState {
    /// Create a new Home state (cold start — no history, loading).
    pub fn new() -> Self {
        HomeState {
            focused_section: 0,
            cursor: 0,
            loading: true,
            has_history: false,
            sections: Vec::new(),
        }
    }

    /// Move the cursor up within the current section.
    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor down within the current section.
    pub fn cursor_down(&mut self, max: usize) {
        if self.cursor + 1 < max {
            self.cursor += 1;
        }
    }

    /// Move to the next section.
    pub fn section_next(&mut self, max_sections: usize) {
        if self.focused_section + 1 < max_sections {
            self.focused_section += 1;
            self.cursor = 0;
        }
    }

    /// Move to the previous section.
    pub fn section_prev(&mut self) {
        if self.focused_section > 0 {
            self.focused_section -= 1;
            self.cursor = 0;
        }
    }
}

/// Render the Home view header (title + description).
pub fn render_header(_area: Rect, state: &HomeState, icons: &IconRenderer) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Title line with icon.
    let title_icon = icons.glyph(Icon::Search);
    let title = Line::from(vec![
        Span::styled(format!("{title_icon} "), Style::default().fg(Color::Cyan)),
        Span::styled(
            "YouTube Home".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    lines.push(title);

    // Status line (loading / ready / offline).
    let status = if state.loading {
        Span::styled(
            format!("loading{}", ellipsis()),
            Style::default().fg(Color::Yellow),
        )
    } else if state.has_history {
        Span::styled("ready".to_string(), Style::default().fg(Color::Green))
    } else {
        Span::styled(
            format!(
                "cold start {} listening to music builds your profile",
                em_dash()
            ),
            Style::default().fg(Color::DarkGray),
        )
    };
    lines.push(Line::from(status));

    lines
}

/// Render a section as a list of items.
pub fn render_section(
    section: &HomeSection,
    items: &[HomeItem],
    icons: &IconRenderer,
    is_focused: bool,
) -> List<'static> {
    let title = section.title();
    let description = section.description();

    let header = if is_focused {
        format!("{} {title} {} {description}", play_glyph(), em_dash())
    } else {
        format!("  {title}")
    };

    let block = Block::default().borders(Borders::TOP).title(Span::styled(
        header,
        Style::default().add_modifier(Modifier::BOLD),
    ));

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let icon = match &item.kind {
                HomeItemKind::Playlist { is_local, .. } => {
                    if *is_local {
                        Icon::Local
                    } else {
                        Icon::Youtube
                    }
                }
                HomeItemKind::Track { is_local, .. } => {
                    if *is_local {
                        Icon::Local
                    } else {
                        Icon::Youtube
                    }
                }
                HomeItemKind::Mix { .. } => Icon::Generated,
                HomeItemKind::RadioSeed { .. } => Icon::Radio,
                HomeItemKind::Explore { .. } => Icon::Search,
                HomeItemKind::LikedSongs => Icon::Like,
                HomeItemKind::Subscription { .. } => Icon::Youtube,
            };

            let glyph = icons.glyph(icon);
            let title = &item.title;
            let subtitle = item.subtitle.as_deref().unwrap_or("");

            let prefix = if is_focused && i == 0 {
                format!("{} ", crate::tui::view::theme::marker_glyph())
            } else {
                "  ".to_string()
            };

            ListItem::new(format!("{prefix}{glyph} {title} {} {subtitle}", em_dash()))
        })
        .collect();

    List::new(list_items).block(block)
}

/// Render the Home view as a single scrollable paragraph (for narrow terminals
/// where multi-section layout doesn't fit).
///
/// RC11-DEF-001: the focused item (the one at `state.cursor` within the
/// `focused_section`) now carries selection styling (`theme.selected_style()`
/// = REVERSED|BOLD, surviving NO_COLOR) plus a `▸` cursor glyph so the
/// selection is visible at all terminal sizes and color modes. Previously
/// every item was a plain `Line::from(format!(...))` with no selection
/// styling, so `j`/`k` moved the (invisible) cursor and `Enter` played an
/// unpredictable item.
pub fn render_compact(
    f: &mut Frame,
    area: Rect,
    sections: &[(HomeSection, Vec<HomeItem>)],
    state: &HomeState,
    icons: &IconRenderer,
) {
    let theme = Theme::default();
    let dash = em_dash();
    let marker = marker_glyph();
    let mut lines = Vec::new();

    // Header
    lines.extend(render_header(area, state, icons));

    // Track the absolute row index of each section's first item so we can
    // map `state.cursor` (within the focused section) to the flat line list.
    // This is used both for the selection glyph and (below) for scrolling.
    let mut item_line_offsets: Vec<(usize, usize)> = Vec::new();
    let mut current_line: usize = lines.len();

    // Each section
    for (s_idx, (section, items)) in sections.iter().enumerate() {
        // Section header
        lines.push(Line::from(""));
        current_line += 1;
        let hl = h_line();
        lines.push(Line::from(Span::styled(
            format!("{hl}{hl} {} {hl}{hl}", section.title()),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        current_line += 1;
        lines.push(Line::from(Span::styled(
            section.description().to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        current_line += 1;

        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "  (no items {} listen to music to build this section)",
                    em_dash()
                ),
                Style::default().fg(Color::DarkGray),
            )));
            current_line += 1;
        } else {
            let first_item_line = current_line;
            for (i, item) in items.iter().enumerate() {
                let icon = match &item.kind {
                    HomeItemKind::Playlist { is_local, .. } => {
                        if *is_local {
                            Icon::Local
                        } else {
                            Icon::Youtube
                        }
                    }
                    HomeItemKind::Track { is_local, .. } => {
                        if *is_local {
                            Icon::Local
                        } else {
                            Icon::Youtube
                        }
                    }
                    HomeItemKind::Mix { .. } => Icon::Generated,
                    HomeItemKind::RadioSeed { .. } => Icon::Radio,
                    HomeItemKind::Explore { .. } => Icon::Search,
                    HomeItemKind::LikedSongs => Icon::Like,
                    HomeItemKind::Subscription { .. } => Icon::Youtube,
                };
                let glyph = icons.glyph(icon);
                let subtitle = item.subtitle.as_deref().unwrap_or("");

                // RC11-DEF-001: apply selection styling + cursor glyph to the
                // focused item (state.cursor within the focused section).
                let is_focused = s_idx == state.focused_section && i == state.cursor;
                let prefix = if is_focused {
                    format!("{marker} ")
                } else {
                    "  ".to_string()
                };
                let text = format!("{prefix}{glyph} {} {} {subtitle}", item.title, dash);
                let style = if is_focused {
                    theme.selected_style()
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(text, style)));
                current_line += 1;

                if let Some(expl) = &item.explanation {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    {corner} {expl}",
                            corner = if is_ascii() { "\\" } else { "└" }
                        ),
                        Style::default().fg(Color::DarkGray),
                    )));
                    current_line += 1;
                }
            }
            let last_item_line = current_line;
            item_line_offsets.push((first_item_line, last_item_line));
        }
    }

    // RC11-DEF-024: at 80×24 the Home overlay can overflow. Show a persistent
    // bottom hint bar so the user knows how to navigate / scroll / close,
    // matching the help + lyrics overlays' persistent footer pattern.
    // Reserve the last row for the hint; the Paragraph above scrolls within
    // the remaining area.
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let body_area = chunks[0];
    let hint_area = chunks[1];

    // RC11-DEF-001: scroll the focused item into view. Paragraph doesn't
    // auto-scroll like List+ListState, so without this the focused item can
    // move below the visible area on small terminals (80×24).
    let visible_h = body_area.height as usize;
    let focused_line = item_line_offsets
        .get(state.focused_section)
        .map(|(first, _)| first + state.cursor)
        .unwrap_or(0);
    let scroll = if focused_line >= visible_h {
        (focused_line - visible_h + 1) as u16
    } else {
        0
    };

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).wrap(Wrap { trim: true }),
        body_area,
    );

    // Persistent hint bar — visible at all terminal sizes so the close /
    // navigate / section-switch keys are discoverable (RC11-DEF-001).
    let dot = sep_dot();
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "j/k navigate {dot} Tab sections {dot} Enter play {dot} ? help {dot} Esc close"
            ),
            Style::default().fg(theme.dim),
        )))
        .alignment(Alignment::Center),
        hint_area,
    );
}

/// Render the empty Home (cold start — no history, no playlists).
pub fn render_empty(icons: &IconRenderer) -> Paragraph<'static> {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{} YouTube Home", icons.glyph(Icon::Search)),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Welcome! Your Home will grow as you listen.".to_string(),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from("To get started:"),
        Line::from(format!(
            "  {} Play a local track (browse with h j k l, Enter to play)",
            bullet()
        )),
        Line::from(format!(
            "  {} Authenticate YouTube (:yt auth browser <name>) for YouTube content",
            bullet()
        )),
        Line::from(format!("  {} Search with / in any view", bullet())),
        Line::from(format!(
            "  {} Open the Discover overlay with S for suggestions",
            bullet()
        )),
        Line::from(""),
        Line::from(Span::styled(
            "As you listen, this Home will show:".to_string(),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format!(
                "  Continue Listening {sd} Quick Picks {sd} Made for You {sd} Start Radio",
                sd = sep_dot()
            ),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format!(
                "  New and Relevant {sd} Your YouTube Library {sd} Explore",
                sd = sep_dot()
            ),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
}

/// Render the offline Home (cached content, no live data).
pub fn render_offline(icons: &IconRenderer) -> Paragraph<'static> {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "{} YouTube Home {} Offline",
                icons.glyph(Icon::Offline),
                em_dash()
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "You're offline. Showing cached content only.".to_string(),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from("Available offline:"),
        Line::from(format!("  {} Local music (fully functional)", bullet())),
        Line::from(format!(
            "  {} Cached YouTube playlists (browse-only, no streaming)",
            bullet()
        )),
        Line::from(format!("  {} Generated mixes from local profile", bullet())),
        Line::from(""),
        Line::from("Press R to retry the connection when you're back online."),
    ];

    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reco::mixes::MixType;
    use crate::tui::view::icons::FontMode;

    #[test]
    fn home_section_titles() {
        assert_eq!(HomeSection::ContinueListening.title(), "Continue Listening");
        assert_eq!(HomeSection::QuickPicks.title(), "Quick Picks");
        assert_eq!(HomeSection::MadeForYou.title(), "Made for You");
    }

    #[test]
    fn home_section_all_has_7_sections() {
        let sections = HomeSection::all();
        assert_eq!(sections.len(), 7);
    }

    #[test]
    fn home_section_requires_history() {
        assert!(HomeSection::ContinueListening.requires_history());
        assert!(HomeSection::QuickPicks.requires_history());
        assert!(!HomeSection::MadeForYou.requires_history());
        assert!(!HomeSection::Explore.requires_history());
    }

    #[test]
    fn home_item_playlist() {
        let item = HomeItem::playlist("PL123".into(), "My Playlist".into(), false);
        assert_eq!(item.title, "My Playlist");
        assert!(matches!(item.kind, HomeItemKind::Playlist { .. }));
    }

    #[test]
    fn home_item_track() {
        let item = HomeItem::track("v123".into(), "Song".into(), "Artist".into(), true);
        assert_eq!(item.title, "Song");
        assert_eq!(item.subtitle, Some("Artist".into()));
        assert!(matches!(
            item.kind,
            HomeItemKind::Track { is_local: true, .. }
        ));
    }

    #[test]
    fn home_item_mix() {
        let item = HomeItem::mix(MixType::DailyMix);
        assert_eq!(item.title, "Daily Mix");
        assert!(matches!(item.kind, HomeItemKind::Mix { .. }));
    }

    #[test]
    fn home_item_radio_seed() {
        let item = HomeItem::radio_seed("Artist Radio".into());
        assert!(matches!(item.kind, HomeItemKind::RadioSeed { .. }));
    }

    #[test]
    fn home_item_explore() {
        let item = HomeItem::explore("Jazz".into());
        assert!(matches!(item.kind, HomeItemKind::Explore { category } if category == "Jazz"));
    }

    #[test]
    fn home_item_liked_songs() {
        let item = HomeItem::liked_songs();
        assert_eq!(item.title, "Liked Songs");
        assert!(matches!(item.kind, HomeItemKind::LikedSongs));
    }

    #[test]
    fn home_item_with_explanation() {
        let item = HomeItem::track("v1".into(), "Song".into(), "Artist".into(), false)
            .with_explanation("from your liked tracks".into());
        assert!(item.explanation.is_some());
        assert!(item.explanation.unwrap().contains("liked"));
    }

    #[test]
    fn home_state_new_is_loading() {
        let state = HomeState::new();
        assert!(state.loading);
        assert!(!state.has_history);
        assert_eq!(state.focused_section, 0);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn home_state_cursor_up() {
        let mut state = HomeState::new();
        state.cursor = 2;
        state.cursor_up();
        assert_eq!(state.cursor, 1);
        state.cursor_up();
        assert_eq!(state.cursor, 0);
        state.cursor_up(); // saturating
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn home_state_cursor_down() {
        let mut state = HomeState::new();
        state.cursor_down(5);
        assert_eq!(state.cursor, 1);
        state.cursor_down(5);
        state.cursor_down(5);
        state.cursor_down(5);
        state.cursor_down(5); // at max (4)
        assert_eq!(state.cursor, 4);
        state.cursor_down(5); // doesn't exceed max
        assert_eq!(state.cursor, 4);
    }

    #[test]
    fn home_state_section_next() {
        let mut state = HomeState::new();
        state.section_next(7);
        assert_eq!(state.focused_section, 1);
        assert_eq!(state.cursor, 0); // cursor resets
    }

    #[test]
    fn home_state_section_prev() {
        let mut state = HomeState::new();
        state.focused_section = 2;
        state.section_prev();
        assert_eq!(state.focused_section, 1);
        state.section_prev();
        assert_eq!(state.focused_section, 0);
        state.section_prev(); // saturating
        assert_eq!(state.focused_section, 0);
    }

    #[test]
    fn render_header_produces_lines() {
        let state = HomeState::new();
        let icons = IconRenderer::new(FontMode::Unicode);
        let lines = render_header(Rect::new(0, 0, 80, 24), &state, &icons);
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_empty_produces_content() {
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render_empty(&icons);
        // The paragraph should have content (we can't easily inspect the lines
        // here, but we verify it doesn't panic).
        let _ = para;
    }

    #[test]
    fn render_offline_produces_content() {
        let icons = IconRenderer::new(FontMode::Unicode);
        let para = render_offline(&icons);
        let _ = para;
    }

    #[test]
    fn render_compact_with_sections() {
        use ratatui::{backend::TestBackend, Terminal};
        let icons = IconRenderer::new(FontMode::Unicode);
        let mut state = HomeState::new();
        state.loading = false;
        let sections = vec![
            (
                HomeSection::MadeForYou,
                vec![HomeItem::mix(MixType::DailyMix)],
            ),
            (HomeSection::Explore, vec![HomeItem::explore("Jazz".into())]),
        ];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &sections, &state, &icons))
            .unwrap();
    }

    /// RC11-DEF-001: the focused item (at `state.cursor` within the focused
    /// section) must carry selection styling (REVERSED|BOLD under NO_COLOR, or
    /// accent bg in color mode) so the cursor is visible at all terminal
    /// sizes and color modes.
    #[test]
    fn render_compact_focused_item_has_selection_style() {
        use ratatui::{
            backend::TestBackend,
            style::{Color, Modifier},
            Terminal,
        };
        let icons = IconRenderer::new(FontMode::Unicode);
        let mut state = HomeState::new();
        state.loading = false;
        state.focused_section = 0;
        state.cursor = 1;
        let sections = vec![(
            HomeSection::QuickPicks,
            vec![
                HomeItem::track("t1".into(), "Song 1".into(), "Artist A".into(), true),
                HomeItem::track("t2".into(), "Song 2".into(), "Artist B".into(), true),
            ],
        )];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &sections, &state, &icons))
            .unwrap();
        let buf = terminal.backend().buffer();
        // Find the row containing "Song 2" (the focused item) and verify it
        // carries the selection style: REVERSED modifier (NO_COLOR) or Cyan
        // background (color mode) — matching `Theme::selected_style`.
        let mut found_selection = false;
        let mut found_song2 = false;
        for y in 0..24u16 {
            let mut row = String::new();
            let mut row_has_selection = false;
            for x in 0..80u16 {
                let cell = &buf[(x, y)];
                row.push(cell.symbol().chars().next().unwrap_or(' '));
                if cell.modifier.contains(Modifier::REVERSED) || cell.bg == Color::Cyan {
                    row_has_selection = true;
                }
            }
            if row.contains("Song 2") {
                found_song2 = true;
                assert!(
                    row_has_selection,
                    "DEF-001: focused Home item 'Song 2' must have selection style: {row:?}"
                );
                found_selection = true;
            }
        }
        assert!(found_song2, "DEF-001: 'Song 2' must render in the Home overlay");
        assert!(
            found_selection,
            "DEF-001: focused Home item must carry selection style"
        );
    }

    /// RC11-DEF-001: exactly one Home item carries the selection style (the
    /// focused one), not all items.
    #[test]
    fn render_compact_exactly_one_focused_item() {
        use ratatui::{
            backend::TestBackend,
            style::{Color, Modifier},
            Terminal,
        };
        let icons = IconRenderer::new(FontMode::Unicode);
        let mut state = HomeState::new();
        state.loading = false;
        state.focused_section = 0;
        state.cursor = 0;
        let sections = vec![(
            HomeSection::QuickPicks,
            vec![
                HomeItem::track("t1".into(), "Song 1".into(), "Artist A".into(), true),
                HomeItem::track("t2".into(), "Song 2".into(), "Artist B".into(), true),
                HomeItem::track("t3".into(), "Song 3".into(), "Artist C".into(), true),
            ],
        )];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &sections, &state, &icons))
            .unwrap();
        let buf = terminal.backend().buffer();
        // Count rows carrying the selection style (REVERSED or Cyan bg).
        // Only the focused item (cursor=0 → "Song 1") should carry it — not
        // the section header, not the other items, not the hint bar.
        let mut selected_rows = 0;
        for y in 0..24u16 {
            let mut row_has_selection = false;
            for x in 0..80u16 {
                let cell = &buf[(x, y)];
                if cell.modifier.contains(Modifier::REVERSED) || cell.bg == Color::Cyan {
                    row_has_selection = true;
                }
            }
            if row_has_selection {
                selected_rows += 1;
            }
        }
        assert_eq!(
            selected_rows, 1,
            "DEF-001: exactly one Home row should carry selection style (the focused item), got {selected_rows}"
        );
    }

    /// RC11-DEF-001: the focused item carries a `▸` cursor glyph (text-visible
    /// cue that survives NO_COLOR).
    #[test]
    fn render_compact_focused_item_has_cursor_glyph() {
        use ratatui::{backend::TestBackend, Terminal};
        let icons = IconRenderer::new(FontMode::Unicode);
        let mut state = HomeState::new();
        state.loading = false;
        state.focused_section = 0;
        state.cursor = 0;
        let sections = vec![(
            HomeSection::QuickPicks,
            vec![HomeItem::track(
                "t1".into(),
                "Song 1".into(),
                "Artist A".into(),
                true,
            )],
        )];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &sections, &state, &icons))
            .unwrap();
        let mut buf = String::new();
        for y in 0..24u16 {
            for x in 0..80u16 {
                let c = &terminal.backend().buffer()[(x, y)];
                buf.push(c.symbol().chars().next().unwrap_or(' '));
            }
            buf.push('\n');
        }
        assert!(
            buf.contains('▸'),
            "DEF-001: focused Home item must show the ▸ cursor glyph: {buf}"
        );
    }

    /// RC11-DEF-024: the Home overlay shows a persistent bottom hint bar at
    /// 80×24 so the user knows how to navigate / scroll / close.
    #[test]
    fn render_compact_shows_bottom_hint_bar() {
        use ratatui::{backend::TestBackend, Terminal};
        let icons = IconRenderer::new(FontMode::Unicode);
        let mut state = HomeState::new();
        state.loading = false;
        state.sections = vec![(
            HomeSection::QuickPicks,
            vec![HomeItem::track(
                "t1".into(),
                "Song 1".into(),
                "Artist A".into(),
                true,
            )],
        )];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_compact(f, f.area(), &state.sections, &state, &icons))
            .unwrap();
        let mut buf = String::new();
        for y in 0..24u16 {
            for x in 0..80u16 {
                let c = &terminal.backend().buffer()[(x, y)];
                buf.push(c.symbol().chars().next().unwrap_or(' '));
            }
            buf.push('\n');
        }
        assert!(
            buf.contains("Enter play"),
            "DEF-024: Home hint bar must mention Enter play: {buf}"
        );
        assert!(
            buf.contains("? help"),
            "DEF-001: Home hint bar must mention ? help: {buf}"
        );
        assert!(
            buf.contains("Esc close"),
            "DEF-024: Home hint bar must mention Esc close: {buf}"
        );
    }

    #[test]
    fn render_section_with_items() {
        let icons = IconRenderer::new(FontMode::Unicode);
        let items = vec![
            HomeItem::playlist("PL1".into(), "Playlist 1".into(), false),
            HomeItem::track("v1".into(), "Track 1".into(), "Artist 1".into(), false),
        ];
        let list = render_section(&HomeSection::Library, &items, &icons, true);
        let _ = list;
    }

    #[test]
    fn render_section_empty_items() {
        let icons = IconRenderer::new(FontMode::Unicode);
        let list = render_section(&HomeSection::QuickPicks, &[], &icons, false);
        let _ = list;
    }
}
