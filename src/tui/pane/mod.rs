//! Modular pane-editing system for the Ratatui TUI.
//!
//! Splits the main content area into a recursive tree of rectangular panes,
//! each assigned a registered module. Layout is stored as normalized split
//! ratios (never terminal cells), so terminal resize requires no layout
//! mutation — `Rect`s are recomputed from the tree on every render.
//!
//! ## Layout
//!
//! - [`model`] — data model + pure mutation (split / close / resize / clamp)
//! - [`layout`] — `resolve_rects` walks the tree and computes a `Rect` per leaf
//! - [`focus`] — spatial (directional) focus movement + cycling
//! - [`registry`] — `PaneModule` trait + built-in module adapters
//! - [`input`] — pane-edit-mode key dispatch + prefix-key routing
//! - [`render`] — pane borders, titles, edit-mode status line, tiny fallback
//! - [`persistence`] — serializable DTOs for `state.rs`
//!
//! ## Modes
//!
//! - [`UiMode::Normal`] — input goes to the focused pane's module; existing
//!   global keybindings keep working. Pane navigation is available via the
//!   `Ctrl+w` prefix.
//! - [`UiMode::PaneEdit`] — pane-edit keybindings active (split / close /
//!   resize / change-module). Module-local navigation is suppressed. Global
//!   playback + quit keys still work.
//! - [`UiMode::PaneModulePicker`] — module-picker overlay open. Treated as a
//!   sub-mode of PaneEdit for focus purposes; the overlay takes input.

pub mod focus;
pub mod input;
pub mod layout;
pub mod model;
pub mod persistence;
pub mod registry;
pub mod render;
pub mod selection;

pub use layout::ResolvedPane;
pub use model::{
    ChildPos, Direction, ModuleId, PaneId, PaneNode, PaneWorkspace, ResizeError, Side, SplitAxis,
    SplitError, UiMode, MAX_RATIO, MIN_RATIO, RESIZE_STEP,
};
pub use registry::{init_registry, registry, ModuleRegistry, PaneModule};
pub use render::render_pane_workspace;
pub use selection::{
    NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase, MIN_SELECTION_HEIGHT,
    MIN_SELECTION_WIDTH,
};
