//! Pane module trait + registry.
//!
//! A "pane module" is a self-contained unit that can render into a Rect
//! and handle a key when its pane is focused. Built-in modules are thin
//! adapters over the existing `view::columns::render_*` functions — the
//! pane system reuses the current rendering logic rather than
//! duplicating it. Third-party modules can be registered via
//! [`register_module`] before the event loop starts.
//!
//! ## Why a global registry, not a field on `App`
//!
//! `PaneModule::render` takes `&mut App` so it can reuse the existing
//! view functions (which take `&mut App`). If the registry lived on
//! `App`, looking up a module would borrow `&app.pane_registry` which
//! would conflict with the `&mut app` needed for `render`. Keeping the
//! registry in a process-global [`OnceLock`] sidesteps the borrow
//! conflict: the registry is `&'static`, so it doesn't borrow from
//! `app` at all.
//!
//! Registration is one-time setup: built-in modules are auto-registered
//! on first access; third-party modules are added via [`init_registry`]
//! before the event loop. There's no runtime registration API (the
//! spec doesn't require it).
//!
//! ## Module state vs App state
//!
//! In Phase 1, all module state lives on `App` (cursors, view, yt_view,
//! etc.). The focused pane's module determines what `App.view` is set
//! to between renders, so the existing view functions read the right
//! state. Per-pane cursor state is a documented Phase 1 limitation —
//! same-module panes share `App.cursors`.
//!
//! The trait uses `&self` (not `&mut self`) for `render` and
//! `handle_key` because built-in modules are stateless. A stateful
//! third-party module would use interior mutability (`Mutex<T>` or
//! `AtomicXxx`) — the `Send + Sync` bound makes that safe.

use std::sync::OnceLock;

use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};

use crate::tui::app::App;
use crate::tui::pane::model::ModuleId;

/// A self-contained pane module. Render + key handling for a single
/// pane's content.
///
/// `render` receives `&mut App` so it can reuse the existing view
/// functions (which take `&mut App`). The module must NOT mutate `App`
/// during render — that's input's job. The `&mut` is for compatibility
/// with existing signatures.
pub trait PaneModule: Send + Sync {
    /// The module's stable id. Used to look it up in the registry and to
    /// persist pane assignments.
    fn id(&self) -> ModuleId;

    /// Human-readable title shown in the pane's border.
    fn title(&self) -> &'static str;

    /// Render the module's content into `area`. The pane layer has already
    /// drawn the border + title; the module owns the inner rect.
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App);

    /// Handle a key in Normal mode (when this module's pane is focused).
    /// Layout-edit commands (the `Ctrl+w` prefix) are handled by the pane
    /// layer before this is called, so the module never sees them.
    /// Returns true if the key was consumed.
    fn handle_key(&self, key: KeyEvent, app: &mut App) -> bool;
}

/// The registry of all known pane modules. Built-in modules are
/// registered in [`ModuleRegistry::with_builtins`]; third-party modules
/// are added via [`ModuleRegistry::register`].
pub struct ModuleRegistry {
    modules: Vec<Box<dyn PaneModule + Send + Sync>>,
}

impl ModuleRegistry {
    /// Build a registry with all the built-in modules. Third-party
    /// modules can be added via [`Self::register`].
    pub fn with_builtins() -> Self {
        let mut reg = Self {
            modules: Vec::new(),
        };
        reg.register(Box::new(ArtistsModule));
        reg.register(Box::new(PlaylistsModule));
        reg.register(Box::new(QueueModule));
        reg.register(Box::new(YoutubeModule));
        reg.register(Box::new(NowPlayingModule));
        reg.register(Box::new(YtHomeModule));
        reg.register(Box::new(YtLibraryModule));
        reg.register(Box::new(YtSearchModule));
        reg.register(Box::new(YtDiscoverModule));
        reg.register(Box::new(YtRadioModule));
        reg.register(Box::new(YtExploreModule));
        reg.register(Box::new(YtChartsModule));
        reg.register(Box::new(PlaceholderModule));
        reg
    }

    /// Register a module. Replaces an existing module with the same id
    /// (so a third-party module can override a built-in).
    pub fn register(&mut self, module: Box<dyn PaneModule + Send + Sync>) {
        let id = module.id();
        if let Some(existing) = self.modules.iter_mut().find(|m| m.id() == id) {
            *existing = module;
        } else {
            self.modules.push(module);
        }
    }

    /// Look up a module by id.
    pub fn get(&self, id: ModuleId) -> Option<&(dyn PaneModule + Send + Sync)> {
        self.modules
            .iter()
            .find(|m| m.id() == id)
            .map(|m| m.as_ref())
    }

    /// All registered module ids, in registration order. Used by the
    /// module picker overlay.
    pub fn all_ids(&self) -> Vec<ModuleId> {
        self.modules.iter().map(|m| m.id()).collect()
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

// ---------------------------------------------------------------------------
// Global registry accessor
// ---------------------------------------------------------------------------

static REGISTRY: OnceLock<ModuleRegistry> = OnceLock::new();

/// Install a custom registry. Must be called before the first
/// [`registry`] call (which auto-initializes with builtins on first
/// access). Returns `true` if installed, `false` if a registry was
/// already installed (the custom one is dropped).
pub fn init_registry(reg: ModuleRegistry) -> bool {
    REGISTRY.set(reg).is_ok()
}

/// Access the global registry. Auto-initializes with built-in modules
/// on first call. Returns a `&'static` reference so it doesn't borrow
/// from `App` (which would conflict with the `&mut App` needed for
/// `PaneModule::render`).
pub fn registry() -> &'static ModuleRegistry {
    REGISTRY.get_or_init(ModuleRegistry::with_builtins)
}

// ---------------------------------------------------------------------------
// Built-in module adapters
// ---------------------------------------------------------------------------

/// Artists browse module. Wraps `view::columns::render_artists_pane`.
/// Sets `app.view = View::Artists` during render so the existing function
/// reads the right state.
pub struct ArtistsModule;

impl PaneModule for ArtistsModule {
    fn id(&self) -> ModuleId {
        ModuleId::Artists
    }
    fn title(&self) -> &'static str {
        "Artists"
    }
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
        let saved = app.view;
        app.view = crate::tui::app::View::Artists;
        crate::tui::view::columns::render_artists_pane(frame, area, app);
        app.view = saved;
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        // Built-in modules delegate key handling to the existing global
        // `input::handle_key` machinery; they don't intercept keys here.
        // Returning false lets the global handler run.
        false
    }
}

pub struct PlaylistsModule;

impl PaneModule for PlaylistsModule {
    fn id(&self) -> ModuleId {
        ModuleId::Playlists
    }
    fn title(&self) -> &'static str {
        "Playlists"
    }
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
        let saved = app.view;
        app.view = crate::tui::app::View::Playlists;
        crate::tui::view::columns::render_playlists_pane(frame, area, app);
        app.view = saved;
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        false
    }
}

pub struct QueueModule;

impl PaneModule for QueueModule {
    fn id(&self) -> ModuleId {
        ModuleId::Queue
    }
    fn title(&self) -> &'static str {
        "Queue"
    }
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
        let saved = app.view;
        app.view = crate::tui::app::View::Queue;
        crate::tui::view::columns::render_queue_pane(frame, area, app);
        app.view = saved;
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        false
    }
}

pub struct YoutubeModule;

impl PaneModule for YoutubeModule {
    fn id(&self) -> ModuleId {
        ModuleId::Youtube
    }
    fn title(&self) -> &'static str {
        "YouTube"
    }
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
        let saved = app.view;
        app.view = crate::tui::app::View::Youtube;
        crate::tui::view::yt_view::render_yt_view(frame, area, app);
        app.view = saved;
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        false
    }
}

/// "Now Playing" pane module: the big player bar as a pane. Renders the
/// full-size now-playing view (title, artist, album, quality, big
/// progress bar, transport, source badge, next-up preview) into a pane.
///
/// Uses [`crate::tui::view::player_bar_big::render_big`] when the pane
/// is tall enough (>= `BIG_BAR_HEIGHT` = 10 rows), otherwise falls back
/// to the side [`crate::tui::view::now_playing_panel::render`] which
/// fits a narrower/shorter pane. Unlike the browse modules, this one
/// doesn't swap `app.view` — the now-playing view is view-independent.
pub struct NowPlayingModule;

impl PaneModule for NowPlayingModule {
    fn id(&self) -> ModuleId {
        ModuleId::NowPlaying
    }
    fn title(&self) -> &'static str {
        "Now Playing"
    }
    fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
        use crate::tui::view::now_playing_panel as npp;
        use crate::tui::view::player_bar_big as pbb;

        // The big player bar is designed for a 10-row-tall area at the
        // bottom of the screen. In a pane, the pane's inner rect (after
        // the pane border) may be smaller. Use the big renderer when
        // the pane is tall enough; otherwise use the side panel
        // renderer (which is more compact and works at any size).
        if area.height >= pbb::BIG_BAR_HEIGHT {
            pbb::render_big(frame, area, app);
        } else {
            npp::render(frame, area, app);
        }
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// YouTube sub-tab modules (Phase 2.5)
// ---------------------------------------------------------------------------

/// Each YouTube sub-tab as its own pane module. Renders just the sub-tab
/// content (no sub-tab bar) via the matching `view::yt_view::render_yt_*`
/// function. Sets `app.view = View::Youtube` AND `app.yt_view.tab` to the
/// sub-tab during render so the existing yt input handlers route keys
/// correctly for the focused pane.
///
/// The `macro_rules!` boilerplate-eliminator defines a module struct + its
/// `PaneModule` impl from three pieces: the struct name, the `ModuleId`
/// variant, and the renderer function. Each renderer takes `&mut App` (or
/// `&App`, which we reborrow as `&*app`) — the macro handles both.
macro_rules! yt_subtab_module {
    ($struct_name:ident, $module_id:expr, $title:expr, $tab:expr, $render_fn:path) => {
        pub struct $struct_name;

        impl PaneModule for $struct_name {
            fn id(&self) -> ModuleId {
                $module_id
            }
            fn title(&self) -> &'static str {
                $title
            }
            fn render(&self, frame: &mut Frame, area: Rect, app: &mut App) {
                let saved_view = app.view;
                let saved_tab = app.yt_view.tab;
                app.view = crate::tui::app::View::Youtube;
                app.yt_view.tab = $tab;
                $render_fn(frame, area, app);
                app.view = saved_view;
                app.yt_view.tab = saved_tab;
            }
            fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
                false
            }
        }
    };
}

yt_subtab_module!(
    YtHomeModule,
    ModuleId::YtHome,
    "YT Home",
    crate::tui::app::YtTab::Home,
    crate::tui::view::yt_view::render_yt_home
);
yt_subtab_module!(
    YtLibraryModule,
    ModuleId::YtLibrary,
    "YT Library",
    crate::tui::app::YtTab::Library,
    crate::tui::view::yt_view::render_yt_library
);
yt_subtab_module!(
    YtSearchModule,
    ModuleId::YtSearch,
    "YT Search",
    crate::tui::app::YtTab::Search,
    crate::tui::view::yt_view::render_yt_search
);
yt_subtab_module!(
    YtDiscoverModule,
    ModuleId::YtDiscover,
    "YT Discover",
    crate::tui::app::YtTab::Discover,
    crate::tui::view::yt_view::render_yt_discover
);
yt_subtab_module!(
    YtRadioModule,
    ModuleId::YtRadio,
    "YT Radio",
    crate::tui::app::YtTab::Radio,
    crate::tui::view::yt_view::render_yt_radio
);
yt_subtab_module!(
    YtExploreModule,
    ModuleId::YtExplore,
    "YT Explore",
    crate::tui::app::YtTab::Explore,
    crate::tui::view::yt_view::render_yt_explore
);
yt_subtab_module!(
    YtChartsModule,
    ModuleId::YtCharts,
    "YT Charts",
    crate::tui::app::YtTab::Charts,
    crate::tui::view::yt_view::render_yt_charts
);

/// Demo / placeholder module. Rendered as a centered "Press `m` to choose
/// a module" hint. Used as the default for a fresh split (the user
/// hasn't picked a real module yet). Proves third-party modules can be
/// registered independently.
pub struct PlaceholderModule;

impl PaneModule for PlaceholderModule {
    fn id(&self) -> ModuleId {
        ModuleId::Placeholder
    }
    fn title(&self) -> &'static str {
        "Placeholder"
    }
    fn render(&self, frame: &mut Frame, area: Rect, _app: &mut App) {
        use ratatui::layout::Alignment;
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Paragraph};

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        if inner.width < 10 || inner.height < 3 {
            return;
        }
        let msg = Line::from(vec![
            Span::styled(
                "Press ",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(
                "m",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to choose a module for this pane",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
        ]);
        let para =
            Paragraph::new(vec![Line::from(""), msg, Line::from("")]).alignment(Alignment::Center);
        frame.render_widget(block, area);
        frame.render_widget(para, inner);
    }
    fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    /// Build a 1-track catalog + App so we can call `module.render` and
    /// `module.handle_key` with a real `&mut App`. The tempdir is leaked
    /// so the catalog's source paths stay valid for the test's lifetime.
    fn build_app() -> App {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("40mP")).unwrap();
        std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1",
               "album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        std::mem::forget(d);
        let cat = Catalog::load(&p).unwrap();
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// `with_builtins` registers all 5 built-in modules in the expected
    /// order. `all_ids` returns them in registration order.
    #[test]
    fn with_builtins_registers_all_modules() {
        let reg = ModuleRegistry::with_builtins();
        let ids = reg.all_ids();
        assert_eq!(
            ids,
            vec![
                ModuleId::Artists,
                ModuleId::Playlists,
                ModuleId::Queue,
                ModuleId::Youtube,
                ModuleId::NowPlaying,
                ModuleId::YtHome,
                ModuleId::YtLibrary,
                ModuleId::YtSearch,
                ModuleId::YtDiscover,
                ModuleId::YtRadio,
                ModuleId::YtExplore,
                ModuleId::YtCharts,
                ModuleId::Placeholder,
            ]
        );
    }

    /// `Default::default()` is `with_builtins()`.
    #[test]
    fn default_is_with_builtins() {
        let def = ModuleRegistry::default();
        assert_eq!(def.all_ids(), ModuleRegistry::with_builtins().all_ids());
    }

    /// `get` returns the module for each registered id.
    #[test]
    fn get_returns_each_module() {
        let reg = ModuleRegistry::with_builtins();
        for id in reg.all_ids() {
            assert!(reg.get(id).is_some(), "expected module for {id:?}");
        }
        assert!(reg.get(ModuleId::Placeholder).is_some());
    }

    /// `get` returns `None` for an unregistered id (defensive — currently
    /// all `ModuleId` variants are registered, but a future variant
    /// might not be).
    #[test]
    fn get_returns_none_for_unknown_id() {
        let reg = ModuleRegistry { modules: vec![] };
        assert!(reg.get(ModuleId::Artists).is_none());
        assert!(reg.all_ids().is_empty());
    }

    /// A custom (third-party) module can be registered.
    #[test]
    fn register_custom_module() {
        struct Custom;
        impl PaneModule for Custom {
            fn id(&self) -> ModuleId {
                ModuleId::Placeholder
            }
            fn title(&self) -> &'static str {
                "Custom"
            }
            fn render(&self, _f: &mut Frame, _area: Rect, _app: &mut App) {}
            fn handle_key(&self, _key: KeyEvent, _app: &mut App) -> bool {
                false
            }
        }
        let custom = Custom;
        // Cover the trait methods on the custom module.
        assert_eq!(custom.id(), ModuleId::Placeholder);
        assert_eq!(custom.title(), "Custom");
        let mut app = build_app();
        assert!(!custom.handle_key(key('x'), &mut app));
        let backend = TestBackend::new(40, 12);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 12);
        term.draw(|f| custom.render(f, area, &mut app)).unwrap();

        let mut reg = ModuleRegistry::with_builtins();
        reg.register(Box::new(Custom));
        // Replaced the built-in Placeholder — same id, new title.
        let m = reg.get(ModuleId::Placeholder).unwrap();
        assert_eq!(m.title(), "Custom");
    }

    /// `register` with a brand-new id pushes onto the list.
    #[test]
    fn register_new_module_pushes() {
        // Pre-condition: only one module.
        let mut reg = ModuleRegistry { modules: vec![] };
        reg.register(Box::new(ArtistsModule));
        assert_eq!(reg.all_ids(), vec![ModuleId::Artists]);
    }

    /// `init_registry` returns true on the first call, false on the
    /// second (the global OnceLock is already set). We isolate the test
    /// by running it in a separate process — but since tests run in the
    /// same process, we just verify the function signature works and
    /// returns a bool. (The global registry may already be initialized
    /// by other tests, so we don't assert on the value.)
    #[test]
    fn init_registry_returns_bool() {
        let reg = ModuleRegistry::with_builtins();
        let _ = init_registry(reg);
    }

    /// `registry()` returns a non-empty registry with built-ins.
    #[test]
    fn registry_has_builtins() {
        let r = registry();
        assert!(r.all_ids().contains(&ModuleId::Artists));
        assert!(r.all_ids().contains(&ModuleId::Queue));
    }

    // ---------------------------------------------------------------------------
    // Per-module trait method coverage
    // ---------------------------------------------------------------------------

    #[test]
    fn artists_module_id_title_handle_key() {
        let m = ArtistsModule;
        assert_eq!(m.id(), ModuleId::Artists);
        assert_eq!(m.title(), "Artists");
        let mut app = build_app();
        assert!(!m.handle_key(key('x'), &mut app));
    }

    #[test]
    fn artists_module_render_no_panic() {
        let m = ArtistsModule;
        let mut app = build_app();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
    }

    #[test]
    fn playlists_module_id_title_handle_key() {
        let m = PlaylistsModule;
        assert_eq!(m.id(), ModuleId::Playlists);
        assert_eq!(m.title(), "Playlists");
        let mut app = build_app();
        assert!(!m.handle_key(key('x'), &mut app));
    }

    #[test]
    fn playlists_module_render_no_panic() {
        let m = PlaylistsModule;
        let mut app = build_app();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
    }

    #[test]
    fn queue_module_id_title_handle_key() {
        let m = QueueModule;
        assert_eq!(m.id(), ModuleId::Queue);
        assert_eq!(m.title(), "Queue");
        let mut app = build_app();
        assert!(!m.handle_key(key('x'), &mut app));
    }

    #[test]
    fn queue_module_render_no_panic() {
        let m = QueueModule;
        let mut app = build_app();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
    }

    #[test]
    fn youtube_module_id_title_handle_key() {
        let m = YoutubeModule;
        assert_eq!(m.id(), ModuleId::Youtube);
        assert_eq!(m.title(), "YouTube");
        let mut app = build_app();
        assert!(!m.handle_key(key('x'), &mut app));
    }

    #[test]
    fn youtube_module_render_no_panic() {
        let m = YoutubeModule;
        let mut app = build_app();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
    }

    #[test]
    fn placeholder_module_id_title_handle_key() {
        let m = PlaceholderModule;
        assert_eq!(m.id(), ModuleId::Placeholder);
        assert_eq!(m.title(), "Placeholder");
        let mut app = build_app();
        assert!(!m.handle_key(key('x'), &mut app));
    }

    /// Placeholder module renders the "Press m" hint at a usable size.
    #[test]
    fn placeholder_module_render_normal_size() {
        let m = PlaceholderModule;
        let mut app = build_app();
        let backend = TestBackend::new(40, 12);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 12);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
        // The buffer should contain "Press" somewhere.
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..12 {
            for x in 0..40 {
                if buf[(x, y)].symbol() == "P" {
                    let mut word = String::new();
                    for dx in 0..5 {
                        if x + dx < 40 {
                            word.push_str(buf[(x + dx, y)].symbol());
                        }
                    }
                    if word.starts_with("Press") {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "placeholder should render 'Press' hint");
    }

    /// Placeholder module at tiny size (inner < 10 wide or < 3 tall)
    /// renders the block but NOT the hint (early return).
    #[test]
    fn placeholder_module_render_tiny_width() {
        let m = PlaceholderModule;
        let mut app = build_app();
        // 8 wide → inner width = 6 (after 1-cell borders) → < 10.
        let backend = TestBackend::new(8, 6);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 8, 6);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
        // No panic; the block renders but the "Press" hint doesn't.
        let buf = term.backend().buffer();
        let mut found_press = false;
        for y in 0..6 {
            for x in 0..8 {
                if buf[(x, y)].symbol() == "P" {
                    found_press = true;
                }
            }
        }
        assert!(!found_press, "should not render 'Press' hint at tiny width");
    }

    /// Placeholder module at tiny height (inner < 3 tall).
    #[test]
    fn placeholder_module_render_tiny_height() {
        let m = PlaceholderModule;
        let mut app = build_app();
        // 2 tall → inner height = 0 (after 2-cell borders) → < 3.
        let backend = TestBackend::new(20, 2);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 2);
        term.draw(|f| m.render(f, area, &mut app)).unwrap();
        // No panic.
    }
}
