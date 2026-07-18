//! Pane workspace rendering: borders, titles, edit-mode status line,
//! tiny-terminal fallback.
//!
//! The render layer reads `app.pane_workspace` (root tree + focused pane
//! + mode) and the global module registry (see
//! [`crate::tui::pane::registry`]).
//!
//! It never mutates them. Layout is recomputed from the tree on every
//! render via [`crate::tui::pane::layout::resolve_rects`].
//!
//! ## Painting order
//!
//! 1. Resolve rects: `let panes = resolve_rects(&root, area);`
//! 2. If a single pane in Normal mode, render the module directly into
//!    `area` (identical to the legacy per-view renderer; no border).
//! 3. Otherwise, for each resolved pane:
//!    - Skip panes whose `Rect` is below the minimum usable size
//!      (`.width < MIN_PANE_WIDTH || .height < MIN_PANE_HEIGHT`).
//!    - Draw a border Block (focused = accent + thick, unfocused = dim
//!      + plain). Title = the module's label. In PaneEdit mode the
//!      focused pane gets an "EDIT" badge.
//!    - Call the module's `render` with the inner rect.
//! 4. If in PaneEdit mode and the workspace is large enough, paint a
//!    one-line status overlay at the bottom with the edit-mode keymap.

// Clippy's `doc_lazy_continuation` lint flags continuation lines of
// multi-paragraph doc comments where the indentation doesn't match the
// list-item style it expects. The doc comments above use a slightly
// different style; the warnings are stylistic, not correctness issues.
#![allow(clippy::doc_lazy_continuation)]

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::pane::layout::{is_usable, resolve_rects, MIN_PANE_HEIGHT};
use crate::tui::pane::model::UiMode;
use crate::tui::pane::registry::registry;
use crate::tui::view::theme::{is_ascii, ASCII_BORDER_SET};

/// Minimum workspace size for the multi-pane layout. Below this we
/// render the focused pane only (with a "terminal too small for panes"
/// toast) so the user isn't stuck with unusable micro-panes.
const MIN_WORKSPACE_W: u16 = 40;
const MIN_WORKSPACE_H: u16 = 8;

/// The edit-mode status line height (1 row).
const STATUS_LINE_H: u16 = 1;

/// Render the pane workspace into `area`. `area` is the main content
/// region (after the rail / sidebar / now-playing panel have been split
/// off — see `view::columns::render`'s pane-mode branch).
///
/// Modules are looked up via the global registry (see
/// [`crate::tui::pane::registry::registry`]) so the registry borrow
/// doesn't conflict with the `&mut App` needed for `PaneModule::render`.
pub fn render_pane_workspace(f: &mut Frame, area: Rect, app: &mut App) {
    // Copy out the pane-workspace state we need for rendering. This
    // releases the immutable borrow of `app.pane_workspace` so we can
    // pass `&mut app` to `module.render` inside the loop. The workspace
    // isn't mutated during render (mutation is input's job), so a
    // snapshot is safe.
    let root_clone = app.pane_workspace.root.clone();
    let focused_pane = app.pane_workspace.focused_pane;
    let mode = app.pane_workspace.mode;
    let panes = resolve_rects(&root_clone, area);

    // Tiny workspace: render the focused pane only with a too-small toast.
    if area.width < MIN_WORKSPACE_W || area.height < MIN_WORKSPACE_H {
        render_tiny_workspace(f, area, app, &panes, focused_pane);
        return;
    }

    // Single pane + Normal mode → render without a border, identical to
    // the legacy per-view renderer. This keeps a fresh app looking
    // exactly like today.
    if panes.len() == 1 && mode == UiMode::Normal {
        render_single_pane(f, area, app, &panes[0]);
        return;
    }

    // Reserve space for the status line in PaneEdit mode.
    let content_area = if mode == UiMode::PaneEdit && area.height > STATUS_LINE_H + MIN_PANE_HEIGHT
    {
        Rect::new(area.x, area.y, area.width, area.height - STATUS_LINE_H)
    } else {
        area
    };
    let panes = resolve_rects(&root_clone, content_area);

    // Paint each pane. We look up the module via the global registry
    // (returns `&'static dyn PaneModule + Send + Sync`), then call its
    // render fn with `&mut app`. The static registry borrow doesn't
    // conflict with `&mut app`.
    for p in &panes {
        if !is_usable(p.rect) {
            continue;
        }
        let is_focused = p.pane_id == focused_pane;
        let block = pane_block(p.module_id, is_focused, mode);
        let inner = block.inner(p.rect);
        if !is_usable(inner) {
            // Inner too small after border — just paint the block.
            f.render_widget(block, p.rect);
            continue;
        }
        // Look up the module via the global registry. The reference is
        // `&'static` so the borrow doesn't conflict with `&mut app`.
        if let Some(module) = registry().get(p.module_id) {
            module.render(f, inner, app);
        }
        f.render_widget(block, p.rect);
    }

    // Status line.
    if mode == UiMode::PaneEdit && area.height > STATUS_LINE_H + MIN_PANE_HEIGHT {
        let status_area = Rect::new(
            area.x,
            area.bottom().saturating_sub(STATUS_LINE_H),
            area.width,
            STATUS_LINE_H,
        );
        render_edit_status_line(f, status_area);
    }
}

/// Render a single-pane workspace without a border. Identical to the
/// legacy per-view renderer (so a fresh app looks the same as today).
fn render_single_pane(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    pane: &crate::tui::pane::ResolvedPane,
) {
    if let Some(module) = registry().get(pane.module_id) {
        module.render(f, area, app);
    }
}

/// Render the focused pane only (with a too-small toast) when the
/// workspace is below `MIN_WORKSPACE_W`x`MIN_WORKSPACE_H`.
fn render_tiny_workspace(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    panes: &[crate::tui::pane::ResolvedPane],
    focused_pane: crate::tui::pane::PaneId,
) {
    let focused = panes.iter().find(|p| p.pane_id == focused_pane).copied();
    if let Some(p) = focused {
        if is_usable(p.rect) {
            if let Some(module) = registry().get(p.module_id) {
                module.render(f, p.rect, app);
            }
        }
    }
    // Toast at the bottom.
    if area.height >= 2 {
        let toast = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let msg = "terminal too small for panes — resize or press Esc";
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(Color::Yellow),
            )))
            .alignment(Alignment::Center),
            toast,
        );
    }
}

/// Build the border block for a pane. Focused panes get accent + thick
/// (Unicode) / accent + ASCII set (ASCII mode). Unfocused get dim +
/// plain. In PaneEdit mode the focused pane's title gets an "EDIT" badge.
fn pane_block(module: crate::tui::pane::ModuleId, focused: bool, mode: UiMode) -> Block<'static> {
    let theme = crate::tui::view::theme::Theme::default();
    let color = if focused { theme.accent } else { theme.dim };

    let title = if focused && mode == UiMode::PaneEdit {
        format!("{} [EDIT]", module.label())
    } else {
        module.label().to_string()
    };

    let mut block = if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(color))
    } else {
        let bt = if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        };
        Block::default()
            .borders(Borders::ALL)
            .border_type(bt)
            .border_style(Style::default().fg(color))
    };
    block = block.title(Span::styled(title, Style::default().fg(color)));
    block
}

/// Edit-mode status line: a one-line keymap hint at the bottom of the
/// workspace. Dim text so it doesn't compete with content. Kept short
/// enough to fit at 80 columns (the narrow path's minimum width).
fn render_edit_status_line(f: &mut Frame, area: Rect) {
    let theme = crate::tui::view::theme::Theme::default();
    let nc = crate::tui::view::theme::no_color();
    let accent = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });

    // Clear the line so the pane content above doesn't bleed through.
    f.render_widget(Clear, area);

    // Keep the status line concise: at 80 cols the content area is ~76
    // cells (after rail). The full keymap doesn't fit, so we show the
    // most important keys and let `?` (Help) carry the rest.
    let spans: Vec<Span> = vec![
        Span::styled("PANE EDIT", accent),
        Span::styled(" ", dim),
        Span::styled("hjkl", dim),
        Span::styled(" move ", dim),
        Span::styled("HJKL", dim),
        Span::styled(" resize ", dim),
        Span::styled("v/x/s", dim),
        Span::styled(" split ", dim),
        Span::styled("d", dim),
        Span::styled(" close ", dim),
        Span::styled("m", dim),
        Span::styled(" module ", dim),
        Span::styled("Tab", dim),
        Span::styled(" cycle ", dim),
        Span::styled("1-4", dim),
        Span::styled(" module ", dim),
        Span::styled("Esc", dim),
        Span::styled(" exit", dim),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}

/// True when the pane workspace should take over rendering from the
/// legacy per-view renderer. Convenience wrapper around
/// `PaneWorkspace::is_active` so `view::columns::render` doesn't have to
/// import the model.
pub fn is_pane_mode_active(app: &App) -> bool {
    app.pane_workspace.is_active()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use crate::tui::pane::model::{ModuleId, PaneId, Side};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
        // Leak the tempdir so the catalog's source paths stay valid for
        // the app's lifetime. The test process exits soon enough.
        std::mem::forget(d);
        let cat = Catalog::load(&p).unwrap();
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    /// A single-pane workspace in Normal mode renders without a border —
    /// identical to the legacy per-view renderer.
    #[test]
    fn single_pane_no_border() {
        let mut app = build_app();
        // Default workspace: single Artists pane, Normal mode.
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The top-left corner should NOT be a border character — the
        // module renders directly into the area. (If a border were
        // drawn, position (0,0) would be '┌' or '+'.)
        let cell = term.backend().buffer()[(0, 0)].symbol();
        assert!(
            !cell.starts_with('┌') && !cell.starts_with('+'),
            "single pane should not have a border, got '{cell}'"
        );
    }

    /// A two-pane workspace renders a border around each pane.
    #[test]
    fn two_panes_have_borders() {
        let mut app = build_app();
        // Split into two panes.
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // Top-left corner should be a border character.
        let cell = term.backend().buffer()[(0, 0)].symbol();
        assert!(
            cell.starts_with('┌') || cell.starts_with('+'),
            "two-pane workspace should have a border at (0,0), got '{cell}'"
        );
    }

    /// Edit mode adds an EDIT badge to the focused pane's title.
    #[test]
    fn edit_mode_badge() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        app.pane_workspace.enter_edit_mode();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The buffer should contain "EDIT" somewhere (in the focused
        // pane's title).
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..30 {
            for x in 0..100 {
                let s = buf[(x, y)].symbol();
                if s.contains('E') {
                    // Look for "EDIT" starting at this position.
                    let mut word = String::new();
                    for dx in 0..5 {
                        if x + dx < 100 {
                            word.push_str(buf[(x + dx, y)].symbol());
                        }
                    }
                    if word.contains("EDIT") {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "edit-mode badge 'EDIT' should be visible");
    }

    /// Tiny terminal: no panic, focused pane still renders.
    #[test]
    fn tiny_terminal_no_panic() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        let backend = TestBackend::new(20, 5);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 5);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic is the assertion.
    }

    /// `is_pane_mode_active` is false for a fresh app.
    #[test]
    fn is_pane_mode_active_false_for_fresh_app() {
        let app = build_app();
        assert!(!is_pane_mode_active(&app));
    }
}
