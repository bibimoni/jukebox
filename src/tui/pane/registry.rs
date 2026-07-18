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
