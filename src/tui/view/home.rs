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
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

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
}

impl HomeState {
    /// Create a new Home state (cold start — no history, loading).
    pub fn new() -> Self {
        HomeState {
            focused_section: 0,
            cursor: 0,
            loading: true,
            has_history: false,
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
        Span::styled("loading…".to_string(), Style::default().fg(Color::Yellow))
    } else if state.has_history {
        Span::styled("ready".to_string(), Style::default().fg(Color::Green))
    } else {
        Span::styled(
            "cold start — listening to music builds your profile".to_string(),
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
        format!("▶ {title} — {description}")
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

            let prefix = if is_focused && i == 0 { "▸ " } else { "  " };

            ListItem::new(format!("{prefix}{glyph} {title} — {subtitle}"))
        })
        .collect();

    List::new(list_items).block(block)
}

/// Render the Home view as a single scrollable paragraph (for narrow terminals
/// where multi-section layout doesn't fit).
pub fn render_compact(
    area: Rect,
    sections: &[(HomeSection, Vec<HomeItem>)],
    state: &HomeState,
    icons: &IconRenderer,
) -> Paragraph<'static> {
    let mut lines = Vec::new();

    // Header
    lines.extend(render_header(area, state, icons));

    // Each section
    for (section, items) in sections {
        // Section header
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("── {} ──", section.title()),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            section.description().to_string(),
            Style::default().fg(Color::DarkGray),
        )));

        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no items — listen to music to build this section)".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for item in items {
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
                lines.push(Line::from(format!("  {glyph} {} — {subtitle}", item.title)));

                if let Some(expl) = &item.explanation {
                    lines.push(Line::from(Span::styled(
                        format!("    └ {expl}"),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }
    }

    Paragraph::new(lines).wrap(Wrap { trim: true })
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
        Line::from("  • Play a local track (browse with h j k l, Enter to play)"),
        Line::from("  • Authenticate YouTube (:yt auth browser <name>) for YouTube content"),
        Line::from("  • Search with / in any view"),
        Line::from("  • Open the Discover overlay with S for suggestions"),
        Line::from(""),
        Line::from(Span::styled(
            "As you listen, this Home will show:".to_string(),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Continue Listening · Quick Picks · Made for You · Start Radio".to_string(),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  New and Relevant · Your YouTube Library · Explore".to_string(),
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
            format!("{} YouTube Home — Offline", icons.glyph(Icon::Offline)),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "You're offline. Showing cached content only.".to_string(),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from("Available offline:"),
        Line::from("  • Local music (fully functional)"),
        Line::from("  • Cached YouTube playlists (browse-only, no streaming)"),
        Line::from("  • Generated mixes from local profile"),
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
        let icons = IconRenderer::new(FontMode::Unicode);
        let state = HomeState::new();
        let sections = vec![
            (
                HomeSection::MadeForYou,
                vec![HomeItem::mix(MixType::DailyMix)],
            ),
            (HomeSection::Explore, vec![HomeItem::explore("Jazz".into())]),
        ];
        let para = render_compact(Rect::new(0, 0, 80, 24), &sections, &state, &icons);
        let _ = para;
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
